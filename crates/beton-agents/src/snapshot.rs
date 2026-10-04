//! Agent-Snapshot pro Session (AGT-004): Beim Start wird der Agent vollständig aufgelöst
//! (Prompts, Skills, Sub-Agent-Definitionen, Policies) und als ein Blob festgehalten. Runner,
//! Resume und Forks lesen den Agent danach nur noch aus dem Snapshot, nie mehr von der Platte;
//! so bleiben Sessions reproduzierbar, auch wenn sich die Agent-Dateien ändern.
//!
//! Inhalt: alle Dateien des Agent-Verzeichnisses unter `agent/`, Sub-Agents außerhalb des
//! Verzeichnisses (z. B. `builtin:<name>` oder `../shared/x`) unter `ext/<n>/` samt Verweis,
//! der ausgeführte Agent (`entry`, bei Inline-Sub-Agents zusätzlich `inline`), die
//! aufgelösten Parameter (AGT-010) und CLI-Overrides.
//!
//! `hash` ist der Inhalts-Hash des Agents: für einen Agent ohne äußere Sub-Agents genau
//! [`AgentDir::hash`] (wie `beton agent show`), sonst erweitert um die äußeren Bäume. Parameter
//! und Overrides gehen nicht in den Hash ein; sie stehen im Event `agent.resolved`.

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::dir::{AgentDir, Builtins, hash_files};
use crate::params::ParamValues;
use crate::resolve::{Located, resolve_subagent};
use crate::spec::AgentSpec;

/// Format-Version der Snapshot-Datei.
pub const FORMAT: u32 = 1;
/// Präfix des Agent-Verzeichnisses im Snapshot.
pub const ROOT: &str = "agent";
/// Obergrenze für die Dateien eines Agents im Snapshot.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Überschreibungen beim Start (`--harness`, `--model`), im Snapshot als `overrides`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.harness.is_none() && self.model.is_none()
    }
}

/// Ein aufgelöster Agent, wie er im Blob-Store liegt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// `project`, `user`, `builtin`, `path` oder `subagent`.
    pub source: String,
    /// Agent-Ref, wie er beim Start angegeben wurde.
    #[serde(rename = "ref")]
    pub reference: String,
    pub hash: String,
    /// Pfad im Snapshot → Inhalt (Base64).
    pub files: BTreeMap<String, String>,
    /// `<präfix des Verweisenden>|<ref>` → Präfix des Sub-Agents (`ext/<n>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refs: BTreeMap<String, String>,
    /// Präfix des ausgeführten Agents, z. B. `agent` oder `agent/agents/reviewer`.
    pub entry: String,
    /// Inline-Sub-Agent (Schlüssel unter `agents`) des Agents unter `entry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
    /// Aufgelöste Parameterwerte (AGT-010).
    #[serde(default)]
    pub params: ParamValues,
    #[serde(default, skip_serializing_if = "Overrides::is_empty")]
    pub overrides: Overrides,
    /// Tiefe im Session-Baum (0 = gestartet von Mensch oder API, 1 = Child).
    #[serde(default)]
    pub depth: u32,
    /// Von den Vorfahren geerbte Grenze für die Tiefe im Session-Baum (AGT-009); fehlt beim
    /// Wurzel-Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_limit: Option<u32>,
}

/// Fehler beim Erfassen oder Lesen eines Snapshots.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("{0}")]
    Agent(String),
    #[error("Agent zu groß für den Snapshot ({size} Bytes, höchstens {max})")]
    TooLarge { size: usize, max: usize },
    #[error("Agent-Snapshot unlesbar: {0}")]
    Format(String),
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Liest `agent.yaml` eines Verzeichnisses; Fehler mit Datei, Zeile und Spalte.
pub fn load_spec(dir: &AgentDir) -> Result<AgentSpec, String> {
    let text = dir
        .read(crate::dir::AGENT_YAML)
        .ok_or_else(|| format!("{}: agent.yaml fehlt", dir.display()))?;
    let text = String::from_utf8_lossy(&text);
    let file = format!("{}/agent.yaml", dir.display());
    let parsed = crate::load::parse_agent_yaml(&text, &file);
    if let Some(d) = parsed.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!("{}:{}:{}: {}", d.file, d.line, d.column, d.message));
    }
    parsed.spec.ok_or_else(|| format!("{file}: Agent ungültig"))
}

