//! Git-Layout eines Workspace (SBX-003, SES-015): Ein Worktree braucht Schreibzugriff auf
//! seinen Eintrag unter `<repo>/.git/worktrees/<name>` und die Objekt-DB des Repos; Hooks und
//! Konfiguration bleiben read-only (sonst führte ein späteres `git` außerhalb der Sandbox
//! Code des Agents aus).

use std::path::{Path, PathBuf};

use crate::error::SandboxError;

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
    Ok(std::fs::read_to_string(p)?.trim_end_matches(['\n', '\r']).to_owned())
}

impl GitLayout {
    /// Ermittelt das Layout eines (kanonischen) Workspace. Ein Worktree gilt nur, wenn die
    /// Verweise in beide Richtungen stimmen: `<ws>/.git` → `<common>/worktrees/<name>`, dessen
    /// `gitdir` → `<ws>/.git` und dessen `commondir` → `<common>`. Sonst Fehler: Ein
    /// gefälschtes `.git` darf keine fremden Schreib-Wurzeln freigeben.
    pub fn discover(workspace: &Path) -> Result<Option<Self>, SandboxError> {
        todo!("SBX-003")
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
