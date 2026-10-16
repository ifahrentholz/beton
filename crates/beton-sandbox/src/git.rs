//! Git-Layout eines Workspace (SBX-003, SES-015): Ein Worktree braucht Schreibzugriff auf
//! seinen Eintrag unter `<repo>/.git/worktrees/<name>` und die Objekt-DB des Repos; Hooks und
//! Konfiguration bleiben read-only (sonst führte ein späteres `git` außerhalb der Sandbox
//! Code des Agents aus).

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