/// Sammelt Dateien und äußere Sub-Agents.
struct Collector<'a> {
    builtins: &'a Builtins,
    files: BTreeMap<String, String>,
    refs: BTreeMap<String, String>,
    /// Identität der erfassten Bäume → Präfix (Zyklen, doppelte Verweise).
    seen: BTreeMap<String, String>,
    /// Bereits durchlaufene Agents (Zyklen in `agents`).
    visited: std::collections::BTreeSet<String>,
    size: usize,
}

impl Collector<'_> {
    fn add_tree(&mut self, dir: &AgentDir, prefix: &str) -> Result<(), SnapshotError> {
        for (rel, bytes) in dir.files() {
            self.size += bytes.len();
            if self.size > MAX_BYTES {
                return Err(SnapshotError::TooLarge {
                    size: self.size,
                    max: MAX_BYTES,
                });
            }
            self.files.insert(format!("{prefix}/{rel}"), b64(&bytes));
        }
        Ok(())
    }

    /// Erfasst die Sub-Agents eines Agents (rekursiv). `tree`/`tree_prefix` sind der erfasste
    /// Baum, in dem `dir` liegt; Sub-Agents außerhalb davon kommen als eigener Baum hinzu.
    fn walk(
        &mut self,
        dir: &AgentDir,
        prefix: &str,
        tree: &AgentDir,
        tree_prefix: &str,
    ) -> Result<(), SnapshotError> {
        if !self.visited.insert(format!("{}|{prefix}", dir.identity())) {
            return Ok(());
        }
        let Ok(spec) = load_spec(dir) else {
            return Ok(());
        };
        let mut refs = Vec::new();
        collect_refs(&spec.agents, &mut refs);
        for reference in refs {
            let Ok(target) = resolve_subagent(dir, &reference, self.builtins) else {
                // Ungültige Verweise meldet die Validierung; der Snapshot nimmt sie nicht auf.
                continue;
            };
            // Im selben Baum: schon erfasst, nur weiter absteigen.
            if let Some(rel) = target.relative_to(tree) {
                if !rel.is_empty() {
                    let inner = format!("{tree_prefix}/{rel}");
                    self.walk(&target, &inner, tree, tree_prefix)?;
                }
                continue;
            }
            let identity = target.identity();
            let target_prefix = match self.seen.get(&identity) {
                Some(p) => p.clone(),
                None => {
                    let p = format!("ext/{}", self.seen.len());
                    self.seen.insert(identity, p.clone());
                    self.add_tree(&target, &p)?;
                    self.walk(&target, &p, &target, &p)?;
                    p
                }
            };
            self.refs
                .insert(format!("{prefix}|{reference}"), target_prefix);
        }
        Ok(())
    }
}

fn collect_refs(agents: &BTreeMap<String, crate::spec::SubAgent>, out: &mut Vec<String>) {
    for sub in agents.values() {
        match &sub.reference {
            Some(r) => out.push(r.clone()),
            None => collect_refs(&sub.agents, out),
        }
    }
}

impl AgentSnapshot {
    /// Erfasst einen gefundenen Agent. `reference` ist der Agent-Ref des Starts.
    pub fn capture(
        located: &Located,
        reference: &str,
        builtins: &Builtins,
    ) -> Result<Self, SnapshotError> {
        let spec = load_spec(&located.dir).map_err(SnapshotError::Agent)?;
        let mut c = Collector {
            builtins,
            files: BTreeMap::new(),
            refs: BTreeMap::new(),
            seen: BTreeMap::from([(located.dir.identity(), ROOT.to_owned())]),
            visited: Default::default(),
            size: 0,
        };
        c.add_tree(&located.dir, ROOT)?;
        c.walk(&located.dir, ROOT, &located.dir, ROOT)?;
        let mut snapshot = Self {
            format: FORMAT,
            name: spec.name.to_string(),
            version: spec.version.clone().unwrap_or_default(),
            source: located.source.as_str().to_owned(),
            reference: reference.to_owned(),
            hash: String::new(),
            files: c.files,
            refs: c.refs,
            entry: ROOT.to_owned(),
            inline: None,
            params: ParamValues::new(),
            overrides: Overrides::default(),
            depth: 0,
            depth_limit: None,
        };
        snapshot.hash = snapshot.content_hash()?;
        Ok(snapshot)
    }

