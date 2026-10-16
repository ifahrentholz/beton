//! Git-Layout eines Workspace (SBX-003, SES-015): Ein Worktree braucht Schreibzugriff auf
//! seinen Eintrag unter `<repo>/.git/worktrees/<name>` und die Objekt-DB des Repos; Hooks und
//! Konfiguration bleiben read-only (sonst führte ein späteres `git` außerhalb der Sandbox
//! Code des Agents aus).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::SandboxError;

/// Steuerdateien eines Git-Verzeichnisses, über die git ein anderes (gemeinsames) Git-
/// Verzeichnis samt dessen Konfiguration und Hooks nimmt. Aus der Sandbox heraus dürfen sie
/// weder entstehen noch sich ändern (SBX-003, #209).
pub const CONTROL_FILES: [&str; 2] = ["commondir", "gitdir"];

/// Wo das Git-Verzeichnis eines Workspace liegt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLayout {
    /// Der Eintrag `<workspace>/.git` (Verzeichnis oder Datei).
    pub dot_git: PathBuf,
    /// `true`: `.git` ist eine Datei (`gitdir: …`, Worktree).
    pub dot_git_is_file: bool,
    /// Git-Verzeichnis des Worktrees (`<repo>/.git/worktrees/<name>`) bzw. `<workspace>/.git`.
    pub git_dir: PathBuf,
    /// Gemeinsames Git-Verzeichnis (`<repo>/.git`) mit Objekt-DB, Refs, Hooks, Config.
    pub common_dir: PathBuf,
}

fn invalid(ws: &Path, why: impl std::fmt::Display) -> SandboxError {
    SandboxError::invalid(
        "workspace",
        format!("{}: kein gültiges Git-Layout: {why}", ws.display()),
    )
}

fn read_trimmed(p: &Path) -> std::io::Result<String> {
    Ok(std::fs::read_to_string(p)?
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}

impl GitLayout {
    /// Ermittelt das Layout eines (kanonischen) Workspace. Ein Worktree gilt nur, wenn die
    /// Verweise in beide Richtungen stimmen: `<ws>/.git` → `<common>/worktrees/<name>`, dessen
    /// `gitdir` → `<ws>/.git` und dessen `commondir` → `<common>`. Sonst Fehler: Ein
    /// gefälschtes `.git` darf keine fremden Schreib-Wurzeln freigeben.
    pub fn discover(workspace: &Path) -> Result<Option<Self>, SandboxError> {
        let dot = workspace.join(".git");
        let meta = match std::fs::symlink_metadata(&dot) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(invalid(workspace, e)),
        };
        if meta.file_type().is_symlink() {
            return Err(invalid(workspace, "`.git` ist ein Symlink"));
        }
        if meta.is_dir() {
            return Ok(Some(Self {
                dot_git: dot.clone(),
                dot_git_is_file: false,
                git_dir: dot.clone(),
                common_dir: dot,
            }));
        }
        let text = read_trimmed(&dot).map_err(|e| invalid(workspace, e))?;
        let target = text
            .strip_prefix("gitdir: ")
            .ok_or_else(|| invalid(workspace, "`.git` enthält kein `gitdir:`"))?;
        let target = Path::new(target);
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            workspace.join(target)
        };
        let git_dir = std::fs::canonicalize(&target).map_err(|e| invalid(workspace, e))?;
        let worktrees = git_dir
            .parent()
            .ok_or_else(|| invalid(workspace, "gitdir ohne Elternverzeichnis"))?;
        if worktrees.file_name().and_then(|n| n.to_str()) != Some("worktrees") {
            return Err(invalid(
                workspace,
                "gitdir liegt nicht unter `…/worktrees/`",
            ));
        }
        let common_dir = worktrees
            .parent()
            .ok_or_else(|| invalid(workspace, "worktrees ohne Elternverzeichnis"))?
            .to_path_buf();
        let commondir =
            read_trimmed(&git_dir.join("commondir")).map_err(|e| invalid(workspace, e))?;
        let commondir = git_dir.join(commondir);
        if std::fs::canonicalize(&commondir).ok().as_deref() != Some(common_dir.as_path()) {
            return Err(invalid(workspace, "`commondir` zeigt nicht auf das Repo"));
        }
        let back = read_trimmed(&git_dir.join("gitdir")).map_err(|e| invalid(workspace, e))?;
        if std::fs::canonicalize(&back).ok().as_deref() != Some(dot.as_path()) {
            return Err(invalid(
                workspace,
                "der Worktree-Eintrag verweist nicht zurück",
            ));
        }
        if !common_dir.join("HEAD").is_file() || !common_dir.join("objects").is_dir() {
            return Err(invalid(
                workspace,
                "gemeinsames Git-Verzeichnis unvollständig",
            ));
        }
        Ok(Some(Self {
            dot_git: dot,
            dot_git_is_file: true,
            git_dir,
            common_dir,
        }))
    }

    /// Pfade der Steuerdateien ([`CONTROL_FILES`]) im Git-Dir, im Common-Dir und in den
    /// vorhandenen Einträgen unter `worktrees/` (auch wenn sie fehlen).
    pub fn control_files(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.git_dir.clone(), self.common_dir.clone()];
        if let Ok(entries) = std::fs::read_dir(self.common_dir.join("worktrees")) {
            let mut wt: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect();
            wt.sort();
            dirs.extend(wt);
        }
        let mut out = Vec::new();
        for d in dirs {
            for name in CONTROL_FILES {
                let p = d.join(name);
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Schreib-Wurzeln außerhalb des Workspace, die `git commit` im Worktree braucht.
    pub fn write_roots(&self) -> Vec<PathBuf> {
        if !self.dot_git_is_file {
            return Vec::new();
        }
        vec![
            self.git_dir.clone(),
            self.common_dir.join("objects"),
            self.common_dir.join("refs"),
            self.common_dir.join("logs"),
        ]
    }
}

/// Höchstgröße einer Steuerdatei, deren Inhalt der Schnappschuss festhält.
const MAX_CONTROL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlEntry {
    Missing,
    File(Vec<u8>),
    /// Verzeichnis, Symlink, zu große Datei oder nicht lesbar.
    Other,
}

fn entry(p: &Path) -> ControlEntry {
    match std::fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ControlEntry::Missing,
        Ok(m) if m.is_file() && m.len() <= MAX_CONTROL_BYTES => std::fs::read(p)
            .map(ControlEntry::File)
            .unwrap_or(ControlEntry::Other),
        _ => ControlEntry::Other,
    }
}

