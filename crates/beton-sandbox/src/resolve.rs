//! Auflösung einer [`SandboxSpec`] zu einer [`ResolvedPolicy`] (SBX-003, SBX-004): absolute,
//! kanonische Pfade, expandierte Masken, explizites Environment, Netzmodus.
//!
//! Fail closed: Ein Grant, der (über Symlinks) in eine verbotene Wurzel zeigt, ist ein
//! [`SandboxError::PathEscape`]; fehlende Pfade werden nicht freigegeben (Notiz in
//! [`ResolvedPolicy::notes`]); fehlt ein Proxy, gibt es gar kein Netz.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::caps::{self, CapSet};
use crate::error::SandboxError;
use crate::git::GitLayout;
use crate::masks::{DEFAULT_MASKS, DEFAULT_WORKSPACE_MASKS, HarnessGrants, KEYCHAINS};
use crate::paths::{Forbidden, canonical, expand};
use crate::provider::{ProbeReport, ProviderId};
use crate::spec::{Limits, SandboxSpec, WorkspaceAccess, env_denied};

/// Stufe einer Sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Ein Geheimnis im Environment einer Stufe (Proxy-URL mit Token, Broker-Token): `Debug`
/// zeigt es nie, beim Fallenlassen wird der Speicher überschrieben.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(zeroize::Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

/// Endpunkt der Proxy-Instanz dieser Stufe (PRX-001); das Token geht nie ins Profil, nur als
/// Proxy-URL ins Environment (PRX-008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyEndpoint {
    pub port: u16,
    /// `http://bt:<token>@127.0.0.1:<port>`; ohne URL setzt die Auflösung keine
    /// Proxy-Variablen.
    pub url: Option<Secret>,
    /// CA-Bundle (beton-CA plus System-Roots); in der Sandbox lesbar.
    pub ca_bundle: Option<PathBuf>,
}

impl ProxyEndpoint {
    /// Nur der Port (ohne Proxy-Variablen im Environment).
    pub fn port(port: u16) -> Self {
        Self {
            port,
            url: None,
            ca_bundle: None,
        }
    }
}

/// Zugang der Stufe 1 zum Exec-Broker (SBX-002): Socket, Verzeichnis des `bt-exec`-Shims und
/// Token der Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecAccess {
    pub socket: PathBuf,
    pub shim_dir: PathBuf,
    pub token: Secret,
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
    /// Session (Marker `BETON_SESSION_ID`).
    pub session_id: Option<String>,
    /// Nur Stufe 1: Zugang zum Exec-Broker.
    pub exec: Option<ExecAccess>,
    /// Von beton verwaltete Schreib-Wurzeln, z. B. der Austausch mit Stufe 2.
    pub shared: Vec<PathBuf>,
    /// Lesbare Programme bzw. deren Verzeichnisse (beton für `bt-exec`, die Harness-CLI).
    pub programs: Vec<PathBuf>,
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
            session_id: None,
            exec: None,
            shared: Vec::new(),
            programs: Vec::new(),
        }
    }
}

/// Eine Maske innerhalb freigegebener Wurzeln.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenDotfiles {
    pub root: PathBuf,
    pub allow: Vec<String>,
    /// Wurzel ist beschreibbar (die erlaubten Dotfiles dann auch).
    pub writable: bool,
}

/// Git-Verzeichnisse, deren Hooks und Konfiguration read-only bleiben (SBX-003 AC4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitProtection {
    /// `<workspace>/.git` darf weder ersetzt noch umbenannt werden.
    pub dot_git: PathBuf,
    pub dot_git_is_file: bool,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

