//! Wächter gegen neue `commondir`/`gitdir` im Git-Dir (SBX-003, #209).
//!
//! Eine fehlende Datei lässt sich nicht per Mount sperren, und Landlock unterscheidet keine
//! Dateinamen; `git` muss im Git-Dir aber beliebige Lock-Dateien anlegen. Der Launcher (L,
//! außerhalb der Sandbox) beobachtet deshalb die Git-Verzeichnisse per inotify. Entsteht dort
//! eine Steuerdatei, die beim Start fehlte, entfernt er sie sofort, beendet die Sandbox, räumt
//! nach deren Ende ein zweites Mal auf und meldet einen `Fs`-Verstoß. Ereignisse werden nicht ausgewertet, nur als
//! Anlass genommen, nachzusehen; ein Überlauf der Ereignis-Warteschlange schadet daher nicht.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

use super::util::{close, cstr, cvt};
use crate::git::CONTROL_FILES;

/// Überwachte Steuerdateien eines Starts.
pub struct GitWatch {
    fd: RawFd,
    /// Steuerdateien, die beim Start fehlten und nicht entstehen dürfen.
    guarded: Vec<PathBuf>,
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

impl GitWatch {
    /// Beginnt die Überwachung von `dirs` (vor dem Start des Ziels). `None`, wenn es nichts zu
    /// überwachen gibt. Fehler → der Start scheitert (fail closed).
    pub fn new(dirs: &[PathBuf]) -> std::io::Result<Option<Self>> {
        let guarded: Vec<PathBuf> = dirs
            .iter()
            .flat_map(|d| CONTROL_FILES.iter().map(move |n| d.join(n)))
            .filter(|p| !exists(p))
            .collect();
        if guarded.is_empty() {
            return Ok(None);
        }
        // SAFETY: inotify_init1 ohne Zeiger.
        let fd = cvt(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) })?;
        let watch = Self { fd, guarded };
        for d in dirs {
            if !watch
                .guarded
                .iter()
                .any(|p| p.parent() == Some(d.as_path()))
            {
                continue;
            }
            let c = cstr(d)?;
            // SAFETY: gültiger Deskriptor und C-String.
            cvt(unsafe {
                libc::inotify_add_watch(
                    fd,
                    c.as_ptr(),
                    libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_ONLYDIR,
                )
            })?;
        }
        Ok(Some(watch))
    }

    /// Deskriptor für `poll`.
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Verwirft anstehende Ereignisse und liefert die überwachten Steuerdateien, die jetzt
    /// existieren.
    pub fn check(&self) -> Vec<PathBuf> {
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: read in einen eigenen Puffer passender Länge.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
        }
        self.guarded.iter().filter(|p| exists(p)).cloned().collect()
    }

    /// Entfernt entstandene Steuerdateien und liefert ihre Pfade.
    pub fn sweep(&self) -> Vec<PathBuf> {
        let found = self.check();
        for p in &found {
            // Folgt keinen Symlinks: ein Symlink wird selbst entfernt.
            let _ = match std::fs::symlink_metadata(p) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
                Ok(_) => std::fs::remove_file(p),
                Err(_) => Ok(()),
            };
        }
        found
    }
}

impl Drop for GitWatch {
    fn drop(&mut self) {
        close(self.fd);
    }
}
