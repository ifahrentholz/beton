//! Turn-Snapshots in einem Schatten-Repository (SES-017 AC3, SES-018).
//!
//! Für jede Session hält beton ein eigenes Git-Verzeichnis außerhalb des Workspace, dessen
//! Worktree der Workspace ist. Ein Snapshot ist `git add -A` in dessen Index plus
//! `git write-tree`: ein inhaltsadressierter Baum des Workspace, der `.gitignore` respektiert,
//! auch in Verzeichnissen ohne Git (dort sind es die „Blob-Snapshots vor erster Änderung“). Das
//! Repository des Nutzers bleibt unberührt: keine Objekte, keine Refs, kein Index.
//!
//! Refs `refs/beton/turns/<turn_id>/{before,after}` halten die Bäume vor und nach jedem Turn.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::changes::{self, ChangedFile, FileDiff};
use crate::cmd::{Git, GitError};

/// Schatten-Repository einer Session.
#[derive(Debug, Clone)]
pub struct ShadowRepo {
    git_dir: PathBuf,
    work_tree: PathBuf,
}

/// Ref des Baums vor (`before`) bzw. nach (`after`) einem Turn.
pub fn turn_ref(turn: &str, side: &str) -> String {
    format!("refs/beton/turns/{turn}/{side}")
}

impl ShadowRepo {
    /// Öffnet das Schatten-Repository und legt es bei Bedarf an.
    pub fn open(git_dir: &Path, work_tree: &Path) -> Result<Self, GitError> {
        let repo = Self {
            git_dir: git_dir.to_path_buf(),
            work_tree: work_tree.to_path_buf(),
        };
        if !git_dir.join("HEAD").is_file() {
            if let Some(parent) = git_dir.parent() {
                std::fs::create_dir_all(parent).map_err(|source| GitError::Io {
                    args: "init".into(),
                    source,
                })?;
            }
            let dir = git_dir.to_string_lossy().into_owned();
            Git::new(work_tree).ok(&["init", "--quiet", "--bare", &dir])?;
            let git = Git::new(git_dir);
            for (key, value) in [
                ("core.bare", "false"),
                // Inhalte unverändert übernehmen, unabhängig von globalen Einstellungen.
                ("core.autocrlf", "false"),
                ("core.safecrlf", "false"),
                ("gc.auto", "0"),
                ("advice.addEmbeddedRepo", "false"),
            ] {
                git.ok(&["config", key, value])?;
            }
        }
        repo.write_excludes().map_err(|source| GitError::Io {
            args: "info/exclude".into(),
            source,
        })?;
        Ok(repo)
    }

    /// Schließt das Schatten-Repository selbst und das Datenverzeichnis von beton
    /// (`<data_dir>/snapshots/<session>.git`) aus, falls sie im Workspace liegen – sonst
    /// sähe jeder Snapshot die Objekte des vorigen.
    fn write_excludes(&self) -> std::io::Result<()> {
        let work = self.work_tree.canonicalize()?;
        let git_dir = self.git_dir.canonicalize()?;
        let mut lines = vec![
            "# Von beton erzeugt: Schatten-Repository und Datenverzeichnis nicht snapshotten."
                .to_owned(),
        ];
        let candidates = [
            Some(git_dir.as_path()),
            git_dir.parent(),
            git_dir.parent().and_then(Path::parent),
        ];
        for dir in candidates.into_iter().flatten() {
            if dir != work
                && let Ok(rel) = dir.strip_prefix(&work)
            {
                let rel = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                lines.push(format!("/{rel}/"));
            }
        }
        let info = git_dir.join("info");
        std::fs::create_dir_all(&info)?;
        std::fs::write(info.join("exclude"), lines.join("\n") + "\n")
    }

    /// Hat der Workspace (ohne ignorierte Dateien) mehr als `limit` Dateien? Bricht beim
    /// Erreichen der Grenze ab, ohne den ganzen Baum zu lesen.
    pub fn exceeds_files(&self, limit: usize) -> Result<bool, GitError> {
        let n = self.git().count_records(
            &[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
            limit + 1,
        )?;
        Ok(n > limit)
    }

    /// Öffnet ein vorhandenes Schatten-Repository, ohne es anzulegen.
    pub fn existing(git_dir: &Path, work_tree: &Path) -> Option<Self> {
        git_dir.join("HEAD").is_file().then(|| Self {
            git_dir: git_dir.to_path_buf(),
            work_tree: work_tree.to_path_buf(),
        })
    }

    pub fn git(&self) -> Git {
        Git::new(&self.work_tree)
            .with_git_dir(&self.git_dir, &self.work_tree)
            .with_config("core.quotePath=false")
    }

    fn write_tree(&self, git: &Git) -> Result<String, GitError> {
        // `--ignore-errors`: eine unlesbare Datei verhindert nicht den ganzen Snapshot.
        git.ok(&["add", "--all", "--ignore-errors", "--", "."])
            .or_else(|e| match e {
                // Fehler einzelner Dateien: der Index enthält den Rest, `write-tree` prüft ihn.
                GitError::Failed { .. } => Ok(Vec::new()),
                other => Err(other),
            })?;
        git.text(&["write-tree"])
    }

    /// Snapshot über den eigenen Index (schnell dank Stat-Cache). Nur ein Schreiber darf diesen
    /// Index gleichzeitig nutzen (der Runner der Session).
    pub fn snapshot(&self) -> Result<String, GitError> {
        self.write_tree(&self.git())
    }

    /// Snapshot über eine Kopie des Index; berührt den Index des Runners nicht.
    pub fn snapshot_detached(&self) -> Result<String, GitError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .git_dir
            .join(format!("index.detached-{}-{n}", std::process::id()));
        let index = self.git_dir.join("index");
        if index.is_file() {
            let _ = std::fs::copy(&index, &tmp);
        }
        let result = self.write_tree(&self.git().with_index(&tmp));
        let _ = std::fs::remove_file(&tmp);
        result
    }

    /// Setzt eine Ref auf einen Baum.
    pub fn set_ref(&self, name: &str, sha: &str) -> Result<(), GitError> {
        self.git().ok(&["update-ref", name, sha]).map(|_| ())
    }

    /// Liest eine Ref; `None`, wenn sie nicht existiert.
    pub fn get_ref(&self, name: &str) -> Option<String> {
        self.git()
            .text(&["rev-parse", "--verify", "--quiet", name])
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// Leerer Baum dieses Repositorys.
    pub fn empty_tree(&self) -> Result<String, GitError> {
        changes::empty_tree(&self.git())
    }

    /// Änderungen zwischen zwei Bäumen.
    pub fn changes(&self, a: &str, b: &str) -> Result<Vec<ChangedFile>, GitError> {
        changes::between(&self.git(), a, b)
    }

    /// Diff einer Datei zwischen zwei Bäumen.
    pub fn diff(&self, a: &str, b: &str, path: &str) -> Result<Option<FileDiff>, GitError> {
        changes::diff_between(&self.git(), a, b, path)
    }
}
