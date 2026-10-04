//! Agent-Verzeichnisse auf der Platte oder eingebettet im Binary (Built-ins, AGT-003) und
//! der Inhalts-Hash eines Agents.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest as _, Sha256};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/builtins.rs"));
}

/// Name der Pflichtdatei eines Agents.
pub const AGENT_YAML: &str = "agent.yaml";

/// Die im Binary eingebetteten Built-in-Agents (Quelle `agents/` im Repo).
#[derive(Debug, Clone, Default)]
pub struct Builtins {
    files: Arc<BTreeMap<String, Cow<'static, [u8]>>>,
}

impl Builtins {
    /// Die eingebetteten Built-ins dieses Binaries.
    pub fn embedded() -> Self {
        Self {
            files: Arc::new(
                embedded::EMBEDDED
                    .iter()
                    .map(|(p, b)| ((*p).to_owned(), Cow::Borrowed(*b)))
                    .collect(),
            ),
        }
    }

    /// Eigene Built-ins, z. B. für Tests: Pfade wie `maestra/agent.yaml`.
    pub fn from_files<P: Into<String>, B: Into<Vec<u8>>>(
        files: impl IntoIterator<Item = (P, B)>,
    ) -> Self {
        Self {
            files: Arc::new(
                files
                    .into_iter()
                    .map(|(p, b)| (p.into(), Cow::Owned(b.into())))
                    .collect(),
            ),
        }
    }

    /// Alle eingebetteten Dateien (relativer Pfad, Inhalt).
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(p, b)| (p.as_str(), b.as_ref()))
    }

    /// Namen der Verzeichnisse auf oberster Ebene, sortiert.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .files
            .keys()
            .filter_map(|p| p.split_once('/').map(|(n, _)| n.to_owned()))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn dir(&self, name: &str) -> AgentDir {
        AgentDir::Builtin {
            builtins: self.clone(),
            prefix: name.to_owned(),
        }
    }

    fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(AsRef::as_ref)
    }

    fn has_dir(&self, prefix: &str) -> bool {
        let prefix = format!("{prefix}/");
        self.files.keys().any(|p| p.starts_with(&prefix))
    }
}

