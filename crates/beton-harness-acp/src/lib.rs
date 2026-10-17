//! Generischer ACP-Harness (Agent Client Protocol, HAR-007) mit Agent-Registrierung und
//! Presets (HAR-008).
//!
//! beton ist ACP-Client und betreibt jeden ACP-fähigen Agent als Harness `acp:<slug>`:
//! `initialize` → `session/new` bzw. `session/load` → je Turn `session/prompt` (Antwort mit
//! `stopReason` am Turn-Ende) mit `session/update`-Notifications; `session/cancel` bricht ab;
//! `session/request_permission` geht an das Gate. Die Client-Methoden `fs/*` und
//! `terminal/*` meldet beton in M1 nicht an und lehnt sie ab: Sie laufen erst mit der
//! Tool-Sandbox Stufe 2 (SBX-002, ab M2) policy-kontrolliert in beton.
//!
//! Der Agent nutzt seinen eigenen Login; beton reicht standardmäßig keine `*_API_KEY`-
//! Variable durch (ADR-0034).

pub mod config;
pub mod mapping;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use beton_core::event::{
    AuthSource, EventPayload, HarnessAuthRequired, HarnessExited, HarnessReady, RawJson,
    TurnCompleted, TurnFailed, TurnInterrupted, TurnStarted,
};
use beton_core::id::{PrincipalId, TurnId, UserId};
use beton_harness::jsonrpc::{Incoming, RpcClient, RpcError, codes};
use beton_harness::process::{
    LaunchSpec, ProcessHandle, ProcessLauncher, RealLauncher, ShutdownTimeouts,
};
use beton_harness::registry::{AcpAgentConfig, HarnessLayers, Registry, resolve_binary};
use beton_harness::{
    Action, AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport,
    ExitInfo, ForkHistory, Gate, GateDecision, GateOrigin, GateRequest, HarnessAdapter,
    HarnessError, HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode, NormalizedEvent,
    PermissionMode, ProbeReport, ResumeSupport, SessionSpec, Shutdown, Subagents, SwitchOutcome,
    SwitchSupport, ToolCallGate, Transport, UsageReporting, UserInput,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};

pub use crate::config::{AcpAgent, ConfigProblem, Origin, PRESETS, agents};
use crate::mapping::{MapState, map_update, unmapped};

/// Unterstützte ACP-Protokollversion.
pub const PROTOCOL_VERSION: u64 = 1;
/// Fehlercode von ACP für fehlende Anmeldung.
pub const AUTH_REQUIRED: i64 = -32000;
/// Frist für den Handshake (auch im Probe).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Höchstens so lange wartet der Adapter auf das Gate; danach Ablehnung (fail closed).
pub const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Registriert alle ACP-Agents aus Presets und Konfiguration (HAR-008) und liefert die
/// übersprungenen Einträge.
pub fn register(registry: &mut Registry, layers: &HarnessLayers) -> Vec<ConfigProblem> {
    let (agents, problems) = config::agents(layers);
    for agent in agents {
        if let Some(adapter) = AcpAdapter::new(agent) {
            registry.register(Arc::new(adapter));
        }
    }
    problems
}

/// Namen der `*_API_KEY`-Variablen der aktuellen Umgebung, die nicht durchgereicht werden.
pub fn api_key_vars(passthrough: &[String]) -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.ends_with("_API_KEY") && !passthrough.contains(k))
        .collect()
}

/// Ergebnis des Handshakes im Probe.
#[derive(Debug, Clone, Default)]
struct AgentProbe {
    version: Option<String>,
    load_session: bool,
    error: Option<String>,
}

type ProbeKey = (PathBuf, Option<SystemTime>, Vec<String>);

/// Ein ACP-Agent als Harness `acp:<slug>`.
#[derive(Clone)]
pub struct AcpAdapter {
    id: HarnessId,
    pub agent: AcpAgent,
    pub gate_timeout: Duration,
    pub handshake_timeout: Duration,
    probes: Arc<std::sync::Mutex<HashMap<ProbeKey, AgentProbe>>>,
}

impl std::fmt::Debug for AcpAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAdapter")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl AcpAdapter {
    /// `None`, wenn der Slug keine gültige Harness-ID ergibt.
    pub fn new(agent: AcpAgent) -> Option<Self> {
        let id = format!("acp:{}", agent.slug).parse().ok()?;
        Some(Self {
            id,
            agent,
            gate_timeout: GATE_TIMEOUT,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            probes: Arc::default(),
        })
    }

    fn config(&self) -> &AcpAgentConfig {
        &self.agent.config
    }

    fn launch_spec(&self, env: &HostEnv, cwd: Option<PathBuf>) -> Option<LaunchSpec> {
        let bin = resolve_binary(&self.id, &self.config().command, env)?;
        let mut args = bin.args;
        args.extend(self.config().args.iter().cloned());
        Some(LaunchSpec {
            program: bin.program,
            args,
            env: Vec::new(),
            env_remove: api_key_vars(&self.config().env_passthrough),
            clear_env: false,
            cwd,
        })
    }

    fn cached(&self, path: Option<&str>) -> Option<AgentProbe> {
        let path = path?;
        let probes = self.probes.lock().ok()?;
        probes
            .iter()
            .find(|((p, _, _), _)| p.display().to_string() == path)
            .map(|(_, v)| v.clone())
    }

    /// Hinweis für den Login in der Agent-CLI (HAR-015).
    pub fn login_hint(&self) -> String {
        format!(
            "Login in der Agent-CLI `{}` ausführen",
            self.config().command
        )
    }
}

/// `initialize` an einen Agent; liefert die Antwort.
async fn initialize(
    rpc: &RpcClient,
    timeout: Duration,
    stage_two: bool,
) -> Result<Value, HarnessError> {
    let result = rpc
        .request_timeout(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                // `fs/write_text_file` führt beton selbst aus, geprüft wie jedes Schreiben (POL-023
                // AC2, HAR-007 AC2); Lesen und `terminal/*` in Stufe 2, nur wenn die Session sie
                // hat (#326, fail closed).
                "clientCapabilities": {"fs": {"readTextFile": stage_two, "writeTextFile": true}, "terminal": stage_two},
                "clientInfo": {"name": "beton", "version": env!("CARGO_PKG_VERSION")},
            }),
            timeout,
        )
        .await;
    let result = match result {
        Ok(v) => v,
        Err(RpcError::Remote { message, .. }) => {
            tracing::warn!(message, "ACP: initialize abgelehnt");
            return Err(HarnessError::Incompatible {
                detected: "unbekannt".into(),
                expected: format!("ACP {PROTOCOL_VERSION}"),
            });
        }
        Err(e) => return Err(e.into()),
    };
    match result["protocolVersion"].as_u64() {
        Some(PROTOCOL_VERSION) => Ok(result),
        other => Err(HarnessError::Incompatible {
            detected: other.map_or_else(
                || result["protocolVersion"].to_string(),
                |v| format!("ACP {v}"),
            ),
            expected: format!("ACP {PROTOCOL_VERSION}"),
        }),
    }
}

