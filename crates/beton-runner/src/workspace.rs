//! Beobachtung des Workspace (SES-017 AC3, SES-018): Während eines Turns vergleicht der Runner
//! den Workspace laufend mit dem letzten Snapshot und meldet Änderungen als `fs.changed`; vor und
//! nach jedem Turn hält er den Stand als Baum im Schatten-Repository fest
//! (`refs/beton/turns/<turn>/{before,after}`), woraus der Server die Turn-Sicht berechnet.
//!
//! Snapshots laufen über die `git`-CLI und blockieren; Aufrufer nutzen `spawn_blocking`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beton_core::event::{FsChange, FsChangeKind};
use beton_git::snapshot::turn_ref;
use beton_git::{ChangeStatus, ChangedFile, GitError, ShadowRepo};

/// `source` von `fs.changed` aus der Beobachtung des Runners.
pub const FS_SOURCE_WATCHER: &str = "watcher";

/// Kürzester Abstand zweier Snapshots während eines Turns.
pub const POLL_MIN: Duration = Duration::from_millis(250);

/// Snapshot-Zustand einer Session.
#[derive(Debug)]
pub struct Tracker {
    shadow: ShadowRepo,
    /// Letzter Snapshot (Baum); Vergleichsbasis für den nächsten.
    last: Option<String>,
}

/// Geteilter Tracker für Runner-Schleife und Beobachter.
pub type Shared = Arc<Mutex<Tracker>>;

/// Mehr Dateien (ohne ignorierte) beobachtet der Runner nicht: jeder Snapshot liest den ganzen
/// Baum, und ein Workspace wie `$HOME` würde vollständig ins Schatten-Repository kopiert.
pub const MAX_FILES: usize = 100_000;

/// Warum es keine Beobachtung gibt.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("Workspace hat mehr als {0} Dateien")]
    TooLarge(usize),
    #[error("{0}")]
    Git(#[from] GitError),
}

impl Tracker {
    pub fn open(git_dir: &Path, work_tree: &Path) -> Result<Self, OpenError> {
        Self::open_with_limit(git_dir, work_tree, MAX_FILES)
    }

    pub fn open_with_limit(
        git_dir: &Path,
        work_tree: &Path,
        max_files: usize,
    ) -> Result<Self, OpenError> {
        let shadow = ShadowRepo::open(git_dir, work_tree)?;
        if shadow.exceeds_files(max_files)? {
            return Err(OpenError::TooLarge(max_files));
        }
        Ok(Self { shadow, last: None })
    }

    /// Neuer Snapshot; liefert die Änderungen seit dem letzten und den neuen Baum.
    pub fn poll(&mut self) -> Result<(Vec<FsChange>, String), GitError> {
        let tree = self.shadow.snapshot()?;
        let changes = match &self.last {
            Some(last) if *last != tree => self.shadow.changes(last, &tree)?,
            _ => Vec::new(),
        };
        self.last = Some(tree.clone());
        Ok((changes.into_iter().map(fs_change).collect(), tree))
    }

    /// Hält einen Baum als Stand vor bzw. nach einem Turn fest.
    pub fn mark(&self, turn: &str, side: &str, tree: &str) -> Result<(), GitError> {
        self.shadow.set_ref(&turn_ref(turn, side), tree)
    }
}

/// Abbildung einer Git-Änderung auf `fs.changed` (PROTO-002).
pub fn fs_change(f: ChangedFile) -> FsChange {
    let change = match f.status {
        ChangeStatus::Added => FsChangeKind::Added,
        ChangeStatus::Modified => FsChangeKind::Modified,
        ChangeStatus::Deleted => FsChangeKind::Deleted,
        ChangeStatus::Renamed => FsChangeKind::Renamed,
    };
    FsChange {
        path: f.path,
        change,
        from: f.old_path,
    }
}

/// Führt `poll` blockierend aus; Fehler werden geloggt und als „keine Änderung“ behandelt,
/// damit eine kaputte Beobachtung den Turn nicht stört.
pub async fn poll(tracker: &Shared) -> Option<(Vec<FsChange>, String)> {
    let t = tracker.clone();
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(move || match t.lock() {
        Ok(mut guard) => guard.poll().map_err(|e| e.to_string()),
        Err(_) => Err("Tracker gesperrt".to_owned()),
    })
    .await;
    match result {
        Ok(Ok(r)) => {
            tracing::trace!(
                elapsed_ms = started.elapsed().as_millis(),
                "Workspace-Snapshot"
            );
            Some(r)
        }
        Ok(Err(e)) => {
            tracing::warn!("Workspace-Snapshot fehlgeschlagen: {e}");
            None
        }
        Err(e) => {
            tracing::warn!("Workspace-Snapshot abgebrochen: {e}");
            None
        }
    }
}

/// Setzt eine Turn-Ref blockierend; Fehler nur ins Log.
pub async fn mark(tracker: &Shared, turn: String, side: &'static str, tree: String) {
    let t = tracker.clone();
    let result = tokio::task::spawn_blocking(move || {
        t.lock()
            .map_err(|_| "Tracker gesperrt".to_owned())
            .and_then(|g| g.mark(&turn, side, &tree).map_err(|e| e.to_string()))
    })
    .await;
    if let Ok(Err(e)) | Err(e) = result.map_err(|e| e.to_string()) {
        tracing::warn!("Turn-Snapshot nicht gespeichert: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ses_017_poll_reports_changes_since_last_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "a\n").unwrap();
        let mut t = Tracker::open(&tmp.path().join("s.git"), &ws).unwrap();
        let (first, before) = t.poll().unwrap();
        assert!(first.is_empty(), "erster Snapshot ist die Basis");
        std::fs::write(ws.join("a.txt"), "b\n").unwrap();
        std::fs::write(ws.join("neu.txt"), "n\n").unwrap();
        let (changes, after) = t.poll().unwrap();
        assert_eq!(
            changes,
            vec![
                FsChange {
                    path: "a.txt".into(),
                    change: FsChangeKind::Modified,
                    from: None
                },
                FsChange {
                    path: "neu.txt".into(),
                    change: FsChangeKind::Added,
                    from: None
                },
            ]
        );
        assert!(t.poll().unwrap().0.is_empty(), "nichts Neues");
        t.mark("trn_x", "before", &before).unwrap();
        t.mark("trn_x", "after", &after).unwrap();
    }

    #[test]
    fn too_large_workspaces_are_not_observed() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        for i in 0..3 {
            std::fs::write(ws.join(format!("{i}.txt")), "x").unwrap();
        }
        let err = Tracker::open_with_limit(&tmp.path().join("s.git"), &ws, 2).unwrap_err();
        assert!(matches!(err, OpenError::TooLarge(2)), "{err}");
        assert!(Tracker::open_with_limit(&tmp.path().join("s.git"), &ws, 3).is_ok());
    }
}
