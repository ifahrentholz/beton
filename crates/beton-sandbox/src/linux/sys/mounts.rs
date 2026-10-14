//! Mounts im eigenen Mount-Namespace (Backend `landlock`, SBX-004, SBX-008): frisches `/proc`
//! für den PID-Namespace, eigene PTYs, private tmpfs für `/tmp`, `/run` und `/dev/shm`, Masken
//! und read-only Git-Hooks. Die Reihenfolge kommt aus [`crate::linux::plan::mount_ops`].

use std::collections::BTreeMap;
use std::io;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

use super::util::{StepError, StepResult, close, cstr, cvt};
use crate::linux::plan::MountOp;

fn mount(
    src: Option<&Path>,
    target: &Path,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let src = src.map(cstr).transpose()?;
    let target = cstr(target)?;
    let fstype = fstype.map(|f| std::ffi::CString::new(f).unwrap_or_default());
    let data = data.map(|d| std::ffi::CString::new(d).unwrap_or_default());
    // SAFETY: Alle Zeiger zeigen auf gültige C-Strings oder sind NULL.
    cvt(unsafe {
        libc::mount(
            src.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            target.as_ptr(),
            fstype.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            flags,
            data.as_ref()
                .map_or(std::ptr::null(), |s| s.as_ptr().cast()),
        )
    })?;
    Ok(())
}

/// Bindet `path` read-only neu ein und behält die gesperrten Flags des Mounts (im
/// User-Namespace dürfen `nosuid`, `nodev`, `noexec` nicht entfernt werden).
fn remount_ro(path: &Path) -> io::Result<()> {
    remount(path, true)
}

fn remount(path: &Path, read_only: bool) -> io::Result<()> {
    let c = cstr(path)?;
    // SAFETY: statvfs füllt eine eigene Struktur.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    cvt(unsafe { libc::statvfs(c.as_ptr(), &mut st) })?;
    let mut flags = libc::MS_REMOUNT | libc::MS_BIND;
    if read_only {
        flags |= libc::MS_RDONLY;
    }
    for (st_flag, ms) in [
        (libc::ST_NOSUID, libc::MS_NOSUID),
        (libc::ST_NODEV, libc::MS_NODEV),
        (libc::ST_NOEXEC, libc::MS_NOEXEC),
        (libc::ST_NOATIME, libc::MS_NOATIME),
        (libc::ST_NODIRATIME, libc::MS_NODIRATIME),
        (libc::ST_RELATIME, libc::MS_RELATIME),
    ] {
        if st.f_flag & st_flag != 0 {
            flags |= ms;
        }
    }
    mount(None, path, None, flags, None)
}

fn open_path(p: &Path) -> io::Result<RawFd> {
    let c = cstr(p)?;
    // SAFETY: open mit O_PATH auf einen gültigen C-String.
    cvt(unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) })
}

fn fd_is_dir(fd: RawFd) -> bool {
    // SAFETY: fstat auf einen gültigen Deskriptor in eine eigene Struktur.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(fd, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
    }
}

fn fd_path(fd: RawFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{fd}"))
}

/// Legt einen Platzhalter (Verzeichnis bzw. leere Datei) für einen Mountpunkt an.
fn placeholder(p: &Path, dir: bool) -> io::Result<()> {
    if p.exists() {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dir {
        std::fs::create_dir(p)
    } else {
        std::fs::File::create(p).map(|_| ())
    }
}

/// Grundeinrichtung: Mounts privat, frisches `/proc`, eigene PTY-Instanz.
pub fn base() -> StepResult<()> {
    mount(
        None,
        Path::new("/"),
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
    )
    .map_err(|e| StepError::io("mount", "/ privat", e))?;
    mount(
        Some(Path::new("proc")),
        Path::new("/proc"),
        Some("proc"),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )
    .map_err(|e| StepError::io("mount", "/proc (PID-Namespace)", e))?;
    let pts = Path::new("/dev/pts");
    let devpts = mount(
        Some(Path::new("devpts")),
        pts,
        Some("devpts"),
        libc::MS_NOSUID | libc::MS_NOEXEC,
        Some("newinstance,ptmxmode=0666,mode=0620"),
    );
    match devpts {
        Ok(()) => {
            mount(
                Some(Path::new("/dev/pts/ptmx")),
                Path::new("/dev/ptmx"),
                None,
                libc::MS_BIND,
                None,
            )
            .map_err(|e| StepError::io("mount", "/dev/ptmx", e))?;
        }
        Err(_) => {
            // Ohne eigene Instanz keine PTYs, aber nie die des Hosts.
            mount(
                Some(Path::new("tmpfs")),
                pts,
                Some("tmpfs"),
                libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | libc::MS_RDONLY,
                Some("mode=000,size=4k"),
            )
            .map_err(|e| StepError::io("mount", "/dev/pts", e))?;
        }
    }
    Ok(())
}

/// Leere Datei mit Modus 0000 in einem kleinen tmpfs als Quelle für Datei-Masken. Das tmpfs
/// liegt auf `/run` und wird danach vom privaten `/run` überdeckt; nach den Masken wird es
/// gelöst. Bind-Mounts verlangen eine Quelle im eigenen Mount-Namespace, deshalb erst danach.
/// Liefert (Datei, Wurzel des tmpfs) als O_PATH-Deskriptoren.
fn mask_source() -> io::Result<(RawFd, RawFd)> {
    let dir = Path::new("/run");
    mount(
        Some(Path::new("tmpfs")),
        dir,
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        Some("mode=000,size=4k,nr_inodes=4"),
    )?;
    let root = open_path(dir)?;
    let file = dir.join("beton-mask");
    let c = cstr(&file)?;
    // SAFETY: open/close auf einen gültigen C-String.
    let fd = cvt(unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC,
            0o000,
        )
    })?;
    close(fd);
    Ok((open_path(&file)?, root))
}