#[async_trait]
impl HarnessAdapter for AcpAdapter {
    fn id(&self) -> HarnessId {
        self.id.clone()
    }

    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }

    fn capabilities(&self, _mode: Mode, probe: &ProbeReport) -> Capabilities {
        let load = self
            .cached(probe.path.as_deref())
            .is_some_and(|p| p.load_session);
        Capabilities {
            mode: Mode::Native,
            transport: Transport::Acp,
            version_range: None,
            auth_sources: vec![AuthSource::VendorCli],
            approval: ApprovalMechanism::AcpPermission,
            tool_call_gate: ToolCallGate::ApprovalOnly,
            // Erfahrungsgemäß fragen ACP-Agents vor Shell-Befehlen und Datei-Änderungen; was ohne
            // Rückfrage läuft, ist nur beobachtbar (POL-024). `fs/*`/`terminal/*` führt beton
            // erst mit der Tool-Sandbox selbst aus (HAR-007).
            approval_kinds: vec!["shell".into(), "file_write".into(), "file_edit".into()],
            model_request_gate: beton_harness::ModelRequestGate::Turn,
            tool_result_gate: ToolCallGate::ObserveOnly,
            // `session/set_model` erst nach `session/new` bekannt; die Session meldet `live`,
            // wenn der Agent Modelle anbietet (HAR-017). ACP kennt keinen Start-Parameter für
            // das Modell, ein Neustart hilft daher nicht.
            model_switch: SwitchSupport::None,
            effort_switch: SwitchSupport::None,
            // Ohne `session/load` nur kalt fortsetzbar; Fork immer per Preamble (HAR-018).
            resume: if load {
                ResumeSupport::Warm
            } else {
                ResumeSupport::Cold
            },
            fork_history: ForkHistory::Preamble,
            interrupt: true,
            steering: false,
            subagents: Subagents::None,
            usage_reporting: UsageReporting::None,
            compaction: CompactionSupport::None,
            instructions_delivery: InstructionsDelivery::FirstMessagePrefix,
            // `session/new.mcpServers` (HAR-009).
            mcp_injection: true,
            images: false,
            transcript_import: false,
            models: self.config().models.clone(),
            models_stale: false,
            efforts: Vec::new(),
            // Weitere Modi meldet die Session, sobald der Agent sie anbietet (HAR-027).
            permission_modes: vec![PermissionMode::Default],
            context_window: None,
            // `terminal/*` und `fs/read_text_file` führt beton schon in Stufe 2 aus (#326), aber
            // `fs/write_text_file` noch in Stufe 0 (#329). ADR-0037 verlangt für `beton` beides;
            // bis dahin degraded (`require_tool_isolation: false`, kein `yolo`).
            tool_isolation: beton_harness::ToolIsolation::Inherited,
            native_project_files: Vec::new(),
        }
    }

    /// Binary auflösen und `initialize` ausführen (ohne Session, ohne Modellaufruf), um
    /// Version und `loadSession` zu erfahren; Ergebnis je (Pfad, mtime, Argumente) gecacht.
    async fn probe(&self, env: &HostEnv) -> ProbeReport {
        let Some(spec) = self.launch_spec(env, None) else {
            return ProbeReport::default();
        };
        let mtime = std::fs::metadata(&spec.program)
            .and_then(|m| m.modified())
            .ok();
        let key = (spec.program.clone(), mtime, spec.args.clone());
        let hit = self.probes.lock().ok().and_then(|p| p.get(&key).cloned());
        let probe = match hit {
            Some(p) => p,
            None => {
                let p = handshake_probe(spec.clone(), self.handshake_timeout).await;
                if let Ok(mut probes) = self.probes.lock() {
                    probes.retain(|(path, _, _), _| *path != key.0);
                    probes.insert(key, p.clone());
                }
                p
            }
        };
        ProbeReport {
            installed: true,
            path: Some(spec.program.display().to_string()),
            version: probe.version,
            auth_status: AuthStatus::Unknown,
            probe_failed: probe.error,
        }
    }

    async fn start(
        &self,
        mut spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        // `harnesses.acp.agents.<slug>.mcp_bridge: false`: keine System-Tools (HAR-009 AC2);
        // der Runner filtert bereits, der Adapter sichert zusätzlich ab.
        if self.config().mcp_bridge == Some(false) {
            spec.mcp.servers.retain(|s| s.name != "beton");
        }
        let launch = self
            .launch_spec(&ctx.env, Some(spec.workdir.clone()))
            .unwrap_or_else(|| LaunchSpec {
                // Ohne Auflösung bleibt der Name, damit Replays (Golden-Tests) laufen.
                program: self.config().command.clone().into(),
                args: self.config().args.clone(),
                env_remove: api_key_vars(&self.config().env_passthrough),
                cwd: Some(spec.workdir.clone()),
                ..LaunchSpec::default()
            });
        let mut process = ctx.launcher.launch(launch).await?;
        let io = process.take_io().ok_or(HarnessError::Closed)?;
        let process = Arc::new(Mutex::new(process));
        let (rpc, incoming) = RpcClient::spawn(io.stdin, io.stdout);
        // Lesen und Terminals in Stufe 2 nur mit deren Launcher und den Terminals des Runners.
        let tools = ctx.stage.as_ref().and_then(|s| s.tools.clone());
        let terminals = ctx.stage.as_ref().and_then(|s| s.terminals.clone());
        let stage_two = tools.is_some() && terminals.is_some();
        let opened = open_session(&rpc, &spec, self.handshake_timeout, stage_two).await;
        let (session_id, modes, models, loaded) = match opened {
            Ok(s) => s,
            Err(e) => {
                rpc.close().await;
                let _ = process.lock().await.kill().await;
                return Err(match e {
                    Opened::Auth(message) => HarnessError::StartRefused(format!(
                        "harness.auth_required: {message}; {}",
                        self.login_hint()
                    )),
                    Opened::Other(e) => e,
                });
            }
        };
        let caps = session_capabilities(
            self.capabilities(Mode::Native, &ProbeReport::default()),
            &modes,
            &models,
        );
        // Startwerte (HAR-017, HAR-027): Ein Modus, den der Agent nicht anbietet, wird
        // abgelehnt statt ignoriert (fail closed).
        if let Err(e) = apply_start_settings(&rpc, &session_id, &spec, &caps, &modes).await {
            rpc.close().await;
            let _ = process.lock().await.kill().await;
            return Err(e);
        }
        let (tx, rx) = mpsc::channel(4096);
        let _ = tx
            .send(NormalizedEvent::new(
                EventPayload::HarnessReady(HarnessReady {
                    harness_session_ref: Some(session_id.clone()),
                    tools: Vec::new(),
                    mcp_servers: spec.mcp.names(),
                }),
                None,
            ))
            .await;
        if spec.model.is_some() && models.is_empty() {
            // Kein stilles Ignorieren: Der Agent wählt sein Modell selbst.
            let _ = tx
                .send(NormalizedEvent::new(
                    EventPayload::Notice(beton_core::event::Notice {
                        level: beton_core::event::NoticeLevel::Warn,
                        text: "Der Agent bietet keine Modellwahl an (ACP `session/set_model`); \
                               er nutzt sein eigenes Modell."
                            .into(),
                    }),
                    None,
                ))
                .await;
        }
        let state = Arc::new(Mutex::new(Turn::default()));
        let closing = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(Notify::new());
        tokio::spawn(dispatch(
            incoming,
            Dispatcher {
                harness: self.id.to_string(),
                hint: self.login_hint(),
                rpc: rpc.clone(),
                state: state.clone(),
                tx: tx.clone(),
                gate: ctx.gate.clone(),
                gate_timeout: self.gate_timeout,
                process: process.clone(),
                closing: closing.clone(),
                cancel: cancel.clone(),
                workdir: spec.workdir.clone(),
                write_policy: ctx.stage.as_ref().and_then(|s| s.harness_policy.clone()),
                fs_calls: Arc::default(),
                tools: tools.filter(|_| stage_two),
                terminals: terminals.filter(|_| stage_two),
                terminal_calls: Arc::default(),
            },
        ));
        Ok(Box::new(AcpSession {
            caps,
            rpc,
            state,
            tx,
            rx: Some(rx),
            process,
            closing,
            cancel,
            session_id,
            modes,
            // `first_message_prefix` (AGT-005); eine per `session/load` fortgesetzte Session
            // kennt sie schon.
            instructions: if loaded { None } else { spec.instructions },
        }))
    }
}

