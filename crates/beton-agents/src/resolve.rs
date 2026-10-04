//! Agent-Refs und Suchpfad (AGT-003): Projekt `.beton/agents/<name>/` → User
//! `~/.beton/agents/<name>/` → Built-ins. Der erste Treffer gewinnt; verschattet er ein
//! Built-in, gibt es eine Warnung. Pfade umgehen den Suchpfad, `builtin:<name>` erzwingt das
//! Built-in.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::dir::{AgentDir, Builtins};
use crate::load::parse_agent_yaml;
use crate::spec::AgentName;

/// Verweis auf einen Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRef {
    /// Über den Suchpfad: `maestra`.
    Name(String),
    /// Nur das Built-in: `builtin:maestra`.
    Builtin(String),
    /// Verzeichnis (oder dessen `agent.yaml`): `./agents/reviewer`, `/abs/x`.
    Path(PathBuf),
}

impl AgentRef {
    /// `builtin:<name>`, ein Pfad (beginnt mit `.`, `/`, `~` oder enthält `/`) oder ein Name.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if let Some(name) = raw.strip_prefix("builtin:") {
            return name
                .parse::<AgentName>()
                .map(|n| AgentRef::Builtin(n.to_string()));
        }
        if raw.is_empty() {
            return Err("Leerer Agent-Verweis".into());
        }
        if raw.starts_with(['.', '/', '~']) || raw.contains('/') || raw.contains('\\') {
            return Ok(AgentRef::Path(PathBuf::from(raw)));
        }
        raw.parse::<AgentName>()
            .map(|n| AgentRef::Name(n.to_string()))
    }
}

impl fmt::Display for AgentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRef::Name(n) => f.write_str(n),
            AgentRef::Builtin(n) => write!(f, "builtin:{n}"),
            AgentRef::Path(p) => write!(f, "{}", p.display()),
        }
    }
}

/// Herkunft eines Agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Project,
    User,
    Builtin,
    /// Direkt über einen Pfad angegeben.
    Path,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Project => "project",
            Source::User => "user",
            Source::Builtin => "builtin",
            Source::Path => "path",
        }
    }
}

/// Ein gefundener Agent.
#[derive(Debug, Clone)]
pub struct Located {
    pub name: String,
    pub source: Source,
    pub dir: AgentDir,
    /// Verschattetes Built-in, z. B. `builtin:maestra`.
    pub shadows: Option<String>,
}

impl Located {
    /// Warnung zur Verschattung, falls vorhanden.
    pub fn shadow_warning(&self) -> Option<String> {
        self.shadows.as_ref().map(|b| {
            format!(
                "{} ({}) verschattet {b}; das Built-in startet mit `beton run {b}`",
                self.name,
                self.source.as_str()
            )
        })
    }
}

/// Fehler beim Auflösen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("Agent „{0}“ nicht gefunden (gesucht: Projekt, User, Built-ins)")]
    NotFound(String),
    #[error("Built-in „{0}“ gibt es nicht")]
    NoSuchBuiltin(String),
    #[error("{0}: kein Agent-Verzeichnis mit agent.yaml")]
    NoAgentYaml(String),
    #[error("{0}")]
    InvalidRef(String),
}

/// Suchpfad eines Aufrufs.
#[derive(Debug, Clone, Default)]
pub struct SearchPath {
    /// `<projekt>/.beton/agents`.
    pub project: Option<PathBuf>,
    /// `~/.beton/agents`.
    pub user: Option<PathBuf>,
    pub builtins: Builtins,
}

/// Eintrag für `beton agent list` (AGT-003 AC2, AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub source: Source,
    pub version: Option<String>,
    pub harness: Option<String>,
    /// Pfad bzw. `builtin:<name>`.
    pub path: String,
    /// `false` ohne `agent.yaml` oder bei Fehlern im Schema.
    pub valid: bool,
    /// Grund der Ungültigkeit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Von welcher Quelle dieser Eintrag verschattet wird (dann nicht wirksam).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<Source>,
    /// Verschattetes Built-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadows: Option<String>,
}

impl SearchPath {
    fn roots(&self) -> Vec<(Source, &Path)> {
        let mut roots = Vec::new();
        if let Some(p) = &self.project {
            roots.push((Source::Project, p.as_path()));
        }
        if let Some(u) = &self.user
            && Some(u) != self.project.as_ref()
        {
            roots.push((Source::User, u.as_path()));
        }
        roots
    }

    fn builtin(&self, name: &str) -> Option<AgentDir> {
        let dir = self.builtins.dir(name);
        dir.has_agent_yaml().then_some(dir)
    }

