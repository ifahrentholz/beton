//! Auflösung einer [`SandboxSpec`] zu einer [`ResolvedPolicy`] (SBX-003, SBX-004): absolute,
//! kanonische Pfade, expandierte Masken, explizites Environment, Netzmodus.
//!
//! Fail closed: Ein Grant, der (über Symlinks) in eine verbotene Wurzel zeigt, ist ein
//! [`SandboxError::PathEscape`]; fehlende Pfade werden nicht freigegeben (Notiz in
//! [`ResolvedPolicy::notes`]); fehlt ein Proxy, gibt es gar kein Netz.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::caps::{self, CapSet};
use crate::error::SandboxError;
use crate::git::GitLayout;
use crate::masks::{DEFAULT_MASKS, DEFAULT_WORKSPACE_MASKS, HarnessGrants, KEYCHAINS};
use crate::paths::{Forbidden, canonical, expand};
use crate::provider::{ProbeReport, ProviderId};
use crate::spec::{SandboxSpec, WorkspaceAccess, env_denied};

/// Stufe einer Sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Stufe 1: der Vendor-Harness-Prozess.
    Harness,
    /// Stufe 2: Tool-Ausführung.
    Tools,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Harness => "harness",
            Stage::Tools => "tools",
        }
    }

    pub fn event(self) -> beton_core::event::SandboxStage {
        match self {
            Stage::Harness => beton_core::event::SandboxStage::Harness,
            Stage::Tools => beton_core::event::SandboxStage::Tools,
        }
    }
}

/// Endpunkt der Proxy-Instanz dieser Stufe (PRX-001); das Token geht nie ins Profil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyEndpoint {
    pub port: u16,
}

/// Kontext der Auflösung.
#[derive(Debug, Clone)]
pub struct ResolveCtx {
    pub stage: Stage,
    /// Workspace bzw. Worktree der Session.
    pub workspace: PathBuf,
    /// Home des Benutzers (für `~`); in Tests ein Temp-Verzeichnis.
    pub home: PathBuf,
    /// Privates, beschreibbares Verzeichnis dieser Stufe (`TMPDIR`).
    pub scratch: PathBuf,
    /// Git-Layout des Workspace (siehe [`GitLayout::discover`]).
    pub git: Option<GitLayout>,
    /// Stufe 1: die Grants des eigenen Harness.
    pub harness: Option<HarnessGrants>,
    /// Stufe 2: die Grants **aller** Adapter (werden maskiert).
    pub adapters: Vec<HarnessGrants>,
    /// Proxy-Instanz dieser Stufe; ohne Proxy gibt es kein Netz.
    pub proxy: Option<ProxyEndpoint>,
    /// Environment des Daemons (Quelle für Basis-Variablen und `passthrough`).
    pub host_env: BTreeMap<String, String>,
}

impl ResolveCtx {
    pub fn new(stage: Stage, workspace: &Path, home: &Path, scratch: &Path) -> Self {
        Self {
            stage,
            workspace: workspace.to_path_buf(),
            home: home.to_path_buf(),
            scratch: scratch.to_path_buf(),
            git: None,
            harness: None,
            adapters: Vec::new(),
            proxy: None,
            host_env: BTreeMap::new(),
        }
    }
}

/// Eine Maske innerhalb freigegebener Wurzeln.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mask {
    /// Fester Pfad (Datei oder Verzeichnis samt Inhalt).
    Path { path: PathBuf, source: String },
    /// Namensmuster (ein Segment mit `*`/`?`) unter `root`, in jeder Tiefe oder nur direkt.
    Pattern {
        root: PathBuf,
        glob: String,
        any_depth: bool,
        source: String,
    },
}

/// Dotfiles direkt in einer Grant-Wurzel sind maskiert, außer `allow` (SBX-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenDotfiles {
    pub root: PathBuf,
    pub allow: Vec<String>,
    /// Wurzel ist beschreibbar (die erlaubten Dotfiles dann auch).
    pub writable: bool,
}

/// Git-Verzeichnisse, deren Hooks und Konfiguration read-only bleiben (SBX-003 AC4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitProtection {
    /// `<workspace>/.git` darf weder ersetzt noch umbenannt werden.
    pub dot_git: PathBuf,
    pub dot_git_is_file: bool,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

/// Netzmodus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetMode {
    /// Gar kein Netz.
    None,
    /// Nur `localhost:<port>` (Proxy-Instanz der Stufe).
    ProxyOnly { port: u16 },
}

/// Aufgelöste, backend-unabhängige Policy einer Stufe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedPolicy {
    pub backend: ProviderId,
    pub stage: Stage,
    pub workspace: PathBuf,
    pub scratch: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub hidden: Vec<HiddenDotfiles>,
    pub masks: Vec<Mask>,
    pub git: Option<GitProtection>,
    /// Mach-Services bzw. Secret Service des Keychains (nur Stufe 1 mit `keychain: true`).
    pub keychain: bool,
    /// Vollständiges, explizites Environment (kein Erben).
    #[serde(skip)]
    pub env: BTreeMap<String, String>,
    pub net: NetMode,
    pub egress_rules: Vec<String>,
    pub egress_allow_private: Vec<String>,
    /// Capabilities, die das Backend erzwingen muss.
    pub required: CapSet,
    /// Ausdrücklich nicht erzwungene Capabilities (Opt-out, z. B. `backend: none`).
    pub degraded: CapSet,
    /// Hinweise der Auflösung (z. B. übersprungene, nicht vorhandene Grants).
    pub notes: Vec<String>,
}