async fn handshake_probe(spec: LaunchSpec, timeout: Duration) -> AgentProbe {
    let mut process = match RealLauncher.launch(spec).await {
        Ok(p) => p,
        Err(e) => {
            return AgentProbe {
                error: Some(format!("Start fehlgeschlagen: {e}")),
                ..AgentProbe::default()
            };
        }
    };
    let Some(io) = process.take_io() else {
        return AgentProbe::default();
    };
    let (rpc, _incoming) = RpcClient::spawn(io.stdin, io.stdout);
    let probe = match initialize(&rpc, timeout, false).await {
        Ok(v) => AgentProbe {
            version: v["agentInfo"]["version"].as_str().map(str::to_owned),
            load_session: v["agentCapabilities"]["loadSession"] == true,
            error: None,
        },
        Err(e) => AgentProbe {
            error: Some(format!("initialize: {e}")),
            ..AgentProbe::default()
        },
    };
    rpc.close().await;
    let _ = process.kill().await;
    probe
}

enum Opened {
    Auth(String),
    Other(HarnessError),
}

impl From<HarnessError> for Opened {
    fn from(e: HarnessError) -> Self {
        Self::Other(e)
    }
}

/// Handshake und Session: `session/load`, wenn fortgesetzt wird und der Agent es kann,
/// sonst `session/new` (kalt, HAR-007 AC3). Liefert Session-ID, angebotene Modi und ob die
/// Session per `session/load` fortgesetzt wurde.
async fn open_session(
    rpc: &RpcClient,
    spec: &SessionSpec,
    timeout: Duration,
    stage_two: bool,
) -> Result<(String, Vec<String>, Vec<String>, bool), Opened> {
    let init = initialize(rpc, timeout, stage_two).await?;
    let load = init["agentCapabilities"]["loadSession"] == true;
    let cwd = spec.workdir.display().to_string();
    let servers = mcp_servers(spec);
    let (method, params) = match &spec.resume {
        Some(id) if load => (
            "session/load",
            json!({"sessionId": id, "cwd": cwd, "mcpServers": servers}),
        ),
        _ => ("session/new", json!({"cwd": cwd, "mcpServers": servers})),
    };
    let result = match rpc.request_timeout(method, params, timeout).await {
        Ok(v) => v,
        Err(RpcError::Remote { code, message, .. }) if code == AUTH_REQUIRED => {
            return Err(Opened::Auth(message));
        }
        Err(RpcError::Remote { message, .. }) => {
            return Err(Opened::Other(HarnessError::StartRefused(format!(
                "{method}: {message}"
            ))));
        }
        Err(e) => return Err(Opened::Other(e.into())),
    };
    let session_id = match (&spec.resume, load) {
        (Some(id), true) => id.clone(),
        _ => result["sessionId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| HarnessError::Protocol("session/new ohne sessionId".into()))?,
    };
    let ids = |list: &Value, key: &str| -> Vec<String> {
        list.as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m[key].as_str().map(str::to_owned))
            .collect()
    };
    let modes = ids(&result["modes"]["availableModes"], "id");
    // `models` (ACP, instabil): angebotene Modelle für `session/set_model`.
    let models = ids(&result["models"]["availableModels"], "modelId");
    Ok((session_id, modes, models, spec.resume.is_some() && load))
}

/// ACP-Modus-ID eines beton-Modus (Benennung von Claude Code über ACP).
pub fn acp_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Yolo => "bypassPermissions",
    }
}

/// Capabilities der Session: Modellwechsel live, wenn der Agent Modelle anbietet;
/// Permission-Modes, die der Agent als Modus anbietet (HAR-017, HAR-027).
pub fn session_capabilities(
    mut caps: Capabilities,
    modes: &[String],
    models: &[String],
) -> Capabilities {
    if !models.is_empty() {
        caps.model_switch = SwitchSupport::Live;
        caps.models = models.to_vec();
    }
    caps.permission_modes = PermissionMode::ALL
        .into_iter()
        .filter(|m| *m == PermissionMode::Default || modes.iter().any(|id| id == acp_mode(*m)))
        .collect();
    caps
}

/// `session/set_model` (HAR-017).
async fn set_model(rpc: &RpcClient, session_id: &str, model: &str) -> Result<(), HarnessError> {
    rpc.request_timeout(
        "session/set_model",
        json!({"sessionId": session_id, "modelId": model}),
        Duration::from_secs(10),
    )
    .await?;
    Ok(())
}