/// Löst das Quell-tmpfs der Datei-Masken (überdeckt, deshalb über seinen Deskriptor).
fn drop_mask_source(root: RawFd) {
    if let Ok(p) = cstr(&fd_path(root)) {
        // SAFETY: umount2 eines eigenen Mounts über den Pfad seines Deskriptors.
        unsafe {
            libc::umount2(p.as_ptr(), libc::MNT_DETACH);
        }
    }
    close(root);
}

/// Führt die Mount-Schritte aus.
pub fn apply(ops: &[MountOp]) -> StepResult<()> {
    // Deskriptoren der wieder einzubindenden Wurzeln, bevor irgendetwas sie verdeckt.
    let mut saved: BTreeMap<PathBuf, RawFd> = BTreeMap::new();
    for op in ops {
        if let MountOp::Rebind { path, .. } = op {
            let fd = open_path(path).map_err(|e| StepError::io("mount", path.display(), e))?;
            saved.insert(path.clone(), fd);
        }
    }
    let needs_file = ops.iter().any(|o| matches!(o, MountOp::MaskFile { .. }));
    let (mask_fd, mask_root) = if needs_file {
        let (f, r) = mask_source().map_err(|e| StepError::io("mount", "Maskendatei", e))?;
        (Some(f), Some(r))
    } else {
        (None, None)
    };
    let mut mask_dirs = Vec::new();
    for op in ops {
        let path = op.path();
        let fail = |e: io::Error| StepError::io("mount", format!("{op:?}"), e);
        match op {
            MountOp::Tmpfs { mode, .. } => {
                if !path.is_dir() {
                    continue;
                }
                mount(
                    Some(Path::new("tmpfs")),
                    path,
                    Some("tmpfs"),
                    libc::MS_NOSUID | libc::MS_NODEV,
                    Some(&format!("mode={mode:o}")),
                )
                .map_err(fail)?;
            }
            MountOp::Anchor { .. } => {
                std::fs::create_dir_all(path).map_err(fail)?;
                mount(
                    Some(Path::new("tmpfs")),
                    path,
                    Some("tmpfs"),
                    libc::MS_NOSUID | libc::MS_NODEV,
                    Some("mode=755,size=64k"),
                )
                .map_err(fail)?;
                mask_dirs.push(path.to_path_buf());
            }
            MountOp::MaskDir { .. } => {
                // Erst beschreibbar (für Wiedereinbindungen darunter), am Ende read-only.
                mount(
                    Some(Path::new("tmpfs")),
                    path,
                    Some("tmpfs"),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                    Some("mode=000,size=64k"),
                )
                .map_err(fail)?;
                mask_dirs.push(path.to_path_buf());
            }
            MountOp::MaskFile { .. } => {
                let fd = mask_fd.ok_or_else(|| StepError::new("mount", "keine Maskendatei"))?;
                mount(Some(&fd_path(fd)), path, None, libc::MS_BIND, None).map_err(fail)?;
                remount_ro(path).map_err(fail)?;
            }
            MountOp::Rebind { write, .. } => {
                let fd = saved[path];
                placeholder(path, fd_is_dir(fd)).map_err(fail)?;
                mount(
                    Some(&fd_path(fd)),
                    path,
                    None,
                    libc::MS_BIND | libc::MS_REC,
                    None,
                )
                .map_err(fail)?;
                if !write {
                    remount_ro(path).map_err(fail)?;
                }
            }
            MountOp::ReadOnly { .. } => {
                mount(Some(path), path, None, libc::MS_BIND | libc::MS_REC, None).map_err(fail)?;
                remount_ro(path).map_err(fail)?;
            }
            MountOp::Pin { writable, .. } => {
                mount(Some(path), path, None, libc::MS_BIND | libc::MS_REC, None).map_err(fail)?;
                if *writable {
                    // Unter einem read-only gebundenen Vorfahren wieder beschreibbar.
                    remount(path, false).map_err(fail)?;
                }
            }
        }
    }
    for d in &mask_dirs {
        remount_ro(d).map_err(|e| StepError::io("mount", d.display(), e))?;
    }
    for fd in saved.values() {
        close(*fd);
    }
    if let Some(fd) = mask_fd {
        close(fd);
    }
    if let Some(root) = mask_root {
        drop_mask_source(root);
    }
    Ok(())
}
