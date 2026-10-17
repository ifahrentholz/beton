//! Runner: führt genau eine Session aus, supervidiert den Harness und streamt Events über
//! den Tunnel an den Home-Knoten (RUN-002, RUN-003, PROTO-015).
//!
//! Der Runner baut nur **ausgehende** Verbindungen auf (Unix-Socket des lokalen Daemons) und
//! öffnet keinen Port. Unbestätigte Events hält er bis 64 MiB vor und sendet sie nach einem
//! Reconnect erneut; darüber pausiert er das Lesen vom Harness.

mod agent_sandbox;
mod credentials;
pub mod fork;
pub mod harden;
pub mod mcp;
pub mod policy;
mod push_config;
pub mod settings;
pub mod stages;
pub mod state;
pub mod terminals;
mod token;
mod vendor_mcp;
pub mod workspace;

pub use token::RunnerToken;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, CompactionTrigger, Event, EventPayload, FsChange, FsChanged, SessionStatus,
    SessionStatusChanged,
};
use beton_core::id::{RunnerId, SessionId, TurnId};
use beton_harness::process::SandboxLauncher;
use beton_harness::registry::{HarnessLayers, Registry, RegistryOptions};
use beton_harness::{
    AdapterContext, GateRequest, HarnessId, HarnessSession, HostEnv, NormalizedEvent, SessionSpec,
    Shutdown, UserInput,
};
use beton_proto::tunnel::{PROTOCOL, PeerKind, RseqEvent, SUBPROTOCOL, TunnelDown, TunnelUp};
use beton_proto::ws::Backoff;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::state::{Lifecycle, RunnerState};

/// Obergrenze für unbestätigte Events (PROTO-015).
pub const MAX_UNACKED_BYTES: usize = 64 * 1024 * 1024;

/// Umgebungsvariablen, mit denen der Host den Runner startet.
pub mod env {
    pub const SOCKET: &str = "BETON_TUNNEL_SOCKET";
    pub const SESSION: &str = "BETON_SESSION_ID";
    pub const RUNNER: &str = "BETON_RUNNER_ID";
    pub const EPOCH: &str = "BETON_EPOCH";
    pub const HARNESS: &str = "BETON_HARNESS";
    pub const SCENARIO: &str = "BETON_SCENARIO";
    pub const MODEL: &str = "BETON_MODEL";
    /// Reasoning-Effort beim Start (HAR-017).
    pub const EFFORT: &str = "BETON_EFFORT";
    /// Permission-Mode beim Start (HAR-027); `yolo` startet ohne Sandbox nie.
    pub const PERMISSION_MODE: &str = "BETON_PERMISSION_MODE";
    pub const PARENT_PID: &str = "BETON_RUNNER_PARENT_PID";
    pub const DEV: &str = "BETON_DEV";
    /// Native Session-Referenz zum Fortsetzen (SES-003).
    pub const RESUME: &str = "BETON_RESUME";
    /// `harnesses:` aus User- und Projekt-Konfiguration als JSON (HAR-003, HAR-015).
    pub const HARNESSES: &str = "BETON_RUNNER_HARNESSES";
    /// Agent der Session (`agent_ref`); bestimmt Tools, System-Tools und Skills (HAR-009).
    pub const AGENT_REF: &str = "BETON_AGENT_REF";
    /// Agent-Snapshot der Session als JSON-Datei (AGT-004); hat Vorrang vor `AGENT_REF`.
    pub const AGENT_SNAPSHOT: &str = "BETON_AGENT_SNAPSHOT";
    /// Schatten-Repository für Turn-Snapshots und `fs.changed` (SES-017, SES-018).
    pub const SNAPSHOTS: &str = "BETON_SNAPSHOTS";
    /// Fork-Plan als JSON-Datei (HAR-018, HAR-019).
    pub const FORK_PLAN: &str = "BETON_FORK_PLAN";
    /// `1`: Das Ergebnis des Fork-Plans steht schon im Log.
    pub const FORK_LOGGED: &str = "BETON_FORK_LOGGED";
    /// `sandbox:` der User-Konfiguration und `proxy.extra_ca_files` als JSON (SBX-002, #191).
    pub const SANDBOX: &str = "BETON_RUNNER_SANDBOX";
    /// `1`: `backend: none` ist für diese Session bestätigt (SBX-006, Audit beim Server).
    pub const UNSANDBOXED_CONFIRMED: &str = "BETON_UNSANDBOXED_CONFIRMED";
    /// Nur mit dem Feature `test-sandbox`: Backends abschalten (Tests, SBX-006 AC1).
    #[cfg(feature = "test-sandbox")]
    pub const TEST_SANDBOX_UNAVAILABLE: &str = beton_sandbox::TEST_UNAVAILABLE_ENV;
    /// Nur mit dem Feature `test-sandbox`: `1` beendet die Proxy-Instanzen der Session gleich
    /// nach dem Start, wie ein Absturz (Tests, SBX-006 AC3).
    #[cfg(feature = "test-sandbox")]
    pub const TEST_SANDBOX_PROXY_DOWN: &str = "BETON_TEST_SANDBOX_PROXY_DOWN";
    /// `compaction:` der User-Konfiguration als JSON (USE-009); fehlt sie, gilt der Default.
    pub const COMPACTION: &str = "BETON_COMPACTION";
    /// Session-Budget in USD (API-006 `--max-cost`); der Runner setzt es als `spend_cap` der
    /// User-Ebene durch (POL-011).
    pub const MAX_COST: &str = "BETON_MAX_COST";
    /// `1`: Child einer `session_spawn` (`start.trigger = "spawn"`, POL-011).
    pub const SPAWNED: &str = "BETON_SPAWNED";
    /// Großzügiges Zeitbudget der Policy-Engine für Integrationstests (#298); nur Builds mit
    /// dem Feature `test-budget` reichen es an den Runner weiter und lesen es (#304).
    #[cfg(feature = "test-budget")]
    pub const TEST_POLICY_BUDGET: &str = beton_policy::cel::TEST_BUDGET_ENV;
}

/// Startparameter eines Runners. Das Token kommt über stdin, nie über Env oder argv
/// (AUTH-011).
#[derive(Debug, Clone)]
pub struct RunnerBoot {
    pub socket: PathBuf,
    /// Aktuelles Runner-Token; Erneuerungen über den Tunnel ersetzen es (AUTH-011 AC2).
    pub token: RunnerToken,
    pub session_id: SessionId,
    pub runner_id: RunnerId,
    pub epoch: u64,
    pub harness: HarnessId,
    pub workdir: PathBuf,
    pub scenario: Option<PathBuf>,
    pub model: Option<String>,
    /// Effort beim Start, noch ungemappt (HAR-017).
    pub effort: Option<String>,
    /// Permission-Mode beim Start (HAR-027).
    pub permission_mode: Option<String>,
    pub parent_pid: Option<u32>,
    pub dev: bool,
    pub resume: Option<String>,
    pub harnesses: HarnessLayers,
    /// Agent-Ref der Session (AGT-003), z. B. ein Pfad oder `builtin:<name>`.
    pub agent_ref: Option<String>,
    /// Agent-Snapshot aus `agent.resolved` (AGT-004); der Runner liest den Agent nur daraus.
    pub agent_snapshot: Option<PathBuf>,
    /// Schatten-Repository für Turn-Snapshots und `fs.changed` (SES-017, SES-018).
    pub snapshots: Option<PathBuf>,
    /// API-Keys für `api_key_env` (HAR-011): kommen als zweite stdin-Zeile vom Daemon, nie
    /// über Env oder argv; `Debug` zeigt nur die Namen.
    pub credentials: beton_harness_direct::secret::KeyMap,
    /// Fork-Plan (HAR-018, HAR-019).
    pub fork: Option<PathBuf>,
    /// Das Ergebnis des Fork-Plans steht schon im Log.
    pub fork_logged: bool,
    /// Zweistufige Sandbox (SBX-002). `None` heißt Default-Konfiguration: Sandbox
    /// erforderlich (SBX-006); ohne Isolation nur mit ausdrücklichem `backend: none`.
    pub sandbox: Option<stages::SandboxBoot>,
    /// `backend: none` ist für diese Session bestätigt (SBX-006); ohne Bestätigung startet
    /// keine Session ohne Sandbox.
    pub unsandboxed_confirmed: bool,
    /// beton-CA für die Proxy-Instanzen der Session (dritte stdin-Zeile, PRX-002).
    pub proxy_ca: Option<stages::CaText>,
    /// Automatische Compaction (USE-009).
    pub compaction: beton_harness::CompactionSettings,
    /// Session-Budget in USD (API-006 `--max-cost`, POL-011).
    pub max_cost_usd: Option<f64>,
    /// Child einer `session_spawn`.
    pub spawned: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("Startparameter fehlen oder sind ungültig: {0}")]
    Boot(String),
    #[error("Harness: {0}")]
    Harness(#[from] beton_harness::HarnessError),
    #[error("Tunnel: {0}")]
    Tunnel(String),
    /// Der Tunnel lehnt das Token ab (401: abgelaufen oder gesperrt, #243).
    #[error("Tunnel: Token abgelehnt (401)")]
    Unauthorized,
    #[error("stale_epoch: Session gehört inzwischen einem anderen Schreiber")]
    StaleEpoch,
}

impl RunnerBoot {
    /// Liest Env, das Token (erste stdin-Zeile) und optional die API-Keys (zweite Zeile,
    /// JSON `{"VARIABLE": "wert"}`, HAR-011).
    pub fn from_env_and_stdin() -> Result<Self, RunnerError> {
        let var = |k: &str| std::env::var(k).map_err(|_| RunnerError::Boot(format!("{k} fehlt")));
        let parse = |k: &str| -> Result<String, RunnerError> { var(k) };
        let mut token = zeroize::Zeroizing::new(String::new());
        std::io::stdin()
            .read_line(&mut token)
            .map_err(|e| RunnerError::Boot(format!("Token von stdin: {e}")))?;
        let mut keys = String::new();
        // Ohne zweite Zeile (stdin geschlossen) gibt es keine Keys.
        let _ = std::io::stdin().read_line(&mut keys);
        let credentials = beton_harness_direct::secret::KeyMap::from_json_line(&keys);
        keys.clear();
        // Dritte Zeile: beton-CA für die Proxy-Instanzen (PRX-002), nur mit Stufenmodell.
        let mut ca = String::new();
        let _ = std::io::stdin().read_line(&mut ca);
        let proxy_ca = Some(ca.trim())
            .filter(|c| !c.is_empty())
            .map(|c| stages::CaText::new(c.to_owned()));
        zeroize::Zeroize::zeroize(&mut ca);
        Ok(Self {
            socket: PathBuf::from(var(env::SOCKET)?),
            token: RunnerToken::new(token.trim().to_owned()),
            session_id: parse(env::SESSION)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            runner_id: parse(env::RUNNER)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            epoch: parse(env::EPOCH)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            harness: parse(env::HARNESS)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            workdir: std::env::current_dir().map_err(|e| RunnerError::Boot(e.to_string()))?,
            scenario: std::env::var_os(env::SCENARIO).map(PathBuf::from),
            model: std::env::var(env::MODEL).ok(),
            effort: std::env::var(env::EFFORT).ok().filter(|v| !v.is_empty()),
            permission_mode: std::env::var(env::PERMISSION_MODE)
                .ok()
                .filter(|v| !v.is_empty()),
            parent_pid: std::env::var(env::PARENT_PID)
                .ok()
                .and_then(|p| p.parse().ok()),
            dev: std::env::var(env::DEV).is_ok_and(|v| v == "1"),
            resume: std::env::var(env::RESUME).ok().filter(|r| !r.is_empty()),
            harnesses: match std::env::var(env::HARNESSES) {
                Ok(json) => serde_json::from_str(&json)
                    .map_err(|e| RunnerError::Boot(format!("{}: {e}", env::HARNESSES)))?,
                Err(_) => HarnessLayers::default(),
            },
            agent_ref: std::env::var(env::AGENT_REF).ok().filter(|r| !r.is_empty()),
            agent_snapshot: std::env::var_os(env::AGENT_SNAPSHOT)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            snapshots: std::env::var_os(env::SNAPSHOTS)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            credentials,
            fork: std::env::var_os(env::FORK_PLAN)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            fork_logged: std::env::var(env::FORK_LOGGED).is_ok_and(|v| v == "1"),
            sandbox: match std::env::var(env::SANDBOX) {
                Ok(json) => Some(
                    serde_json::from_str(&json)
                        .map_err(|e| RunnerError::Boot(format!("{}: {e}", env::SANDBOX)))?,
                ),
                Err(_) => None,
            },
            unsandboxed_confirmed: std::env::var(env::UNSANDBOXED_CONFIRMED)
                .is_ok_and(|v| v == "1"),
            proxy_ca,
            compaction: match std::env::var(env::COMPACTION) {
                Ok(json) => serde_json::from_str(&json)
                    .map_err(|e| RunnerError::Boot(format!("{}: {e}", env::COMPACTION)))?,
                Err(_) => beton_harness::CompactionSettings::default(),
            },
            // Ein unlesbares Budget ist ein Startfehler, nie „kein Budget“ (fail closed).
            max_cost_usd: match std::env::var(env::MAX_COST) {
                Ok(v) => Some(
                    v.parse()
                        .map_err(|e| RunnerError::Boot(format!("{}: {e}", env::MAX_COST)))?,
                ),
                Err(_) => None,
            },
            spawned: std::env::var(env::SPAWNED).is_ok_and(|v| v == "1"),
        })
    }

    /// Harness-Registry dieses Runners; `direct:*` nutzt die übergebenen Keys.
    pub fn registry(&self) -> Registry {
        builtin_registry_with_options(
            &self.harnesses,
            self.dev,
            &beton_harness_direct::DirectOptions {
                keys: Arc::new(self.credentials.clone()),
                discover_models: false,
                egress: None,
            },
        )
        .0
    }
}

/// Proxy-Weg des Direkt-API-Harness dieser Session (HAR-011 AC2): nur für Provider mit Key.
fn direct_route(boot: &RunnerBoot) -> Option<stages::DirectRoute> {
    let name = boot.harness.as_str().strip_prefix("direct:")?;
    let p = beton_harness_direct::config::parse_provider(name, boot.harnesses.providers.get(name)?)
        .ok()?;
    p.key.lookup()?;
    // Nur https: Loopback-Endpunkte über `http` behalten den Key über stdin (HAR-011).
    if p.base_url.scheme() != "https" {
        return None;
    }
    Some(stages::DirectRoute {
        binding: format!("provider:{name}"),
        host: p.base_url.host_str()?.to_owned(),
        port: p.base_url.port_or_known_default()?,
    })
}

/// Der Direkt-API-Harness der Session mit Platzhalter statt Key und Weg über den Proxy.
fn direct_via_proxy(boot: &RunnerBoot, registry: &mut Registry, egress: &stages::DirectEgress) {
    let Some(name) = boot.harness.as_str().strip_prefix("direct:") else {
        return;
    };
    let Some(Ok(p)) = boot
        .harnesses
        .providers
        .get(name)
        .map(|v| beton_harness_direct::config::parse_provider(name, v))
    else {
        return;
    };
    let Some(lookup) = p.key.lookup().map(str::to_owned) else {
        return;
    };
    let keys = beton_harness_direct::secret::KeyMap::new(
        [(
            lookup,
            beton_harness_direct::secret::ApiKey::new(egress.placeholder.as_str()),
        )]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect(),
    );
    registry.register(Arc::new(beton_harness_direct::DirectAdapter::new(
        p,
        &beton_harness_direct::DirectOptions {
            keys: Arc::new(keys),
            discover_models: false,
            egress: Some(beton_harness_direct::Egress {
                proxy_url: egress.proxy_url.to_string(),
                ca_pem: egress.ca_pem.clone(),
            }),
        },
    )));
}

/// Eingebaute Adapter mit den Optionen aus der Konfiguration; der Fake nur mit `dev`
/// (HAR-026 AC3). Die Auth-Herkunft gilt nur aus der User-Konfiguration (HAR-015), damit ein
/// Repository nicht auf API-Billing umschalten kann.
pub fn builtin_registry(layers: &HarnessLayers, dev: bool) -> Registry {
    builtin_registry_with_problems(layers, dev).0
}

/// Wie [`builtin_registry`]; liefert zusätzlich die übersprungenen ACP-Einträge mit Datei
/// und Zeile (HAR-008 AC3). Ungültige Provider (`direct:*`) meldet es als Warnung.
pub fn builtin_registry_with_problems(
    layers: &HarnessLayers,
    dev: bool,
) -> (Registry, Vec<beton_harness_acp::ConfigProblem>) {
    let (registry, acp, direct) =
        builtin_registry_with_options(layers, dev, &beton_harness_direct::DirectOptions::default());
    for p in direct {
        tracing::warn!("Konfiguration: Provider übersprungen: {p}");
    }
    (registry, acp)
}

/// Registry mit allen eingebauten Adaptern. `direct:*` gibt es nur für Provider aus der
/// User-Konfiguration (`providers:`, HAR-011; ohne Eintrag kein Direkt-API-Harness,
/// ADR-0034). Liefert die übersprungenen ACP-Agents (Datei und Zeile, HAR-008 AC3) und
/// Provider (Pfad und Grund, HAR-011 AC4).
pub fn builtin_registry_with_options(
    layers: &HarnessLayers,
    dev: bool,
    direct: &beton_harness_direct::DirectOptions,
) -> (
    Registry,
    Vec<beton_harness_acp::ConfigProblem>,
    Vec<beton_harness_direct::config::ConfigProblem>,
) {
    let mut registry = Registry::new(RegistryOptions { dev });
    registry.register(Arc::new(claude_adapter(layers)));
    registry.register(Arc::new(beton_harness_codex::CodexAdapter {
        auth: layers.user.auth("codex").source(),
        ..beton_harness_codex::CodexAdapter::default()
    }));
    let acp = beton_harness_acp::register(&mut registry, layers);
    let providers = beton_harness_direct::register(&mut registry, &layers.providers, direct);
    (registry, acp, providers)
}

fn claude_adapter(layers: &HarnessLayers) -> beton_harness_claude::ClaudeAdapter {
    let user = layers.user.entries.get("claude");
    let project = layers.project.entries.get("claude");
    beton_harness_claude::ClaudeAdapter {
        auth: layers.user.auth("claude").source(),
        // Die Projektebene darf nur verschärfen (SBX-002, HAR-004).
        isolated: user.and_then(|e| e.isolated).unwrap_or(false)
            || project.and_then(|e| e.isolated) == Some(true),
        // Managed Settings können Hooks abschalten (#258 R5).
        managed_settings: beton_harness_claude::managed_settings_paths(),
        ..beton_harness_claude::ClaudeAdapter::default()
    }
}