    /// Löst einen Verweis auf. Relative Pfade gelten ab `base`.
    pub fn resolve(&self, reference: &AgentRef, base: &Path) -> Result<Located, ResolveError> {
        match reference {
            AgentRef::Builtin(name) => self
                .builtin(name)
                .map(|dir| Located {
                    name: name.clone(),
                    source: Source::Builtin,
                    dir,
                    shadows: None,
                })
                .ok_or_else(|| ResolveError::NoSuchBuiltin(name.clone())),
            AgentRef::Path(path) => {
                let mut path = expand_home(path);
                if path.is_relative() {
                    path = base.join(path);
                }
                if path
                    .file_name()
                    .is_some_and(|n| n == crate::dir::AGENT_YAML)
                    && path.is_file()
                {
                    path.pop();
                }
                let dir = AgentDir::Fs(path.clone());
                if !dir.has_agent_yaml() {
                    return Err(ResolveError::NoAgentYaml(path.display().to_string()));
                }
                Ok(Located {
                    name: dir.dir_name(),
                    source: Source::Path,
                    dir,
                    shadows: None,
                })
            }
            AgentRef::Name(name) => {
                for (source, root) in self.roots() {
                    let dir = AgentDir::Fs(root.join(name));
                    if dir.has_agent_yaml() {
                        return Ok(Located {
                            name: name.clone(),
                            source,
                            dir,
                            shadows: self.builtin(name).map(|_| format!("builtin:{name}")),
                        });
                    }
                }
                self.builtin(name)
                    .map(|dir| Located {
                        name: name.clone(),
                        source: Source::Builtin,
                        dir,
                        shadows: None,
                    })
                    .ok_or_else(|| ResolveError::NotFound(name.clone()))
            }
        }
    }

    /// Alle Agents im Suchpfad. `all = false`: nur wirksame Einträge mit `agent.yaml`;
    /// `all = true`: zusätzlich verschattete und ungültige (AGT-003 AC3).
    pub fn list(&self, all: bool) -> Vec<ListEntry> {
        let mut entries: Vec<ListEntry> = Vec::new();
        let mut candidates: Vec<(Source, String, AgentDir, String)> = Vec::new();
        for (source, root) in self.roots() {
            let Ok(read) = std::fs::read_dir(root) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = read
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .collect();
            dirs.sort();
            for path in dirs {
                let dir = AgentDir::Fs(path.clone());
                candidates.push((source, dir.dir_name(), dir, path.display().to_string()));
            }
        }
        for name in self.builtins.names() {
            candidates.push((
                Source::Builtin,
                name.clone(),
                self.builtins.dir(&name),
                format!("builtin:{name}"),
            ));
        }
        let builtin_names = self.builtins.names();
        let mut winner: std::collections::BTreeMap<String, Source> = Default::default();
        for (source, name, dir, path) in candidates {
            let mut entry = ListEntry {
                name: name.clone(),
                source,
                version: None,
                harness: None,
                path,
                valid: false,
                problem: None,
                shadowed_by: None,
                shadows: None,
            };
            if !dir.has_agent_yaml() {
                entry.problem = Some("keine agent.yaml".into());
                if all {
                    entries.push(entry);
                }
                continue;
            }
            let text = dir
                .read(crate::dir::AGENT_YAML)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let parsed = parse_agent_yaml(&text, crate::dir::AGENT_YAML);
            if let Some(spec) = &parsed.spec {
                entry.version = spec.version.clone();
                entry.harness = Some(spec.executor.harness.to_string());
            }
            match parsed.diagnostics.iter().find(|d| d.is_error()) {
                Some(d) => entry.problem = Some(format!("{d}")),
                None => entry.valid = true,
            }
            match winner.get(&name) {
                Some(by) => {
                    entry.shadowed_by = Some(*by);
                    if all {
                        entries.push(entry);
                    }
                }
                None => {
                    winner.insert(name.clone(), source);
                    if source != Source::Builtin && builtin_names.contains(&name) {
                        entry.shadows = Some(format!("builtin:{name}"));
                    }
                    entries.push(entry);
                }
            }
        }
        entries
    }
}