    /// Dateien des Snapshots als Baum im Speicher.
    fn tree(&self) -> Result<Builtins, SnapshotError> {
        let engine = base64::engine::general_purpose::STANDARD;
        let mut files = Vec::with_capacity(self.files.len());
        for (path, data) in &self.files {
            let bytes = engine
                .decode(data)
                .map_err(|e| SnapshotError::Format(format!("{path}: {e}")))?;
            files.push((path.clone(), bytes));
        }
        Ok(Builtins::from_files(files))
    }

    fn dir_at(&self, tree: &Builtins, prefix: &str) -> AgentDir {
        AgentDir::Snapshot {
            files: tree.clone(),
            prefix: prefix.to_owned(),
            refs: Arc::new(self.refs.clone()),
        }
    }

    /// Inhalts-Hash (siehe Modul-Doku).
    fn content_hash(&self) -> Result<String, SnapshotError> {
        let tree = self.tree()?;
        let base = self.dir_at(&tree, ROOT).hash();
        if self.refs.is_empty() && self.entry == ROOT && self.inline.is_none() {
            return Ok(base);
        }
        let mut h = Sha256::new();
        h.update(base.as_bytes());
        let mut externals: Vec<&String> = self.refs.values().collect();
        externals.sort();
        externals.dedup();
        for (key, prefix) in &self.refs {
            h.update(key.as_bytes());
            h.update([0]);
            h.update(prefix.as_bytes());
            h.update([0]);
        }
        for prefix in externals {
            let files = self.dir_at(&tree, prefix).files();
            h.update(hash_files(&files).as_bytes());
        }
        h.update(self.entry.as_bytes());
        h.update([0]);
        if let Some(inline) = &self.inline {
            h.update(inline.as_bytes());
        }
        Ok(format!("sha256:{}", hex::encode(h.finalize())))
    }

    /// Der ausgeführte Agent: Definition und sein Verzeichnis im Snapshot.
    pub fn agent(&self) -> Result<(AgentSpec, AgentDir), SnapshotError> {
        let tree = self.tree()?;
        let dir = self.dir_at(&tree, &self.entry);
        let mut spec = load_spec(&dir).map_err(SnapshotError::Agent)?;
        // Inline-Sub-Agents, auch verschachtelt (`a/b`): relative Dateien gelten weiter ab dem
        // Verzeichnis des Agents unter `entry` (AGT-009).
        for name in self.inline.iter().flat_map(|p| p.split('/')) {
            let sub = spec.agents.get(name).ok_or_else(|| {
                SnapshotError::Agent(format!("Sub-Agent „{name}“ fehlt im Snapshot"))
            })?;
            spec = AgentSpec::from_inline(name, sub).map_err(SnapshotError::Agent)?;
        }
        Ok((spec, dir))
    }

    /// Größte erlaubte Tiefe für Nachfahren dieser Session (AGT-009): eigene Tiefe plus
    /// `spawn.max_depth` (Default [`crate::spec::DEFAULT_MAX_DEPTH`]), höchstens die geerbte
    /// Grenze. Ein Child mit größerer Tiefe wird nicht gestartet (`spawn_denied: max_depth`).
    pub fn spawn_depth_limit(&self, spec: &AgentSpec) -> u32 {
        let own = self.depth.saturating_add(
            spec.spawn
                .as_ref()
                .and_then(|s| s.max_depth)
                .unwrap_or(crate::spec::DEFAULT_MAX_DEPTH),
        );
        self.depth_limit.map_or(own, |l| l.min(own))
    }