/// Modell und Permission-Mode beim Start setzen.
async fn apply_start_settings(
    rpc: &RpcClient,
    session_id: &str,
    spec: &SessionSpec,
    caps: &Capabilities,
    modes: &[String],
) -> Result<(), HarnessError> {
    if let Some(mode) = spec.permission_mode {
        if !caps.permission_modes.contains(&mode) {
            return Err(beton_harness::CapabilityUnsupported(Action::PermissionMode).into());
        }
        let id = acp_mode(mode);
        if modes.iter().any(|m| m == id) {
            rpc.request_timeout(
                "session/set_mode",
                json!({"sessionId": session_id, "modeId": id}),
                Duration::from_secs(10),
            )
            .await?;
        }
    }
    if let Some(model) = &spec.model
        && caps.model_switch == SwitchSupport::Live
    {
        set_model(rpc, session_id, model).await?;
    }
    Ok(())
}

/// `mcpServers` für `session/new` bzw. `session/load` (HAR-009): stdio-Relays ohne Env.
pub fn mcp_servers(spec: &SessionSpec) -> Value {
    Value::Array(
        spec.mcp
            .servers
            .iter()
            .map(|s| json!({"name": s.name, "command": s.command, "args": s.args, "env": []}))
            .collect(),
    )
}

/// Zustand des laufenden Turns.
#[derive(Debug, Default)]
struct Turn {
    turn: Option<TurnId>,
    prompt_request: Option<u64>,
    map: MapState,
}

/// Tool-Call (`call_id`, Turn) je Terminal eines Agents.
type TerminalCalls =
    Arc<std::sync::Mutex<std::collections::HashMap<String, (String, Option<TurnId>)>>>;

#[derive(Clone)]
struct Dispatcher {
    harness: String,
    hint: String,
    rpc: RpcClient,
    state: Arc<Mutex<Turn>>,
    tx: mpsc::Sender<NormalizedEvent>,
    gate: Arc<dyn Gate>,
    gate_timeout: Duration,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<AtomicBool>,
    cancel: Arc<Notify>,
    /// Arbeitsverzeichnis der Session; `fs/write_text_file` schreibt nur darin.
    workdir: std::path::PathBuf,
    /// Policy der Stufe 1: `fs/write_text_file` schreibt nur, was der Harness-Prozess selbst
    /// schreiben dürfte (Security-Review #327).
    write_policy: Option<Arc<beton_sandbox::ResolvedPolicy>>,
    /// Laufende Nummer für Call-IDs der Client-Aufrufe (`fs-<n>`).
    fs_calls: Arc<std::sync::atomic::AtomicU64>,
    /// Startet Tools in Stufe 2 (`fs/read_text_file`, #326); ohne: nicht angeboten.
    tools: Option<Arc<dyn beton_harness::process::ProcessLauncher>>,
    /// Terminals des Runners in Stufe 2 (`terminal/*`, #326); ohne: nicht angeboten.
    terminals: Option<Arc<dyn beton_harness::terminal::TerminalHost>>,
    /// Tool-Call je Terminal (für `tool.call.completed` bei `terminal/release`).
    terminal_calls: TerminalCalls,
}

async fn send_all(
    tx: &mpsc::Sender<NormalizedEvent>,
    events: Vec<EventPayload>,
    mut raw: Option<RawJson>,
    turn: Option<TurnId>,
) {
    for payload in events {
        // `raw` hängt am ersten Event der Nachricht (HAR-001 AC3).
        let _ = tx
            .send(NormalizedEvent {
                payload,
                raw: raw.take(),
                turn_id: turn,
            })
            .await;
    }
}

async fn dispatch(mut incoming: mpsc::Receiver<Incoming>, d: Dispatcher) {
    while let Some(msg) = incoming.recv().await {
        match msg {
            Incoming::Notification {
                method,
                params,
                raw,
            } => {
                let (events, turn) = {
                    let mut st = d.state.lock().await;
                    let events = if method == "session/update" {
                        map_update(&params["update"], &mut st.map)
                    } else {
                        vec![unmapped(&json!({"method": method, "params": params}))]
                    };
                    (events, st.turn)
                };
                send_all(&d.tx, events, raw, turn).await;
            }
            Incoming::Request {
                id,
                method,
                params,
                raw,
            } => {
                if method == "session/request_permission" {
                    permission(&d, &id, &params, raw).await;
                } else if method == "fs/write_text_file" {
                    fs_write(&d, &id, &params).await;
                } else if method == "fs/read_text_file" && d.tools.is_some() {
                    fs_read(&d, &id, &params).await;
                } else if method.starts_with("terminal/") && d.terminals.is_some() {
                    // `wait_for_exit` kann dauern: nicht im Lese-Loop warten.
                    let d = d.clone();
                    tokio::spawn(async move { terminal(&d, &method, &id, &params).await });
                } else {
                    // fs/* und terminal/* erst mit der Tool-Sandbox (SBX-002, ab M2).
                    if !(method.starts_with("fs/") || method.starts_with("terminal/")) {
                        let turn = d.state.lock().await.turn;
                        send_all(
                            &d.tx,
                            vec![unmapped(&json!({"method": method, "params": params}))],
                            raw,
                            turn,
                        )
                        .await;
                    }
                    let _ = d
                        .rpc
                        .respond_error(&id, codes::METHOD_NOT_FOUND, "von beton nicht unterstützt")
                        .await;
                }
            }
            Incoming::Response { id, result, raw } => {
                let mut st = d.state.lock().await;
                if st.prompt_request != Some(id) {
                    continue;
                }
                st.prompt_request = None;
                let turn = st.turn.take();
                let mut events = st.map.flush();
                drop(st);
                events.extend(turn_end(&d, turn.unwrap_or_default(), result));
                send_all(&d.tx, events, raw, turn).await;
            }
            Incoming::Invalid { line } => {
                tracing::warn!("ACP: ungültige stdout-Zeile");
                let turn = d.state.lock().await.turn;
                send_all(&d.tx, vec![unmapped(&Value::String(line))], None, turn).await;
            }
        }
    }
    // stdout zu: Prozess endet. Mitten im Turn → turn.failed und harness.exited (HAR-007 AC4).
    if !d.closing.load(Ordering::SeqCst) {
        let exit = d.process.lock().await.wait().await.unwrap_or_default();
        let turn = d.state.lock().await.turn.take();
        let mut events = Vec::new();
        if let Some(turn_id) = turn {
            events.extend(d.state.lock().await.map.flush());
            events.push(EventPayload::TurnFailed(TurnFailed {
                turn_id,
                problem: json!({"type": "urn:beton:problem:harness_exited", "title": "ACP-Agent ist beendet"}),
            }));
        }
        events.push(EventPayload::HarnessExited(HarnessExited {
            code: exit.code,
            signal: exit.signal,
            stderr_tail: exit.stderr_tail,
        }));
        send_all(&d.tx, events, None, turn).await;
    }
}

