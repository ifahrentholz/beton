//! Runner: führt genau eine Session aus, supervidiert den Harness und streamt Events über
//! den Tunnel an den Home-Knoten (RUN-002, RUN-003, PROTO-015).
//!
//! Der Runner baut nur **ausgehende** Verbindungen auf (Unix-Socket des lokalen Daemons) und
//! öffnet keinen Port. Unbestätigte Events hält er bis 64 MiB vor und sendet sie nach einem
//! Reconnect erneut; darüber pausiert er das Lesen vom Harness.

pub mod fork;
pub mod mcp;
pub mod settings;
pub mod state;
pub mod workspace;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, Event, EventPayload, FsChange, FsChanged, SessionStatus, SessionStatusChanged,
};
use beton_core::id::{RunnerId, SessionId, TurnId};
use beton_harness::process::RealLauncher;
use beton_harness::registry::{HarnessLayers, Registry, RegistryOptions};
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessId, HarnessSession, HostEnv,
    NormalizedEvent, SessionSpec, Shutdown, UserInput,
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
}

/// Startparameter eines Runners. Das Token kommt über stdin, nie über Env oder argv
/// (AUTH-011).
#[derive(Debug, Clone)]
pub struct RunnerBoot {
    pub socket: PathBuf,
    pub token: String,
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
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("Startparameter fehlen oder sind ungültig: {0}")]
    Boot(String),
    #[error("Harness: {0}")]
    Harness(#[from] beton_harness::HarnessError),
    #[error("Tunnel: {0}")]
    Tunnel(String),
    #[error("stale_epoch: Session gehört inzwischen einem anderen Schreiber")]
    StaleEpoch,
}

impl RunnerBoot {
    /// Liest Env, das Token (erste stdin-Zeile) und optional die API-Keys (zweite Zeile,
    /// JSON `{"VARIABLE": "wert"}`, HAR-011).
    pub fn from_env_and_stdin() -> Result<Self, RunnerError> {
        let var = |k: &str| std::env::var(k).map_err(|_| RunnerError::Boot(format!("{k} fehlt")));
        let parse = |k: &str| -> Result<String, RunnerError> { var(k) };
        let mut token = String::new();
        std::io::stdin()
            .read_line(&mut token)
            .map_err(|e| RunnerError::Boot(format!("Token von stdin: {e}")))?;
        let mut keys = String::new();
        // Ohne zweite Zeile (stdin geschlossen) gibt es keine Keys.
        let _ = std::io::stdin().read_line(&mut keys);
        let credentials = beton_harness_direct::secret::KeyMap::from_json_line(&keys);
        Ok(Self {
            socket: PathBuf::from(var(env::SOCKET)?),
            token: token.trim().to_owned(),
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
            },
        )
        .0
    }
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
        isolated: project
            .and_then(|e| e.isolated)
            .or_else(|| user.and_then(|e| e.isolated))
            .unwrap_or(false),
        ..beton_harness_claude::ClaudeAdapter::default()
    }
}