/// Zustand der Git-Steuerdateien eines Workspace vor dem ersten Tool-Lauf: die zweite Linie
/// hinter den Sperren der Backends (SBX-003, #209). Der Runner vergleicht nach jedem Tool-Lauf
/// und beendet die Session, wenn eine Steuerdatei entstanden ist oder sich geändert hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlSnapshot {
    entries: BTreeMap<PathBuf, ControlEntry>,
}

impl ControlSnapshot {
    pub fn capture(layout: &GitLayout) -> Self {
        Self {
            entries: layout
                .control_files()
                .into_iter()
                .map(|p| {
                    let e = entry(&p);
                    (p, e)
                })
                .collect(),
        }
    }

    /// Steuerdateien, die seit dem Schnappschuss entstanden sind oder sich geändert haben.
    /// Verschwundene zählen nicht (git selbst räumt Worktrees ab), ebenso wenig Einträge neuer
    /// Worktrees (eine parallele Session legt ihren an).
    pub fn changes(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|(p, before)| {
                let now = entry(p);
                now != ControlEntry::Missing && now != **before
            })
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Stellt für `paths` den Zustand des Schnappschusses wieder her: Neues wird entfernt
    /// (ohne Symlinks zu folgen), Geändertes durch den alten Inhalt ersetzt.
    pub fn restore(&self, paths: &[PathBuf]) -> std::io::Result<()> {
        for p in paths {
            let Some(before) = self.entries.get(p) else {
                continue;
            };
            match std::fs::symlink_metadata(p) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(p)?,
                Ok(_) => std::fs::remove_file(p)?,
                Err(_) => {}
            }
            if let ControlEntry::File(content) = before {
                use std::io::Write as _;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(p)?
                    .write_all(content)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn layout(root: &Path) -> GitLayout {
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("worktrees/wt2")).unwrap();
        std::fs::write(git.join("worktrees/wt2/commondir"), "../..\n").unwrap();
        std::fs::write(git.join("worktrees/wt2/gitdir"), "/x/wt2/.git\n").unwrap();
        GitLayout {
            dot_git: git.clone(),
            dot_git_is_file: false,
            git_dir: git.clone(),
            common_dir: git,
        }
    }

    #[test]
    fn sbx_003_snapshot_finds_new_and_changed_control_files_and_restores_them() {
        let tmp = tempfile::tempdir().unwrap();
        let l = layout(tmp.path());
        let git = &l.git_dir;
        let snap = ControlSnapshot::capture(&l);
        assert!(snap.changes().is_empty());
        // Normale git-Arbeit und neue Worktrees ändern nichts.
        std::fs::write(git.join("index.lock"), "x").unwrap();
        std::fs::create_dir_all(git.join("worktrees/neu")).unwrap();
        std::fs::write(git.join("worktrees/neu/commondir"), "../..\n").unwrap();
        assert!(snap.changes().is_empty());

        std::fs::write(git.join("commondir"), "/tmp/x\n").unwrap();
        std::fs::write(git.join("worktrees/wt2/gitdir"), "/tmp/x\n").unwrap();
        let changed = snap.changes();
        assert_eq!(
            changed,
            vec![git.join("commondir"), git.join("worktrees/wt2/gitdir")]
        );
        snap.restore(&changed).unwrap();
        assert!(!git.join("commondir").exists());
        assert_eq!(
            std::fs::read_to_string(git.join("worktrees/wt2/gitdir")).unwrap(),
            "/x/wt2/.git\n"
        );
        assert!(snap.changes().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn sbx_003_restore_removes_a_symlink_without_following_it() {
        let tmp = tempfile::tempdir().unwrap();
        let l = layout(tmp.path());
        let victim = tmp.path().join("victim");
        std::fs::write(&victim, "bleibt").unwrap();
        let snap = ControlSnapshot::capture(&l);
        let gitdir = l.git_dir.join("worktrees/wt2/gitdir");
        std::fs::remove_file(&gitdir).unwrap();
        std::os::unix::fs::symlink(&victim, &gitdir).unwrap();
        std::os::unix::fs::symlink(&victim, l.git_dir.join("commondir")).unwrap();
        let changed = snap.changes();
        assert_eq!(changed.len(), 2, "{changed:?}");
        snap.restore(&changed).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "bleibt");
        assert!(std::fs::symlink_metadata(l.git_dir.join("commondir")).is_err());
        assert_eq!(std::fs::read_to_string(&gitdir).unwrap(), "/x/wt2/.git\n");
    }
}