    /// Snapshot für einen Sub-Agent aus `agents` (Child-Session von `session_spawn`); die
    /// Parameter des Childs löst der Aufrufer danach auf.
    pub fn subagent(&self, name: &str) -> Result<Self, SnapshotError> {
        let (spec, dir) = self.agent()?;
        let sub = spec
            .agents
            .get(name)
            .ok_or_else(|| SnapshotError::Agent(format!("Sub-Agent „{name}“ ist unbekannt")))?;
        let mut child = Self {
            format: FORMAT,
            name: name.to_owned(),
            version: String::new(),
            source: "subagent".into(),
            reference: format!("{}#{name}", self.reference),
            hash: String::new(),
            files: self.files.clone(),
            refs: self.refs.clone(),
            entry: self.entry.clone(),
            inline: None,
            params: ParamValues::new(),
            overrides: Overrides::default(),
            depth: self.depth + 1,
            depth_limit: Some(self.spawn_depth_limit(&spec)),
        };
        match &sub.reference {
            Some(reference) => {
                let target = resolve_subagent(&dir, reference, &Builtins::default())
                    .map_err(SnapshotError::Agent)?;
                let AgentDir::Snapshot { prefix, .. } = &target else {
                    return Err(SnapshotError::Agent(format!(
                        "{reference} liegt nicht im Snapshot"
                    )));
                };
                child.entry.clone_from(prefix);
                let spec = load_spec(&target).map_err(SnapshotError::Agent)?;
                child.name = spec.name.to_string();
                child.version = spec.version.unwrap_or_default();
            }
            None => {
                child.inline = Some(match &self.inline {
                    Some(path) => format!("{path}/{name}"),
                    None => name.to_owned(),
                });
            }
        }
        child.hash = child.content_hash()?;
        Ok(child)
    }