/// Netzmodus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Unix-Sockets, mit denen sich die Stufe verbinden darf (Stufe 1: Exec-Broker).
    pub unix_sockets: Vec<PathBuf>,
    /// Vollständiges, explizites Environment (kein Erben).
    #[serde(skip)]
    pub env: BTreeMap<String, String>,
    pub net: NetMode,
    pub egress_rules: Vec<String>,
    pub egress_allow_private: Vec<String>,
    /// Passthrough-Ziele dieser Stufe (PRX-005).
    pub egress_passthrough: Vec<String>,
    /// Loopback-Ports für Dev-Server (`bind`/`connect`, PRX-008 AC3).
    pub dev_ports: Vec<u16>,
    /// Ressourcenlimits des Prozessbaums (SBX-013).
    pub limits: Limits,
    /// Capabilities, die das Backend erzwingen muss.
    pub required: CapSet,
    /// Ausdrücklich nicht erzwungene Capabilities (Opt-out, z. B. `backend: none`).
    pub degraded: CapSet,
    /// Stufe 1 eines Harness, der Tools selbst ausführt (`tool_isolation: inherited`, nur mit
    /// `require_tool_isolation: false`): *degraded* (SBX-002 AC4).
    pub inherited_tools: bool,
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
            unix_sockets: Vec::new(),
            env: BTreeMap::new(),
            net: NetMode::None,
            egress_rules: Vec::new(),
            egress_allow_private: Vec::new(),
            egress_passthrough: Vec::new(),
            dev_ports: Vec::new(),
            limits: Limits::default(),
            required: CapSet::new(),
            degraded: caps::required_default(),
            inherited_tools: false,
            notes: Vec::new(),
        }
    }
}

/// Variablen der Basis-Umgebung, die aus dem Daemon übernommen werden (SBX-005).
const BASE_ENV: [&str; 6] = ["PATH", "USER", "LOGNAME", "SHELL", "TERM", "LANG"];

/// Proxy-Variablen (PRX-008), groß und klein.
pub const PROXY_ENV: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// Ausnahmen vom Proxy (PRX-008).
pub const NO_PROXY_ENV: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// CA-Variablen gängiger Toolchains; alle zeigen auf das CA-Bundle der Stufe (PRX-008).
pub const CA_ENV: [&str; 9] = [
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "GIT_SSL_CAINFO",
    "PIP_CERT",
    "CARGO_HTTP_CAINFO",
    "AWS_CA_BUNDLE",
    "DENO_CERT",
];

/// Loopback, wenn er sandbox-lokal ist (Linux) bzw. Dev-Ports freigegeben sind.
pub const NO_PROXY_LOOPBACK: &str = "localhost,127.0.0.1,::1";

/// Variablen, die beton je Stufe selbst setzt: weder `env.set` noch Overrides an den
/// Exec-Broker dürfen sie ändern (SBX-002 AC3, SBX-005, PRX-008).
pub fn controlled_env(name: &str) -> bool {
    name.starts_with("BETON_")
        || PROXY_ENV.contains(&name)
        || NO_PROXY_ENV.contains(&name)
        || CA_ENV.contains(&name)
        || matches!(
            name,
            "HOME"
                | "TMPDIR"
                | "XDG_RUNTIME_DIR"
                | "SSL_CERT_DIR"
                | "NODE_USE_ENV_PROXY"
                | "PIP_USE_DEPRECATED"
        )
}

/// Von beton verwaltete Worktrees (SES-015) unter dem sonst maskierten `~/.beton`.
pub const BETON_WORKTREES: &str = "~/.beton/worktrees";

/// Liegt `p` als eigener Worktree unter `<home>/.beton/worktrees/<…>`?
fn beton_worktree(p: &Path, home: &Path) -> bool {
    let root = crate::paths::expand(BETON_WORKTREES, home);
    p.starts_with(&root) && p != root
}

/// Ein beton-eigener Grant (Programm, Austausch): kanonisch, vorhanden, nicht verboten. Anders
/// als Grants der Konfiguration dürfen Programme mehrere Hardlinks haben (Cargo verlinkt
/// Binaries hart), sie sind nur lesbar.
fn own_grant(
    field: &str,
    p: &Path,
    forbidden: &Forbidden,
    write: bool,
) -> Result<PathBuf, SandboxError> {
    let canon = std::fs::canonicalize(p)
        .map_err(|e| SandboxError::invalid(field, format!("{}: {e}", p.display())))?;
    if let Some(reason) = forbidden.reason(&canon, write) {
        return Err(SandboxError::escape(
            field,
            format!("{} {reason}", canon.display()),
        ));
    }
    Ok(canon)
}

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
            notes.push(format!(
                "{field}: `{raw}` existiert nicht und wird nicht freigegeben"
            ));
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
        let meta =
            std::fs::metadata(&canon).map_err(|e| SandboxError::invalid(field, e.to_string()))?;
        if meta.is_file() && meta.nlink() > 1 {
            return Err(SandboxError::escape(
                field,
                format!(
                    "`{raw}` ist ein Hardlink; Dateien mit mehreren Links werden nicht freigegeben"
                ),
            ));
        }
    }
    Ok(Some(canon))
}