impl ResolvedPolicy {
    /// Policy ohne Isolation (`backend: none`): Bis SBX-006 (WP-49) laufen Harness- und
    /// Tool-Prozesse so, ausdrücklich über den Provider `none`.
    pub fn unsandboxed(stage: Stage) -> Self {
        Self {
            backend: ProviderId::None,
            stage,
            workspace: PathBuf::new(),
            scratch: PathBuf::new(),
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            hidden: Vec::new(),
            masks: Vec::new(),
            git: None,
            keychain: false,
            env: BTreeMap::new(),
            net: NetMode::None,
            egress_rules: Vec::new(),
            egress_allow_private: Vec::new(),
            required: CapSet::new(),
            degraded: caps::required_default(),
            notes: Vec::new(),
        }
    }
}

/// Variablen der Basis-Umgebung, die aus dem Daemon übernommen werden (SBX-005).
const BASE_ENV: [&str; 6] = ["PATH", "USER", "LOGNAME", "SHELL", "TERM", "LANG"];

fn push_unique(list: &mut Vec<PathBuf>, p: PathBuf) {
    if !list.contains(&p) {
        list.push(p);
    }
}

/// Löst einen Grant auf: `None`, wenn er nicht existiert.
fn grant(
    field: &str,
    raw: &str,
    home: &Path,
    forbidden: &Forbidden,
    write: bool,
    notes: &mut Vec<String>,
) -> Result<Option<PathBuf>, SandboxError> {
    let lexical = expand(raw, home);
    if let Some(reason) = forbidden.reason(&lexical, write) {
        return Err(SandboxError::invalid(field, format!("`{raw}` {reason}")));
    }
    let canon = match std::fs::canonicalize(&lexical) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!("{field}: `{raw}` existiert nicht und wird nicht freigegeben"));
            return Ok(None);
        }
        Err(e) => {
            return Err(SandboxError::invalid(field, format!("`{raw}`: {e}")));
        }
    };
    if let Some(reason) = forbidden.reason(&canon, write) {
        return Err(SandboxError::escape(
            field,
            format!("`{raw}` zeigt auf {}, das {reason}", canon.display()),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let meta = std::fs::metadata(&canon).map_err(|e| SandboxError::invalid(field, e.to_string()))?;
        if meta.is_file() && meta.nlink() > 1 {
            return Err(SandboxError::escape(
                field,
                format!("`{raw}` ist ein Hardlink; Dateien mit mehreren Links werden nicht freigegeben"),
            ));
        }
    }
    Ok(Some(canon))
}

/// Ein absoluter Masken-Eintrag (`~/…`, `/…`, Muster im letzten Segment).
fn absolute_mask(raw: &str, home: &Path, source: &str, out: &mut Vec<Mask>) {
    let path = expand(raw, home);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.contains(['*', '?']) {
        let Some(parent) = path.parent() else { return };
        let mut parents = vec![parent.to_path_buf()];
        if let Some(c) = canonical(parent) {
            parents.push(c);
        }
        parents.dedup();
        for root in parents {
            out.push(Mask::Pattern {
                root,
                glob: name.to_owned(),
                any_depth: false,
                source: source.to_owned(),
            });
        }
        return;
    }
    let mut paths = vec![path.clone()];
    if let Some(c) = canonical(&path) {
        paths.push(c);
    }
    paths.dedup();
    for p in paths {
        let m = Mask::Path {
            path: p,
            source: source.to_owned(),
        };
        if !out.contains(&m) {
            out.push(m);
        }
    }
}

/// Löst `spec` für `backend` auf.
pub fn resolve(
    backend: ProviderId,
    spec: &SandboxSpec,
    ctx: &ResolveCtx,
) -> Result<ResolvedPolicy, SandboxError> {
        todo!("SBX-003")
    }

/// Kann das Backend laut Probe alle geforderten Capabilities erzwingen?
pub fn check_caps(probe: &ProbeReport, policy: &ResolvedPolicy) -> Result<(), SandboxError> {
    if probe.id != policy.backend {
        return Err(SandboxError::spawn(format!(
            "Policy für {} an Backend {} übergeben",
            policy.backend, probe.id
        )));
    }
    if !probe.available {
        return Err(SandboxError::Unavailable {
            provider: probe.id,
            missing: policy.required.clone(),
            hint: probe
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "nicht verfügbar".into()),
        });
    }
    let missing: CapSet = policy.required.difference(&probe.caps).copied().collect();
    if missing.is_empty() {
        return Ok(());
    }
    let hint = missing
        .iter()
        .filter_map(|c| probe.degraded_reasons.get(c).map(|r| format!("{c}: {r}")))
        .collect::<Vec<_>>()
        .join("; ");
    Err(SandboxError::Unavailable {
        provider: probe.id,
        missing,
        hint,
    })
}