    /// Als JSON für den Blob-Store.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let s: Self =
            serde_json::from_slice(bytes).map_err(|e| SnapshotError::Format(e.to_string()))?;
        if s.format != FORMAT {
            return Err(SnapshotError::Format(format!(
                "Format {} wird nicht unterstützt",
                s.format
            )));
        }
        Ok(s)
    }

    /// Nutzlast für `agent.resolved`: `overrides` und `params` nur, wenn gesetzt.
    pub fn resolved_fields(&self) -> (Option<Value>, Option<Value>) {
        let overrides = (!self.overrides.is_empty())
            .then(|| serde_json::to_value(&self.overrides).unwrap_or(Value::Null));
        let params = (!self.params.is_empty())
            .then(|| serde_json::to_value(&self.params).unwrap_or(Value::Null));
        (overrides, params)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::path::Path;

    use super::*;
    use crate::resolve::Source;

    const AGENT: &str = "spec_version: 1\nname: pr-fixer\nversion: 0.3.0\nexecutor: { harness: claude }\ninstructions: { file: prompts/system.md }\nagents:\n  reviewer: { ref: ./agents/reviewer }\n  shared: { ref: builtin:helper }\n  quick-check:\n    executor: { harness: codex }\n    instructions: { text: \"Nur den Diff.\" }\n";

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture(dir: &Path) -> Located {
        write(&dir.join("agent.yaml"), AGENT);
        write(&dir.join("prompts/system.md"), "Behebe die CI.");
        write(
            &dir.join("agents/reviewer/agent.yaml"),
            "spec_version: 1\nname: reviewer\nversion: 2\nexecutor: { harness: codex }\n",
        );
        Located {
            name: "pr-fixer".into(),
            source: Source::Project,
            dir: AgentDir::Fs(dir.to_path_buf()),
            shadows: None,
        }
    }

    fn builtins() -> Builtins {
        Builtins::from_files([(
            "helper/agent.yaml",
            "spec_version: 1\nname: helper\nexecutor: { harness: claude }\n",
        )])
    }

    #[test]
    fn agt_004_ac1_unchanged_files_give_the_same_hash_and_a_prompt_change_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let located = fixture(tmp.path());
        let first = AgentSnapshot::capture(&located, "pr-fixer", &builtins()).unwrap();
        let second = AgentSnapshot::capture(&located, "pr-fixer", &builtins()).unwrap();
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.to_bytes(), second.to_bytes());
        std::fs::write(
            tmp.path().join("prompts/system.md"),
            "Behebe die CI. Sofort.",
        )
        .unwrap();
        let changed = AgentSnapshot::capture(&located, "pr-fixer", &builtins()).unwrap();
        assert_ne!(first.hash, changed.hash);
    }

    #[test]
    fn hash_without_external_subagents_equals_the_directory_hash() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join("agent.yaml"),
            "spec_version: 1\nname: solo\nexecutor: { harness: claude }\n",
        );
        let located = Located {
            name: "solo".into(),
            source: Source::Path,
            dir: AgentDir::Fs(tmp.path().to_path_buf()),
            shadows: None,
        };
        let s = AgentSnapshot::capture(&located, "./solo", &Builtins::default()).unwrap();
        assert_eq!(s.hash, located.dir.hash());
    }

    #[test]
    fn agt_004_snapshot_is_independent_of_later_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let located = fixture(tmp.path());
        let snap = AgentSnapshot::capture(&located, "pr-fixer", &builtins()).unwrap();
        let bytes = snap.to_bytes();
        std::fs::write(tmp.path().join("prompts/system.md"), "geändert").unwrap();
        std::fs::remove_dir_all(tmp.path().join("agents")).unwrap();
        let back = AgentSnapshot::from_bytes(&bytes).unwrap();
        let (spec, dir) = back.agent().unwrap();
        assert_eq!(spec.name.as_str(), "pr-fixer");
        assert_eq!(dir.read("prompts/system.md").unwrap(), b"Behebe die CI.");
        // Sub-Agents im Verzeichnis, außerhalb (Built-in) und inline.
        let reviewer = back.subagent("reviewer").unwrap();
        assert_eq!(reviewer.entry, "agent/agents/reviewer");
        assert_eq!(reviewer.depth, 1);
        assert_eq!(reviewer.agent().unwrap().0.name.as_str(), "reviewer");
        let shared = back.subagent("shared").unwrap();
        assert_eq!(shared.agent().unwrap().0.name.as_str(), "helper");
        let quick = back.subagent("quick-check").unwrap();
        let (spec, _) = quick.agent().unwrap();
        assert_eq!(spec.executor.harness.as_str(), "codex");
        assert_eq!(
            spec.instructions.unwrap().text.as_deref(),
            Some("Nur den Diff.")
        );
        assert!(back.subagent("fehlt").is_err());
    }

    #[test]
    fn agt_009_nested_inline_subagents_and_depth_limits() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join("agent.yaml"),
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  kind:\n    executor: { harness: codex }\n    agents:\n      enkel:\n        executor: { harness: claude }\n        agents:\n          urenkel: { executor: { harness: codex } }\n        spawn: { agents: [urenkel] }\n    spawn: { agents: [enkel], max_depth: 5 }\nspawn: { agents: [kind], max_depth: 2 }\n",
        );
        let located = Located {
            name: "lead".into(),
            source: Source::Project,
            dir: AgentDir::Fs(tmp.path().to_path_buf()),
            shadows: None,
        };
        let root = AgentSnapshot::capture(&located, "lead", &Builtins::default()).unwrap();
        let (spec, _) = root.agent().unwrap();
        assert_eq!(root.spawn_depth_limit(&spec), 2);
        let child = root.subagent("kind").unwrap();
        let (cspec, _) = child.agent().unwrap();
        assert_eq!(cspec.executor.harness.as_str(), "codex");
        // Die eigene Grenze (1 + 5) verschärft die geerbte nicht.
        assert_eq!(child.spawn_depth_limit(&cspec), 2);
        let grandchild = child.subagent("enkel").unwrap();
        assert_eq!(grandchild.inline.as_deref(), Some("kind/enkel"));
        assert_eq!(grandchild.depth, 2);
        let (gspec, _) = grandchild.agent().unwrap();
        assert_eq!(gspec.executor.harness.as_str(), "claude");
        // Ein Urenkel hätte Tiefe 3 > 2.
        assert_eq!(grandchild.spawn_depth_limit(&gspec), 2);
        assert!(grandchild.depth + 1 > grandchild.spawn_depth_limit(&gspec));
        // Ohne `max_depth`: nur direkte Childs.
        let mut solo = root.clone();
        solo.depth_limit = None;
        let (mut s2, _) = solo.agent().unwrap();
        s2.spawn.as_mut().unwrap().max_depth = None;
        assert_eq!(solo.spawn_depth_limit(&s2), 1);
        // Der Snapshot übersteht die Serialisierung.
        let back = AgentSnapshot::from_bytes(&grandchild.to_bytes()).unwrap();
        assert_eq!(back.depth_limit, Some(2));
    }
}