/// Lexikalisch normalisierter, relativer Pfad mit `/`; `None`, wenn er über die Wurzel
/// hinausführt oder absolut ist.
pub fn normalize_relative(base: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

/// Ein Agent-Verzeichnis.
#[derive(Debug, Clone)]
pub enum AgentDir {
    Fs(PathBuf),
    Builtin { builtins: Builtins, prefix: String },
}

impl AgentDir {
    /// Inhalt einer Datei relativ zum Verzeichnis.
    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        match self {
            AgentDir::Fs(dir) => std::fs::read(dir.join(rel)).ok(),
            AgentDir::Builtin { builtins, prefix } => {
                let path = normalize_relative(prefix, rel)?;
                builtins.get(&path).map(<[u8]>::to_vec)
            }
        }
    }

    /// Existiert die Datei oder das Verzeichnis?
    pub fn exists(&self, rel: &str) -> bool {
        match self {
            AgentDir::Fs(dir) => dir.join(rel).exists(),
            AgentDir::Builtin { builtins, prefix } => normalize_relative(prefix, rel)
                .is_some_and(|p| builtins.get(&p).is_some() || builtins.has_dir(&p)),
        }
    }

    /// Bleibt der relative Pfad innerhalb des Verzeichnisses (lexikalisch)?
    pub fn contains(rel: &str) -> bool {
        normalize_relative("", rel).is_some_and(|p| !p.is_empty())
    }

    pub fn has_agent_yaml(&self) -> bool {
        match self {
            AgentDir::Fs(dir) => dir.join(AGENT_YAML).is_file(),
            AgentDir::Builtin { .. } => self.read(AGENT_YAML).is_some(),
        }
    }

    /// Unterverzeichnis bzw. Verweis relativ zu diesem Verzeichnis. Bei Built-ins darf der
    /// Pfad die eingebetteten Agents nicht verlassen.
    pub fn join(&self, rel: &str) -> Option<AgentDir> {
        match self {
            AgentDir::Fs(dir) => Some(AgentDir::Fs(dir.join(rel))),
            AgentDir::Builtin { builtins, prefix } => Some(AgentDir::Builtin {
                builtins: builtins.clone(),
                prefix: normalize_relative(prefix, rel).filter(|p| !p.is_empty())?,
            }),
        }
    }

    /// Letzte Pfadkomponente.
    pub fn dir_name(&self) -> String {
        match self {
            AgentDir::Fs(dir) => dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            AgentDir::Builtin { prefix, .. } => {
                prefix.rsplit('/').next().unwrap_or_default().to_owned()
            }
        }
    }

    /// Eindeutige Identität (für Zyklen): kanonischer Pfad bzw. `builtin:<pfad>`.
    pub fn identity(&self) -> String {
        match self {
            AgentDir::Fs(dir) => dir
                .canonicalize()
                .unwrap_or_else(|_| dir.clone())
                .display()
                .to_string(),
            AgentDir::Builtin { prefix, .. } => format!("builtin:{prefix}"),
        }
    }

    /// Anzeige: Pfad bzw. `builtin:<name>`.
    pub fn display(&self) -> String {
        match self {
            AgentDir::Fs(dir) => dir.display().to_string(),
            AgentDir::Builtin { prefix, .. } => format!("builtin:{prefix}"),
        }
    }

    /// Pfad von `self` relativ zu `root`, wenn `self` darin liegt.
    pub fn relative_to(&self, root: &AgentDir) -> Option<String> {
        match (self, root) {
            (AgentDir::Fs(a), AgentDir::Fs(b)) => {
                let a = a.canonicalize().ok()?;
                let b = b.canonicalize().ok()?;
                let rel = a.strip_prefix(&b).ok()?;
                Some(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                )
            }
            (AgentDir::Builtin { prefix: a, .. }, AgentDir::Builtin { prefix: b, .. }) => {
                if a == b {
                    Some(String::new())
                } else {
                    a.strip_prefix(&format!("{b}/")).map(str::to_owned)
                }
            }
            _ => None,
        }
    }

    /// Alle Dateien des Agents (ohne versteckte), sortiert nach relativem Pfad.
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        match self {
            AgentDir::Fs(dir) => {
                let mut out = Vec::new();
                walk(dir, dir, &mut out);
                out.sort();
                out
            }
            AgentDir::Builtin { builtins, prefix } => {
                let p = format!("{prefix}/");
                builtins
                    .files()
                    .filter_map(|(path, bytes)| {
                        path.strip_prefix(&p)
                            .map(|rel| (rel.to_owned(), bytes.to_vec()))
                    })
                    .collect()
            }
        }
    }

    /// Inhalts-Hash über alle Dateien des Agents (Pfad und Inhalt), `sha256:<hex>`.
    /// Unveränderte Dateien ergeben denselben Hash, jede Änderung einen anderen.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        for (path, bytes) in self.files() {
            h.update(path.as_bytes());
            h.update([0]);
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(&bytes);
        }
        format!("sha256:{}", hex::encode(h.finalize()))
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(root, &path, out);
        } else if kind.is_file()
            && let (Ok(rel), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path))
        {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn agt_003_builtins_are_embedded_from_the_repo_agents_dir() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        let mut on_disk = Vec::new();
        walk(&root, &root, &mut on_disk);
        on_disk.sort();
        let embedded: Vec<(String, Vec<u8>)> = Builtins::embedded()
            .files()
            .map(|(p, b)| (p.to_owned(), b.to_vec()))
            .collect();
        assert_eq!(embedded, on_disk);
    }

    #[test]
    fn builtin_paths_cannot_escape_the_embedded_tree() {
        let b = Builtins::from_files([("maestra/agent.yaml", "x"), ("maestra/p/s.md", "y")]);
        let dir = b.dir("maestra");
        assert!(dir.exists("p/s.md"));
        assert!(dir.exists("p"));
        assert_eq!(dir.read("./p/../p/s.md").unwrap(), b"y");
        assert!(dir.join("../..").is_none());
        assert_eq!(b.names(), ["maestra"]);
        assert!(!AgentDir::contains("../x"));
        assert!(AgentDir::contains("prompts/system.md"));
    }

    #[test]
    fn hash_changes_with_content() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent.yaml"), "a").unwrap();
        let dir = AgentDir::Fs(tmp.path().to_path_buf());
        let first = dir.hash();
        assert_eq!(first, dir.hash());
        assert!(first.starts_with("sha256:"));
        std::fs::write(tmp.path().join("agent.yaml"), "b").unwrap();
        assert_ne!(first, dir.hash());
    }
}