/// Einstieg des Runner-Prozesses: Startdaten aus Env und stdin lesen, Session ausführen.
/// Exit-Code: 0 nach `runner.stop` oder Ende des Daemons, sonst der des Harness bzw. 1/2.
pub async fn main_from_env() -> std::process::ExitCode {
    use std::process::ExitCode;
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
        Err(e) => {
            eprintln!("beton runner: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Gate, das Freigaben über `approval.resolve`-Kommandos vom Server erhält.
#[derive(Default)]
pub struct RunnerGate {
    pending: Mutex<HashMap<String, oneshot::Sender<GateDecision>>>,
}

#[async_trait]
impl Gate for RunnerGate {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(request.call_id.clone(), tx);
        }
        // Ohne Entscheidung (Runner endet) gilt fail closed.
        rx.await.unwrap_or(GateDecision::Deny {
            reason: Some("Runner beendet".into()),
        })
    }
}

impl RunnerGate {
    pub fn resolve(&self, call_id: &str, decision: GateDecision) -> bool {
        self.pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(call_id))
            .is_some_and(|tx| tx.send(decision).is_ok())
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
}

impl StatusTracker {
    fn set(&mut self, boot: &RunnerBoot, to: SessionStatus, out: &mut Vec<Event>) {
        if self.current == Some(to) {
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
        format!("Bearer {}", boot.token)
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
        .map_err(|e| RunnerError::Tunnel(e.to_string()))?;
    Ok(ws)
}

async fn send(ws: &mut Ws, msg: &TunnelUp) -> bool {
    let text = serde_json::to_string(msg).unwrap_or_default();
    ws.send(Message::Text(text.into())).await.is_ok()
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
    let gate = Arc::new(RunnerGate::default());
    let mut env = HostEnv::from_process();
    env.user = boot.harnesses.user.clone();
    env.project = boot.harnesses.project.clone();
    env.vars.remove("BETON_RUNNER_TOKEN");
    let ctx = AdapterContext {
        gate: gate.clone(),
        launcher: Arc::new(RealLauncher),
        env,
    };
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
        Some(adapter) => adapter.probe(&ctx.env).await,
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
                &mcp::Paths::from_process(run_dir),
            )
            .await
        }
        None => Ok(mcp::McpSetup::default()),
    };
    let mut setup = match setup {
        Ok(s) => s,
        Err(e) => {
            // Fail closed: ohne ladbaren Agent startet die Session nicht.
            let e = beton_harness::HarnessError::StartRefused(format!("Agent: {e}"));
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
    // Übernommener Verlauf (Fork, Resume eines Imports): native, Rebuild oder Präambel.
    let forked = match &boot.fork {
        Some(path) => match fork::load(path) {
            Ok(plan) => Some(
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
                .await,
            ),
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
    // Modell, Effort und Permission-Mode ab dem Start (HAR-017, HAR-027); `yolo` ohne Sandbox
    // scheitert hier (fail closed).
    let fallback_caps = beton_harness::Capabilities::minimal(
        beton_harness::Mode::Native,
        beton_harness::Transport::Native,
    );
    let (start_settings, mapped_effort) = match settings::at_start(
        boot.model.clone(),
        boot.effort.as_deref(),
        boot.permission_mode.as_deref(),
        caps.as_ref().unwrap_or(&fallback_caps),
        beton_harness::SandboxStatus::current(),
    ) {
        Ok(s) => s,
        Err(rejected) => {
            let e = settings::start_error(&rejected);
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
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
    let (fs_tx, mut fs_rx) = mpsc::channel::<(TurnId, Vec<FsChange>)>(64);
    if let Some(t) = &files {
        tokio::spawn(watch_workspace(t.clone(), turn_rx, fs_tx.clone()));
    }
    let mut pending_before: Option<String> = None;

    let mut unacked = Unacked::default();
    let mut tracker = StatusTracker::default();
    let mut turn_running = false;
    // SES-011 AC1: je Turn genau ein `context.usage` (der letzte Stand vor dem Turn-Ende).
    let mut held_context: Option<NormalizedEvent> = None;
    for e in pending_status.drain(..) {
        unacked.push(e);
    }
    let mut backoff = Backoff::default();
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

    loop {
        // (Wieder-)Verbinden.
        let mut ws = match connect(&boot).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::warn!("Tunnel nicht erreichbar: {e}");
                if !parent_alive(boot.parent_pid) {
                    return finish(session, Exit::ParentGone).await;
                }
                tokio::time::sleep(backoff.next_delay()).await;
                continue;
            }
        };
        backoff.reset();
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
        };
        if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
            continue;
        }
        if lifecycle.state() == RunnerState::Lost {
            let _ = lifecycle.go(RunnerState::Reconnecting, None);
        }
        let mut extra = Vec::new();
        status(&mut lifecycle, RunnerState::Connected, None, &mut extra);
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
                    let mut exited = None;
                    let mut run_over = false;
                    for mut ev in incoming {
                        if timed_out && let EventPayload::TurnInterrupted(t) = &mut ev.payload {
                            t.reason = Some(TIMED_OUT.into());
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
                            EventPayload::TurnStarted(_) | EventPayload::ApprovalResolved(_) => Some(SessionStatus::Running),
                            EventPayload::ApprovalRequested(_) => Some(SessionStatus::WaitingApproval),
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
                        batch.push(to_event(&boot, &actor, ev));
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
                call = recv_opt(&mut system_calls) => {
                    let Some(call) = call else { system_calls = None; continue };
                    next_call += 1;
                    let call_id = format!("sys_{next_call}");
                    let up = TunnelUp::SystemCall { call_id: call_id.clone(), tool: call.tool, args: call.args };
                    pending_calls.insert(call_id, call.reply);
                    if !send(&mut ws, &up).await { break None; }
                }
                msg = ws.next() => {
                    let Some(Ok(msg)) = msg else { break None };
                    let Message::Text(text) = msg else { continue };
                    let Ok(down) = serde_json::from_str::<TunnelDown>(&text) else { continue };
                    match down {
                        TunnelDown::EventsAck { upto_rseq, .. } => unacked.ack(upto_rseq),
                        TunnelDown::Bound { acked_rseq, .. } => unacked.ack(acked_rseq),
                        TunnelDown::CmdDeliver { cmd_id, name, args } => {
                            // Stand vor dem Turn festhalten, bevor der Harness die Eingabe sieht.
                            if name == "input.submit"
                                && turn_tx.borrow().is_none()
                                && let Some(t) = &files
                            {
                                pending_before = workspace::poll(t).await.map(|(_, tree)| tree);
                            }
                            if name == "session.set" {
                                // HAR-017, HAR-027: prüfen, dann sofort bzw. nach dem Turn anwenden.
                                match settings::parse(&args, &session_caps, beton_harness::SandboxStatus::current()) {
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
                            let reply = deliver(&mut *session, &gate, &name, args, turn_running, &mut handover, &mut next_turn).await;
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
                            let mut batch = Vec::new();
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
                        }
                        TunnelDown::SystemResult { call_id, result, problem } => {
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
                _ = parent_check.tick() => {
                    if !parent_alive(boot.parent_pid) {
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
            return finish(session, exit).await;
        }
        // Verbindung verloren: weiterlaufen, puffern, neu verbinden.
        if lifecycle
            .go(RunnerState::Lost, Some("Tunnel getrennt".into()))
            .is_ok()
        {
            tracing::warn!("Tunnel getrennt, verbinde neu");
        }
        tokio::time::sleep(backoff.next_delay()).await;
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
/// große Repositories nicht dauerhaft Last erzeugen.
async fn watch_workspace(
    tracker: workspace::Shared,
    mut turn: watch::Receiver<Option<TurnId>>,
    out: mpsc::Sender<(TurnId, Vec<FsChange>)>,
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
        let started = std::time::Instant::now();
        if let Some((changes, _)) = workspace::poll(&tracker).await
            && !changes.is_empty()
            && out.send((t, changes)).await.is_err()
        {
            return;
        }
        let pause = workspace::POLL_MIN.max(started.elapsed() * 2);
        tokio::time::sleep(pause.min(Duration::from_secs(5))).await;
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

/// Entscheidung aus `approval.resolve`. `updated_args: null` (so serialisiert die REST-API
/// eine fehlende Änderung) heißt „unverändert“, nicht „Argumente leeren“.
fn gate_decision(args: &Value) -> GateDecision {
    if args["decision"] == "allow" {
        GateDecision::Allow {
            updated_args: args.get("updated_args").filter(|a| !a.is_null()).cloned(),
        }
    } else {
        GateDecision::Deny {
            reason: args["reason"].as_str().map(str::to_owned),
        }
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

async fn deliver(
    session: &mut dyn HarnessSession,
    gate: &RunnerGate,
    name: &str,
    args: Value,
    turn_running: bool,
    handover: &mut Option<String>,
    next_turn: &mut Option<TurnId>,
) -> Result<Value, Value> {
    let problem = |code: &str, detail: String| json!({"code": code, "detail": detail});
    match name {
        "input.submit" => {
            let mut text = args["text"].as_str().unwrap_or_default().to_owned();
            // Der erste Turn nach einem Fork per Präambel bekommt die Kurzfassung (HAR-018).
            if let Some(brief) = handover.as_deref() {
                text = beton_harness::handover::Preamble::first_message(brief, &text);
            }
            let sent = session
                .send(UserInput {
                    text,
                    turn_id: *next_turn,
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
                .steer(UserInput {
                    text,
                    turn_id: None,
                })
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

#[cfg(test)]
mod tests {
    use beton_core::event::AuthSource;
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
    fn har_004_isolated_from_project_overrides_user() {
        let a = claude_adapter(&HarnessLayers {
            user: layer(None, Some(true)),
            project: HarnessesConfig::default(),
            ..HarnessLayers::default()
        });
        assert!(a.isolated);
        let b = claude_adapter(&HarnessLayers {
            user: layer(None, Some(true)),
            project: layer(None, Some(false)),
            ..HarnessLayers::default()
        });
        assert!(!b.isolated);
    }

    use super::*;

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
            GateDecision::Allow { updated_args: None }
        );
        assert_eq!(
            gate_decision(&json!({"decision": "allow", "updated_args": {"a": 1}})),
            GateDecision::Allow {
                updated_args: Some(json!({"a": 1}))
            }
        );
        assert_eq!(
            gate_decision(&json!({"decision": "deny", "reason": "nein"})),
            GateDecision::Deny {
                reason: Some("nein".into())
            }
        );
    }

    #[tokio::test]
    async fn gate_waits_for_resolution_and_fails_closed() {
        let gate = Arc::new(RunnerGate::default());
        let g = gate.clone();
        let waiting = tokio::spawn(async move {
            g.decide(GateRequest {
                turn_id: None,
                call_id: "c1".into(),
                tool: "Bash".into(),
                kind: "shell".into(),
                args: Value::Null,
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(gate.resolve("c1", GateDecision::Allow { updated_args: None }));
        assert_eq!(
            waiting.await.unwrap(),
            GateDecision::Allow { updated_args: None }
        );
        assert!(!gate.resolve("c1", GateDecision::Deny { reason: None }));

        let g = gate.clone();
        let waiting = tokio::spawn(async move {
            g.decide(GateRequest {
                turn_id: None,
                call_id: "c2".into(),
                tool: "Bash".into(),
                kind: "shell".into(),
                args: Value::Null,
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        gate.pending.lock().unwrap().clear();
        assert!(matches!(waiting.await.unwrap(), GateDecision::Deny { .. }));
    }
}