fn turn_end(d: &Dispatcher, turn_id: TurnId, result: Result<Value, RpcError>) -> Vec<EventPayload> {
    match result {
        Ok(v) => match v["stopReason"].as_str().unwrap_or("end_turn") {
            "cancelled" => vec![EventPayload::TurnInterrupted(TurnInterrupted {
                turn_id,
                by: PrincipalId::User(UserId::LOCAL),
                reason: None,
            })],
            "refusal" => vec![EventPayload::TurnFailed(TurnFailed {
                turn_id,
                problem: json!({"type": "urn:beton:problem:harness_refusal", "title": "Der Agent hat abgelehnt"}),
            })],
            reason => vec![EventPayload::TurnCompleted(TurnCompleted {
                turn_id,
                stop_reason: reason.to_owned(),
                usage_summary: json!({}),
            })],
        },
        Err(RpcError::Remote { code, message, .. }) => {
            let mut out = Vec::new();
            if code == AUTH_REQUIRED {
                // Login in der Agent-CLI, nie in beton (HAR-015).
                out.push(EventPayload::HarnessAuthRequired(HarnessAuthRequired {
                    harness: d.harness.clone(),
                    hint: d.hint.clone(),
                }));
            }
            out.push(EventPayload::TurnFailed(TurnFailed {
                turn_id,
                problem: json!({"type": "urn:beton:problem:harness_error", "title": message, "code": code}),
            }));
            out
        }
        Err(e) => vec![EventPayload::TurnFailed(TurnFailed {
            turn_id,
            problem: json!({"type": "urn:beton:problem:harness_error", "title": e.to_string()}),
        })],
    }
}

/// Option eines Permission-Requests der Art `kind`.
fn option_of(options: &Value, kind: &str) -> Option<String> {
    options
        .as_array()?
        .iter()
        .find(|o| o["kind"] == kind)
        .and_then(|o| o["optionId"].as_str().map(str::to_owned))
}

/// Antwort auf einen Permission-Request (POL-023 AC1): nur einmalige Optionen, nie „immer“,
/// damit jede weitere Aktion erneut geprüft wird. Ohne `allow_once` lehnt beton einmalig ab.
/// Liefert die Option (`None`: abbrechen) und ob der Call damit erlaubt ist.
fn choose(options: &Value, allow: bool) -> (Option<String>, bool) {
    if allow && let Some(o) = option_of(options, "allow_once") {
        return (Some(o), true);
    }
    (option_of(options, "reject_once"), false)
}

/// JSON-RPC-Fehler, wenn eine beton-Policy einen Client-Aufruf (`fs/*`) ablehnt.
const POLICY_DENIED: i64 = -32001;

/// `session/request_permission` → Gate (HAR-007, POL-023 ab M2).
async fn permission(d: &Dispatcher, id: &Value, params: &Value, raw: Option<RawJson>) {
    let tool = &params["toolCall"];
    let call_id = tool["toolCallId"].as_str().unwrap_or_default().to_owned();
    let (turn, pre) = {
        let mut st = d.state.lock().await;
        let mut pre = st.map.flush();
        pre.extend(st.map.requested(tool));
        (st.turn, pre)
    };
    let name = tool["title"].as_str().unwrap_or("tool").to_owned();
    let args = match &tool["rawInput"] {
        Value::Null => json!({}),
        v => v.clone(),
    };
    send_all(&d.tx, pre, None, turn).await;
    let request = GateRequest {
        turn_id: turn,
        call_id: call_id.clone(),
        tool: name,
        kind: mapping::tool_kind(tool["kind"].as_str()),
        args: args.clone(),
        mcp_server: None,
        acp_kind: tool["kind"].as_str().map(str::to_owned),
        cwd: None,
        origin: GateOrigin::VendorAsk,
        raw,
    };
    let cancelled = d.cancel.notified();
    let outcome = tokio::select! {
        r = tokio::time::timeout(d.gate_timeout, d.gate.decide(request)) => Some(r),
        () = cancelled => None,
    };
    let (allow, cancelled) = match outcome {
        None => (false, true),
        Some(Ok(GateDecision::Allow { updated_args })) => match updated_args {
            // ACP kann geänderte Argumente nicht übergeben: fail closed.
            Some(changed) if changed != args => (false, false),
            _ => (true, false),
        },
        Some(Ok(GateDecision::Deny { .. })) | Some(Err(_)) => (false, false),
    };
    // Nur einmalige Optionen, nie „immer erlauben“ (POL-023 AC1).
    let (option, allow) = if cancelled {
        (None, false)
    } else {
        choose(&params["options"], allow)
    };
    let response = match &option {
        Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    };
    let started = {
        let mut st = d.state.lock().await;
        if allow {
            st.map.started(&call_id)
        } else {
            st.map.deny(&call_id);
            None
        }
    };
    let _ = d.rpc.respond(id, response).await;
    send_all(&d.tx, started.into_iter().collect(), None, turn).await;
}

/// Ziel eines `fs/write_text_file`, wenn es im Arbeitsverzeichnis liegt: absoluter Pfad, das
/// Elternverzeichnis existiert und liegt (aufgelöst) im Workspace, das Ziel ist kein Symlink.
/// Nie unter einer Pfadkomponente `.git` (Groß/klein egal; Hooks, Konfiguration, die
/// `.git`-Datei eines Worktrees, SBX-003 AC4); mit `policy` nur, was Stufe 1 selbst schreiben
/// darf (Masken, nur lesbare Teilbäume, Security-Review #327).
fn write_target(
    workdir: &std::path::Path,
    raw: &str,
    policy: Option<&beton_sandbox::ResolvedPolicy>,
) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(raw);
    if !path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
        return None;
    }
    let root = workdir.canonicalize().ok()?;
    let parent = path.parent()?.canonicalize().ok()?;
    if !parent.starts_with(&root) {
        return None;
    }
    let target = parent.join(path.file_name()?);
    if target
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
    {
        return None;
    }
    if policy.is_some_and(|p| !p.may_write(&target)) {
        return None;
    }
    if std::fs::symlink_metadata(&target).is_ok_and(|m| !m.is_file()) {
        return None;
    }
    Some(target)
}

/// Schreibt `content` ohne Symlinks zu folgen (O_NOFOLLOW).
fn write_no_follow(target: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.custom_flags(libc::O_NOFOLLOW);
    }
    o.open(target)?.write_all(content.as_bytes())
}