/// Einstieg des Runner-Prozesses: Startdaten aus Env und stdin lesen, Session ausführen.
/// Exit-Code: 0 nach `runner.stop` oder Ende des Daemons, sonst der des Harness bzw. 1/2.
pub async fn main_from_env() -> std::process::ExitCode {
    use std::process::ExitCode;
    // #304: Ein Testbudget der Policy-Engine (Feature `test-budget`) gehört nie in den Betrieb.
    if let Some(ms) = beton_policy::cel::Limits::active_test_budget_ms() {
        tracing::warn!(
            budget_ms = ms,
            "Testbudget der Policy-Engine aktiv: {ms} ms je Auswertung statt 10 ms (nur für Tests)"
        );
    }
    // Keine Core-Dumps mit Credential-Werten (#295). Nicht dumpable ist nur der Daemon: Der
    // Runner bleibt für Diagnose und `RUN-002` über `/proc/<pid>/cwd` lesbar (Restrisiko).
    if let Err(e) = harden::no_core_dumps() {
        tracing::warn!("Core-Dumps nicht abschaltbar: {e}");
    }
    let boot = match RunnerBoot::from_env_and_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("beton runner: {e}");
            return ExitCode::from(2);
        }
    };
    let (registry, problems, providers) = builtin_registry_with_options(
        &boot.harnesses,
        boot.dev,
        &beton_harness_direct::DirectOptions {
            keys: Arc::new(boot.credentials.clone()),
            discover_models: false,
            egress: None,
        },
    );
    for p in problems {
        tracing::warn!("Konfiguration: ACP-Agent übersprungen: {p}");
    }
    for p in providers {
        tracing::warn!("Konfiguration: Provider übersprungen: {p}");
    }
    match run(boot, registry).await {
        Ok(Exit::Stopped | Exit::ParentGone | Exit::TimedOut) => ExitCode::SUCCESS,
        Ok(Exit::HarnessExited { code }) => {
            ExitCode::from(u8::try_from(code.unwrap_or(1)).unwrap_or(1))
        }
        Ok(Exit::PolicyViolation | Exit::Unauthorized) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("beton runner: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Verbrauchswerte für Policies über den Tunnel (System-Call `policy.usage`, POL-004).
struct TunnelUsage {
    calls: mpsc::UnboundedSender<mcp::SystemRequest>,
}

#[async_trait]
impl policy::UsageSource for TunnelUsage {
    async fn usage(&self) -> Result<policy::Usage, String> {
        let (reply, rx) = oneshot::channel();
        self.calls
            .send(mcp::SystemRequest {
                tool: "policy.usage".into(),
                args: json!({}),
                reply,
            })
            .map_err(|_| "Runner beendet".to_owned())?;
        match rx.await {
            Ok(Ok(v)) => serde_json::from_value(v).map_err(|e| e.to_string()),
            Ok(Err(p)) => Err(p["code"].as_str().unwrap_or("failed").to_owned()),
            Err(_) => Err("keine Antwort".into()),
        }
    }
}

/// Zustand der Policies über den Tunnel (System-Calls `policy.state`, `policy.state.apply`,
/// POL-009/POL-010).
struct TunnelState {
    calls: mpsc::UnboundedSender<mcp::SystemRequest>,
}

impl TunnelState {
    async fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let (reply, rx) = oneshot::channel();
        self.calls
            .send(mcp::SystemRequest {
                tool: tool.into(),
                args,
                reply,
            })
            .map_err(|_| "Runner beendet".to_owned())?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(p)) => Err(p["code"].as_str().unwrap_or("failed").to_owned()),
            Err(_) => Err("keine Antwort".into()),
        }
    }
}

#[async_trait]
impl policy::StateSource for TunnelState {
    async fn load(&self) -> Result<policy::LoadedState, String> {
        let v = self.call("policy.state", json!({})).await?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    async fn apply(&self, op: &beton_policy::state::StateOp) -> Result<(), String> {
        let args = serde_json::to_value(op).map_err(|e| e.to_string())?;
        self.call("policy.state.apply", args).await.map(|_| ())
    }
}

/// Abfrage-Budget je Session-Baum (System-Call `policy.query`, #276).
#[async_trait]
impl policy::QueryBudgetSource for TunnelState {
    async fn take(&self) -> Result<policy::QueryTaken, String> {
        let v = self.call("policy.query", json!({})).await?;
        match (v["allowed"].as_bool(), v["crossed"].as_bool()) {
            (Some(allowed), Some(crossed)) => Ok(policy::QueryTaken { allowed, crossed }),
            _ => Err("ungültige Antwort".into()),
        }
    }
}

#[async_trait]
impl policy::SnapshotSink for TunnelState {
    async fn store(
        &self,
        manifest: &beton_policy::snapshot::Manifest,
        sources: &[(String, Vec<u8>)],
    ) -> Result<(), String> {
        // Policy-Dateien und Inline-Regeln sind UTF-8 (sonst lädt das Set nicht).
        let sources: Vec<Value> = sources
            .iter()
            .map(|(version, bytes)| {
                let text = String::from_utf8(bytes.clone()).map_err(|e| e.to_string())?;
                Ok(json!({ "version": version, "text": text }))
            })
            .collect::<Result<_, String>>()?;
        self.call(
            "policy.snapshot",
            json!({ "manifest": manifest, "sources": sources }),
        )
        .await
        .map(|_| ())
    }
}

/// Unbestätigte Events in Runner-Reihenfolge.
#[derive(Default)]
struct Unacked {
    next_rseq: u64,
    queue: VecDeque<(RseqEvent, usize)>,
    bytes: usize,
}

impl Unacked {
    fn push(&mut self, event: Event) -> RseqEvent {
        self.next_rseq += 1;
        let item = RseqEvent {
            rseq: self.next_rseq,
            event,
        };
        let size = serde_json::to_string(&item).map(|s| s.len()).unwrap_or(256);
        self.bytes += size;
        self.queue.push_back((item.clone(), size));
        item
    }

    fn ack(&mut self, upto: u64) {
        while self.queue.front().is_some_and(|(e, _)| e.rseq <= upto) {
            if let Some((_, size)) = self.queue.pop_front() {
                self.bytes -= size;
            }
        }
    }

    fn acked(&self) -> u64 {
        self.queue
            .front()
            .map(|(e, _)| e.rseq - 1)
            .unwrap_or(self.next_rseq)
    }
}

/// Leitet `session.status` aus dem Event-Strom ab und meldet nur Änderungen.
#[derive(Default)]
struct StatusTracker {
    current: Option<SessionStatus>,
    /// Die Sandbox hat eine Voraussetzung verloren (SBX-006 AC3): Die Session bleibt
    /// `paused`, bis der Runner endet; nur `failed` und `stopped` lösen das ab.
    paused: bool,
}

impl StatusTracker {
    fn set(&mut self, boot: &RunnerBoot, to: SessionStatus, out: &mut Vec<Event>) {
        if self.current == Some(to) {
            return;
        }
        if self.paused && !matches!(to, SessionStatus::Failed | SessionStatus::Stopped) {
            return;
        }
        self.current = Some(to);
        out.push(Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::SessionStatus(SessionStatusChanged {
                status: to,
                reason: None,
            }),
        ));
    }
}

impl StatusTracker {
    /// `paused` mit Grund (SBX-006 AC3); spätere Wechsel außer `failed`/`stopped` entfallen.
    fn pause(&mut self, boot: &RunnerBoot, reason: &str, out: &mut Vec<Event>) {
        self.paused = true;
        self.current = Some(SessionStatus::Paused);
        out.push(Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::SessionStatus(SessionStatusChanged {
                status: SessionStatus::Paused,
                reason: Some(reason.to_owned()),
            }),
        ));
    }
}

/// Wie der Runner endete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Server hat `runner.stop` geschickt (z. B. Idle-Timeout).
    Stopped,
    /// Harness beendet (Absturz oder Ende).
    HarnessExited { code: Option<i32> },
    /// Elternprozess (Daemon) ist weg.
    ParentGone,
    /// `executor.timeout` des Agents abgelaufen (AGT-004 AC3).
    TimedOut,
    /// Integritätsverletzung des Policy-Gates (HAR-005, #233): Session beendet.
    PolicyViolation,
    /// Der Tunnel lehnt das Token ab (abgelaufen oder gesperrt, #243): ohne neues Token kommt
    /// der Runner nie wieder hinein und endet (fail closed).
    Unauthorized,
}

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
/// Windows (Beta): Der Tunnel über Named Pipes folgt; bis dahin bricht `connect` ab.
#[cfg(not(unix))]
type Stream = tokio::net::TcpStream;

type Ws = tokio_tungstenite::WebSocketStream<Stream>;

#[cfg(unix)]
async fn open_socket(boot: &RunnerBoot) -> Result<Stream, RunnerError> {
    tokio::net::UnixStream::connect(&boot.socket)
        .await
        .map_err(|e| RunnerError::Tunnel(format!("{}: {e}", boot.socket.display())))
}

#[cfg(not(unix))]
async fn open_socket(boot: &RunnerBoot) -> Result<Stream, RunnerError> {
    Err(RunnerError::Tunnel(format!(
        "{}: Tunnel-Socket wird unter Windows noch nicht unterstützt",
        boot.socket.display()
    )))
}

async fn connect(boot: &RunnerBoot) -> Result<Ws, RunnerError> {
    let stream = open_socket(boot).await?;
    let mut req = "ws://localhost/v1/tunnel"
        .into_client_request()
        .map_err(|e| RunnerError::Tunnel(e.to_string()))?;
    let headers = req.headers_mut();
    headers.insert(
        "authorization",
        boot.token
            .bearer()
            .parse()
            .map_err(|_| RunnerError::Boot("Token ungültig".into()))?,
    );
    headers.insert(
        "sec-websocket-protocol",
        SUBPROTOCOL
            .parse()
            .map_err(|_| RunnerError::Boot("Subprotokoll".into()))?,
    );
    let (ws, _) = tokio_tungstenite::client_async(req, stream)
        .await
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r)
                if r.status() == tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED =>
            {
                RunnerError::Unauthorized
            }
            e => RunnerError::Tunnel(e.to_string()),
        })?;
    Ok(ws)
}

async fn send(ws: &mut Ws, msg: &TunnelUp) -> bool {
    let text = serde_json::to_string(msg).unwrap_or_default();
    ws.send(Message::Text(text.into())).await.is_ok()
}

/// Höchstdauer, die ein abgekoppelter Runner auf den nächsten Daemon wartet (POL-010 AC4).
const DETACH_MAX: Duration = Duration::from_secs(3600);

/// Signal der geordneten Abkopplung im Close-Frame des Daemons (POL-010 AC4).
fn is_detach_signal(code: u16, reason: &str) -> bool {
    code == 4503 && reason == "detached"
}

/// Weiterleben ohne Daemon nur nach geordneter Abkopplung, mit offener Policy-Freigabe und
/// höchstens [`DETACH_MAX`] lang (POL-010 AC4). Ein abgestürzter Daemon sendet kein Signal: dann
/// endet der Runner, und der neue Daemon setzt die Session nach HAR-020 fort.
fn detached_keep(
    detached_at: Option<std::time::Instant>,
    now: std::time::Instant,
    pending_policy: bool,
) -> bool {
    pending_policy && detached_at.is_some_and(|t| now.saturating_duration_since(t) <= DETACH_MAX)
}

/// Endet ein Runner ohne Verbindung zum Daemon? Ist der Daemon weg (Elternprozess tot) oder
/// hat ein späterer Daemon den Runner übernommen, wartet er nur, wenn `keep` gilt
/// ([`detached_keep`]) – dann verbindet ihn der nächste Daemon neu (POL-010 AC4). Sonst endet
/// er (RUN-002 AC2).
fn orphan_exit(parent_alive: bool, adopted: bool, keep: bool) -> bool {
    (!parent_alive || adopted) && !keep
}

