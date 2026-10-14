//! Ressourcenlimits über cgroup v2 (SBX-013): nur mit Delegation, etwa durch systemd
//! (`user@<uid>.service` bzw. eine Unit mit `Delegate=yes`) oder einen Administrator.
//!
//! Gesucht wird der nächste Vorfahre der eigenen cgroup, in dem der Benutzer cgroups anlegen
//! und Prozesse verschieben darf und in dem `memory` und `pids` als Controller aktiv sind. Pro
//! Sandbox entsteht darunter eine eigene cgroup; der Launcher verschiebt den Prozessbaum
//! hinein, liest `pids.events`/`memory.events` für Verstoßmeldungen und räumt sie am Ende ab.
//! Ohne Delegation bleibt es bei rlimits; die Probe meldet dann `resources: degraded`.

use std::path::{Path, PathBuf};

/// Wurzel der cgroup-v2-Hierarchie.
pub const ROOT: &str = "/sys/fs/cgroup";

fn writable(p: &Path) -> bool {
    let Ok(c) = super::util::cstr(p) else {
        return false;
    };
    // SAFETY: access(2) auf einen gültigen C-String.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Eigene cgroup (Pfad relativ zur Wurzel, z. B. `/user.slice/…/app.slice/x.scope`).
fn own() -> Option<PathBuf> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| PathBuf::from(p.trim()))
}

/// Delegierte Basis-cgroup oder `None`.
pub fn discover() -> Option<PathBuf> {
    let root = Path::new(ROOT);
    if !root.join("cgroup.controllers").is_file() {
        return None;
    }
    let own = own()?;
    let rel = own.strip_prefix("/").ok()?;
    let mut candidate = root.join(rel);
    while candidate.pop() {
        if candidate == root || !candidate.starts_with(root) {
            break;
        }
        let controllers =
            std::fs::read_to_string(candidate.join("cgroup.subtree_control")).unwrap_or_default();
        let has = |c: &str| controllers.split_whitespace().any(|x| x == c);
        if has("memory")
            && has("pids")
            && writable(&candidate)
            && writable(&candidate.join("cgroup.procs"))
        {
            return Some(candidate);
        }
    }
    None
}

/// Die cgroup einer Sandbox.
#[derive(Debug)]
pub struct Cgroup {
    pub path: PathBuf,
    seen_pids: u64,
    seen_oom: u64,
}

fn counter(file: &Path, key: &str) -> u64 {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next() == Some(key))
                .then(|| it.next().and_then(|v| v.parse().ok()))
                .flatten()
        })
        .unwrap_or(0)
}

impl Cgroup {
    /// Legt die cgroup an und setzt die Limits.
    pub fn create(base: &Path, memory_mb: Option<u64>, pids: Option<u32>) -> std::io::Result<Self> {
        let name = format!("beton-sbx-{}", std::process::id());
        let path = base.join(name);
        std::fs::create_dir(&path)?;
        let cg = Self {
            path,
            seen_pids: 0,
            seen_oom: 0,
        };
        if let Some(p) = pids {
            std::fs::write(cg.path.join("pids.max"), p.to_string())?;
        }
        if let Some(m) = memory_mb {
            std::fs::write(
                cg.path.join("memory.max"),
                m.saturating_mul(1024 * 1024).to_string(),
            )?;
            let swap = cg.path.join("memory.swap.max");
            if swap.exists() {
                std::fs::write(swap, "0")?;
            }
        }
        Ok(cg)
    }

    /// Verschiebt einen Prozess hinein.
    pub fn add(&self, pid: libc::pid_t) -> std::io::Result<()> {
        std::fs::write(self.path.join("cgroup.procs"), pid.to_string())
    }

    /// Neu erreichte Limits seit dem letzten Aufruf (`pids`, `memory`).
    pub fn new_events(&mut self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let pids = counter(&self.path.join("pids.events"), "max");
        if pids > self.seen_pids {
            self.seen_pids = pids;
            out.push("pids");
        }
        let oom = counter(&self.path.join("memory.events"), "oom_kill");
        if oom > self.seen_oom {
            self.seen_oom = oom;
            out.push("memory");
        }
        out
    }

    /// Beendet alles darin und entfernt die cgroup.
    pub fn remove(&self) {
        let _ = std::fs::write(self.path.join("cgroup.kill"), "1");
        for _ in 0..50 {
            if std::fs::remove_dir(&self.path).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sbx_013_cgroup_files_and_events() {
        // Gleiche Dateien wie in cgroupfs, hier in einem normalen Verzeichnis.
        let tmp = tempfile::tempdir().unwrap();
        let mut cg = Cgroup::create(tmp.path(), Some(64), Some(32)).unwrap();
        assert_eq!(
            std::fs::read_to_string(cg.path.join("pids.max")).unwrap(),
            "32"
        );
        assert_eq!(
            std::fs::read_to_string(cg.path.join("memory.max")).unwrap(),
            (64 * 1024 * 1024).to_string()
        );
        cg.add(4711).unwrap();
        assert!(cg.new_events().is_empty());
        std::fs::write(cg.path.join("pids.events"), "max 3\n").unwrap();
        std::fs::write(cg.path.join("memory.events"), "low 0\noom_kill 1\n").unwrap();
        assert_eq!(cg.new_events(), vec!["pids", "memory"]);
        assert!(cg.new_events().is_empty(), "nur neue Ereignisse");
        assert_eq!(counter(&cg.path.join("fehlt"), "max"), 0);
        // Ohne Delegation (CI im User-Namespace) oder mit: discover liefert einen Pfad unter
        // der cgroup-Wurzel oder nichts.
        if let Some(base) = discover() {
            assert!(base.starts_with(ROOT));
        }
    }
}