/// `fs/write_text_file` (POL-023 AC2, HAR-007 AC2): unabhängig von Permission-Requests als
/// Datei-Schreiben vor der Ausführung durch das Gate (`path_guard`, volles Gate), nur im
/// Arbeitsverzeichnis; jede Ablehnung ist ein JSON-RPC-Fehler (fail closed).
async fn fs_write(d: &Dispatcher, id: &Value, params: &Value) {
    let (Some(raw), Some(content)) = (params["path"].as_str(), params["content"].as_str()) else {
        let _ = d
            .rpc
            .respond_error(id, codes::INVALID_PARAMS, "`path` und `content` fehlen")
            .await;
        return;
    };
    let Some(target) = write_target(&d.workdir, raw, d.write_policy.as_deref()) else {
        let _ = d
            .rpc
            .respond_error(
                id,
                codes::INVALID_PARAMS,
                "beton schreibt per fs/write_text_file nur im Arbeitsverzeichnis der Session und \
                 nur, was die Sandbox des Agents erlaubt (nie unter `.git`)",
            )
            .await;
        return;
    };
    let n = d.fs_calls.fetch_add(1, Ordering::SeqCst) + 1;
    let call_id = format!("fs-{n}");
    let turn = d.state.lock().await.turn;
    // Das Gate prüft den aufgelösten Pfad, in den beton schreibt, nicht den Rohpfad.
    let args = json!({ "path": target.display().to_string() });
    send_all(
        &d.tx,
        vec![EventPayload::ToolCallRequested(
            beton_core::event::ToolCallRequested {
                call_id: call_id.clone(),
                tool: "fs/write_text_file".into(),
                kind: Some("file_write".into()),
                mcp_server: None,
                args: args.clone(),
                source: beton_core::event::ToolSource::Acp,
                parent_call_id: None,
            },
        )],
        None,
        turn,
    )
    .await;
    let request = GateRequest {
        turn_id: turn,
        call_id: call_id.clone(),
        tool: "fs/write_text_file".into(),
        kind: "file_write".into(),
        args,
        mcp_server: None,
        acp_kind: Some("edit".into()),
        cwd: None,
        origin: GateOrigin::PreTool,
        raw: None,
    };
    let decision = tokio::time::timeout(d.gate_timeout, d.gate.decide(request)).await;
    let denied = match decision {
        Ok(GateDecision::Allow { updated_args: None }) => None,
        Ok(GateDecision::Allow { .. }) => {
            Some("geänderte Argumente kann beton hier nicht übernehmen".to_owned())
        }
        Ok(GateDecision::Deny { reason }) => {
            Some(reason.unwrap_or_else(|| "von der beton-Policy abgelehnt".into()))
        }
        Err(_) => Some("Gate antwortet nicht".to_owned()),
    };
    let (status, result) = match &denied {
        Some(reason) => {
            let _ = d.rpc.respond_error(id, POLICY_DENIED, reason).await;
            (beton_core::event::ToolStatus::Denied, json!(reason))
        }
        None => {
            send_all(
                &d.tx,
                vec![EventPayload::ToolCallStarted(
                    beton_core::event::ToolCallStarted {
                        call_id: call_id.clone(),
                        sandbox_stage: None,
                        args: None,
                    },
                )],
                None,
                turn,
            )
            .await;
            match write_no_follow(&target, content) {
                Ok(()) => {
                    let _ = d.rpc.respond(id, Value::Null).await;
                    (beton_core::event::ToolStatus::Ok, json!("geschrieben"))
                }
                Err(e) => {
                    let _ = d
                        .rpc
                        .respond_error(id, codes::INTERNAL_ERROR, &format!("schreiben: {e}"))
                        .await;
                    (beton_core::event::ToolStatus::Error, json!(e.to_string()))
                }
            }
        }
    };
    send_all(
        &d.tx,
        vec![EventPayload::ToolCallCompleted(
            beton_core::event::ToolCallCompleted {
                call_id,
                status,
                result: Some(result),
                result_ref: None,
                duration_ms: 0,
            },
        )],
        None,
        turn,
    )
    .await;
}

/// Größte Datei für `fs/read_text_file`.
const READ_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Ergebnis des Gates für einen Client-Aufruf: `None` = erlaubt, sonst der Grund.
async fn gate_client_call(d: &Dispatcher, request: GateRequest) -> Option<String> {
    match tokio::time::timeout(d.gate_timeout, d.gate.decide(request)).await {
        Ok(GateDecision::Allow { updated_args: None }) => None,
        Ok(GateDecision::Allow { .. }) => {
            Some("geänderte Argumente kann beton hier nicht übernehmen".to_owned())
        }
        Ok(GateDecision::Deny { reason }) => {
            Some(reason.unwrap_or_else(|| "von der beton-Policy abgelehnt".into()))
        }
        Err(_) => Some("Gate antwortet nicht".to_owned()),
    }
}

fn tool_event(call_id: &str, tool: &str, kind: &str, args: &Value) -> EventPayload {
    EventPayload::ToolCallRequested(beton_core::event::ToolCallRequested {
        call_id: call_id.to_owned(),
        tool: tool.into(),
        kind: Some(kind.into()),
        mcp_server: None,
        args: args.clone(),
        source: beton_core::event::ToolSource::Acp,
        parent_call_id: None,
    })
}

fn started_event(call_id: &str) -> EventPayload {
    EventPayload::ToolCallStarted(beton_core::event::ToolCallStarted {
        call_id: call_id.to_owned(),
        sandbox_stage: Some(beton_core::event::SandboxStage::Tools),
        args: None,
    })
}

fn completed_event(
    call_id: &str,
    status: beton_core::event::ToolStatus,
    result: Value,
) -> EventPayload {
    EventPayload::ToolCallCompleted(beton_core::event::ToolCallCompleted {
        call_id: call_id.to_owned(),
        status,
        result: Some(result),
        result_ref: None,
        duration_ms: 0,
    })
}

/// Zeilen `line` (ab 1) bis höchstens `limit` aus `text`.
fn select_lines(text: &str, line: Option<u64>, limit: Option<u64>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_owned();
    }
    let skip = line.unwrap_or(1).saturating_sub(1) as usize;
    let take = limit.map_or(usize::MAX, |l| l as usize);
    text.split_inclusive('\n').skip(skip).take(take).collect()
}