/// Lebt der Elternprozess noch? (RUN-002 AC2)
fn parent_alive(pid: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        match pid {
            Some(p) => {
                rustix::process::getppid().is_some_and(|pp| pp.as_raw_nonzero().get() as u32 == p)
            }
            None => true,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Führt die Session aus, bis der Server stoppt, der Harness endet oder der Daemon stirbt.
pub async fn run(boot: RunnerBoot, registry: Registry) -> Result<Exit, RunnerError> {
    let mut registry = registry;
    let mut env = HostEnv::from_process();
    env.user = boot.harnesses.user.clone();
    env.project = boot.harnesses.project.clone();
    env.vars.remove("BETON_RUNNER_TOKEN");
    // SBX-001 AC4: Jeder Start des Harness meldet `sandbox.started`; bis SBX-006 (WP-49)
    // ausdrücklich ohne Isolation (`backend: none`).
    let (sandbox_tx, sandbox_rx) = mpsc::unbounded_channel::<EventPayload>();
    let mut sandbox_events = Some(sandbox_rx);
    let stage_tx = sandbox_tx.clone();
    let sandbox = beton_sandbox::Sandbox::unsandboxed(
        beton_sandbox::Stage::Harness,
        Some(Arc::new(move |ev| {
            let _ = sandbox_tx.send(EventPayload::SandboxStarted(ev));
        })),
    );
    let actor = Actor::Agent {
        id: None,
        harness: boot.harness.to_string(),
        agent_ref: boot.agent_ref.clone(),
    };
    let mut lifecycle = Lifecycle::new(boot.runner_id);
    let mut pending_status: Vec<Event> = Vec::new();
    let status =
        |lc: &mut Lifecycle, to: RunnerState, reason: Option<String>, out: &mut Vec<Event>| {
            if let Ok(s) = lc.go(to, reason) {
                out.push(Event::new(
                    boot.session_id,
                    0,
                    Actor::System {
                        component: beton_core::event::SystemComponent::Runner,
                    },
                    EventPayload::RunnerStatus(s),
                ));
            }
        };
    status(
        &mut lifecycle,
        RunnerState::Provisioning,
        None,
        &mut pending_status,
    );
    status(
        &mut lifecycle,
        RunnerState::Starting,
        None,
        &mut pending_status,
    );

    let probe = match registry.get(&boot.harness) {
        Some(adapter) => adapter.probe(&env).await,
        None => beton_harness::ProbeReport::default(),
    };
    let caps = registry
        .get(&boot.harness)
        .map(|a| a.capabilities(beton_harness::Mode::Native, &probe));
    let capabilities = caps
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or(Value::Null);
    // MCP-Server, System-Tools und Skills der Session (HAR-009, AGT-006 bis AGT-008).
    let run_dir = boot
        .socket
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    // Agent aus dem Snapshot (AGT-004); ohne lesbaren Snapshot startet die Session nicht.
    let snapshot = match &boot.agent_snapshot {
        Some(path) => match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| beton_agents::AgentSnapshot::from_bytes(&b).map_err(|e| e.to_string()))
        {
            Ok(s) => Some(s),
            Err(e) => {
                let e = beton_harness::HarnessError::StartRefused(format!("Agent-Snapshot: {e}"));
                report_start_failure(&boot, &e).await;
                return Err(e.into());
            }
        },
        None => None,
    };
    // Policy-Gate der Session (POL-003): Ereignisse über `gate_rx`, Verbrauchswerte über den
    // Tunnel (`usage_rx`).
    let paths = mcp::Paths::from_process(run_dir.clone());
    let (gate_tx, mut gate_rx) = mpsc::unbounded_channel::<policy::GateEvent>();
    let (usage_tx, mut usage_rx) = mpsc::unbounded_channel::<mcp::SystemRequest>();
    let gate = policy::PolicyGate::new(
        policy::PolicyOptions {
            session_id: Some(boot.session_id.to_string()),
            harness: boot.harness.clone(),
            workdir: boot.workdir.clone(),
            home: paths.home.clone(),
            project_root: Some(beton_agents::resolve::project_root(
                &boot.workdir,
                &paths.beton_home,
            )),
            dirs: policy::policy_dirs(&paths.beton_home, &boot.workdir),
            approval_timeout: policy::APPROVAL_TIMEOUT,
            agent: snapshot.as_ref().map(|s| beton_policy::vars::AgentVars {
                name: Some(s.name.clone()),
                version: Some(s.version.clone()),
                source: Some(s.source.clone()),
                hash: Some(s.hash.clone()),
            }),
            // Weitere Konfigurationsverzeichnisse der Vendor-CLIs (#258).
            vendor_dirs: ["CLAUDE_CONFIG_DIR", "CODEX_HOME"]
                .iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect(),
        },
        gate_tx,
    )
    .with_usage(Arc::new(TunnelUsage {
        calls: usage_tx.clone(),
    }))
    .with_state(Arc::new(TunnelState {
        calls: usage_tx.clone(),
    }))
    // Set-Snapshots je Version im Blob-Store der Session (POL-027 AC2).
    .with_snapshots(Arc::new(TunnelState {
        calls: usage_tx.clone(),
    }))
    // `policy_query`-Limit je Session-Baum, gezählt vom Server (#276).
    .with_query_budget(Arc::new(TunnelState {
        calls: usage_tx.clone(),
    }));
    // Credentials und Egress-Events des Stufenmodells (PRX-006, PRX-009): Werte und Audit über
    // den Tunnel, Events ins Session-Log.
    // Snapshots der Terminals als Blob über den Tunnel (DATA-011).
    let term_calls = usage_tx.clone();
    let credentials = Arc::new(credentials::TunnelCredentials::new(
        credentials::channel_calls(usage_tx),
        stage_tx.clone(),
    ));
    // Zweistufige Sandbox (SBX-002, SBX-006, PRX-001, PRX-005, PRX-008, #191): Default
    // „Sandbox erforderlich“; jeder Fehler verhindert den Start (fail closed). Ohne Isolation
    // nur mit ausdrücklichem `backend: none`, das für diese Session bestätigt ist.
    let sandbox_boot = boot.sandbox.clone().unwrap_or_default();
    // AGT-014: `sandbox:` des Agents verschärft die Sandbox der Session nur; eine Anforderung,
    // die der Host nicht erfüllt, oder eine Lockerung verhindert den Start (fail closed).
    let agent_sandbox = snapshot
        .as_ref()
        .and_then(|s| s.agent().ok())
        .and_then(|(spec, _)| spec.sandbox);
    let sandbox_spec = match &agent_sandbox {
        Some(a) => {
            match agent_sandbox::tighten(&sandbox_boot.spec, a, agent_sandbox::auto_choice(a)) {
                Ok(s) => s,
                Err(e) => {
                    let e = beton_harness::HarnessError::StartRefused(e.to_string());
                    report_start_failure(&boot, &e).await;
                    return Err(e.into());
                }
            }
        }
        None => sandbox_boot.spec.clone(),
    };
    if !stages::active(&beton_sandbox::SandboxSpec::merged(&[&sandbox_spec]))
        && !boot.unsandboxed_confirmed
    {
        let e = beton_harness::HarnessError::StartRefused(
            "sandbox_confirmation_required: `sandbox.backend: none` startet nur mit Bestätigung \
             für diese Session (`beton run --unsandboxed`)"
                .into(),
        );
        report_start_failure(&boot, &e).await;
        return Err(e.into());
    }
    let mut stages = match &sandbox_boot {
        sb if stages::active(&beton_sandbox::SandboxSpec::merged(&[&sandbox_spec])) => {
            let ca = match boot.proxy_ca.as_ref().map(stages::CaText::ca).transpose() {
                Ok(ca) => ca,
                Err(e) => {
                    let e = beton_harness::HarnessError::StartRefused(format!(
                        "sandbox_unavailable: beton-CA: {e}"
                    ));
                    report_start_failure(&boot, &e).await;
                    return Err(e.into());
                }
            };
            let events: beton_sandbox::EventSink = {
                let tx = stage_tx.clone();
                Arc::new(move |ev| {
                    let _ = tx.send(EventPayload::SandboxStarted(ev));
                })
            };
            let input = stages::StageInput {
                session_id: boot.session_id.to_string(),
                harness: boot.harness.as_str(),
                tool_isolation: predicted_caps(&boot, caps.as_ref(), true).tool_isolation,
                workdir: &boot.workdir,
                // Ohne Home scheitert die Auflösung (fail closed).
                home: paths.home.clone().unwrap_or_default(),
                host_env: std::env::vars()
                    .filter(|(k, _)| !k.starts_with("BETON_"))
                    .collect(),
                program: beton_harness::registry::resolve_binary(
                    &boot.harness,
                    boot.harness.as_str(),
                    &env,
                )
                .map(|b| b.program),
                events: Some(events),
                // Verstöße der Backends als `sandbox.violation` (SBX-013 AC1, SBX-014).
                violations: Some({
                    let tx = stage_tx.clone();
                    Arc::new(move |v| {
                        let _ = tx.send(EventPayload::SandboxViolation(v));
                    })
                }),
            };
            // HAR-015 AC4: Mit `auth: api_key` sieht die Vendor-CLI nur einen Platzhalter.
            let harness_key = (boot.harnesses.user.auth(boot.harness.as_str())
                == beton_harness::registry::HarnessAuth::ApiKey)
                .then(|| stages::vendor_key(boot.harness.as_str()))
                .flatten()
                .map(|(var, hosts)| stages::HarnessKey {
                    binding: format!("harness:{}", boot.harness),
                    var: var.to_owned(),
                    hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
                });
            // HAR-011 AC2: Der Loop des Direkt-API-Harness erreicht seinen Anbieter nur über
            // eine eigene Proxy-Instanz, die den Key einsetzt.
            let direct = direct_route(&boot);
            let opts = stages::StageOptions {
                direct,
                spec: sandbox_spec.clone(),
                proxy_ca: ca,
                proxy: beton_proxy::ProxyOptions {
                    extra_ca_files: sb.extra_ca_files.clone(),
                },
                exe: std::env::current_exe().unwrap_or_default(),
                services: Some(stages::StageServices {
                    source: credentials.clone(),
                    audit: credentials.clone(),
                    events: Some(credentials.clone()),
                }),
                harness_key,
            };
            match stages::compose(input, opts).await {
                Ok(s) => s,
                Err(e) => {
                    let e = beton_harness::HarnessError::StartRefused(e.to_string());
                    report_start_failure(&boot, &e).await;
                    return Err(e.into());
                }
            }
        }
        _ => None,
    };
    // #206: Hook-Server, MCP-Relay-Hub und Session-Skills liegen im Stufenmodell im
    // Laufzeitverzeichnis der Stufe 1, damit der Harness sie aus seiner Sandbox erreicht.
    let run_dir = stages
        .as_ref()
        .map(|s| s.runtime_dir().to_path_buf())
        .unwrap_or(run_dir);
    let paths = mcp::Paths {
        run_dir: run_dir.clone(),
        ..paths
    };
    // Was das Stufenmodell tatsächlich durchsetzt; nur das schaltet `yolo` frei (HAR-027).
    let sandbox_status = stages
        .as_ref()
        .map(|s| s.env.sandbox)
        .unwrap_or(beton_harness::SandboxStatus::NONE);
    if let Some(egress) = stages.as_ref().and_then(|s| s.direct()) {
        direct_via_proxy(&boot, &mut registry, egress);
    }
    // PRX-005: Passthrough in Stufe 2 ist ausdrücklich konfiguriert und steht im Audit-Log.
    if let Some(hosts) = stages
        .as_ref()
        .map(|s| s.passthrough(beton_sandbox::Stage::Tools))
        .filter(|h| !h.is_empty())
    {
        let c = credentials.clone();
        tokio::spawn(async move { c.audit_passthrough(hosts).await });
    }
    // Workspace-Terminals (SES-019, SBX-017): Prozesse in der Tool-Sandbox der Session; ohne
    // Stufenmodell (`backend: none`, bestätigt) ohne Isolation wie jedes Tool.
    let (term_tx, mut term_rx) = mpsc::unbounded_channel::<terminals::Out>();
    let term_sandbox = match &stages {
        Some(s) => s.tools.clone(),
        None => {
            let tx = stage_tx.clone();
            beton_sandbox::Sandbox::unsandboxed(
                beton_sandbox::Stage::Tools,
                Some(Arc::new(move |ev| {
                    let _ = tx.send(EventPayload::SandboxStarted(ev));
                })),
            )
        }
    };
    let terminals = terminals::Terminals::new(
        term_sandbox,
        boot.workdir.clone(),
        terminals::Terminals::default_shell(std::env::var("SHELL").ok().as_deref()),
        term_tx,
        term_calls,
    );
    let in_proc = caps
        .as_ref()
        .is_some_and(|c| c.transport == beton_harness::Transport::InProc);
    let (launcher, stage): (
        Arc<dyn beton_harness::process::ProcessLauncher>,
        Option<beton_harness::StageEnv>,
    ) = match &stages {
        // Direkt-API und Fake laufen im Runner; nur ihre Tools gehen in Stufe 2.
        Some(st) if in_proc => (
            Arc::new(SandboxLauncher::new(sandbox)),
            Some(beton_harness::StageEnv {
                tools: Some(Arc::new(SandboxLauncher::new(st.tools.clone()))),
                ..st.env.clone()
            }),
        ),
        // Tools, die beton selbst ausführt (ACP `fs/read_text_file`, `terminal/*`, #326): in
        // Stufe 2 und über die Terminals des Runners, auch solange ACP degraded ist (#329).
        Some(st)
            if beton_tools_in_stage_two(
                &boot.harness,
                predicted_caps(&boot, caps.as_ref(), true).tool_isolation,
            ) =>
        {
            (
                Arc::new(SandboxLauncher::new(st.harness.clone())),
                Some(beton_harness::StageEnv {
                    tools: Some(Arc::new(SandboxLauncher::new(st.tools.clone()))),
                    terminals: Some(Arc::new(terminals::ToolTerminals {
                        terminals: terminals.clone(),
                        actor: actor.clone(),
                    })),
                    ..st.env.clone()
                }),
            )
        }
        Some(st) => (
            Arc::new(SandboxLauncher::new(st.harness.clone())),
            Some(st.env.clone()),
        ),
        None => (Arc::new(SandboxLauncher::new(sandbox)), None),
    };
    // Vor dem Start prüft der Runner Policies selbst (Start verweigert, #182, POL-024).
    gate.defer_rechecks();
    let ctx = AdapterContext {
        stage,
        gate: Arc::new(gate.clone()),
        launcher,
        env,
    };
    // `PreToolUse`-Hook (HAR-005): `beton hook claude …` aus dem eigenen Binary.
    let hook = beton_harness::hook::HookCommand::current_exe().map(|command| {
        beton_harness::hook::HookSetup {
            command,
            run_dir: run_dir.clone(),
        }
    });
    // Capabilities, mit denen der Harness laufen wird (Fake: samt Szenario), für die
    // Enforcement-Matrix (POL-024) schon vor dem Start.
    let predicted = predicted_caps(&boot, caps.as_ref(), hook.is_some());
    gate.set_matrix(predicted.enforcement());
    let harness_gates_all = predicted.tool_call_gate == beton_harness::ToolCallGate::Full;
    let source = match (&snapshot, &boot.agent_ref) {
        (Some(s), _) => mcp::AgentSource::Snapshot(s),
        (None, Some(r)) => mcp::AgentSource::Ref(r),
        (None, None) => mcp::AgentSource::None,
    };
    let setup = match caps.as_ref() {
        Some(caps) => {
            mcp::prepare(
                boot.session_id,
                &boot.harness,
                caps,
                &boot.workdir,
                source,
                &boot.harnesses,
                &paths,
                Some((&registry, &ctx.env)),
                // MCP- und System-Tools im Relay-Hub prüfen, wenn der Harness nicht jeden
                // Tool-Call selbst durch das Gate schickt (POL-024).
                (predicted.tool_call_gate != beton_harness::ToolCallGate::Full).then(|| {
                    gate.gate_mcp_in_hub();
                    Arc::new(gate.clone()) as Arc<dyn beton_mcp::hub::McpGate>
                }),
                // `ask_user` fragt über das Gate (Inbox-Frage, AGT-007 AC4).
                Some(Arc::new(gate.clone()) as Arc<dyn mcp::Asker>),
                stages.as_ref().map(|s| s.tools.clone()),
                // `policy_query` wertet wie ein echter Call aus, ohne Seiteneffekte (AGT-007 AC3).
                Some(Arc::new(gate.clone()) as Arc<dyn mcp::PolicyOracle>),
                // `${secret:…}` → Platzhalter, HTTP-Server mit Credential über den Proxy.
                stages.as_mut(),
            )
            .await
        }
        None => Ok(mcp::McpSetup::default()),
    };
    let mut setup = match setup {
        Ok(s) => {
            // Nur von beton eingebundene Server tragen die Klassen reservierter Namen (#234).
            let mut trusted = s.injection.names();
            if boot.harness.as_str().starts_with("direct") {
                trusted.push("beton-workspace".into());
            }
            // #252: Ein gleichnamiger Server aus der Vendor-Konfiguration wäre nicht von dem
            // von beton zu unterscheiden: Start verweigern (fail closed).
            let injected_reserved = s
                .injection
                .names()
                .iter()
                .any(|n| beton_mcp::config::reserved_name(n));
            if injected_reserved {
                let family = beton_policy::normalize::harness_family(boot.harness.as_str());
                let vendor = vendor_mcp::VendorPaths::from_process(paths.home.clone());
                let conflict = match vendor_mcp::reserved_servers(family, &vendor, &boot.workdir) {
                    Ok(names) if names.is_empty() => None,
                    Ok(names) => Some(format!(
                        "mcp_server_conflict: Die Konfiguration von {family} definiert den reservierten MCP-Server {}; beton kann ihn nicht von seinem eigenen unterscheiden. Bitte umbenennen oder entfernen.",
                        names
                            .iter()
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    Err(why) => Some(format!(
                        "mcp_server_conflict: MCP-Server der Konfiguration von {family} nicht prüfbar ({why})"
                    )),
                };
                if let Some(reason) = conflict {
                    let e = beton_harness::HarnessError::StartRefused(reason);
                    report_start_failure(&boot, &e).await;
                    return Err(e.into());
                }
            }
            gate.set_trusted_servers(trusted);
            s
        }
        Err(e) => {
            // Fail closed: ohne ladbaren Agent startet die Session nicht.
            let e = beton_harness::HarnessError::StartRefused(format!("Agent: {e}"));
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
    // AGT-014: Ohne Snapshot ließe sich `sandbox:` nicht vor dem Aufbau der Stufen anwenden.
    if setup.agent_sandbox.is_some() && agent_sandbox.is_none() {
        let e = beton_harness::HarnessError::StartRefused(
            "sandbox_unavailable: Agent mit `sandbox:` ohne Snapshot (AGT-004); Start verweigert"
                .into(),
        );
        report_start_failure(&boot, &e).await;
        return Err(e.into());
    }
    // AGT-014: Agent-Ebene der Policy-Hierarchie.
    gate.set_inline(
        beton_policy::load::Scope::Agent,
        std::mem::take(&mut setup.agent_policies),
    );
    // Übernommener Verlauf (Fork, Resume eines Imports): native, Rebuild oder Präambel.
    let mut start_fork_of = None;
    let forked = match &boot.fork {
        Some(path) => match fork::load(path) {
            Ok(plan) => Some({
                start_fork_of = fork_of(Some(&plan));
                fork::prepare(
                    &plan,
                    boot.fork_logged,
                    boot.session_id,
                    &boot.harness,
                    registry.get(&boot.harness).map(|a| a.as_ref()),
                    caps.as_ref(),
                    &ctx.env,
                    &boot.workdir,
                    boot.model.clone(),
                )
                .await
            }),
            Err(e) => {
                let e = beton_harness::HarnessError::StartRefused(e);
                report_start_failure(&boot, &e).await;
                return Err(e.into());
            }
        },
        None => None,
    };
    let forked = forked.unwrap_or_default();
    let mut handover = forked.handover;
    // Einmal-Aufrufe (SES-010) laufen über denselben Adapter und dieselbe Umgebung.
    let one_shot_adapter = registry.get(&boot.harness).cloned();
    let one_shot_ctx = ctx.clone();
    let (one_shot_tx, mut one_shot_rx) =
        mpsc::unbounded_channel::<(String, Result<Value, Value>)>();
    // Modell, Effort und Permission-Mode ab dem Start (HAR-017, HAR-027); `yolo` ohne Sandbox
    // scheitert hier (fail closed).
    let fallback_caps = beton_harness::Capabilities::minimal(
        beton_harness::Mode::Native,
        beton_harness::Transport::Native,
    );
    let (mut start_settings, mapped_effort) = match settings::at_start(
        boot.model.clone(),
        boot.effort.as_deref(),
        boot.permission_mode.as_deref(),
        caps.as_ref().unwrap_or(&fallback_caps),
        sandbox_status,
    ) {
        Ok(s) => s,
        Err(rejected) => {
            let e = settings::start_error(&rejected);
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
    // Vor dem Start (POL-001, POL-024, POL-003 `session_start`): nie gültige Policy-Dateien,
    // nicht durchsetzbare Pflichtregeln und ein `deny` verhindern den Harness-Start.
    let mut unacked = Unacked::default();
    gate.set_settings(
        start_settings.model.clone(),
        start_settings.effort.clone(),
        start_settings.permission_mode,
    );
    // Session-Budget (API-006 `--max-cost`): eine `spend_cap`-Regel der User-Ebene (POL-011).
    let mut refusal: Option<(Vec<String>, Vec<String>)> = None;
    if let Some(max) = boot.max_cost_usd {
        match beton_policy::budget::session_budget(max) {
            Ok(budget) => gate.set_inline(beton_policy::load::Scope::User, vec![budget]),
            Err(e) => {
                refusal = Some((
                    vec![format!(
                        "policy_invalid: Session-Budget ungültig: {}",
                        e.message
                    )],
                    Vec::new(),
                ));
            }
        }
    }
    let set = gate.refresh(None);
    if refusal.is_none() && !set.never_valid().is_empty() {
        // #182: fail closed – Pfad und Ladefehler, nie Inhalte der Datei.
        let reasons = set
            .never_valid()
            .iter()
            .map(|f| {
                format!(
                    "policy_invalid: Policy-Datei {} ist ungültig (Zeile {}): {}",
                    f.file, f.line, f.error
                )
            })
            .collect();
        refusal = Some((reasons, Vec::new()));
    }
    if refusal.is_none() {
        let check = gate.start_check(&set);
        if !check.refused.is_empty() {
            refusal = Some((
                check
                    .refused
                    .iter()
                    .map(|g| format!("enforcement_unavailable: {}", g.describe()))
                    .collect(),
                check.refused_rules(),
            ));
        }
    }
    let mut pre = PreStart::default();
    // Von einer Policy beim Start gesetzter Permission-Mode (bindet `plan`, HAR-027).
    let mut policy_mode: Option<beton_harness::PermissionMode> = None;
    if refusal.is_none() {
        let start = beton_policy::vars::StartVars {
            trigger: Some(
                if boot.fork.is_some() {
                    "fork"
                } else if boot.spawned {
                    "spawn"
                } else if boot.resume.is_some() {
                    "resume"
                } else {
                    "interactive"
                }
                .into(),
            ),
            harness: Some(boot.harness.to_string()),
            model: start_settings.model.clone(),
            reasoning_effort: start_settings.effort.clone(),
            permission_mode: start_settings
                .permission_mode
                .map(|m| m.as_str().to_owned()),
            sandbox: Some(stages::sandbox_vars(stages.as_ref())),
            network: Some(beton_policy::vars::StartNetworkVars {
                proxy_enabled: Some(sandbox_status.egress_proxy),
            }),
            worktree: Some(boot.workdir.display().to_string()),
            fork_of: start_fork_of.clone(),
            agent_ref: boot.agent_ref.clone(),
        };
        let checked = pre
            .check_start(
                &boot,
                &gate,
                start,
                &mut gate_rx,
                &mut usage_rx,
                &mut unacked,
            )
            .await;
        match checked {
            policy::Checked::Allow(m) => {
                match apply_start_modifications(
                    &mut start_settings,
                    &m,
                    caps.as_ref().unwrap_or(&fallback_caps),
                    sandbox_status,
                ) {
                    Ok(mode) => {
                        policy_mode = mode;
                        gate.set_settings(
                            start_settings.model.clone(),
                            start_settings.effort.clone(),
                            start_settings.permission_mode,
                        );
                    }
                    // Fail closed (#231): z. B. `yolo` ohne Sandbox.
                    Err(r) => {
                        refusal = Some((vec![format!("{}: {}", r.code, r.detail)], Vec::new()))
                    }
                }
            }
            policy::Checked::Deny { reason, rules } => refusal = Some((vec![reason], rules)),
            policy::Checked::AskNeeded => {
                refusal = Some((vec!["Start ohne Freigabe nicht möglich".into()], Vec::new()));
            }
        }
    }
    if let Some((reasons, rules)) = refusal {
        pre.deny_start(&boot, &mut gate_rx, &mut unacked, reasons.clone(), rules)
            .await;
        return Err(beton_harness::HarnessError::StartRefused(reasons.join("; ")).into());
    }
    let mut pre_events = pre.finish(&boot, &mut gate_rx, &mut unacked).await;
    // Set-Snapshot aus `session_start` jetzt über den Tunnel (POL-027 AC2).
    gate.flush_snapshot();
    let mut spec = SessionSpec {
        workdir: boot.workdir.clone(),
        model: start_settings.model.clone(),
        effort: start_settings.effort.clone(),
        permission_mode: start_settings.permission_mode,
        scenario: boot.scenario.clone(),
        resume: forked.resume.or_else(|| boot.resume.clone()),
        fork_session: forked.fork_session,
        mcp: setup.injection.clone(),
        max_turns: setup.max_turns,
        instructions: setup.instructions.clone(),
        // Ein Agent mit `permission_mode: plan` bzw. eine Policy, die `plan` erzwingt, verlässt
        // `plan` nicht selbst (AGT-011, HAR-027).
        plan_locked: settings::plan_locked(
            setup.agent_permission_mode,
            start_settings.permission_mode,
            policy_mode,
        ),
        hook: hook.clone(),
        compaction: boot.compaction,
        ..SessionSpec::default()
    };
    // Für einen Neustart mit Resume (Capability `restart`, HAR-017 AC2).
    let restart_ctx = ctx.clone();
    let session = registry.start(&boot.harness, spec.clone(), ctx).await;
    let mut session = match session {
        Ok(s) => s,
        Err(e) => {
            // Start gescheitert (z. B. inkompatible CLI, HAR-002 AC2): melden, dann enden.
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
    // Weicht die Session vom Adapter ab (Fake-Szenario, ACP), gelten ihre Capabilities.
    let mut session_caps = session
        .capabilities()
        .or_else(|| caps.clone())
        .unwrap_or_else(|| fallback_caps.clone());
    // Die Matrix der laufenden Session; weicht sie ab, erneut prüfen (POL-024, fail closed).
    if session_caps != predicted {
        gate.set_matrix(session_caps.enforcement());
        let check = beton_policy::enforcement::check_start(
            &gate.refresh(None),
            &session_caps.enforcement(),
        );
        if !check.refused.is_empty() {
            let _ = session.shutdown(Shutdown::Kill).await;
            let reasons: Vec<String> = check
                .refused
                .iter()
                .map(|g| format!("enforcement_unavailable: {}", g.describe()))
                .collect();
            let mut pre = PreStart::default();
            pre.deny_start(
                &boot,
                &mut gate_rx,
                &mut unacked,
                reasons.clone(),
                check.refused_rules(),
            )
            .await;
            return Err(beton_harness::HarnessError::StartRefused(reasons.join("; ")).into());
        }
    }
    // Ab jetzt prüft jeder Reload der Policies wie der Start (#234).
    gate.mark_started();
    let capabilities = session
        .capabilities()
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or(capabilities);
    // HAR-020 AC2: Die native Referenz steht im Log, bevor der erste Turn startet.
    let native_ref = session.native_session_ref().or_else(|| boot.resume.clone());
    // Ein Neustart setzt diese Session fort, nicht die Quelle eines Forks.
    spec.fork_session = false;
    // SES-001: nach `session.created` folgt `session.started`.
    pending_status.insert(
        0,
        Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::SessionStarted(beton_core::event::SessionStarted {
                runner_id: boot.runner_id,
                host_id: beton_core::id::HostId::LOCAL,
                harness: boot.harness.to_string(),
                harness_version: probe.version.unwrap_or_default(),
                capabilities,
                harness_session_ref: native_ref,
                sandbox: Some(beton_core::event::SessionSandbox {
                    tool_sandbox: sandbox_status.tool_sandbox,
                    egress_proxy: sandbox_status.egress_proxy,
                }),
            }),
        ),
    );
    for (i, payload) in forked.first.into_iter().enumerate() {
        pending_status.insert(i, system_event(&boot, payload));
    }
    for payload in forked.notices {
        pending_status.push(system_event(&boot, payload));
    }
    if let Some(mapped) = mapped_effort {
        // HAR-017 AC3: der gemappte Startwert, mit dem angefragten.
        pending_status.push(system_event(
            &boot,
            EventPayload::SessionSettingsChanged(mapped),
        ));
    }
    if boot.resume.is_some() {
        pending_status.insert(
            1,
            Event::new(
                boot.session_id,
                0,
                Actor::System {
                    component: beton_core::event::SystemComponent::Runner,
                },
                EventPayload::SessionResumed(beton_core::event::SessionResumed {
                    mode: beton_core::event::ResumeMode::Native,
                }),
            ),
        );
    }
    let mut harness_rx = session
        .events()
        .ok_or(RunnerError::Boot("Event-Strom fehlt".into()))?;
    // Hinweise und nicht injizierbare MCP-Server direkt nach dem Start.
    let mut reported_failures: std::collections::HashSet<String> = Default::default();
    for payload in std::mem::take(&mut setup.initial) {
        if let EventPayload::McpServerFailed(f) = &payload {
            reported_failures.insert(f.name.clone());
        }
        pending_status.push(system_event(&boot, payload));
    }
    let mut hub_events = setup.events.take();
    let mut system_calls = setup.calls.take();
    let mut pending_calls: HashMap<String, oneshot::Sender<Result<Value, Value>>> = HashMap::new();
    let mut next_call: u64 = 0;

    // Workspace-Beobachtung (SES-017 AC3) und Turn-Snapshots (SES-018).
    let (files, files_notice) = open_tracker(&boot).await;
    if let Some(text) = files_notice {
        pending_status.push(Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::Notice(beton_core::event::Notice {
                level: beton_core::event::NoticeLevel::Warn,
                text,
            }),
        ));
    }
    let (turn_tx, turn_rx) = watch::channel::<Option<TurnId>>(None);
    // Ungebunden: Der Beobachter meldet unter der Sperre des Trackers und darf dort nicht
    // warten (höchstens eine Meldung je Snapshot-Abstand).
    let (fs_tx, mut fs_rx) = mpsc::unbounded_channel::<(TurnId, Vec<FsChange>)>();
    if let Some(t) = &files {
        tokio::spawn(watch_workspace(t.clone(), turn_rx, fs_tx.clone()));
    }
    let mut pending_before: Option<String> = None;
    let mut pending_attachments: HashMap<String, beton_harness::InputAttachment> = HashMap::new();

    let mut tracker = StatusTracker::default();
    let mut turn_running = false;
    // Offene Freigaben (gestapelt, WEB-007): `waiting_approval`, solange eine offen ist.
    let mut open_approvals: std::collections::HashSet<beton_core::id::ApprovalId> =
        std::collections::HashSet::new();
    // SES-011 AC1: je Turn genau ein `context.usage` (der letzte Stand vor dem Turn-Ende).
    let mut held_context: Option<NormalizedEvent> = None;
    // Auslöser der angestoßenen Compaction (USE-009), bis `compaction.completed` kommt.
    let mut compaction_trigger: Option<CompactionTrigger> = None;
    // Policy-Ereignisse vor dem Start (Prüfung, `session_start`) stehen vor `session.started`.
    pre_events.extend(drain_gate(&boot, &mut gate_rx, &HashMap::new()));
    pending_status.splice(0..0, pre_events);
    while let Some(Ok(payload)) = sandbox_events.as_mut().map(|rx| rx.try_recv()) {
        pending_status.push(system_event(&boot, payload));
    }
    for e in pending_status.drain(..) {
        unacked.push(e);
    }
    // SBX-006 AC3: Endet ein Egress-Proxy der Session, pausiert sie, und neue
    // Tool-Ausführungen werden abgelehnt.
    let mut proxy_down: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> =
        stages.as_ref().map(|s| {
            Box::pin(s.proxy_down())
                as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        });
    #[cfg(feature = "test-sandbox")]
    if std::env::var(env::TEST_SANDBOX_PROXY_DOWN).is_ok_and(|v| v == "1")
        && let Some(s) = &stages
    {
        tracing::warn!(
            "Test-Schalter {}: Proxy beendet",
            env::TEST_SANDBOX_PROXY_DOWN
        );
        s.kill_proxies();
    }
    // Beobachtete Tool-Calls, die eine `deny`-Regel getroffen hätte (POL-023 AC3).
    let (observed_tx, mut observed_rx) =
        mpsc::unbounded_channel::<(beton_policy::load::RuleRef, String)>();
    let mut backoff = Backoff::default();
    // Letzter vom Server abgelehnter Push (PROTO-015, #72): Der Server trennt danach; der
    // Runner sendet nach dem Reconnect mit wachsendem Abstand erneut und meldet es.
    let mut push_failed: Option<String> = None;
    let mut parent_check = tokio::time::interval(Duration::from_millis(500));
    // HAR-017 AC4: Wechsel während eines Turns warten auf dessen Ende.
    let mut deferred: Option<settings::Change> = None;
    // Anzuwendender Wechsel und ggf. das Kommando, das auf die Antwort wartet.
    let mut apply: Option<(Option<String>, settings::Change)> = None;
    // Vorab vergebene ID des nächsten Turns (`effective_from_turn`).
    let mut next_turn: Option<TurnId> = None;
    // `executor.timeout` (AGT-004 AC3): Wanduhr ab dem ersten Turn dieses Runs. Danach wird der
    // laufende Turn unterbrochen und der Run endet mit `timed_out`.
    let run_timeout = setup.timeout;
    let mut deadline: Option<tokio::time::Instant> = None;
    let mut timed_out = false;
    // Reagiert der Harness nicht auf den Interrupt, endet der Run nach dieser Frist trotzdem.
    let mut force_end: Option<tokio::time::Instant> = None;
    // Policy (POL-025): Event-ID je `tool.call.requested` für `subject_ref`, Tool-Calls ohne
    // Gate-Anfrage (beobachtet), Ergebnisse geprüfter Eingaben (SES-004, #101).
    let mut call_events: HashMap<String, beton_core::id::EventId> = HashMap::new();
    let mut unobserved: HashMap<String, GateRequest> = HashMap::new();
    // Gestartete Calls (`tool.call.started`): nur ein nie gestarteter Call gilt mit `cancelled`
    // als nicht ausgeführt (HAR-005, #268).
    let mut started_calls: std::collections::HashSet<String> = std::collections::HashSet::new();
    // SBX-002: Mit `shell_override` muss jeder abgeschlossene Shell-Aufruf über den Exec-Broker
    // gelaufen sein, sonst endet die Session (fail closed).
    let mut integrity = stages
        .as_ref()
        .filter(|s| {
            !s.harness.policy().inherited_tools
                && predicted_caps(&boot, caps.as_ref(), true).tool_isolation
                    == beton_harness::ToolIsolation::ShellOverride
        })
        .map(|_| stages::ShellIntegrity::new(&beton_harness_claude::SHELL_TOOLS));
    // SBX-003 (#311): Weckt den Runner, wenn der Git-Shim der Stufe 1 einen Verstoß findet.
    let git_alarm = stages
        .as_ref()
        .and_then(stages::SessionStages::git_alarm)
        .unwrap_or_default();
    let (turn_gate_tx, mut turn_gate_rx) = mpsc::unbounded_channel::<TurnGate>();
    gate.order_after_requests();
    let mut answered: std::collections::HashSet<String> = Default::default();
    // Von einem späteren Daemon übernommen bzw. geordnet abgekoppelt (POL-010 AC4).
    let mut adopted = false;
    let mut detached_at: Option<std::time::Instant> = None;
    // Erneuerung des Runner-Tokens über den Tunnel (AUTH-011 AC2, #243).
    let mut renewal = token::Renewal::default();

    loop {
        // (Wieder-)Verbinden.
        let mut ws = match connect(&boot).await {
            Ok(ws) => ws,
            Err(RunnerError::Unauthorized) => {
                tracing::warn!("Tunnel lehnt das Runner-Token ab; Runner endet");
                return finish(session, Exit::Unauthorized).await;
            }
            Err(e) => {
                tracing::warn!("Tunnel nicht erreichbar: {e}");
                let keep = detached_keep(
                    detached_at,
                    std::time::Instant::now(),
                    gate.has_pending_policy(),
                );
                if orphan_exit(parent_alive(boot.parent_pid), adopted, keep) {
                    return finish(session, Exit::ParentGone).await;
                }
                tokio::time::sleep(backoff.next_delay()).await;
                continue;
            }
        };
        // Verbunden, obwohl der Elternprozess weg ist: ein neuer Daemon hat den Runner
        // übernommen (POL-010 AC4); ab jetzt zählt die Verbindung, nicht der Elternprozess.
        adopted |= !parent_alive(boot.parent_pid);
        detached_at = None;
        // Nach einem abgelehnten Push wächst der Abstand weiter, bis ein Push bestätigt ist.
        if push_failed.is_none() {
            backoff.reset();
        }
        renewal.reset();
        let hello = TunnelUp::Hello {
            kind: PeerKind::Runner,
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL.into(),
            harnesses: vec![boot.harness.to_string()],
        };
        let bind = TunnelUp::SessionBind {
            session_id: boot.session_id,
            epoch: boot.epoch,
            last_acked_rseq: unacked.acked(),
            prestart: false,
        };
        if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
            continue;
        }
        if lifecycle.state() == RunnerState::Lost {
            let _ = lifecycle.go(RunnerState::Reconnecting, None);
        }
        let mut extra = Vec::new();
        status(
            &mut lifecycle,
            RunnerState::Connected,
            push_failed
                .as_ref()
                .map(|code| format!("push_failed: {code}; Unbestätigtes wird erneut gesendet")),
            &mut extra,
        );
        status(&mut lifecycle, RunnerState::Idle, None, &mut extra);
        if !turn_running {
            tracker.set(&boot, SessionStatus::Idle, &mut extra);
        }
        for e in extra {
            unacked.push(e);
        }
        // Alles Unbestätigte erneut senden (Server dedupliziert über Event-IDs).
        let resend: Vec<RseqEvent> = unacked.queue.iter().map(|(e, _)| e.clone()).collect();
        if !resend.is_empty()
            && !send(
                &mut ws,
                &TunnelUp::EventsPush {
                    session_id: boot.session_id,
                    epoch: boot.epoch,
                    batch: resend,
                },
            )
            .await
        {
            continue;
        }
        // Offene Terminals erneut anmelden (PROTO-007); Frames erst im neuen Kredit.
        let mut announced = true;
        for up in terminals.connected() {
            announced &= send(&mut ws, &up).await;
        }
        if !announced {
            terminals.disconnected();
            continue;
        }

        let outcome: Option<Exit> = loop {
            // Einstellungen wechseln, solange kein Turn läuft (HAR-017, HAR-027).
            if !turn_running && let Some((cmd_id, change)) = apply.take() {
                let mechanism = change.mechanism(&session_caps);
                let result = if mechanism == beton_core::event::SettingsMechanism::Restart {
                    // Neustart mit Resume: den alten Harness beenden, den neuen mit der nativen
                    // Referenz und den neuen Einstellungen starten (HAR-017 AC2).
                    spec.resume = session.native_session_ref().or_else(|| spec.resume.clone());
                    change.apply_to(&mut spec);
                    let _ = session
                        .shutdown(Shutdown::Graceful {
                            timeout: Duration::from_secs(10),
                        })
                        .await;
                    match registry
                        .start(&boot.harness, spec.clone(), restart_ctx.clone())
                        .await
                    {
                        Ok(mut s) => {
                            let Some(rx) = s.events() else {
                                return Err(RunnerError::Boot("Event-Strom fehlt".into()));
                            };
                            harness_rx = rx;
                            session_caps = s.capabilities().unwrap_or_else(|| session_caps.clone());
                            session = s;
                            Ok(())
                        }
                        Err(e) => {
                            // Ohne Harness geht es nicht weiter: melden, Session `failed`.
                            let problem = json!({"type": format!("urn:beton:problem:{}", e.code()), "code": e.code(), "title": e.to_string()});
                            let mut batch = vec![system_event(
                                &boot,
                                EventPayload::Error(beton_core::event::ErrorEvent {
                                    problem: problem.clone(),
                                }),
                            )];
                            status(
                                &mut lifecycle,
                                RunnerState::Failed,
                                Some(e.to_string()),
                                &mut batch,
                            );
                            batch.push(system_event(
                                &boot,
                                EventPayload::SessionStatus(SessionStatusChanged {
                                    status: SessionStatus::Failed,
                                    reason: Some("Neustart des Harness gescheitert".into()),
                                }),
                            ));
                            if let Some(cmd_id) = cmd_id {
                                let _ = send(
                                    &mut ws,
                                    &TunnelUp::CmdResult {
                                        cmd_id,
                                        result: None,
                                        problem: Some(problem),
                                    },
                                )
                                .await;
                            }
                            let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                            drain_acks(&mut ws, &mut unacked).await;
                            return Ok(Exit::HarnessExited { code: None });
                        }
                    }
                } else {
                    apply_live(&mut *session, &change).await
                };
                let reply = match result {
                    Ok(()) => {
                        // Gilt ab dem nächsten Turn; seine ID steht schon im Event (HAR-017 AC4).
                        let turn = *next_turn.get_or_insert_with(TurnId::new);
                        change.apply_to(&mut spec);
                        let event = system_event(
                            &boot,
                            EventPayload::SessionSettingsChanged(
                                change.event(Some(mechanism), Some(turn)),
                            ),
                        );
                        if !push(&mut ws, &boot, &mut unacked, vec![event]).await {
                            break None;
                        }
                        TunnelUp::CmdResult {
                            cmd_id: cmd_id.clone().unwrap_or_default(),
                            result: Some(
                                json!({"mechanism": mechanism, "effective_from_turn": turn}),
                            ),
                            problem: None,
                        }
                    }
                    Err(r) => TunnelUp::CmdResult {
                        cmd_id: cmd_id.clone().unwrap_or_default(),
                        result: None,
                        problem: Some(r.to_problem()),
                    },
                };
                if cmd_id.is_some() && !send(&mut ws, &reply).await {
                    break None;
                }
            }
            let paused = unacked.bytes > MAX_UNACKED_BYTES;
            tokio::select! {
                // Gate-Events zuerst: was das Gate vor einer Antwort an den Harness meldet,
                // steht im Log vor den Folge-Events des Harness; ein `policy.decision` wartet
                // seinerseits auf sein `tool.call.requested` (POL-025).
                biased;
                () = async {
                    match proxy_down.as_mut() {
                        Some(f) => f.await,
                        None => std::future::pending().await,
                    }
                } => {
                    proxy_down = None;
                    if let Some(st) = &stages {
                        st.halt("proxy_down");
                    }
                    gate.halt_tools(
                        "proxy_down: Der Egress-Proxy der Session ist beendet; neue \
                         Tool-Ausführungen sind abgelehnt, die Session ist pausiert (SBX-006)",
                    );
                    let mut batch = vec![system_event(
                        &boot,
                        EventPayload::SandboxViolation(beton_core::event::SandboxViolation {
                            stage: beton_core::event::SandboxStage::Tools,
                            kind: beton_core::event::ViolationKind::ProxyDown,
                            target: None,
                            suppressed_count: 0,
                        }),
                    )];
                    tracker.pause(&boot, "proxy_down: Egress-Proxy beendet", &mut batch);
                    if !push(&mut ws, &boot, &mut unacked, batch).await {
                        break None;
                    }
                }
                Some((rule, reason)) = observed_rx.recv() => {
                    // POL-023 AC3: `notify` und Abbruch des Turns (`session/cancel` bei ACP).
                    let event = system_event(
                        &boot,
                        EventPayload::PolicyNotified(beton_core::event::PolicyNotified {
                            rule_id: rule.rule_id.clone(),
                            scope: rule.scope.as_str().to_owned(),
                            set: rule.set.clone(),
                            channels: vec!["ui".into()],
                            message: Some(format!(
                                "Nur beobachteter Tool-Call verstößt gegen eine Policy: {reason}; \
                                 der Turn wird abgebrochen"
                            )),
                        }),
                    );
                    let mut batch = drain_gate(&boot, &mut gate_rx, &call_events);
                    batch.push(event);
                    if !push(&mut ws, &boot, &mut unacked, batch).await {
                        break None;
                    }
                    if turn_running {
                        let _ = session.interrupt().await;
                    }
                }
                ge = gate_rx.recv() => {
                    let Some(first) = ge else { continue };
                    let mut fatal = first.fatal.then(|| fatal_reason(&first));
                    let mut events = vec![gate_event(&boot, first, &call_events)];
                    while let Ok(ge) = gate_rx.try_recv() {
                        if ge.fatal && fatal.is_none() {
                            fatal = Some(fatal_reason(&ge));
                        }
                        events.push(gate_event(&boot, ge, &call_events));
                    }
                    let mut batch = Vec::with_capacity(events.len() + 1);
                    let mut abort = false;
                    for e in events {
                        if let Some(EventPayload::ApprovalResolved(r)) = e.payload() {
                            abort |= r.decision == beton_core::event::ApprovalDecision::Abort;
                        }
                        let next = approval_status(e.payload(), &mut open_approvals, turn_running);
                        batch.push(e);
                        if let Some(st) = next {
                            tracker.set(&boot, st, &mut batch);
                        }
                    }
                    if let Some(why) = fatal {
                        // Fail closed (#234): Policies nach dem Start nicht durchsetzbar.
                        end_policy_failure(&boot, &mut lifecycle, &why, &mut batch);
                        let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                        drain_acks(&mut ws, &mut unacked).await;
                        let _ = session.shutdown(Shutdown::Kill).await;
                        return Ok(Exit::PolicyViolation);
                    }
                    if !push(&mut ws, &boot, &mut unacked, batch).await {
                        break None;
                    }
                    // Abgelaufen mit `on_timeout: abort` (POL-010): Turn abbrechen.
                    if abort && turn_running {
                        let _ = session.interrupt().await;
                    }
                }
                ev = harness_rx.recv(), if !paused => {
                    let Some(first) = ev else { break Some(Exit::HarnessExited { code: None }) };
                    // Bereits anstehende Events bündeln: ein Push, eine Transaktion beim Server.
                    let mut incoming = vec![first];
                    while incoming.len() < 256 {
                        match harness_rx.try_recv() {
                            Ok(e) => incoming.push(e),
                            Err(_) => break,
                        }
                    }
                    let mut batch = Vec::with_capacity(incoming.len() + 4);
                    // Meldungen des MCP-Hubs, die vor diesen Harness-Events entstanden sind (etwa
                    // `mcp.server_failed` beim MCP-Start des Harness), stehen auch im Log davor;
                    // `biased` würde sie sonst hinter das Turn-Ende schieben (#193).
                    if let Some(rx) = hub_events.as_mut() {
                        while let Ok(payload) = rx.try_recv() {
                            if let EventPayload::McpServerFailed(f) = &payload
                                && !reported_failures.insert(f.name.clone())
                            {
                                continue;
                            }
                            batch.push(system_event(&boot, payload));
                        }
                    }
                    let mut exited = None;
                    let mut run_over = false;
                    let mut sandbox_violation = None;
                    for mut ev in incoming {
                        // SBX-002: Shell-Aufruf ohne Start über den Exec-Broker (Stufe 1).
                        let sandbox_hit = match (integrity.as_mut(), stages.as_ref()) {
                            (Some(i), Some(s)) => i.observe(&ev.payload, s.broker_starts()).err().map(|r| (SHELL_OUTSIDE_STAGE_TWO, r)),
                            _ => None,
                        };
                        // SBX-003 (#209): Nach jedem Tool-Lauf dürfen keine Git-Steuerdateien
                        // (`commondir`/`gitdir`) entstanden oder geändert sein.
                        let sandbox_hit = sandbox_hit.or_else(|| match (&ev.payload, stages.as_ref()) {
                            (EventPayload::ToolCallCompleted(_), Some(s)) => s.check_git().err().map(|r| (GIT_CONTROL_CHANGED, r)),
                            _ => None,
                        });
                        if timed_out && let EventPayload::TurnInterrupted(t) = &mut ev.payload {
                            t.reason = Some(TIMED_OUT.into());
                        }
                        stamp_compaction(&mut ev.payload, &mut compaction_trigger);
                        if let EventPayload::ContextUsage(c) = &mut ev.payload {
                            fill_window(c, spec.model.as_deref());
                        }
                        if deadline.is_none()
                            && let (Some(limit), EventPayload::TurnStarted(_)) = (run_timeout, &ev.payload)
                        {
                            deadline = Some(tokio::time::Instant::now() + limit);
                        }
                        // `mcp.server_failed` je Server nur einmal (Hub und Harness melden beide).
                        if let EventPayload::McpServerFailed(f) = &ev.payload
                            && !reported_failures.insert(f.name.clone())
                        {
                            continue;
                        }
                        if let EventPayload::HarnessExited(x) = &ev.payload {
                            exited = Some(x.clone());
                        }
                        let busy = matches!(ev.payload, EventPayload::TurnStarted(_));
                        let next_status = match &ev.payload {
                            EventPayload::TurnStarted(_) if !open_approvals.is_empty() => Some(SessionStatus::WaitingApproval),
                            EventPayload::TurnStarted(_) => Some(SessionStatus::Running),
                            EventPayload::ApprovalRequested(_) | EventPayload::ApprovalResolved(_) => {
                                approval_status(Some(&ev.payload), &mut open_approvals, true)
                            }
                            EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_) | EventPayload::TurnInterrupted(_) => Some(SessionStatus::Idle),
                            _ => None,
                        };
                        let done = matches!(ev.payload, EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_) | EventPayload::TurnInterrupted(_));
                        if turn_running && matches!(ev.payload, EventPayload::ContextUsage(_)) {
                            held_context = Some(ev);
                            continue;
                        }
                        if let Some(t) = &files {
                            if let EventPayload::TurnStarted(ts) = &ev.payload {
                                let tree = match pending_before.take() {
                                    Some(b) => Some(b),
                                    None => workspace::poll(t).await.map(|(_, tree)| tree),
                                };
                                if let Some(tree) = tree {
                                    workspace::mark(t, ts.turn_id.to_string(), "before", tree).await;
                                }
                                let _ = turn_tx.send(Some(ts.turn_id));
                            }
                            if done {
                                // Letzte Änderungen des Turns vor seinem Ende melden.
                                let turn = (*turn_tx.borrow()).or(ev.turn_id);
                                let _ = turn_tx.send(None);
                                if let Some(turn) = turn
                                    && let Some((changes, tree)) = workspace::poll(t).await
                                {
                                    // Was der Beobachter vor diesem Snapshot gefunden hat, steht
                                    // ebenfalls vor dem Turn-Ende im Log (#189).
                                    while let Ok((seen, found)) = fs_rx.try_recv() {
                                        batch.push(fs_changed(&boot, &actor, seen, found));
                                    }
                                    if !changes.is_empty() {
                                        batch.push(fs_changed(&boot, &actor, turn, changes));
                                    }
                                    workspace::mark(t, turn.to_string(), "after", tree).await;
                                }
                            }
                        }
                        if done && let Some(context) = held_context.take() {
                            batch.push(to_event(&boot, &actor, context));
                        }
                        // Integrität (HAR-005, #233): Ein ausgeführter Call ohne Gate-Entscheidung
                        // bei Matrix `full` heißt, dass der Hook umgangen wurde.
                        // Nur, wenn der Harness selbst jeden Call durch das Gate schickt (Hook,
                        // Direkt-API); MCP-Aufrufe über den Relay-Hub prüft der Hub synchron.
                        let violation = match &ev.payload {
                            EventPayload::ToolCallStarted(st) => {
                                started_calls.insert(st.call_id.clone());
                                None
                            }
                            EventPayload::ToolCallCompleted(c) => {
                                let started = started_calls.remove(&c.call_id);
                                if harness_gates_all && executed(c.status, started) {
                                    unobserved
                                        .get(&c.call_id)
                                        .and_then(|r| gate.integrity_violation(r).map(|why| (r.clone(), why)))
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        let observe = track_tool_calls(&gate, &ev, &mut unobserved, done, !harness_gates_all);
                        let event = to_event(&boot, &actor, ev);
                        if let Some(EventPayload::ToolCallRequested(r)) = event.payload() {
                            call_events.insert(r.call_id.clone(), event.id);
                        }
                        batch.push(event);
                        if let Some((req, why)) = violation {
                            // Audit (genau ein `policy.decision`, POL-025 AC1), dann Ende mit
                            // Fehler (fail closed).
                            gate.observe(req).await;
                            batch.extend(drain_gate(&boot, &mut gate_rx, &call_events));
                            end_integrity_violation(&boot, &mut lifecycle, &why, &mut batch);
                            let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                            drain_acks(&mut ws, &mut unacked).await;
                            let _ = session.shutdown(Shutdown::Kill).await;
                            return Ok(Exit::PolicyViolation);
                        }
                        // Nach der Policy-Integrität (die hat Vorrang und endet oben sofort):
                        // höchstens ein Integritätsfehler, keine weiteren Events dieses Stapels.
                        if let Some(reason) = sandbox_hit {
                            sandbox_violation = Some(reason);
                            break;
                        }
                        for req in observe {
                            // Ohne Gate-Anfrage des Harness: nur beobachtet (POL-025 AC1); trifft
                            // eine `deny`-Regel, meldet der Runner es und bricht den Turn ab
                            // (POL-023 AC3).
                            let g = gate.clone();
                            let tx = observed_tx.clone();
                            tokio::spawn(async move {
                                if let Some(hit) = g.observe(req).await {
                                    let _ = tx.send(hit);
                                }
                            });
                        }
                        if busy { status(&mut lifecycle, RunnerState::Busy, None, &mut batch); turn_running = true; held_context = None; }
                        if done {
                            status(&mut lifecycle, RunnerState::Idle, None, &mut batch);
                            turn_running = false;
                            // Aufgeschobene Wechsel jetzt anwenden (HAR-017 AC4).
                            if let Some(change) = deferred.take() {
                                apply = Some((None, change));
                            }
                        }
                        if let Some(st) = next_status {
                            tracker.set(&boot, st, &mut batch);
                        }
                        if done && timed_out {
                            run_over = true;
                            break;
                        }
                    }
                    if let Some((title, reason)) = sandbox_violation {
                        // Kommandos liefen in Stufe 1: sofort beenden, Session `failed`.
                        tracing::error!("Integritätsprüfung der Sandbox: {reason}");
                        let _ = session.shutdown(Shutdown::Kill).await;
                        // Erst beenden, dann (erneut) zurücksetzen: Prozesse der Stufe 2 enden
                        // mit ihren `bt-exec`-Verbindungen (SBX-003, #209).
                        if let Some(s) = stages.as_ref() { s.restore_git(); }
                        end_sandbox_integrity_violation(&boot, &mut lifecycle, title, &reason, &mut batch);
                        let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                        drain_acks(&mut ws, &mut unacked).await;
                        if let Some(s) = stages.as_ref() { s.restore_git(); }
                        return Ok(Exit::PolicyViolation);
                    }
                    if run_over {
                        end_timed_out(&boot, &mut lifecycle, &mut batch);
                        let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                        drain_acks(&mut ws, &mut unacked).await;
                        let _ = session.shutdown(Shutdown::Graceful { timeout: Duration::from_secs(5) }).await;
                        return Ok(Exit::TimedOut);
                    }
                    if let Some(x) = &exited {
                        // RUN-003 AC3 / HAR-001 AC1: Absturz → Runner failed, Session failed.
                        let tail: Vec<&str> = x.stderr_tail.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev().collect();
                        let reason = format!("Exit-Code {:?}, Signal {:?}\n{}", x.code, x.signal, tail.join("\n"));
                        status(&mut lifecycle, RunnerState::Failed, Some(reason), &mut batch);
                        batch.push(Event::new(boot.session_id, 0, Actor::System { component: beton_core::event::SystemComponent::Runner },
                            EventPayload::SessionStatus(SessionStatusChanged { status: SessionStatus::Failed, reason: Some("Harness beendet".into()) })));
                    }
                    if !push(&mut ws, &boot, &mut unacked, batch).await {
                        break None;
                    }
                    if let Some(x) = exited {
                        // Bestätigung abwarten, damit nichts verloren geht.
                        drain_acks(&mut ws, &mut unacked).await;
                        return Ok(Exit::HarnessExited { code: x.code });
                    }
                }
                out = term_rx.recv() => {
                    let Some(out) = out else { continue };
                    if !forward_terminal(&mut ws, &boot, &mut unacked, out).await { break None; }
                }
                ev = recv_opt(&mut hub_events) => {
                    let Some(payload) = ev else { hub_events = None; continue };
                    if let EventPayload::McpServerFailed(f) = &payload
                        && !reported_failures.insert(f.name.clone())
                    {
                        continue;
                    }
                    if !push(&mut ws, &boot, &mut unacked, vec![system_event(&boot, payload)]).await {
                        break None;
                    }
                }
                call = usage_rx.recv() => {
                    let Some(call) = call else { continue };
                    next_call += 1;
                    let call_id = format!("sys_{next_call}");
                    let up = TunnelUp::SystemCall { call_id: call_id.clone(), tool: call.tool, args: call.args };
                    pending_calls.insert(call_id, call.reply);
                    if !send(&mut ws, &up).await { break None; }
                }
                tg = turn_gate_rx.recv() => {
                    let Some(tg) = tg else { continue };
                    match tg {
                        TurnGate::Waiting { cmd_id, turn } => {
                            // `ask`: Antwort sofort, der Turn startet nach der Freigabe.
                            answered.insert(cmd_id.clone());
                            let msg = TunnelUp::CmdResult { cmd_id, result: Some(json!({"turn_id": turn, "status": "waiting_approval"})), problem: None };
                            if !send(&mut ws, &msg).await { break None; }
                        }
                        TurnGate::Done { cmd_id, name, args, turn, checked } => {
                            let waited = answered.remove(&cmd_id);
                            let reply = match checked {
                                policy::Checked::Allow(m) => {
                                    match apply_turn_modifications(&mut *session, &m, &session_caps, &mut spec).await {
                                        Err(problem) => Err(problem),
                                        Ok(changed) => {
                                            if let Some(change) = changed {
                                                // HAR-017: Auch ein Wechsel durch eine Policy steht im
                                                // Log, ab diesem Turn (POL-011 AC2, POL-013).
                                                let event = system_event(&boot, EventPayload::SessionSettingsChanged(
                                                    change.event(Some(beton_core::event::SettingsMechanism::Live), turn.or(next_turn)),
                                                ));
                                                if !push(&mut ws, &boot, &mut unacked, vec![event]).await { break None; }
                                            }
                                            gate.set_settings(spec.model.clone(), spec.effort.clone(), spec.permission_mode);
                                            deliver(&mut *session, &gate, &name, args, turn_running, &mut handover, &mut next_turn, &mut pending_attachments).await
                                        }
                                    }
                                }
                                policy::Checked::Deny { reason, .. } => Err(json!({"code": "policy_denied", "detail": reason})),
                                policy::Checked::AskNeeded => Err(json!({"code": "approval_required", "detail": "Die Eingabe braucht eine Freigabe; sie wird als eigener Turn eingereiht"})),
                            };
                            if name == "input.submit" {
                                turn_running = reply.is_ok();
                                if reply.is_err() {
                                    next_turn = None;
                                }
                            }
                            if waited {
                                // Der Client wartet nicht mehr: als Turn-Ende im Verlauf (die Queue
                                // arbeitet danach weiter).
                                if let (Err(problem), Some(turn)) = (&reply, turn) {
                                    let mut batch = vec![system_event(&boot, EventPayload::TurnFailed(beton_core::event::TurnFailed {
                                        turn_id: turn,
                                        problem: json!({"type": format!("urn:beton:problem:{}", problem["code"].as_str().unwrap_or("policy_denied")), "code": problem["code"], "title": "Eingabe nicht freigegeben", "detail": problem["detail"]}),
                                    }))];
                                    tracker.set(&boot, SessionStatus::Idle, &mut batch);
                                    if !push(&mut ws, &boot, &mut unacked, batch).await { break None; }
                                }
                            } else {
                                let msg = match reply {
                                    Ok(result) => TunnelUp::CmdResult { cmd_id, result: Some(result), problem: None },
                                    Err(problem) => TunnelUp::CmdResult { cmd_id, result: None, problem: Some(problem) },
                                };
                                if !send(&mut ws, &msg).await { break None; }
                            }
                        }
                    }
                }
                ev = recv_opt(&mut sandbox_events) => {
                    let Some(payload) = ev else { sandbox_events = None; continue };
                    if !push(&mut ws, &boot, &mut unacked, vec![system_event(&boot, payload)]).await {
                        break None;
                    }
                }
                call = recv_opt(&mut system_calls) => {
                    let Some(call) = call else { system_calls = None; continue };
                    next_call += 1;
                    let call_id = format!("sys_{next_call}");
                    let up = TunnelUp::SystemCall { call_id: call_id.clone(), tool: call.tool, args: call.args };
                    pending_calls.insert(call_id, call.reply);
                    if !send(&mut ws, &up).await { break None; }
                }
                () = sleep_until_opt(renewal.due()) => {
                    let up = renewal.request();
                    if !send(&mut ws, &up).await { break None; }
                }
                msg = ws.next() => {
                    let Some(Ok(msg)) = msg else { break None };
                    if let Message::Close(Some(frame)) = &msg
                        && is_detach_signal(u16::from(frame.code), &frame.reason)
                    {
                        detached_at = Some(std::time::Instant::now());
                    }
                    if let Message::Binary(bytes) = &msg {
                        // Eingaben an ein Terminal (PROTO-007).
                        terminals.input_frame(bytes);
                        continue;
                    }
                    let Message::Text(text) = msg else { continue };
                    let Ok(down) = serde_json::from_str::<TunnelDown>(&text) else { continue };
                    match down {
                        TunnelDown::ChannelCredit { channel_id, bytes } => terminals.credit(channel_id, bytes),
                        TunnelDown::ChannelKeyframe { channel_id } => terminals.keyframe(channel_id),
                        TunnelDown::CmdDeliver { cmd_id, name, args, .. } if name == "workspace.user_write" => {
                            // #93: Der User schreibt gleich über die Workspace-API; derselbe Stand
                            // ist dann keine Änderung des Agents.
                            if let (Some(t), Some(path), Some(sha)) = (&files, args["path"].as_str(), args["sha256"].as_str()) {
                                workspace::expect_user_write(t, path.to_owned(), sha.to_owned());
                            }
                            let msg = TunnelUp::CmdResult { cmd_id, result: Some(Value::Null), problem: None };
                            if !send(&mut ws, &msg).await { break None; }
                        }
                        TunnelDown::CmdDeliver { cmd_id, name, args, actor } if name.starts_with("terminal.") => {
                            // Workspace-Terminals (SES-019): PTY hier, Prozess in Stufe 2.
                            let msg = match terminals.command(&name, &args, actor).await {
                                Ok(result) => TunnelUp::CmdResult { cmd_id, result: Some(result), problem: None },
                                Err(e) => TunnelUp::CmdResult { cmd_id, result: None, problem: Some(e.to_problem()) },
                            };
                            // Kanal und `terminal.opened` vor der Antwort: Ein Client, der danach
                            // den Kanal öffnet, findet ihn (PROTO-007).
                            let mut ok = true;
                            while let Ok(out) = term_rx.try_recv() {
                                ok &= forward_terminal(&mut ws, &boot, &mut unacked, out).await;
                            }
                            if !ok || !send(&mut ws, &msg).await { break None; }
                        }
                        TunnelDown::EventsAck { upto_rseq, .. } => {
                            unacked.ack(upto_rseq);
                            if push_failed.take().is_some() {
                                backoff.reset();
                            }
                        }
                        TunnelDown::Bound { acked_rseq, .. } => unacked.ack(acked_rseq),
                        TunnelDown::CmdDeliver { cmd_id, name, args, .. } if name == ONE_SHOT_CMD => {
                            // Kann dauern (Modellaufruf): nicht im Lese-Loop warten.
                            let adapter = one_shot_adapter.clone();
                            let ctx = one_shot_ctx.clone();
                            let tx = one_shot_tx.clone();
                            let request = one_shot_request(&boot, &args);
                            tokio::spawn(async move {
                                let reply = run_one_shot(adapter, request, ctx).await;
                                let _ = tx.send((cmd_id, reply));
                            });
                        }
                        TunnelDown::CmdDeliver { cmd_id, name, args, .. } => {
                            // Stand vor dem Turn festhalten, bevor der Harness die Eingabe sieht.
                            if name == "input.submit"
                                && turn_tx.borrow().is_none()
                                && let Some(t) = &files
                            {
                                pending_before = workspace::poll(t).await.map(|(_, tree)| tree);
                            }
                            // Phase `model_request` je Turn (POL-003, SES-004 #101): Eingabe und
                            // Steer prüft das Gate, bevor der Harness sie sieht.
                            if (name == "input.submit" || (name == "input.steer" && turn_running))
                                && session_caps.model_request_gate == beton_harness::ModelRequestGate::Turn
                            {
                                let submit = name == "input.submit";
                                let turn = submit.then(|| *next_turn.get_or_insert_with(TurnId::new));
                                if submit {
                                    turn_running = true;
                                }
                                let request = beton_harness::ModelGateRequest {
                                    turn_id: turn,
                                    model: spec.model.clone(),
                                    reasoning_effort: spec.effort.clone(),
                                    prompt: args["text"].as_str().map(str::to_owned),
                                    ..beton_harness::ModelGateRequest::default()
                                };
                                let (g, tx) = (gate.clone(), turn_gate_tx.clone());
                                tokio::spawn(async move {
                                    let (wait_tx, wait_rx) = oneshot::channel();
                                    let waiting_cmd = cmd_id.clone();
                                    let notify = tx.clone();
                                    tokio::spawn(async move {
                                        if wait_rx.await.is_ok() {
                                            let _ = notify.send(TurnGate::Waiting { cmd_id: waiting_cmd, turn });
                                        }
                                    });
                                    // Steer wartet nicht auf Menschen: `ask` reiht ein (Server).
                                    let checked = g.check_model_request(request, "turn", submit, submit.then_some(wait_tx)).await;
                                    let _ = tx.send(TurnGate::Done { cmd_id, name, args, turn, checked });
                                });
                                continue;
                            }
                            if name == "session.set" {
                                // HAR-017, HAR-027: prüfen, dann sofort bzw. nach dem Turn anwenden.
                                match settings::parse(&args, &session_caps, sandbox_status) {
                                    Err(r) => {
                                        let msg = TunnelUp::CmdResult { cmd_id, result: None, problem: Some(r.to_problem()) };
                                        if !send(&mut ws, &msg).await { break None; }
                                    }
                                    Ok(change) if change.is_empty() => {
                                        let msg = TunnelUp::CmdResult { cmd_id, result: Some(Value::Null), problem: None };
                                        if !send(&mut ws, &msg).await { break None; }
                                    }
                                    Ok(change) if turn_running => {
                                        deferred.get_or_insert_with(settings::Change::default).merge(change);
                                        let msg = TunnelUp::CmdResult { cmd_id, result: Some(json!({"deferred": true})), problem: None };
                                        if !send(&mut ws, &msg).await { break None; }
                                    }
                                    Ok(change) => apply = Some((Some(cmd_id), change)),
                                }
                                continue;
                            }
                            if name == "session.compact" && !turn_running {
                                compaction_trigger = Some(requested_trigger(&args));
                            }
                            let reply = deliver(&mut *session, &gate, &name, args, turn_running, &mut handover, &mut next_turn, &mut pending_attachments).await;
                            if name == "session.compact" && reply.is_err() {
                                compaction_trigger = None;
                            }
                            // Ab der Zustellung läuft ein Turn, auch bevor `turn.started` ankommt.
                            if name == "input.submit" && reply.is_ok() {
                                turn_running = true;
                            }
                            let msg = match reply {
                                Ok(result) => TunnelUp::CmdResult { cmd_id, result: Some(result), problem: None },
                                Err(problem) => TunnelUp::CmdResult { cmd_id, result: None, problem: Some(problem) },
                            };
                            if !send(&mut ws, &msg).await { break None; }
                        }
                        TunnelDown::RunnerStop { grace_s, .. } => {
                            // Terminals enden mit dem Runner (SES-019).
                            let mut batch: Vec<Event> = terminals.shutdown().into_iter().map(|p| system_event(&boot, p)).collect();
                            status(&mut lifecycle, RunnerState::Draining, Some("runner.stop".into()), &mut batch);
                            status(&mut lifecycle, RunnerState::Terminated, None, &mut batch);
                            let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                            drain_acks(&mut ws, &mut unacked).await;
                            let _ = session.shutdown(Shutdown::Graceful { timeout: Duration::from_secs(grace_s) }).await;
                            return Ok(Exit::Stopped);
                        }
                        TunnelDown::Problem { problem } => {
                            if problem["code"] == "stale_epoch" {
                                let _ = session.shutdown(Shutdown::Kill).await;
                                return Err(RunnerError::StaleEpoch);
                            }
                            tracing::warn!(?problem, "Tunnel-Problem");
                            // Ein nicht gespeicherter Push: Der Server trennt gleich (4500).
                            push_failed = Some(problem["code"].as_str().unwrap_or("internal").to_owned());
                        }
                        TunnelDown::SystemResult { call_id, mut result, problem } => {
                            if renewal.on_result(&call_id, &mut result, &boot.token) {
                                continue;
                            }
                            if let Some(reply) = pending_calls.remove(&call_id) {
                                let _ = reply.send(match (result, problem) {
                                    (Some(r), None) => Ok(r),
                                    (_, p) => Err(p.unwrap_or_else(|| json!({"code": "internal"}))),
                                });
                            }
                        }
                        TunnelDown::Welcome { .. } => {}
                    }
                }
                Some((cmd_id, reply)) = one_shot_rx.recv() => {
                    let msg = match reply {
                        Ok(result) => TunnelUp::CmdResult { cmd_id, result: Some(result), problem: None },
                        Err(problem) => TunnelUp::CmdResult { cmd_id, result: None, problem: Some(problem) },
                    };
                    if !send(&mut ws, &msg).await { break None; }
                }
                Some((turn, changes)) = fs_rx.recv() => {
                    if !push(&mut ws, &boot, &mut unacked, vec![fs_changed(&boot, &actor, turn, changes)]).await {
                        break None;
                    }
                }
                () = sleep_until_opt(deadline), if !timed_out => {
                    timed_out = true;
                    let limit = run_timeout.unwrap_or_default();
                    let mut batch = vec![system_event(&boot, EventPayload::Notice(beton_core::event::Notice {
                        level: beton_core::event::NoticeLevel::Warn,
                        text: format!("Zeitlimit des Agents erreicht (executor.timeout, {} s); der Run endet mit timed_out.", limit.as_secs_f64()),
                    }))];
                    if turn_running {
                        if let Err(e) = session.interrupt().await {
                            tracing::warn!("Interrupt nach Zeitlimit: {e}");
                        }
                        force_end = Some(tokio::time::Instant::now() + TIMEOUT_GRACE);
                        if !push(&mut ws, &boot, &mut unacked, batch).await { break None; }
                    } else {
                        end_timed_out(&boot, &mut lifecycle, &mut batch);
                        let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                        drain_acks(&mut ws, &mut unacked).await;
                        let _ = session.shutdown(Shutdown::Graceful { timeout: Duration::from_secs(5) }).await;
                        return Ok(Exit::TimedOut);
                    }
                }
                () = sleep_until_opt(force_end) => {
                    // Der Harness hat den Turn nicht beendet: Run trotzdem beenden (fail closed).
                    let mut batch = Vec::new();
                    end_timed_out(&boot, &mut lifecycle, &mut batch);
                    let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                    drain_acks(&mut ws, &mut unacked).await;
                    let _ = session.shutdown(Shutdown::Kill).await;
                    return Ok(Exit::TimedOut);
                }
                // SBX-003 (#311): Der Git-Shim der Stufe 1 hat vor einem `git` eine neue oder
                // geänderte Git-Steuerdatei gefunden und git nicht gestartet; die Session
                // endet sofort, nicht erst nach dem nächsten Tool-Lauf.
                _ = git_alarm.notified() => {
                    let Some(Err(reason)) = stages.as_ref().map(stages::SessionStages::check_git) else { continue };
                    tracing::error!("Integritätsprüfung der Sandbox: {reason}");
                    let _ = session.shutdown(Shutdown::Kill).await;
                    if let Some(s) = stages.as_ref() { s.restore_git(); }
                    let mut batch = Vec::new();
                    end_sandbox_integrity_violation(&boot, &mut lifecycle, GIT_CONTROL_CHANGED, &reason, &mut batch);
                    let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                    drain_acks(&mut ws, &mut unacked).await;
                    if let Some(s) = stages.as_ref() { s.restore_git(); }
                    return Ok(Exit::PolicyViolation);
                }
                _ = parent_check.tick() => {
                    // Mit offener Freigabe auf den nächsten Daemon warten (POL-010 AC4);
                    // ein übernommener Runner folgt der Verbindung, nicht dem Elternprozess.
                    let keep = detached_keep(detached_at, std::time::Instant::now(), gate.has_pending_policy());
                    if !adopted && !parent_alive(boot.parent_pid) && !keep {
                        return finish(session, Exit::ParentGone).await;
                    }
                }
            }
        };
        // Offene System-Tool-Aufrufe scheitern mit der Verbindung (fail closed).
        for (_, reply) in pending_calls.drain() {
            let _ = reply.send(Err(
                json!({"code": "unavailable", "detail": "Verbindung zum Server getrennt"}),
            ));
        }
        if let Some(exit) = outcome {
            terminals.shutdown();
            return finish(session, exit).await;
        }
        // Scheiterte ein Senden, weil der Server nach einem nicht gespeicherten Push trennte,
        // steht sein `problem` noch im Socket (PROTO-015, #72).
        while let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), ws.next()).await
        {
            if let Ok(TunnelDown::Problem { problem }) = serde_json::from_str::<TunnelDown>(&text)
                && problem["code"] != "stale_epoch"
            {
                tracing::warn!(?problem, "Push nicht gespeichert");
                push_failed = Some(problem["code"].as_str().unwrap_or("internal").to_owned());
            }
        }
        // Verbindung verloren: weiterlaufen, puffern, neu verbinden; Terminals lesen frei weiter.
        terminals.disconnected();
        if lifecycle
            .go(RunnerState::Lost, Some("Tunnel getrennt".into()))
            .is_ok()
        {
            tracing::warn!("Tunnel getrennt, verbinde neu");
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// Ergebnis der Phase `model_request` für eine Eingabe (SES-004).
enum TurnGate {
    /// Eine Approval-Card ist offen; die Eingabe wartet.
    Waiting {
        cmd_id: String,
        turn: Option<TurnId>,
    },
    Done {
        cmd_id: String,
        name: String,
        args: Value,
        turn: Option<TurnId>,
        checked: policy::Checked,
    },
}

/// Capabilities, mit denen der Harness laufen wird: beim Fake samt Szenario, bei Claude ohne
/// Hook-Brücke nur die Rückfragen (POL-024).
/// Bekommt der Harness die Stufe-2-Tools und Terminals des Runners für Tools, die beton selbst
/// ausführt? Mit `tool_isolation: beton` immer, außerdem ACP (`fs/read_text_file`,
/// `terminal/*`), das bis #329 als `inherited` gilt.
fn beton_tools_in_stage_two(harness: &HarnessId, isolation: beton_harness::ToolIsolation) -> bool {
    isolation == beton_harness::ToolIsolation::Beton || harness.is_acp()
}

fn predicted_caps(
    boot: &RunnerBoot,
    caps: Option<&beton_harness::Capabilities>,
    hook: bool,
) -> beton_harness::Capabilities {
    let mut predicted = caps.cloned().unwrap_or_else(|| {
        beton_harness::Capabilities::minimal(
            beton_harness::Mode::Native,
            beton_harness::Transport::Native,
        )
    });
    if boot.harness.as_str() == HarnessId::FAKE
        && let Some(path) = &boot.scenario
        && let Ok(scenario) = beton_harness::scenario::Scenario::load(path)
        && let Ok(c) = beton_harness::fake::scenario_capabilities(&scenario)
    {
        predicted = c;
    }
    if boot.harness.as_str() == HarnessId::CLAUDE
        && !hook
        && predicted.tool_call_gate == beton_harness::ToolCallGate::Full
    {
        predicted.tool_call_gate = beton_harness::ToolCallGate::ApprovalOnly;
    }
    predicted
}

/// `modify` in `session_start` (POL-003): Modell, Effort (gemappt) und Permission-Mode (nur
/// strenger als der angefragte, danach erneut gegen die Sandbox geprüft, #231) für den Start.
fn apply_start_modifications(
    start: &mut settings::Change,
    m: &beton_policy::engine::Modifications,
    caps: &beton_harness::Capabilities,
    sandbox: beton_harness::SandboxStatus,
) -> Result<Option<beton_harness::PermissionMode>, settings::Rejected> {
    let mode = settings::policy_start_mode(
        start.permission_mode,
        m.permission_mode.as_deref(),
        caps,
        sandbox,
    )?;
    // Zweite Linie hinter `on_unsupported` (#283): nie still verwerfen.
    let unsupported = |what: &str| settings::Rejected {
        code: "capability_unsupported",
        detail: format!("Der Harness kann {what} beim Start nicht setzen (Policy-Modifikation)"),
    };
    if m.model.is_some() && !caps.start_model() {
        return Err(unsupported("das Modell"));
    }
    let effort = match &m.reasoning_effort {
        Some(effort) => Some(
            beton_harness::map_effort(effort, &caps.efforts)
                .ok_or_else(|| unsupported("diesen Effort"))?,
        ),
        None => None,
    };
    if let Some(model) = &m.model {
        start.model = Some(model.clone());
    }
    if let Some(mapped) = effort {
        start.effort = Some(mapped);
    }
    if let Some(mode) = mode {
        start.permission_mode = Some(mode);
    }
    Ok(mode)
}

/// `modify` in `model_request` je Turn: Modell und Effort live vor dem Turn (POL-024: nur mit
/// `model_switch: live`, sonst hat die Engine `on_unsupported` angewendet). Liefert die
/// angewendete Änderung für `session.settings_changed` (HAR-017).
async fn apply_turn_modifications(
    session: &mut dyn HarnessSession,
    m: &beton_policy::engine::Modifications,
    caps: &beton_harness::Capabilities,
    spec: &mut SessionSpec,
) -> Result<Option<settings::Change>, Value> {
    let mapped = m
        .reasoning_effort
        .as_deref()
        .and_then(|e| beton_harness::map_effort(e, &caps.efforts));
    let model = m.model.clone().filter(|v| spec.model.as_ref() != Some(v));
    let effort = mapped.filter(|v| spec.effort.as_ref() != Some(v));
    if model.is_none() && effort.is_none() {
        return Ok(None);
    }
    session
        .set_model(model.clone(), effort.clone())
        .await
        .map_err(|e| json!({"code": e.code(), "detail": e.to_string()}))?;
    let change = settings::Change {
        model: model.clone(),
        requested_effort: m
            .reasoning_effort
            .clone()
            .filter(|r| effort.is_some() && effort.as_ref() != Some(r)),
        effort: effort.clone(),
        permission_mode: None,
    };
    if model.is_some() {
        spec.model = model;
    }
    if effort.is_some() {
        spec.effort = effort;
    }
    Ok(Some(change))
}

/// Buchführung für POL-025 AC1: Tool-Calls ohne Gate-Anfrage bis zum Ende merken; liefert die,
/// die jetzt nur noch beobachtet werden können (abgeschlossen bzw. Turn-Ende).
fn track_tool_calls(
    gate: &policy::PolicyGate,
    ev: &NormalizedEvent,
    unobserved: &mut HashMap<String, GateRequest>,
    turn_done: bool,
    observe_at_start: bool,
) -> Vec<GateRequest> {
    let mut out = Vec::new();
    match &ev.payload {
        EventPayload::ToolCallRequested(r) => {
            gate.note_requested(&r.call_id, r.mcp_server.as_deref(), &r.tool);
            // ACP ordnet über die Tool-Art zu (`execute`, `edit` …); ohne sie fände die
            // Normalisierung weder Kommando noch Pfade (POL-023 AC3).
            let acp_kind = (r.source == beton_core::event::ToolSource::Acp).then(|| {
                match r.kind.as_deref() {
                    Some("shell") => "execute",
                    Some("file_edit" | "file_write") => "edit",
                    Some("file_read") => "read",
                    Some("search") => "search",
                    Some("web_fetch") => "fetch",
                    _ => "other",
                }
                .to_owned()
            });
            unobserved.insert(
                r.call_id.clone(),
                GateRequest {
                    turn_id: ev.turn_id,
                    call_id: r.call_id.clone(),
                    tool: r.tool.clone(),
                    kind: r.kind.clone().unwrap_or_default(),
                    args: r.args.clone(),
                    mcp_server: r.mcp_server.clone(),
                    acp_kind,
                    ..GateRequest::default()
                },
            );
        }
        // Ohne Gate-Anfrage gestartet (POL-023 AC3): schon beim Start beobachten, damit eine
        // `deny`-Regel den Turn noch abbrechen kann.
        // Nur für Harnesses ohne volles Gate; bei vollem Gate ist ein Call ohne Entscheidung
        // ein Integritätsfehler (HAR-005), den keine Beobachtung verdecken darf.
        EventPayload::ToolCallStarted(st) if observe_at_start => {
            if !gate.decided(&st.call_id)
                && let Some(req) = unobserved.remove(&st.call_id)
            {
                out.push(req);
            }
        }
        EventPayload::ToolCallCompleted(c) => {
            gate.note_completed(c.status);
            if let Some(req) = unobserved.remove(&c.call_id)
                && !gate.decided(&c.call_id)
            {
                out.push(req);
            }
        }
        EventPayload::TurnStarted(_) => gate.note_turn(),
        _ => {}
    }
    if turn_done {
        out.extend(
            unobserved
                .drain()
                .map(|(_, r)| r)
                .filter(|r| !gate.decided(&r.call_id)),
        );
    }
    out
}

/// Status nach einer Freigabe: `waiting_approval`, solange eine offen ist (gestapelte
/// Freigaben, WEB-007), danach `running` bzw. `idle`.
fn approval_status(
    payload: Option<&EventPayload>,
    open: &mut std::collections::HashSet<beton_core::id::ApprovalId>,
    turn_running: bool,
) -> Option<SessionStatus> {
    match payload? {
        EventPayload::ApprovalRequested(r) => {
            open.insert(r.approval_id);
            Some(SessionStatus::WaitingApproval)
        }
        EventPayload::ApprovalResolved(r) => {
            open.remove(&r.approval_id);
            Some(if !open.is_empty() {
                SessionStatus::WaitingApproval
            } else if turn_running {
                SessionStatus::Running
            } else {
                SessionStatus::Idle
            })
        }
        _ => None,
    }
}

/// Ein Gate-Ereignis als Event der Session; `policy.decision` erhält die Event-ID des
/// zugehörigen `tool.call.requested` in `subject_ref` (POL-025).
fn gate_event(
    boot: &RunnerBoot,
    ge: policy::GateEvent,
    call_events: &HashMap<String, beton_core::id::EventId>,
) -> Event {
    let mut payload = ge.payload;
    if let EventPayload::PolicyDecision(d) = &mut payload
        && let Some(call) = d.subject_ref["call_id"].as_str()
        && let Some(id) = call_events.get(call)
    {
        d.subject_ref["event_id"] = Value::String(id.to_string());
    }
    let mut e = Event::new(
        boot.session_id,
        0,
        Actor::System {
            component: beton_core::event::SystemComponent::Policy,
        },
        payload,
    );
    e.raw = ge.raw;
    e.turn_id = ge.turn;
    e
}

/// Alle anstehenden Gate-Ereignisse.
fn drain_gate(
    boot: &RunnerBoot,
    rx: &mut mpsc::UnboundedReceiver<policy::GateEvent>,
    call_events: &HashMap<String, beton_core::id::EventId>,
) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(ge) = rx.try_recv() {
        out.push(gate_event(boot, ge, call_events));
    }
    out
}

/// Verbindung vor dem Start des Harness: nur, wenn das Gate dafür den Server braucht
/// (Verbrauchswerte, Freigabe für `session_start` mit `ask`) bzw. eine Ablehnung zu melden ist.
#[derive(Default)]
struct PreStart {
    ws: Option<Ws>,
    pending: HashMap<String, oneshot::Sender<Result<Value, Value>>>,
    next: u64,
    /// Auch eine Vorab-Verbindung erneuert das Token: Eine Freigabe kann länger dauern als
    /// seine Gültigkeit (AUTH-011 AC2).
    renewal: token::Renewal,
}

impl PreStart {
    async fn connect(&mut self, boot: &RunnerBoot, unacked: &mut Unacked) -> bool {
        if self.ws.is_some() {
            return true;
        }
        let Ok(mut ws) = connect(boot).await else {
            return false;
        };
        let hello = TunnelUp::Hello {
            kind: PeerKind::Runner,
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL.into(),
            harnesses: vec![boot.harness.to_string()],
        };
        let bind = TunnelUp::SessionBind {
            session_id: boot.session_id,
            epoch: boot.epoch,
            last_acked_rseq: unacked.acked(),
            prestart: true,
        };
        if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
            return false;
        }
        self.ws = Some(ws);
        self.renewal.reset();
        true
    }

    async fn push_gate(
        &mut self,
        boot: &RunnerBoot,
        rx: &mut mpsc::UnboundedReceiver<policy::GateEvent>,
        unacked: &mut Unacked,
    ) {
        let events = drain_gate(boot, rx, &HashMap::new());
        if let Some(ws) = self.ws.as_mut()
            && !events.is_empty()
        {
            let _ = push(ws, boot, unacked, events).await;
        }
    }

    /// `session_start` auswerten; bedient dabei Verbrauchswerte und Freigaben über den Tunnel.
    async fn check_start(
        &mut self,
        boot: &RunnerBoot,
        gate: &policy::PolicyGate,
        start: beton_policy::vars::StartVars,
        gate_rx: &mut mpsc::UnboundedReceiver<policy::GateEvent>,
        usage_rx: &mut mpsc::UnboundedReceiver<mcp::SystemRequest>,
        unacked: &mut Unacked,
    ) -> policy::Checked {
        let (wait_tx, mut wait_rx) = oneshot::channel();
        let g = gate.clone();
        let mut task = tokio::spawn(async move { g.check_start(start, Some(wait_tx)).await });
        let mut waiting = false;
        loop {
            tokio::select! {
                done = &mut task => {
                    return done.unwrap_or(policy::Checked::Deny {
                        reason: "Policy-Prüfung abgebrochen".into(),
                        rules: Vec::new(),
                    });
                }
                w = &mut wait_rx, if !waiting => {
                    waiting = true;
                    if w.is_ok() && self.connect(boot, unacked).await {
                        self.push_gate(boot, gate_rx, unacked).await;
                    }
                }
                ge = gate_rx.recv(), if self.ws.is_some() => {
                    if let Some(ge) = ge {
                        let e = gate_event(boot, ge, &HashMap::new());
                        if let Some(ws) = self.ws.as_mut() {
                            let _ = push(ws, boot, unacked, vec![e]).await;
                        }
                    }
                }
                () = sleep_until_opt(self.renewal.due()), if self.ws.is_some() => {
                    let up = self.renewal.request();
                    if let Some(ws) = self.ws.as_mut() {
                        let _ = send(ws, &up).await;
                    }
                }
                call = usage_rx.recv() => {
                    let Some(call) = call else { continue };
                    if !self.connect(boot, unacked).await {
                        let _ = call.reply.send(Err(json!({"code": "unavailable"})));
                        continue;
                    }
                    self.next += 1;
                    let id = format!("pre_{}", self.next);
                    let up = TunnelUp::SystemCall { call_id: id.clone(), tool: call.tool, args: call.args };
                    self.pending.insert(id, call.reply);
                    if let Some(ws) = self.ws.as_mut() {
                        let _ = send(ws, &up).await;
                    }
                }
                msg = async { match self.ws.as_mut() { Some(ws) => ws.next().await, None => std::future::pending().await } } => {
                    let Some(Ok(Message::Text(text))) = msg else {
                        self.ws = None;
                        continue;
                    };
                    let Ok(down) = serde_json::from_str::<TunnelDown>(&text) else { continue };
                    match down {
                        TunnelDown::EventsAck { upto_rseq, .. } => unacked.ack(upto_rseq),
                        TunnelDown::Bound { acked_rseq, .. } => unacked.ack(acked_rseq),
                        TunnelDown::SystemResult { call_id, mut result, problem } => {
                            if self.renewal.on_result(&call_id, &mut result, &boot.token) {
                                continue;
                            }
                            if let Some(reply) = self.pending.remove(&call_id) {
                                let _ = reply.send(match (result, problem) {
                                    (Some(r), None) => Ok(r),
                                    (_, p) => Err(p.unwrap_or_else(|| json!({"code": "internal"}))),
                                });
                            }
                        }
                        TunnelDown::CmdDeliver { cmd_id, name, args, .. } => {
                            let (result, problem) = if name == "approval.resolve" {
                                let key = args["call_id"].as_str().unwrap_or_default();
                                if gate.resolve(key, gate_decision(&args)) {
                                    (Some(Value::Null), None)
                                } else {
                                    (None, Some(json!({"code": "not_found", "detail": "keine offene Freigabe"})))
                                }
                            } else {
                                (None, Some(json!({"code": "session_closed", "detail": "Die Session startet noch (Freigabe offen)"})))
                            };
                            if let Some(ws) = self.ws.as_mut() {
                                let _ = send(ws, &TunnelUp::CmdResult { cmd_id, result, problem }).await;
                            }
                        }
                        TunnelDown::Problem { problem } => {
                            tracing::warn!(?problem, "Tunnel-Problem vor dem Start");
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Start verweigert (POL-003, POL-024, #182): Policy-Ereignisse, `session.start_denied`
    /// und Status `failed` melden.
    async fn deny_start(
        &mut self,
        boot: &RunnerBoot,
        gate_rx: &mut mpsc::UnboundedReceiver<policy::GateEvent>,
        unacked: &mut Unacked,
        reasons: Vec<String>,
        rules: Vec<String>,
    ) {
        if !self.connect(boot, unacked).await {
            return;
        }
        let mut events = drain_gate(boot, gate_rx, &HashMap::new());
        events.push(system_event(
            boot,
            EventPayload::SessionStartDenied(beton_core::event::SessionStartDenied {
                reasons: reasons.clone(),
                rules: (!rules.is_empty()).then_some(rules),
            }),
        ));
        events.push(system_event(
            boot,
            EventPayload::SessionStatus(SessionStatusChanged {
                status: SessionStatus::Failed,
                reason: Some(format!("Start verweigert: {}", reasons.join("; "))),
            }),
        ));
        if let Some(mut ws) = self.ws.take()
            && push(&mut ws, boot, unacked, events).await
        {
            drain_acks(&mut ws, unacked).await;
        }
    }

    /// Nach erlaubtem Start: über die Vorab-Verbindung Gesendetes ist bestätigt; der Rest geht
    /// mit den ersten Events der Session.
    async fn finish(
        mut self,
        boot: &RunnerBoot,
        gate_rx: &mut mpsc::UnboundedReceiver<policy::GateEvent>,
        unacked: &mut Unacked,
    ) -> Vec<Event> {
        match self.ws.take() {
            Some(mut ws) => {
                let events = drain_gate(boot, gate_rx, &HashMap::new());
                if !events.is_empty() {
                    let _ = push(&mut ws, boot, unacked, events).await;
                }
                drain_acks(&mut ws, unacked).await;
                Vec::new()
            }
            None => drain_gate(boot, gate_rx, &HashMap::new()),
        }
    }
}

/// Grund in `turn.interrupted` und `session.status`, wenn `executor.timeout` abläuft.
pub const TIMED_OUT: &str = "timed_out";
/// Frist für das Turn-Ende nach dem Interrupt wegen Zeitlimit.
const TIMEOUT_GRACE: Duration = Duration::from_secs(10);

/// Wartet bis `at`; ohne Zeitpunkt nie.
async fn sleep_until_opt(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Events bei einer Integritätsverletzung des Policy-Gates (HAR-005, #233): `error` mit
/// Problem `policy_integrity_violation`, Runner `failed`, Session `failed`.
fn end_integrity_violation(
    boot: &RunnerBoot,
    lifecycle: &mut Lifecycle,
    why: &str,
    out: &mut Vec<Event>,
) {
    out.push(system_event(
        boot,
        EventPayload::Error(beton_core::event::ErrorEvent {
            problem: json!({
                "type": "urn:beton:problem:policy_integrity_violation",
                "code": "policy_integrity_violation",
                "title": "Tool-Call ohne Policy-Prüfung; Session aus Sicherheitsgründen beendet",
                "detail": why,
            }),
        }),
    ));
    end_policy_failure(boot, lifecycle, "policy_integrity_violation", out);
}

/// Wurde ein Tool-Call (womöglich) ausgeführt? Auch ein Fehler (`…; exit 1`, Claude
/// `is_error`) kann Wirkung gehabt haben (Re-Review R2 zu #253). Nicht ausgeführt sind nur
/// abgelehnte Calls und solche, die vor der Ausführung abgebrochen wurden: Ein Call ohne
/// Gate-Entscheidung mit `cancelled`, der nie gestartet ist (`tool.call.started`), wartete noch
/// auf den Hook (z. B. Interrupt während einer offenen Freigabe). Nach dem Start kann er
/// gelaufen sein (#268).
fn executed(status: beton_core::event::ToolStatus, started: bool) -> bool {
    match status {
        beton_core::event::ToolStatus::Denied => false,
        beton_core::event::ToolStatus::Cancelled => started,
        _ => true,
    }
}

/// Grund eines `fatal`-Gate-Events: der Problem-Code des `error`.
fn fatal_reason(ge: &policy::GateEvent) -> String {
    match &ge.payload {
        EventPayload::Error(e) => e.problem["code"]
            .as_str()
            .unwrap_or("policy_failure")
            .to_owned(),
        _ => "policy_failure".into(),
    }
}

/// Runner und Session `failed` nach einem Policy-Fehler, der die Session beendet.
fn end_policy_failure(
    boot: &RunnerBoot,
    lifecycle: &mut Lifecycle,
    why: &str,
    out: &mut Vec<Event>,
) {
    if let Ok(s) = lifecycle.go(RunnerState::Failed, Some(why.to_owned())) {
        out.push(system_event(boot, EventPayload::RunnerStatus(s)));
    }
    out.push(system_event(
        boot,
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Failed,
            reason: Some(why.to_owned()),
        }),
    ));
}

/// Titel der Integritätsverstöße der Sandbox.
const SHELL_OUTSIDE_STAGE_TWO: &str = "Ein Shell-Kommando lief nicht in der Tool-Sandbox (Stufe 2)";
const GIT_CONTROL_CHANGED: &str =
    "In der Sandbox entstand oder änderte sich eine Git-Steuerdatei (commondir/gitdir)";

/// Integritätsverstoß des Stufenmodells (SBX-002, SBX-003): `error` mit Problem
/// `sandbox_integrity`, Runner und Session `failed` (wie bei der Policy-Integrität, HAR-005).
fn end_sandbox_integrity_violation(
    boot: &RunnerBoot,
    lifecycle: &mut Lifecycle,
    title: &str,
    reason: &str,
    out: &mut Vec<Event>,
) {
    out.push(system_event(
        boot,
        EventPayload::Error(beton_core::event::ErrorEvent {
            problem: json!({
                "type": "urn:beton:problem:sandbox_integrity",
                "code": "sandbox_integrity",
                "title": title,
                "detail": reason,
            }),
        }),
    ));
    end_policy_failure(boot, lifecycle, "sandbox_integrity", out);
}

/// Events am Ende eines Runs mit Zeitüberschreitung: Session `stopped` mit Grund `timed_out`,
/// Runner beendet.
fn end_timed_out(boot: &RunnerBoot, lifecycle: &mut Lifecycle, out: &mut Vec<Event>) {
    out.push(system_event(
        boot,
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Stopped,
            reason: Some(TIMED_OUT.into()),
        }),
    ));
    for to in [RunnerState::Draining, RunnerState::Terminated] {
        if let Ok(s) = lifecycle.go(to, Some(TIMED_OUT.into())) {
            out.push(system_event(boot, EventPayload::RunnerStatus(s)));
        }
    }
}

/// Wartet auf den nächsten Wert; ohne Kanal nie.
async fn recv_opt<T>(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Meldung der Terminals an den Server (SES-019): Event ins Log, Binärframe, Steuernachricht.
async fn forward_terminal(
    ws: &mut Ws,
    boot: &RunnerBoot,
    unacked: &mut Unacked,
    out: terminals::Out,
) -> bool {
    match out {
        terminals::Out::Event(payload, by) => {
            let event = match by {
                Some(a) => Event::new(boot.session_id, 0, a, *payload),
                None => system_event(boot, *payload),
            };
            push(ws, boot, unacked, vec![event]).await
        }
        terminals::Out::Frame(bytes) => ws.send(Message::Binary(bytes.into())).await.is_ok(),
        terminals::Out::Up(up) => send(ws, &up).await,
    }
}

fn system_event(boot: &RunnerBoot, payload: EventPayload) -> Event {
    Event::new(
        boot.session_id,
        0,
        Actor::System {
            component: beton_core::event::SystemComponent::Runner,
        },
        payload,
    )
}

/// Öffnet das Schatten-Repository der Session; ohne Konfiguration oder ohne `git` keine
/// Beobachtung (der Turn läuft trotzdem). Bei zu großem Workspace ein Hinweis für die Session.
async fn open_tracker(boot: &RunnerBoot) -> (Option<workspace::Shared>, Option<String>) {
    let Some(dir) = boot.snapshots.clone() else {
        return (None, None);
    };
    let work = boot.workdir.clone();
    match tokio::task::spawn_blocking(move || workspace::Tracker::open(&dir, &work)).await {
        Ok(Ok(t)) => (Some(Arc::new(Mutex::new(t))), None),
        Ok(Err(workspace::OpenError::TooLarge(n))) => (
            None,
            Some(format!(
                "Der Workspace hat mehr als {n} Dateien (ohne ignorierte). Änderungen des Agents \
                 erscheinen daher nicht als `fs.changed`, und die Turn-Sicht der Änderungen fehlt."
            )),
        ),
        Ok(Err(e)) => {
            tracing::warn!("Workspace-Beobachtung nicht verfügbar: {e}");
            (None, None)
        }
        Err(e) => {
            tracing::warn!("Workspace-Beobachtung nicht verfügbar: {e}");
            (None, None)
        }
    }
}

/// Vergleicht den Workspace während eines Turns laufend mit dem letzten Snapshot und meldet
/// Änderungen (SES-017 AC3: ≤ 1 s). Der Abstand wächst mit der Dauer eines Snapshots, damit
/// große Repositories nicht dauerhaft Last erzeugen. Meldungen gehen unter der Sperre des
/// Trackers raus; das Turn-Ende übernimmt sie vor seinem eigenen Snapshot (#189).
async fn watch_workspace(
    tracker: workspace::Shared,
    mut turn: watch::Receiver<Option<TurnId>>,
    out: mpsc::UnboundedSender<(TurnId, Vec<FsChange>)>,
) {
    // Basis-Snapshot vorab: füllt den Index, damit spätere Snapshots schnell sind.
    let _ = workspace::poll(&tracker).await;
    loop {
        let current = *turn.borrow_and_update();
        let Some(t) = current else {
            if turn.changed().await.is_err() {
                return;
            }
            continue;
        };
        let started = tokio::time::Instant::now();
        let still = turn.clone();
        let tx = out.clone();
        workspace::poll_and_deliver(
            &tracker,
            move || *still.borrow() == Some(t),
            move |changes| {
                let _ = tx.send((t, changes));
            },
        )
        .await;
        if out.is_closed() {
            return;
        }
        // Abstand von Start zu Start: höchstens die Hälfte der Zeit läuft ein Snapshot. Eine
        // Änderung kurz nach dem Lesen des laufenden Snapshots erscheint so spätestens nach
        // drei Snapshot-Dauern statt nach vier, wenn die Pause erst nach dem Snapshot begänne
        // (#116: unter Last ~250 ms je Snapshot).
        let period = workspace::POLL_MIN.max(started.elapsed() * 2);
        tokio::time::sleep_until(started + period.min(Duration::from_secs(5))).await;
    }
}

/// `fs.changed` des Agents für einen Turn.
fn fs_changed(boot: &RunnerBoot, actor: &Actor, turn: TurnId, changes: Vec<FsChange>) -> Event {
    let mut e = Event::new(
        boot.session_id,
        0,
        actor.clone(),
        EventPayload::FsChanged(FsChanged {
            changes,
            source: workspace::FS_SOURCE_WATCHER.into(),
        }),
    );
    e.turn_id = Some(turn);
    e
}

/// Meldet einen gescheiterten Harness-Start über den Tunnel und setzt die Session auf `failed`.
async fn report_start_failure(boot: &RunnerBoot, error: &beton_harness::HarnessError) {
    let Ok(mut ws) = connect(boot).await else {
        return;
    };
    let hello = TunnelUp::Hello {
        kind: PeerKind::Runner,
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL.into(),
        harnesses: vec![boot.harness.to_string()],
    };
    let bind = TunnelUp::SessionBind {
        session_id: boot.session_id,
        epoch: boot.epoch,
        last_acked_rseq: 0,
        // Nur die Meldung des gescheiterten Starts, keine Kommandos (#218).
        prestart: true,
    };
    if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
        return;
    }
    let system = Actor::System {
        component: beton_core::event::SystemComponent::Runner,
    };
    let mut events = Vec::new();
    if let beton_harness::HarnessError::Incompatible { detected, expected } = error {
        events.push(Event::new(
            boot.session_id,
            0,
            system.clone(),
            EventPayload::HarnessIncompatible(beton_core::event::HarnessIncompatible {
                detected_version: detected.clone(),
                expected_range: expected.clone(),
            }),
        ));
    } else {
        events.push(Event::new(boot.session_id, 0, system.clone(), EventPayload::Error(beton_core::event::ErrorEvent {
            problem: json!({"type": format!("urn:beton:problem:{}", error.code()), "code": error.code(), "title": error.to_string()}),
        })));
    }
    events.push(Event::new(
        boot.session_id,
        0,
        system,
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Failed,
            reason: Some(error.to_string()),
        }),
    ));
    let mut unacked = Unacked::default();
    if push(&mut ws, boot, &mut unacked, events).await {
        drain_acks(&mut ws, &mut unacked).await;
    }
}

async fn finish(session: Box<dyn HarnessSession>, exit: Exit) -> Result<Exit, RunnerError> {
    let _ = session.shutdown(Shutdown::Kill).await;
    Ok(exit)
}

fn to_event(boot: &RunnerBoot, actor: &Actor, ev: NormalizedEvent) -> Event {
    let mut e = Event::new(boot.session_id, 0, actor.clone(), ev.payload);
    e.raw = ev.raw;
    e.turn_id = ev.turn_id;
    e
}

/// Sendet dauerhafte Events (mit `rseq`) bzw. transiente direkt – in der Reihenfolge des
/// Batches: aufeinanderfolgende Events gleicher Art gehen gemeinsam, ein `turn.started` kommt
/// also vor den Deltas desselben Turns an.
async fn push(ws: &mut Ws, boot: &RunnerBoot, unacked: &mut Unacked, events: Vec<Event>) -> bool {
    let mut runs: Vec<(bool, Vec<Event>)> = Vec::new();
    for e in events {
        let transient = e.payload().is_some_and(EventPayload::is_transient);
        match runs.last_mut() {
            Some((t, run)) if *t == transient => run.push(e),
            _ => runs.push((transient, vec![e])),
        }
    }
    for (transient, run) in runs {
        let msg = if transient {
            TunnelUp::TransientPush {
                session_id: boot.session_id,
                events: run,
            }
        } else {
            TunnelUp::EventsPush {
                session_id: boot.session_id,
                epoch: boot.epoch,
                batch: run.into_iter().map(|e| unacked.push(e)).collect(),
            }
        };
        if !send(ws, &msg).await {
            return false;
        }
    }
    true
}

/// Wartet kurz auf ausstehende Bestätigungen.
async fn drain_acks(ws: &mut Ws, unacked: &mut Unacked) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !unacked.queue.is_empty() {
        let Ok(Some(Ok(Message::Text(text)))) = tokio::time::timeout_at(deadline, ws.next()).await
        else {
            return;
        };
        if let Ok(TunnelDown::EventsAck { upto_rseq, .. }) = serde_json::from_str(&text) {
            unacked.ack(upto_rseq);
        }
    }
}

/// Kommando für einen Einmal-Aufruf über den Harness der Session (SES-010, z. B. Titel).
/// Argumente: `instructions`, `prompt`, optional `model` und `timeout_ms`; Ergebnis:
/// [`beton_harness::OneShotReply`].
pub const ONE_SHOT_CMD: &str = "harness.one_shot";

/// Höchstdauer eines Einmal-Aufrufs ohne Angabe des Servers.
pub const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(60);

fn one_shot_request(boot: &RunnerBoot, args: &Value) -> beton_harness::OneShotRequest {
    beton_harness::OneShotRequest {
        instructions: args["instructions"].as_str().unwrap_or_default().to_owned(),
        prompt: args["prompt"].as_str().unwrap_or_default().to_owned(),
        workdir: boot.workdir.clone(),
        model: args["model"].as_str().map(str::to_owned),
        timeout: args["timeout_ms"]
            .as_u64()
            .map_or(ONE_SHOT_TIMEOUT, Duration::from_millis),
        scenario: boot.scenario.clone(),
    }
}

async fn run_one_shot(
    adapter: Option<Arc<dyn beton_harness::HarnessAdapter>>,
    request: beton_harness::OneShotRequest,
    ctx: AdapterContext,
) -> Result<Value, Value> {
    let problem = |code: &str, detail: String| json!({"code": code, "detail": detail});
    let Some(adapter) = adapter else {
        return Err(problem("unavailable", "Harness nicht verfügbar".into()));
    };
    match adapter.one_shot(&request, &ctx).await {
        Ok(reply) => serde_json::to_value(reply).map_err(|e| problem("internal", e.to_string())),
        Err(e) => {
            // Inhalte der CLI-Ausgabe nicht ins Log (Prompts, OBS-001).
            tracing::debug!(code = e.code(), "Einmal-Aufruf gescheitert");
            Err(problem(e.code(), e.to_string()))
        }
    }
}

/// Entscheidung aus `approval.resolve`. `updated_args: null` (so serialisiert die REST-API
/// eine fehlende Änderung) heißt „unverändert“, nicht „Argumente leeren“. `remember: session`
/// merkt die Freigabe (POL-010), `answer` beantwortet eine Frage (AGT-007 AC4), `actor` nennt,
/// wer entschieden hat (WEB-007 AC2).
fn gate_decision(args: &Value) -> policy::Human {
    let allow = args["decision"] == "allow";
    policy::Human {
        allow,
        remember: allow && args["remember"] == "session",
        reason: args["reason"].as_str().map(str::to_owned),
        updated_args: if allow {
            args.get("updated_args").filter(|a| !a.is_null()).cloned()
        } else {
            None
        },
        answer: (args["decision"] == "answer")
            .then(|| args["answer"].as_str().map(str::to_owned))
            .flatten(),
        actor: args
            .get("actor")
            .filter(|a| !a.is_null())
            .and_then(|a| serde_json::from_value(a.clone()).ok()),
    }
}

/// Auslöser aus `session.compact {trigger}`: `beton` für die Automatik des Servers
/// (`compaction.auto: beton`), sonst ein Mensch (USE-009).
fn requested_trigger(args: &Value) -> CompactionTrigger {
    match args["trigger"].as_str() {
        Some("beton") => CompactionTrigger::Beton,
        _ => CompactionTrigger::User,
    }
}

/// Setzt den Auslöser an `compaction.*` (USE-009): den des Kommandos, sonst hat der Harness
/// selbst kompaktiert (eigene Automatik der CLI bzw. des Direkt-API-Loops).
fn stamp_compaction(payload: &mut EventPayload, pending: &mut Option<CompactionTrigger>) {
    let (EventPayload::CompactionStarted(c) | EventPayload::CompactionCompleted(c)) = payload
    else {
        return;
    };
    if c.trigger.is_none() {
        c.trigger = Some(pending.unwrap_or(CompactionTrigger::Harness));
    }
    if matches!(payload, EventPayload::CompactionCompleted(_)) {
        *pending = None;
    }
}

/// Fehlt das Kontextfenster (`window_tokens: 0`), ergänzt es der Runner aus dem Preis-Katalog
/// des Modells (USE-008); ist es auch dort unbekannt, bleibt es `0` („Fenster unbekannt“).
fn fill_window(c: &mut beton_core::event::ContextUsage, model: Option<&str>) {
    if c.window_tokens > 0 {
        return;
    }
    if let Some(window) = model.and_then(|m| {
        beton_core::pricing::Catalog::embedded()
            .resolve(m)
            .and_then(|e| e.context_window)
    }) {
        c.window_tokens = window;
    }
}

/// Kommandos vom Server an den Harness (`cmd.deliver`).
/// Wechselt Modell, Effort und Permission-Mode live (HAR-017, HAR-027).
async fn apply_live(
    session: &mut dyn HarnessSession,
    change: &settings::Change,
) -> Result<(), settings::Rejected> {
    if change.model.is_some() || change.effort.is_some() {
        session
            .set_model(change.model.clone(), change.effort.clone())
            .await?;
    }
    if let Some(mode) = change.permission_mode {
        session.set_permission_mode(mode).await?;
    }
    Ok(())
}

// Zustand des Runner-Loops, den genau dieses Kommando braucht; ein eigener Typ brächte nichts.
#[allow(clippy::too_many_arguments)]
async fn deliver(
    session: &mut dyn HarnessSession,
    gate: &policy::PolicyGate,
    name: &str,
    args: Value,
    turn_running: bool,
    handover: &mut Option<String>,
    next_turn: &mut Option<TurnId>,
    pending: &mut HashMap<String, beton_harness::InputAttachment>,
) -> Result<Value, Value> {
    let problem = |code: &str, detail: String| json!({"code": code, "detail": detail});
    match name {
        // Anhang für die nächste Eingabe (WEB-006); `input.submit` nennt seine `id`.
        "input.attachment" => {
            let id = args["id"].as_str().unwrap_or_default().to_owned();
            if id.is_empty() || pending.len() >= 64 {
                return Err(problem("validation_failed", "Anhang ohne id".into()));
            }
            pending.insert(
                id,
                beton_harness::InputAttachment {
                    name: args["name"].as_str().unwrap_or_default().to_owned(),
                    mime: args["mime"].as_str().unwrap_or_default().to_owned(),
                    data_base64: args["data"].as_str().unwrap_or_default().to_owned(),
                },
            );
            Ok(Value::Null)
        }
        "input.submit" => {
            let mut text = args["text"].as_str().unwrap_or_default().to_owned();
            // Der erste Turn nach einem Fork per Präambel bekommt die Kurzfassung (HAR-018).
            if let Some(brief) = handover.as_deref() {
                text = beton_harness::handover::Preamble::first_message(brief, &text);
            }
            // Vorher zugestellte Anhänge dieser Eingabe (WEB-006).
            let attachments = args["attachments"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().and_then(|id| pending.remove(id)))
                        .collect()
                })
                .unwrap_or_default();
            pending.clear();
            let sent = session
                .send(UserInput {
                    text,
                    turn_id: *next_turn,
                    attachments,
                })
                .await;
            if sent.is_ok() {
                *handover = None;
                *next_turn = None;
            }
            sent.map(|turn| json!({"turn_id": turn}))
                .map_err(|e| problem(e.code(), e.to_string()))
        }
        // SES-004: Eingabe in den laufenden Turn; ohne Turn entscheidet der Server neu.
        "input.steer" if !turn_running => {
            Err(problem("no_active_turn", "kein laufender Turn".into()))
        }
        "input.steer" => {
            let text = args["text"].as_str().unwrap_or_default().to_owned();
            session
                .steer(UserInput::from(text.as_str()))
                .await
                .map(|()| Value::Null)
                .map_err(|e| problem(e.code(), e.to_string()))
        }
        // SES-005 AC3: ohne laufenden Turn ist Interrupt ein No-op.
        "turn.interrupt" if !turn_running => Ok(Value::Null),
        "turn.interrupt" => session
            .interrupt()
            .await
            .map(|()| Value::Null)
            .map_err(|e| problem(e.code(), e.to_string())),
        // SES-011: Compaction nur ohne laufenden Turn; ohne Capability `capability_unsupported`.
        "session.compact" if turn_running => Err(problem(
            "turn_active",
            "Compaction erst nach dem laufenden Turn".into(),
        )),
        "session.compact" => session
            .compact()
            .await
            .map(|()| Value::Null)
            .map_err(|e| problem(e.code(), e.to_string())),
        "approval.resolve" => {
            let call_id = args["call_id"].as_str().unwrap_or_default();
            let decision = gate_decision(&args);
            if gate.resolve(call_id, decision) {
                Ok(Value::Null)
            } else {
                Err(problem(
                    "not_found",
                    format!("keine offene Freigabe für {call_id}"),
                ))
            }
        }
        _ => Err(problem(
            "unknown_command",
            format!("`{name}` kennt der Runner nicht"),
        )),
    }
}

/// `start.fork_of` (POL-004, #196): Quelle eines Forks laut Plan des Servers; das Fortsetzen
/// eines Imports ist kein Fork.
fn fork_of(plan: Option<&beton_harness::handover::ForkPlan>) -> Option<String> {
    plan.filter(|p| p.kind == beton_harness::handover::PlanKind::Fork)
        .map(|p| p.from_session.to_string())
}

#[cfg(test)]
mod tests {
    /// #283: Eine Modifikation beim Start, die der Harness nicht anwenden kann, lehnt den
    /// Start ab (zweite Linie hinter `on_unsupported`), statt still verworfen zu werden.
    #[test]
    fn pol_013_start_modifications_are_never_dropped_silently() {
        use beton_harness::{Capabilities, Mode, SwitchSupport, Transport};
        let mut acp = Capabilities::minimal(Mode::Native, Transport::Acp);
        acp.model_switch = SwitchSupport::None;
        let model = beton_policy::engine::Modifications {
            model: Some("claude-haiku-4-5".into()),
            ..Default::default()
        };
        let mut start = super::settings::Change::default();
        let r = super::apply_start_modifications(&mut start, &model, &acp, Default::default());
        assert!(r.is_err(), "ACP ohne set_model: {r:?}");
        assert_eq!(start.model, None);
        acp.model_switch = SwitchSupport::Live;
        let r = super::apply_start_modifications(&mut start, &model, &acp, Default::default());
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(start.model.as_deref(), Some("claude-haiku-4-5"));

        // Effort ohne abbildbare Stufe.
        let native = Capabilities::minimal(Mode::Native, Transport::Native);
        let effort = beton_policy::engine::Modifications {
            reasoning_effort: Some("high".into()),
            ..Default::default()
        };
        let mut start = super::settings::Change::default();
        let r = super::apply_start_modifications(&mut start, &effort, &native, Default::default());
        assert!(r.is_err(), "Effort ohne Stufen: {r:?}");
    }

    #[test]
    fn pol_004_start_fork_of_names_the_source_of_a_fork() {
        let plan = |kind: &str| -> beton_harness::handover::ForkPlan {
            serde_json::from_value(serde_json::json!({
                "kind": kind,
                "from_session": "ses_01JZ0000000000000000000001",
                "from_harness": "claude",
                "at_seq": 3,
                "reason": "user",
                "history": [],
            }))
            .unwrap()
        };
        assert_eq!(
            super::fork_of(Some(&plan("fork"))).as_deref(),
            Some("ses_01JZ0000000000000000000001")
        );
        assert_eq!(super::fork_of(Some(&plan("resume"))), None);
        assert_eq!(super::fork_of(None), None);
    }

    use beton_core::event::AuthSource;

    /// #326, Review #370: ACP bekommt die Stufe-2-Tools und Terminals, obwohl es bis #329 als
    /// `inherited` gilt; andere Harnesses mit `inherited` (Codex) nicht.
    #[test]
    fn har_007_acp_gets_stage_two_tools_while_inherited() {
        use beton_harness::ToolIsolation::{Beton, Inherited, ShellOverride};
        let id = |s: &str| s.parse::<super::HarnessId>().unwrap();
        assert!(super::beton_tools_in_stage_two(
            &id("acp:gemini"),
            Inherited
        ));
        assert!(super::beton_tools_in_stage_two(&id("fake"), Beton));
        assert!(!super::beton_tools_in_stage_two(&id("codex"), Inherited));
        assert!(!super::beton_tools_in_stage_two(
            &id("claude"),
            ShellOverride
        ));
    }
    use beton_harness::registry::{HarnessAuth, HarnessCommandConfig, HarnessesConfig};

    fn layer(auth: Option<HarnessAuth>, isolated: Option<bool>) -> HarnessesConfig {
        HarnessesConfig {
            default: None,
            acp: Default::default(),
            entries: [(
                "claude".to_owned(),
                HarnessCommandConfig {
                    auth,
                    isolated,
                    ..HarnessCommandConfig::default()
                },
            )]
            .into(),
        }
    }

    #[test]
    fn har_015_auth_source_comes_from_user_config_only() {
        let default = claude_adapter(&HarnessLayers::default());
        assert_eq!(default.auth, AuthSource::VendorCli, "Default: Subscription");
        let user = claude_adapter(&HarnessLayers {
            user: layer(Some(HarnessAuth::ApiKey), None),
            project: HarnessesConfig::default(),
            ..HarnessLayers::default()
        });
        assert_eq!(user.auth, AuthSource::ApiKey);
        // Ein Repository kann nicht auf API-Billing umschalten.
        let project = claude_adapter(&HarnessLayers {
            user: HarnessesConfig::default(),
            project: layer(Some(HarnessAuth::ApiKey), None),
            ..HarnessLayers::default()
        });
        assert_eq!(project.auth, AuthSource::VendorCli);
    }

    #[test]
    fn har_004_project_may_isolate_but_never_lift_the_user_isolation() {
        let a = claude_adapter(&HarnessLayers {
            user: layer(None, Some(true)),
            project: HarnessesConfig::default(),
            ..HarnessLayers::default()
        });
        assert!(a.isolated);
        // SBX-002: Die Projektebene (Repo-Inhalt) verschärft nur.
        let b = claude_adapter(&HarnessLayers {
            user: layer(None, Some(true)),
            project: layer(None, Some(false)),
            ..HarnessLayers::default()
        });
        assert!(b.isolated);
        let c = claude_adapter(&HarnessLayers {
            user: layer(None, Some(false)),
            project: layer(None, Some(true)),
            ..HarnessLayers::default()
        });
        assert!(c.isolated);
    }

    use super::*;

    /// USE-009: Der Runner nennt den Auslöser; ohne Kommando hat der Harness kompaktiert.
    #[test]
    fn compaction_trigger_is_stamped() {
        let compaction = |done: bool| {
            let c = beton_core::event::Compaction::default();
            if done {
                EventPayload::CompactionCompleted(c)
            } else {
                EventPayload::CompactionStarted(c)
            }
        };
        let trigger = |p: &EventPayload| match p {
            EventPayload::CompactionStarted(c) | EventPayload::CompactionCompleted(c) => c.trigger,
            _ => None,
        };
        let mut pending = Some(requested_trigger(&serde_json::json!({"trigger": "beton"})));
        let (mut a, mut b) = (compaction(false), compaction(true));
        stamp_compaction(&mut a, &mut pending);
        stamp_compaction(&mut b, &mut pending);
        assert_eq!(trigger(&a), Some(CompactionTrigger::Beton));
        assert_eq!(trigger(&b), Some(CompactionTrigger::Beton));
        assert_eq!(pending, None);
        let mut c = compaction(false);
        stamp_compaction(&mut c, &mut pending);
        assert_eq!(trigger(&c), Some(CompactionTrigger::Harness));
        assert_eq!(requested_trigger(&Value::Null), CompactionTrigger::User);
    }

    /// USE-008: Ein fehlendes Fenster kommt aus dem Katalog; unbekannt bleibt es `0`.
    #[test]
    fn missing_context_window_comes_from_the_catalog() {
        let usage = |window| beton_core::event::ContextUsage {
            used_tokens: 150_000,
            window_tokens: window,
            source: beton_core::event::ContextSource::Harness,
        };
        let mut known = usage(0);
        fill_window(&mut known, Some("claude-opus-5-5"));
        assert!(known.window_tokens > 0);
        let mut unknown = usage(0);
        fill_window(&mut unknown, Some("ein-eigenes-modell"));
        assert_eq!(unknown.window_tokens, 0);
        let mut reported = usage(200_000);
        fill_window(&mut reported, Some("claude-opus-5-5"));
        assert_eq!(reported.window_tokens, 200_000);
    }

    fn notice() -> Event {
        Event::new(
            SessionId::new(),
            0,
            Actor::default(),
            EventPayload::default(),
        )
    }

    #[test]
    fn unacked_buffer_tracks_rseq_and_bytes() {
        let mut u = Unacked::default();
        assert_eq!(u.acked(), 0);
        for _ in 0..5 {
            u.push(notice());
        }
        assert_eq!(u.acked(), 0);
        u.ack(3);
        assert_eq!(u.acked(), 3);
        assert_eq!(u.queue.len(), 2);
        u.ack(5);
        assert_eq!(u.bytes, 0);
        assert_eq!(u.acked(), 5);
    }

    #[test]
    fn web_018_null_updated_args_mean_unchanged() {
        assert_eq!(
            gate_decision(&json!({"decision": "allow", "updated_args": null})),
            policy::Human {
                allow: true,
                ..policy::Human::default()
            }
        );
        assert_eq!(
            gate_decision(&json!({"decision": "allow", "updated_args": {"a": 1}})).updated_args,
            Some(json!({"a": 1}))
        );
        assert_eq!(
            gate_decision(&json!({"decision": "deny", "reason": "nein"})),
            policy::Human {
                allow: false,
                reason: Some("nein".into()),
                ..policy::Human::default()
            }
        );
    }

    #[test]
    fn pol_010_ac4_runner_outlives_the_daemon_only_while_approvals_are_open() {
        // Elternprozess lebt: weiter verbinden wie bisher.
        assert!(!orphan_exit(true, false, false));
        // Daemon weg, keine offene Freigabe: Runner endet.
        assert!(orphan_exit(false, false, false));
        // Daemon weg, Freigabe offen: auf den neuen Daemon warten.
        assert!(!orphan_exit(false, false, true));
        // Vom neuen Daemon übernommen: ohne Verbindung und ohne Freigabe endet er.
        assert!(orphan_exit(true, true, false));
        assert!(!orphan_exit(true, true, true));
    }

    #[test]
    fn pol_010_ac4_only_an_orderly_detach_with_a_policy_approval_keeps_the_runner() {
        let now = std::time::Instant::now();
        // Ohne geordnete Abkopplung (Daemon-Absturz): nicht weiterleben.
        assert!(!detached_keep(None, now, true));
        // Abgekoppelt mit offener Policy-Freigabe: weiterleben, höchstens eine Stunde.
        assert!(detached_keep(Some(now), now, true));
        assert!(!detached_keep(
            Some(now),
            now + DETACH_MAX + Duration::from_secs(1),
            true
        ));
        // Ohne offene Policy-Freigabe (z. B. nur eine Frage): nicht.
        assert!(!detached_keep(Some(now), now, false));
        // Das Signal steht im Close-Frame.
        assert!(is_detach_signal(4503, "detached"));
        assert!(!is_detach_signal(4503, "Daemon fährt herunter"));
        assert!(!is_detach_signal(1000, "detached"));
    }

    #[test]
    fn web_007_status_stays_waiting_while_stacked_approvals_are_open() {
        use beton_core::event::{
            ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, ResolvedVia,
            TimeoutAction,
        };
        use beton_core::id::ApprovalId;
        let (a, b) = (ApprovalId::new(), ApprovalId::new());
        let req = |id| {
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: id,
                kind: ApprovalKind::Policy,
                subject: json!({}),
                options: vec![],
                expires_at: beton_core::time::Timestamp::now(),
                on_timeout: TimeoutAction::Deny,
            })
        };
        let res = |id| {
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: id,
                decision: ApprovalDecision::Allow,
                answer: None,
                actor: Actor::default(),
                via: ResolvedVia::User,
                remember: None,
                comment: None,
                on_timeout_applied: None,
            })
        };
        let mut open = std::collections::HashSet::new();
        assert_eq!(
            approval_status(Some(&req(a)), &mut open, true),
            Some(SessionStatus::WaitingApproval)
        );
        assert_eq!(
            approval_status(Some(&req(b)), &mut open, true),
            Some(SessionStatus::WaitingApproval)
        );
        assert_eq!(
            approval_status(Some(&res(a)), &mut open, true),
            Some(SessionStatus::WaitingApproval)
        );
        assert_eq!(
            approval_status(Some(&res(b)), &mut open, true),
            Some(SessionStatus::Running)
        );
        assert_eq!(
            approval_status(Some(&req(a)), &mut open, false),
            Some(SessionStatus::WaitingApproval)
        );
        assert_eq!(
            approval_status(Some(&res(a)), &mut open, false),
            Some(SessionStatus::Idle)
        );
    }

    #[test]
    fn pol_010_resolve_args_carry_remember_answer_and_actor() {
        let h = gate_decision(&json!({"decision": "allow", "remember": "session",
            "actor": {"kind": "user", "id": "usr_local"}}));
        assert!(h.allow && h.remember);
        assert!(h.actor.is_some());
        // Merken nur beim Erlauben.
        assert!(!gate_decision(&json!({"decision": "deny", "remember": "session"})).remember);
        let a = gate_decision(&json!({"decision": "answer", "answer": "Pro IP"}));
        assert_eq!(a.answer.as_deref(), Some("Pro IP"));
        assert!(!a.allow);
    }
}