/// Ein absoluter Masken-Eintrag (`~/…`, `/…`, Muster im letzten Segment).
fn absolute_mask(raw: &str, home: &Path, source: &str, out: &mut Vec<Mask>) {
    let path = expand(raw, home);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
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
    spec.validate()?;
    // Das Preset gilt immer, auch wenn der Aufrufer nicht gemergt hat (SBX-012); auf eine
    // schon gemergte Konfiguration angewendet ändert es nichts.
    let merged;
    let spec = if spec.preset.is_some() {
        merged = SandboxSpec::merged(&[spec]);
        &merged
    } else {
        spec
    };
    let home = std::fs::canonicalize(&ctx.home)
        .map_err(|e| SandboxError::invalid("home", format!("{}: {e}", ctx.home.display())))?;
    let forbidden = Forbidden::new(&home);
    let mut notes = Vec::new();

    let writable_ws = spec.workspace.unwrap_or_default() == WorkspaceAccess::Rw;
    let workspace = std::fs::canonicalize(&ctx.workspace).map_err(|e| {
        SandboxError::invalid("workspace", format!("{}: {e}", ctx.workspace.display()))
    })?;
    if !workspace.is_dir() {
        return Err(SandboxError::invalid(
            "workspace",
            format!("{} ist kein Verzeichnis", workspace.display()),
        ));
    }
    if !beton_worktree(&workspace, &home)
        && let Some(reason) = forbidden.reason(&workspace, writable_ws)
    {
        return Err(SandboxError::invalid(
            "workspace",
            format!("{} {reason}", workspace.display()),
        ));
    }
    let scratch = std::fs::canonicalize(&ctx.scratch)
        .map_err(|e| SandboxError::invalid("scratch", format!("{}: {e}", ctx.scratch.display())))?;
    if let Some(reason) = forbidden.reason(&scratch, true) {
        return Err(SandboxError::invalid(
            "scratch",
            format!("{} {reason}", scratch.display()),
        ));
    }

    // Grants aus der Konfiguration.
    let mut read_roots = Vec::new();
    let mut write_roots = Vec::new();
    let mut grant_roots: Vec<(PathBuf, bool)> = Vec::new();
    for (i, raw) in spec.read_paths.iter().flatten().enumerate() {
        if let Some(p) = grant(
            &format!("read_paths[{i}]"),
            raw,
            &home,
            &forbidden,
            false,
            &mut notes,
        )? {
            push_unique(&mut read_roots, p.clone());
            grant_roots.push((p, false));
        }
    }
    for (i, raw) in spec.write_paths.iter().flatten().enumerate() {
        if let Some(p) = grant(
            &format!("write_paths[{i}]"),
            raw,
            &home,
            &forbidden,
            true,
            &mut notes,
        )? {
            push_unique(&mut write_roots, p.clone());
            grant_roots.push((p, true));
        }
    }
    let allow_hidden = spec.allow_hidden.clone().unwrap_or_default();
    let hidden = grant_roots
        .iter()
        .map(|(root, writable)| HiddenDotfiles {
            root: root.clone(),
            allow: allow_hidden.clone(),
            writable: *writable,
        })
        .collect();

    // Workspace, Scratch und Git.
    if writable_ws {
        push_unique(&mut write_roots, workspace.clone());
    } else {
        push_unique(&mut read_roots, workspace.clone());
    }
    push_unique(&mut write_roots, scratch.clone());
    let git = ctx.git.as_ref().map(|g| GitProtection {
        dot_git: g.dot_git.clone(),
        dot_git_is_file: g.dot_git_is_file,
        git_dir: g.git_dir.clone(),
        common_dir: g.common_dir.clone(),
    });
    if let Some(g) = &ctx.git {
        if !g.dot_git.starts_with(&workspace) {
            return Err(SandboxError::invalid(
                "workspace",
                "das Git-Layout gehört nicht zu diesem Workspace",
            ));
        }
        for root in g.write_roots() {
            let field = "workspace (git)";
            let p = grant(
                field,
                &root.to_string_lossy(),
                &home,
                &forbidden,
                writable_ws,
                &mut notes,
            )?;
            if let Some(p) = p {
                if writable_ws {
                    push_unique(&mut write_roots, p);
                } else {
                    push_unique(&mut read_roots, p);
                }
            }
        }
        if g.dot_git_is_file {
            // Hooks und Konfiguration des Repos lesen (`git` braucht sie).
            if let Some(p) = grant(
                "workspace (git)",
                &g.common_dir.to_string_lossy(),
                &home,
                &forbidden,
                false,
                &mut notes,
            )? {
                push_unique(&mut read_roots, p);
            }
        }
    }

    // Stufe 1: eigene Credentials (lesen) und State (lesen/schreiben).
    let own = match ctx.stage {
        Stage::Harness => ctx.harness.clone(),
        Stage::Tools => None,
    };
    if let Some(h) = &own {
        for (i, raw) in h.credential_paths.iter().enumerate() {
            let field = format!("harness.{}.credential_paths[{i}]", h.harness);
            if let Some(p) = grant(&field, raw, &home, &forbidden, false, &mut notes)? {
                push_unique(&mut read_roots, p);
            }
        }
        for (i, raw) in h.state_paths.iter().enumerate() {
            let field = format!("harness.{}.state_paths[{i}]", h.harness);
            if let Some(p) = grant(&field, raw, &home, &forbidden, true, &mut notes)? {
                push_unique(&mut write_roots, p);
            }
        }
    }
    let keychain = own.as_ref().is_some_and(|h| h.keychain);
    // Keychain (macOS): Die Keychain-Dateien liest und aktualisiert der Client selbst (z. B.
    // beim Erneuern eines Logins); geschützt sind einzelne Einträge nur durch ihre ACLs
    // (Restrisiko, siehe Bedrohungsmodell). Dazu die Suchliste des Benutzers.
    if keychain && cfg!(target_os = "macos") {
        for (raw, write) in [
            (KEYCHAINS, true),
            ("~/Library/Preferences/com.apple.security.plist", false),
        ] {
            let field = format!("harness.keychain ({raw})");
            if let Some(p) = grant(&field, raw, &home, &forbidden, write, &mut notes)? {
                if write {
                    push_unique(&mut write_roots, p);
                } else {
                    push_unique(&mut read_roots, p);
                }
            }
        }
    }

    // Von beton verwaltete Grants: Austausch mit Stufe 2, Programme, Broker-Zugang (nur
    // Stufe 1).
    for (i, p) in ctx.shared.iter().enumerate() {
        let p = own_grant(&format!("shared[{i}]"), p, &forbidden, true)?;
        push_unique(&mut write_roots, p);
    }
    for (i, p) in ctx.programs.iter().enumerate() {
        let field = format!("programs[{i}]");
        let canon = own_grant(&field, p, &forbidden, false)?;
        push_unique(&mut read_roots, canon);
        // Ein Programm, das über einen Symlink aufgerufen wird (z. B. `~/.local/bin/claude`),
        // braucht auch den Link selbst (sonst scheitert `execve` unter Seatbelt).
        if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            if let Some(reason) = forbidden.reason(p, false) {
                return Err(SandboxError::escape(
                    field,
                    format!("{} {reason}", p.display()),
                ));
            }
            push_unique(&mut read_roots, p.clone());
        }
    }
    let mut unix_sockets = Vec::new();
    if let Some(exec) = &ctx.exec {
        if ctx.stage != Stage::Harness {
            return Err(SandboxError::invalid(
                "exec",
                "Zugang zum Exec-Broker gibt es nur in Stufe 1 (SBX-002)",
            ));
        }
        let shim = own_grant("exec.shim_dir", &exec.shim_dir, &forbidden, false)?;
        push_unique(&mut read_roots, shim);
        let dir = exec
            .socket
            .parent()
            .ok_or_else(|| SandboxError::invalid("exec.socket", "Socket ohne Verzeichnis"))?;
        let dir = own_grant("exec.socket", dir, &forbidden, false)?;
        let name = exec
            .socket
            .file_name()
            .ok_or_else(|| SandboxError::invalid("exec.socket", "Socket ohne Namen"))?;
        // Das Verzeichnis muss unter Linux sichtbar sein (Mount-Namespace); verbunden wird
        // nur mit dem Socket selbst.
        push_unique(&mut read_roots, dir.clone());
        unix_sockets.push(dir.join(name));
    }

    // Masken: Default (Stufe 1 und 2), in Stufe 2 alle Adapter-Pfade, eigene Masken.
    let mut masks = Vec::new();
    for raw in DEFAULT_MASKS {
        if raw == "$XDG_RUNTIME_DIR" {
            if let Some(dir) = ctx
                .host_env
                .get("XDG_RUNTIME_DIR")
                .filter(|d| d.starts_with('/'))
            {
                absolute_mask(dir, &home, "default", &mut masks);
            }
            continue;
        }
        if keychain && raw == KEYCHAINS {
            continue;
        }
        absolute_mask(raw, &home, "default", &mut masks);
    }
    if ctx.stage == Stage::Tools {
        for a in &ctx.adapters {
            for raw in a.credential_paths.iter().chain(&a.state_paths) {
                absolute_mask(raw, &home, &format!("harness:{}", a.harness), &mut masks);
            }
        }
    }
    let user_masks: Vec<String> = match &spec.masks {
        Some(m) => m.clone(),
        None => DEFAULT_WORKSPACE_MASKS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    };
    let mut pattern_roots: Vec<PathBuf> = vec![workspace.clone()];
    for (root, _) in &grant_roots {
        push_unique(&mut pattern_roots, root.clone());
    }
    for raw in &user_masks {
        if raw.starts_with('~') || raw.starts_with('/') {
            absolute_mask(raw, &home, "masks", &mut masks);
            continue;
        }
        let glob = raw.strip_suffix('/').unwrap_or(raw).to_owned();
        // `allow_hidden` hebt ein gleichnamiges Muster auf (SBX-004 AC2).
        if allow_hidden.contains(&glob) {
            continue;
        }
        for root in &pattern_roots {
            masks.push(Mask::Pattern {
                root: root.clone(),
                glob: glob.clone(),
                any_depth: true,
                source: "masks".into(),
            });
        }
    }
    // Ein Workspace oder Grant in einem maskierten Pfad wäre unsichtbar bzw. eine Lücke.
    for m in &masks {
        if let Mask::Path { path, .. } = m {
            if workspace.starts_with(path) && !beton_worktree(&workspace, &home) {
                return Err(SandboxError::invalid(
                    "workspace",
                    format!(
                        "{} liegt in der Maske {}",
                        workspace.display(),
                        path.display()
                    ),
                ));
            }
            for root in read_roots.iter().chain(&write_roots) {
                if root.starts_with(path) && !beton_worktree(root, &home) {
                    return Err(SandboxError::escape(
                        "grants",
                        format!("{} liegt in der Maske {}", root.display(), path.display()),
                    ));
                }
            }
        }
    }

    // Environment (explizit, kein Erben).
    let mut env = BTreeMap::new();
    for k in BASE_ENV {
        if let Some(v) = ctx.host_env.get(k) {
            env.insert(k.to_owned(), v.clone());
        }
    }
    env.entry("PATH".to_owned())
        .or_insert_with(|| "/usr/bin:/bin:/usr/sbin:/sbin".to_owned());
    for (k, v) in &ctx.host_env {
        if k.starts_with("LC_") || k == "TZ" {
            env.insert(k.clone(), v.clone());
        }
    }
    // Stufe 1: die Env-Familie des eigenen Harness (z. B. `CLAUDE_CONFIG_DIR`).
    if let Some(h) = &own {
        for (k, v) in &ctx.host_env {
            if h.env_families.iter().any(|f| k.starts_with(f.as_str())) {
                env.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(cfg) = &spec.env {
        for pattern in cfg.passthrough.iter().flatten() {
            match pattern.strip_suffix('*') {
                Some(prefix) => {
                    for (k, v) in &ctx.host_env {
                        if k.starts_with(prefix) && !env_denied(k) {
                            env.insert(k.clone(), v.clone());
                        }
                    }
                }
                None => {
                    if let Some(v) = ctx.host_env.get(pattern) {
                        env.insert(pattern.clone(), v.clone());
                    }
                }
            }
        }
        for (k, v) in &cfg.set {
            env.insert(k.clone(), v.clone());
        }
    }
    env.retain(|k, _| !env_denied(k) && !controlled_env(k));
    env.insert("HOME".into(), home.to_string_lossy().into_owned());
    env.insert("TMPDIR".into(), scratch.to_string_lossy().into_owned());
    // `XDG_RUNTIME_DIR` des Hosts nie (SBX-005), stattdessen der Scratch der Stufe.
    env.insert(
        "XDG_RUNTIME_DIR".into(),
        scratch.to_string_lossy().into_owned(),
    );
    env.insert("BETON_STAGE".into(), ctx.stage.as_str().into());
    if let Some(id) = &ctx.session_id {
        env.insert("BETON_SESSION_ID".into(), id.clone());
    }
    if let Some(exec) = &ctx.exec {
        env.insert(
            "BETON_EXEC_SOCKET".into(),
            unix_sockets
                .first()
                .unwrap_or(&exec.socket)
                .to_string_lossy()
                .into_owned(),
        );
        env.insert("BETON_EXEC_TOKEN".into(), exec.token.expose().to_owned());
    }

    // Netz: nur über den Proxy, ohne Proxy gar nicht.
    let net = match (spec.allow_network.unwrap_or(true), ctx.proxy.as_ref()) {
        (true, Some(p)) => NetMode::ProxyOnly { port: p.port },
        (true, None) => {
            notes.push("allow_network: kein Proxy angebunden, daher kein Netz".into());
            NetMode::None
        }
        (false, _) => NetMode::None,
    };
    let dev_ports = spec.dev_ports.clone().unwrap_or_default();
    // Proxy- und CA-Variablen für die Toolchains (PRX-008).
    if let (NetMode::ProxyOnly { .. }, Some(p)) = (net, ctx.proxy.as_ref()) {
        if let Some(url) = &p.url {
            for k in PROXY_ENV {
                env.insert(k.into(), url.expose().to_owned());
            }
            // Unter Linux ist Loopback sandbox-lokal (eigener Net-Namespace); unter macOS teilt
            // die Sandbox das Loopback des Hosts, dort geht es über den Proxy, außer für
            // ausdrücklich freigegebene Dev-Ports (die Seatbelt-Regel sperrt alle anderen).
            let local = matches!(backend, ProviderId::Landlock | ProviderId::Bwrap)
                || !dev_ports.is_empty();
            for k in NO_PROXY_ENV {
                env.insert(k.into(), if local { NO_PROXY_LOOPBACK } else { "" }.into());
            }
            env.insert("NODE_USE_ENV_PROXY".into(), "1".into());
        }
        if let Some(bundle) = &p.ca_bundle {
            let bundle = own_grant("proxy.ca_bundle", bundle, &forbidden, false)?;
            for k in CA_ENV {
                env.insert(k.into(), bundle.to_string_lossy().into_owned());
            }
            // pip ≥ 24.2 prüft unter macOS mit `truststore` gegen den Trust-Store des Systems
            // (ohne beton-CA) statt gegen `PIP_CERT`; unter Linux wertet `truststore`
            // `SSL_CERT_FILE` aus. Ältere pip kennen den Wert nicht, daher nur unter macOS.
            if backend == ProviderId::Seatbelt {
                env.insert("PIP_USE_DEPRECATED".into(), "legacy-certs".into());
            }
            push_unique(&mut read_roots, bundle);
        }
    }

    let (required, degraded) = if backend == ProviderId::None {
        (CapSet::new(), caps::required_default())
    } else {
        (caps::required_default(), CapSet::new())
    };
    Ok(ResolvedPolicy {
        backend,
        stage: ctx.stage,
        workspace,
        scratch,
        read_roots,
        write_roots,
        hidden,
        masks,
        git,
        keychain,
        unix_sockets,
        env,
        net,
        egress_rules: spec.egress_rules.clone().unwrap_or_default(),
        egress_allow_private: spec.egress_allow_private.clone().unwrap_or_default(),
        egress_passthrough: spec.egress_passthrough.clone().unwrap_or_default(),
        dev_ports,
        limits: spec.limits.unwrap_or_default(),
        required,
        degraded,
        inherited_tools: false,
        notes,
    })
}

/// Markiert Limits, die das Backend laut Probe nicht durchsetzen kann, als *degraded* (sichtbar
/// in `sandbox.started` und `explain`). Ressourcenlimits sind kein Pflicht-Cap: Seatbelt kennt
/// kein Speicherlimit, Linux nur mit cgroup-Delegation (SBX-013 AC3).
pub fn mark_unenforced_limits(probe: &ProbeReport, policy: &mut ResolvedPolicy) {
    if policy.limits.memory_mb.is_some() && !probe.caps.contains(&crate::caps::Cap::Resources) {
        policy.degraded.insert(crate::caps::Cap::Resources);
        let why = probe
            .degraded_reasons
            .get(&crate::caps::Cap::Resources)
            .cloned()
            .unwrap_or_else(|| "nicht durchsetzbar".into());
        policy
            .notes
            .push(format!("limits.memory_mb wird nicht durchgesetzt: {why}"));
    }
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