/// `fs/read_text_file` (#326, HAR-007): Datei-Lesen durch das Gate (`file_read`), gelesen in
/// Stufe 2 (`/bin/cat` über den Launcher der Tools): Masken und Grenzen der Sandbox gelten im
/// Kernel, ohne Zeitfenster zwischen Prüfung und Öffnen. Ins Log kommt nur die Größe.
async fn fs_read(d: &Dispatcher, id: &Value, params: &Value) {
    let Some(tools) = &d.tools else { return };
    let Some(raw) = params["path"].as_str() else {
        let _ = d
            .rpc
            .respond_error(id, codes::INVALID_PARAMS, "`path` fehlt")
            .await;
        return;
    };
    let path = std::path::Path::new(raw);
    if !path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
        let _ = d
            .rpc
            .respond_error(
                id,
                codes::INVALID_PARAMS,
                "`path` muss absolut sein (ohne `..`)",
            )
            .await;
        return;
    }
    let n = d.fs_calls.fetch_add(1, Ordering::SeqCst) + 1;
    let call_id = format!("fs-{n}");
    let turn = d.state.lock().await.turn;
    let args = json!({ "path": raw });
    send_all(
        &d.tx,
        vec![tool_event(
            &call_id,
            "fs/read_text_file",
            "file_read",
            &args,
        )],
        None,
        turn,
    )
    .await;
    let request = GateRequest {
        turn_id: turn,
        call_id: call_id.clone(),
        tool: "fs/read_text_file".into(),
        kind: "file_read".into(),
        args,
        mcp_server: None,
        acp_kind: Some("read".into()),
        origin: GateOrigin::PreTool,
        raw: None,
    };
    if let Some(reason) = gate_client_call(d, request).await {
        let _ = d.rpc.respond_error(id, POLICY_DENIED, &reason).await;
        send_all(
            &d.tx,
            vec![completed_event(
                &call_id,
                beton_core::event::ToolStatus::Denied,
                json!(reason),
            )],
            None,
            turn,
        )
        .await;
        return;
    }
    send_all(&d.tx, vec![started_event(&call_id)], None, turn).await;
    let read = async {
        use tokio::io::AsyncReadExt as _;
        let mut child = tools
            .launch(LaunchSpec {
                program: "/bin/cat".into(),
                args: vec!["--".into(), raw.to_owned()],
                cwd: Some(d.workdir.clone()),
                ..LaunchSpec::default()
            })
            .await
            .map_err(|e| format!("Stufe 2: {e}"))?;
        let io = child.take_io().ok_or("Stufe 2 ohne Ausgabe")?;
        drop(io.stdin);
        let mut buf = Vec::new();
        let mut limited = io.stdout.take(READ_MAX_BYTES as u64 + 1);
        limited
            .read_to_end(&mut buf)
            .await
            .map_err(|e| e.to_string())?;
        if buf.len() > READ_MAX_BYTES {
            let _ = child.kill().await;
            return Err(format!("größer als {READ_MAX_BYTES} Bytes"));
        }
        let exit = child.wait().await.map_err(|e| e.to_string())?;
        if exit.code != Some(0) {
            // Masken und fehlende Rechte sehen für den Agent gleich aus.
            return Err("nicht lesbar (Sandbox der Tools)".into());
        }
        String::from_utf8(buf).map_err(|_| "kein UTF-8-Text".to_owned())
    };
    let (status, result) = match tokio::time::timeout(Duration::from_secs(30), read).await {
        Ok(Ok(text)) => {
            let content = select_lines(&text, params["line"].as_u64(), params["limit"].as_u64());
            let _ = d.rpc.respond(id, json!({ "content": content })).await;
            (
                beton_core::event::ToolStatus::Ok,
                json!({ "bytes": text.len() }),
            )
        }
        Ok(Err(e)) => {
            let _ = d.rpc.respond_error(id, codes::INTERNAL_ERROR, &e).await;
            (beton_core::event::ToolStatus::Error, json!(e))
        }
        Err(_) => {
            let _ = d
                .rpc
                .respond_error(id, codes::INTERNAL_ERROR, "Zeitüberschreitung")
                .await;
            (
                beton_core::event::ToolStatus::Error,
                json!("Zeitüberschreitung"),
            )
        }
    };
    send_all(
        &d.tx,
        vec![completed_event(&call_id, status, result)],
        None,
        turn,
    )
    .await;
}

