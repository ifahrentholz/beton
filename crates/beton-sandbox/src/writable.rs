//! Darf Stufe 1 diesen Pfad schreiben? Für Schreibzugriffe, die beton im Auftrag des Harness
//! außerhalb der Sandbox ausführt (ACP `fs/write_text_file`, HAR-007): Sie dürfen nicht mehr,
//! als die Policy der Stufe dem Harness-Prozess selbst erlaubt (Security-Review #327).

use std::path::{Component, Path};

use crate::resolve::{Mask, ResolvedPolicy};

impl ResolvedPolicy {
    /// `path`: absolut, ohne `..`, mit aufgelöstem Elternverzeichnis. Erlaubt nur in einer
    /// Schreib-Wurzel, nie in nur lesbaren Teilbäumen, Git-Verzeichnissen, Masken oder
    /// versteckten Dotfiles einer Grant-Wurzel (fail closed).
    pub fn may_write(&self, path: &Path) -> bool {
        if !path.is_absolute() || path.components().any(|c| !normal_or_root(c)) {
            return false;
        }
        // Jede Komponente `.git` (Groß/klein egal: macOS) bleibt unberührt (SBX-003 AC4).
        if path
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
        {
            return false;
        }
        if !self.write_roots.iter().any(|w| path.starts_with(w)) {
            return false;
        }
        if self.read_only.iter().any(|r| path.starts_with(r)) {
            return false;
        }
        if let Some(g) = &self.git
            && [&g.dot_git, &g.git_dir, &g.common_dir]
                .iter()
                .any(|d| path.starts_with(d))
        {
            return false;
        }
        if self.masks.iter().any(|m| masked(m, path)) {
            return false;
        }
        !self.hidden.iter().any(|h| {
            let Ok(rel) = path.strip_prefix(&h.root) else {
                return false;
            };
            let Some(first) = rel.components().next() else {
                return false;
            };
            let name = first.as_os_str().to_string_lossy();
            name.starts_with('.') && !(h.writable && h.allow.iter().any(|a| *a == name))
        })
    }
}

fn normal_or_root(c: Component<'_>) -> bool {
    matches!(c, Component::RootDir | Component::Normal(_))
}

/// Trifft die Maske `path` oder einen seiner Vorfahren? Namensmuster ohne Rücksicht auf
/// Groß/klein (auf macOS sind `.ENV` und `.env` dieselbe Datei; sonst nur strenger).
fn masked(mask: &Mask, path: &Path) -> bool {
    match mask {
        Mask::Path { path: m, .. } => path.starts_with(m),
        Mask::Pattern {
            root,
            glob,
            any_depth,
            ..
        } => {
            let Ok(rel) = path.strip_prefix(root) else {
                return false;
            };
            let glob = glob.to_lowercase();
            let mut names = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase());
            if *any_depth {
                names.any(|n| crate::linux::plan::glob_match(&glob, &n))
            } else {
                names
                    .next()
                    .is_some_and(|n| crate::linux::plan::glob_match(&glob, &n))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::path::PathBuf;

    use super::*;
    use crate::resolve::{GitProtection, HiddenDotfiles, Stage};

    fn policy(ws: &Path) -> ResolvedPolicy {
        let mut p = ResolvedPolicy::unsandboxed(Stage::Harness);
        p.workspace = ws.to_path_buf();
        p.write_roots = vec![ws.to_path_buf(), PathBuf::from("/scratch")];
        p.read_only = vec![ws.join(".beton")];
        p.masks = vec![
            Mask::Pattern {
                root: ws.to_path_buf(),
                glob: ".env".into(),
                any_depth: true,
                source: "masks".into(),
            },
            Mask::Pattern {
                root: ws.to_path_buf(),
                glob: ".env.*".into(),
                any_depth: true,
                source: "masks".into(),
            },
            Mask::Path {
                path: ws.join("geheim"),
                source: "masks".into(),
            },
        ];
        p.hidden = vec![HiddenDotfiles {
            root: PathBuf::from("/scratch"),
            allow: vec![".erlaubt".into()],
            writable: true,
        }];
        p.git = Some(GitProtection {
            dot_git: ws.join(".git"),
            dot_git_is_file: true,
            git_dir: PathBuf::from("/repo/.git/worktrees/ws"),
            common_dir: PathBuf::from("/repo/.git"),
        });
        p
    }

    #[test]
    fn sbx_003_stage_one_writes_only_where_its_policy_allows() {
        let ws = Path::new("/ws");
        let p = policy(ws);
        for ok in [
            "/ws/a.txt",
            "/ws/src/lib.rs",
            "/scratch/x",
            "/scratch/.erlaubt",
        ] {
            assert!(p.may_write(Path::new(ok)), "{ok}");
        }
        for denied in [
            // Git (SBX-003 AC4): Hooks, Konfiguration, die `.git`-Datei des Worktrees.
            "/ws/.git",
            "/ws/.git/hooks/pre-commit",
            "/ws/.git/config",
            "/ws/.GIT/config",
            "/ws/sub/.Git/hooks/post-checkout",
            "/repo/.git/worktrees/ws/commondir",
            "/repo/.git/worktrees/ws/gitdir",
            "/repo/.git/hooks/pre-commit",
            // Masken, auch in anderer Schreibweise (macOS: Groß/klein egal).
            "/ws/.env",
            "/ws/.ENV",
            "/ws/app/.env.local",
            "/ws/geheim",
            "/ws/geheim/schluessel",
            // Nur lesbar, außerhalb, versteckt.
            "/ws/.beton/policies/p.yaml",
            "/etc/passwd",
            "/wsx/a.txt",
            "/scratch/.versteckt",
            "relativ.txt",
        ] {
            assert!(!p.may_write(Path::new(denied)), "{denied}");
        }
    }
}