/// `~/x` → `$HOME/x`.
fn expand_home(path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn agent(dir: &Path, name: &str, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("agent.yaml"),
            format!(
                "spec_version: 1\nname: {name}\nversion: {version}\nexecutor:\n  harness: claude\n"
            ),
        )
        .unwrap();
    }

    fn builtins() -> Builtins {
        Builtins::from_files([(
            "maestra/agent.yaml",
            "spec_version: 1\nname: maestra\nversion: 1.0.0\nexecutor:\n  harness: claude\n",
        )])
    }

    #[test]
    fn agent_refs_distinguish_names_paths_and_builtins() {
        assert_eq!(
            AgentRef::parse("maestra").unwrap(),
            AgentRef::Name("maestra".into())
        );
        assert_eq!(
            AgentRef::parse("builtin:maestra").unwrap(),
            AgentRef::Builtin("maestra".into())
        );
        assert_eq!(
            AgentRef::parse("./agents/reviewer").unwrap(),
            AgentRef::Path("./agents/reviewer".into())
        );
        assert!(matches!(
            AgentRef::parse("/abs/x").unwrap(),
            AgentRef::Path(_)
        ));
        assert!(AgentRef::parse("Bad Name").is_err());
    }

    #[test]
    fn agt_003_ac1_project_shadows_builtin_with_warning_and_builtin_prefix_forces_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj/.beton/agents");
        agent(&project.join("maestra"), "maestra", "1.0.0-shop");
        let search = SearchPath {
            project: Some(project.clone()),
            user: Some(tmp.path().join("home/agents")),
            builtins: builtins(),
        };
        let found = search
            .resolve(&AgentRef::parse("maestra").unwrap(), tmp.path())
            .unwrap();
        assert_eq!(found.source, Source::Project);
        assert_eq!(found.shadows.as_deref(), Some("builtin:maestra"));
        assert!(
            found
                .shadow_warning()
                .unwrap()
                .contains("verschattet builtin:maestra")
        );
        let forced = search
            .resolve(&AgentRef::parse("builtin:maestra").unwrap(), tmp.path())
            .unwrap();
        assert_eq!(forced.source, Source::Builtin);
        assert!(forced.shadow_warning().is_none());
        // Pfade umgehen den Suchpfad.
        let by_path = search
            .resolve(
                &AgentRef::parse("./proj/.beton/agents/maestra").unwrap(),
                tmp.path(),
            )
            .unwrap();
        assert_eq!(by_path.source, Source::Path);
    }

    #[test]
    fn agt_003_project_wins_over_user_and_user_over_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p");
        let user = tmp.path().join("u");
        agent(&user.join("maestra"), "maestra", "0.9.0");
        agent(&user.join("helper"), "helper", "0.1.0");
        agent(&project.join("helper"), "helper", "0.2.0");
        let search = SearchPath {
            project: Some(project),
            user: Some(user),
            builtins: builtins(),
        };
        let helper = search
            .resolve(&AgentRef::Name("helper".into()), tmp.path())
            .unwrap();
        assert_eq!(helper.source, Source::Project);
        assert!(helper.shadows.is_none());
        let maestra = search
            .resolve(&AgentRef::Name("maestra".into()), tmp.path())
            .unwrap();
        assert_eq!(maestra.source, Source::User);
        assert_eq!(maestra.shadows.as_deref(), Some("builtin:maestra"));
        assert!(matches!(
            search.resolve(&AgentRef::Name("nope".into()), tmp.path()),
            Err(ResolveError::NotFound(_))
        ));
    }

    #[test]
    fn agt_003_ac2_list_shows_name_source_version_harness_and_path() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p");
        agent(&project.join("pr-fixer"), "pr-fixer", "0.3.0");
        let search = SearchPath {
            project: Some(project.clone()),
            user: None,
            builtins: builtins(),
        };
        let list = search.list(false);
        let names: Vec<(&str, Source)> = list.iter().map(|e| (e.name.as_str(), e.source)).collect();
        assert_eq!(
            names,
            [("pr-fixer", Source::Project), ("maestra", Source::Builtin)]
        );
        let e = &list[0];
        assert_eq!(e.version.as_deref(), Some("0.3.0"));
        assert_eq!(e.harness.as_deref(), Some("claude"));
        assert_eq!(e.path, project.join("pr-fixer").display().to_string());
        assert_eq!(list[1].path, "builtin:maestra");
    }

    #[test]
    fn agt_003_ac3_dir_without_agent_yaml_is_skipped_and_listed_as_invalid_with_all() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p");
        std::fs::create_dir_all(project.join("old-reviewer/prompts")).unwrap();
        agent(&project.join("maestra"), "maestra", "1.0.0-shop");
        let search = SearchPath {
            project: Some(project),
            user: None,
            builtins: builtins(),
        };
        let default = search.list(false);
        assert!(default.iter().all(|e| e.name != "old-reviewer"));
        assert!(
            search
                .resolve(&AgentRef::Name("old-reviewer".into()), tmp.path())
                .is_err()
        );
        let all = search.list(true);
        let old = all.iter().find(|e| e.name == "old-reviewer").unwrap();
        assert!(!old.valid);
        assert_eq!(old.problem.as_deref(), Some("keine agent.yaml"));
        // Mit --all erscheint auch das verschattete Built-in.
        let shadowed = all
            .iter()
            .find(|e| e.name == "maestra" && e.source == Source::Builtin)
            .unwrap();
        assert_eq!(shadowed.shadowed_by, Some(Source::Project));
        let project_maestra = all
            .iter()
            .find(|e| e.name == "maestra" && e.source == Source::Project)
            .unwrap();
        assert_eq!(project_maestra.shadows.as_deref(), Some("builtin:maestra"));
    }
}