/// `terminal/*` (#326, HAR-007, SES-019): Kommandos des Agents laufen am PTY in Stufe 2 über
/// die Terminals des Runners und erscheinen als Terminal (`kind: tool`). `terminal/create`
/// geht vorher als Shell-Kommando durch das Gate.
async fn terminal(d: &Dispatcher, method: &str, id: &Value, params: &Value) {
    let Some(host) = &d.terminals else { return };
    let term_id = params["terminalId"].as_str().unwrap_or_default().to_owned();
    let reply = match method {
        "terminal/create" => {
            let Some(command) = params["command"].as_str() else {
                let _ = d
                    .rpc
                    .respond_error(id, codes::INVALID_PARAMS, "`command` fehlt")
                    .await;
                return;
            };
            let args: Vec<String> = params["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let env: Vec<(String, String)> = params["env"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| {
                            Some((
                                v["name"].as_str()?.to_owned(),
                                v["value"].as_str()?.to_owned(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let line = std::iter::once(command.to_owned())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ");
            let n = d.fs_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let call_id = format!("term-{n}");
            let turn = d.state.lock().await.turn;
            let gate_args = json!({ "command": line });
            send_all(
                &d.tx,
                vec![tool_event(&call_id, "terminal/create", "shell", &gate_args)],
                None,
                turn,
            )
            .await;
            let request = GateRequest {
                turn_id: turn,
                call_id: call_id.clone(),
                tool: "terminal/create".into(),
                kind: "shell".into(),
                args: gate_args,
                mcp_server: None,
                acp_kind: Some("execute".into()),
                origin: GateOrigin::PreTool,
                raw: None,
            };
            if let Some(reason) = gate_client_call(d, request).await {
                let _ = d.rpc.respond_error(id, POLICY_DENIED, &reason).await;
                send_all(
                    &d.tx,
                    vec![completed_event(
                        &call_id,
                        beton_core::event::ToolStatus::Denied,
                        json!(reason),
                    )],
                    None,
                    turn,
                )
                .await;
                return;
            }
            let req = beton_harness::terminal::ToolTerminal {
                command: command.to_owned(),
                args,
                env,
                cwd: params["cwd"].as_str().map(std::path::PathBuf::from),
                output_limit: params["outputByteLimit"].as_u64().map(|l| l as usize),
            };
            match host.create(req).await {
                Ok(tid) => {
                    send_all(&d.tx, vec![started_event(&call_id)], None, turn).await;
                    d.terminal_calls
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(tid.clone(), (call_id, turn));
                    Ok(json!({ "terminalId": tid }))
                }
                Err(e) => {
                    send_all(
                        &d.tx,
                        vec![completed_event(
                            &call_id,
                            beton_core::event::ToolStatus::Error,
                            json!(e),
                        )],
                        None,
                        turn,
                    )
                    .await;
                    Err(e)
                }
            }
        }
        "terminal/output" => host.output(&term_id).await.map(|o| {
            let mut v = json!({ "output": o.output, "truncated": o.truncated });
            if let Some(e) = o.exit {
                v["exitStatus"] = json!({ "exitCode": e.exit_code, "signal": e.signal });
            }
            v
        }),
        "terminal/wait_for_exit" => host
            .wait_for_exit(&term_id)
            .await
            .map(|e| json!({ "exitCode": e.exit_code, "signal": e.signal })),
        "terminal/kill" => host.kill(&term_id).await.map(|()| Value::Null),
        "terminal/release" => {
            let r = host.release(&term_id).await.map(|()| Value::Null);
            let call = d
                .terminal_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&term_id);
            if let Some((call_id, turn)) = call {
                send_all(
                    &d.tx,
                    vec![completed_event(
                        &call_id,
                        beton_core::event::ToolStatus::Ok,
                        json!({ "terminal_id": term_id }),
                    )],
                    None,
                    turn,
                )
                .await;
            }
            r
        }
        _ => {
            let _ = d
                .rpc
                .respond_error(id, codes::METHOD_NOT_FOUND, "von beton nicht unterstützt")
                .await;
            return;
        }
    };
    match reply {
        Ok(v) => {
            let _ = d.rpc.respond(id, v).await;
        }
        Err(e) => {
            let _ = d.rpc.respond_error(id, codes::INTERNAL_ERROR, &e).await;
        }
    }
}

/// Eine laufende ACP-Session.
pub struct AcpSession {
    caps: Capabilities,
    rpc: RpcClient,
    state: Arc<Mutex<Turn>>,
    tx: mpsc::Sender<NormalizedEvent>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<AtomicBool>,
    cancel: Arc<Notify>,
    session_id: String,
    modes: Vec<String>,
    /// Instructions für die erste Nachricht (`first_message_prefix`, AGT-005).
    instructions: Option<String>,
}

#[async_trait]
impl HarnessSession for AcpSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let turn = input.turn_id.unwrap_or_default();
        {
            let mut st = self.state.lock().await;
            if st.turn.is_some() {
                return Err(HarnessError::Protocol("es läuft bereits ein Turn".into()));
            }
            st.turn = Some(turn);
            st.map.begin_turn();
        }
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::TurnStarted(TurnStarted {
                    turn_id: turn,
                    input_id: None,
                    author: PrincipalId::User(UserId::LOCAL),
                }),
                Some(turn),
            ))
            .await;
        let text = match self.instructions.take() {
            Some(i) => beton_harness::prefix_instructions(&i, &input.text),
            None => input.text,
        };
        // Die ID vor dem Senden ablegen: Antwortet der Agent schneller, als das Senden
        // zurückkehrt, verwarf die Auswertung seine Antwort, und der Turn endete nie (#193).
        let id = self.rpc.reserve_id();
        self.state.lock().await.prompt_request = Some(id);
        let sent = self
            .rpc
            .send_reserved(
                id,
                "session/prompt",
                json!({"sessionId": self.session_id, "prompt": [{"type": "text", "text": text}]}),
            )
            .await;
        if let Err(e) = sent {
            self.state.lock().await.prompt_request = None;
            return Err(e.into());
        }
        Ok(turn)
    }

    async fn steer(&mut self, _input: UserInput) -> Result<(), HarnessError> {
        self.caps.check(Action::Steer)?;
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        if self.state.lock().await.turn.is_none() {
            return Ok(());
        }
        // Offene Rückfragen beantwortet der Dispatcher mit `cancelled`.
        self.cancel.notify_waiters();
        self.rpc
            .notify("session/cancel", json!({"sessionId": self.session_id}))
            .await?;
        Ok(())
    }

    async fn set_model(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        if effort.is_some() {
            self.caps.check(Action::EffortSwitch)?;
        }
        if let Some(model) = model {
            self.caps.check(Action::ModelSwitch)?;
            set_model(&self.rpc, &self.session_id, &model).await?;
        }
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        if !self.caps.permission_modes.contains(&mode) {
            return Err(beton_harness::CapabilityUnsupported(Action::PermissionMode).into());
        }
        let wanted = acp_mode(mode);
        if !self.modes.iter().any(|m| m == wanted) {
            // `default` ohne eigenen Modus des Agents: nichts zu tun.
            return Ok(());
        }
        self.rpc
            .request_timeout(
                "session/set_mode",
                json!({"sessionId": self.session_id, "modeId": wanted}),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    async fn compact(&mut self) -> Result<(), HarnessError> {
        self.caps.check(Action::Compact)?;
        Ok(())
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        Some(self.session_id.clone())
    }

    fn capabilities(&self) -> Option<Capabilities> {
        Some(self.caps.clone())
    }

    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError> {
        self.closing.store(true, Ordering::SeqCst);
        self.rpc.close().await;
        let mut process = self.process.lock().await;
        let exit = match how {
            Shutdown::Kill => process.kill().await?,
            Shutdown::Graceful { timeout } => {
                process
                    .terminate(ShutdownTimeouts {
                        graceful: timeout,
                        ..ShutdownTimeouts::default()
                    })
                    .await?
            }
        };
        Ok(exit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// POL-023 AC1: beton antwortet nur mit einer einmaligen Option, nie mit „immer“; ohne
    /// `allow_once` lehnt es einmalig ab, statt `allow_always` zu wählen.
    #[test]
    fn pol_023_ac1_only_once_options_are_ever_chosen() {
        let all = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "o", "kind": "allow_once"},
            {"optionId": "r", "kind": "reject_once"},
            {"optionId": "ra", "kind": "reject_always"},
        ]);
        assert_eq!(choose(&all, true), (Some("o".into()), true));
        assert_eq!(choose(&all, false), (Some("r".into()), false));
        let always_only = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "r", "kind": "reject_once"},
        ]);
        assert_eq!(choose(&always_only, true), (Some("r".into()), false));
        let no_once = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "ra", "kind": "reject_always"},
        ]);
        assert_eq!(choose(&no_once, true), (None, false));
        assert_eq!(choose(&no_once, false), (None, false));
    }

    #[test]
    fn permission_options_never_pick_always_first() {
        let options = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "o", "kind": "allow_once"},
            {"optionId": "r", "kind": "reject_once"},
        ]);
        assert_eq!(choose(&options, true).0.as_deref(), Some("o"));
        assert_eq!(choose(&options, false).0.as_deref(), Some("r"));
        assert_eq!(choose(&json!([]), true), (None, false));
    }
}

#[cfg(test)]
mod mcp_tests {
    use super::*;
    use beton_harness::{McpInjection, McpLaunch};

    #[test]
    fn har_009_acp_gets_relays_as_stdio_mcp_servers() {
        assert_eq!(mcp_servers(&SessionSpec::default()), json!([]));
        let spec = SessionSpec {
            mcp: McpInjection {
                servers: vec![McpLaunch {
                    name: "beton".into(),
                    command: "/bin/beton".into(),
                    args: vec!["mcp".into(), "serve".into()],
                }],
                skills_dir: None,
            },
            ..SessionSpec::default()
        };
        assert_eq!(
            mcp_servers(&spec),
            json!([{"name": "beton", "command": "/bin/beton", "args": ["mcp", "serve"], "env": []}])
        );
    }
}
