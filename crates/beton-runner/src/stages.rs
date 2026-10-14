//! Zweistufige Sandbox einer Session (SBX-002, PRX-001, PRX-005, PRX-008, #191): Proxy-Instanz
//! je Stufe, Stufe-2-Sandbox mit Exec-Broker, Stufe-1-Sandbox für den Harness.
//!
//! ```text
//!   Stufe 0 (Runner)                     Stufe 1 (Harness)          Stufe 2 (Tools)
//!   EgressProxy ─ Instanz harness ◄────── claude (Passthrough: api.anthropic.com …)
//!               └ Instanz tools   ◄────────────────────────────────── npm, git, curl …
//!   Broker  ◄──── bt-exec ─────────────── Bash-Tool ─── Sandbox::spawn ──► /bin/zsh -c …
//! ```
//!
//! Bis WP-49 (SBX-006) greift das Modell nur, wenn `sandbox.backend` ausdrücklich gesetzt und
//! nicht `none` ist; sonst laufen Harness und Tools wie bisher über `backend: none`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beton_harness::{StageEnv, ToolIsolation};
use beton_proxy::{BetonCa, ProxyOptions};
use beton_sandbox::registry::EventSink;
use beton_sandbox::{Sandbox, SandboxError, SandboxSpec, Stage};

/// Einstellungen des Daemons für das Stufenmodell.
pub struct StageOptions {
    /// `sandbox:` der User-Konfiguration (Projekt- und Agent-Ebene folgen mit #200).
    pub spec: SandboxSpec,
    /// beton-CA vom Daemon (über stdin); ohne CA gibt es keinen Proxy und damit keine Session
    /// mit Netz (PRX-002, fail closed).
    pub proxy_ca: Option<BetonCa>,
    /// `proxy.extra_ca_files` u. a.
    pub proxy: ProxyOptions,
    /// beton selbst: `bt-exec` in Stufe 1 und Launcher unter Linux.
    pub exe: PathBuf,
}

impl std::fmt::Debug for StageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageOptions")
            .field("spec", &self.spec)
            .field("proxy_ca", &self.proxy_ca.is_some())
            .finish_non_exhaustive()
    }
}

/// Was der Runner über die Session weiß.
pub struct StageInput<'a> {
    pub session_id: String,
    pub harness: &'a str,
    pub tool_isolation: ToolIsolation,
    pub workdir: &'a Path,
    pub home: PathBuf,
    pub host_env: BTreeMap<String, String>,
    /// Programm des Harness (aufgelöst), für die Lese-Grants der Stufe 1.
    pub program: Option<PathBuf>,
    pub events: Option<EventSink>,
}

/// Die laufenden Stufen einer Session; alles endet mit `Drop`.
pub struct SessionStages {
    pub harness: Sandbox,
    pub tools: Sandbox,
    pub env: StageEnv,
}

impl std::fmt::Debug for SessionStages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStages").finish_non_exhaustive()
    }
}

impl SessionStages {
    /// Passthrough-Ziele der Proxy-Instanz einer Stufe.
    pub fn passthrough(&self, stage: Stage) -> Vec<String> {
        let _ = stage;
        Vec::new()
    }
}

/// Ist das Stufenmodell konfiguriert?
pub fn active(spec: &SandboxSpec) -> bool {
    let _ = spec;
    false
}

/// Baut die Stufen der Session. `Ok(None)`: kein Stufenmodell konfiguriert (bis WP-49).
pub async fn compose(
    input: StageInput<'_>,
    opts: StageOptions,
) -> Result<Option<SessionStages>, SandboxError> {
    let _ = (input, opts);
    Ok(None)
}
