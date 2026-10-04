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
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    EventPayload, HarnessAuthRequired, HarnessExited, HarnessReady, RawJson, ResolvedVia,
    TimeoutAction, TurnCompleted, TurnFailed, TurnInterrupted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::jsonrpc::{Incoming, RpcClient, RpcError, codes};
use beton_harness::process::{
    LaunchSpec, ProcessHandle, ProcessLauncher, RealLauncher, ShutdownTimeouts,
};
use beton_harness::registry::{AcpAgentConfig, HarnessLayers, Registry, resolve_binary};
use beton_harness::{
    Action, AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport,
    ExitInfo, ForkHistory, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessError,
    HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode, NormalizedEvent,
    PermissionMode, ProbeReport, ResumeSupport, SessionSpec, Shutdown, Subagents, SwitchOutcome,
    SwitchSupport, ToolCallGate, Transport, UsageReporting, UserInput,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};

pub use crate::config::{AcpAgent, ConfigProblem, Origin, PRESETS, agents};
use crate::mapping::{MapState, map_update, tool_kind, unmapped};

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
async fn initialize(rpc: &RpcClient, timeout: Duration) -> Result<Value, HarnessError> {
    let result = rpc
        .request_timeout(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                // fs/* und terminal/* erst mit der Tool-Sandbox (SBX-002, ab M2).
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
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
            // Modellwechsel (`session/set_model`) folgt mit HAR-017.
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
            efforts: Vec::new(),
            context_window: None,
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
        let opened = open_session(&rpc, &spec, self.handshake_timeout).await;
        let (session_id, modes) = match opened {
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
            },
        ));
        Ok(Box::new(AcpSession {
            caps: self.capabilities(Mode::Native, &ProbeReport::default()),
            rpc,
            state,
            tx,
            rx: Some(rx),
            process,
            closing,
            cancel,
            session_id,
            modes,
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
    let probe = match initialize(&rpc, timeout).await {
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
/// sonst `session/new` (kalt, HAR-007 AC3). Liefert Session-ID und angebotene Modi.
async fn open_session(
    rpc: &RpcClient,
    spec: &SessionSpec,
    timeout: Duration,
) -> Result<(String, Vec<String>), Opened> {
    let init = initialize(rpc, timeout).await?;
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
    let modes = result["modes"]["availableModes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect();
    Ok((session_id, modes))
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

/// Wählt die Option eines Permission-Requests: nie automatisch „immer“, wenn „einmal“ geht.
fn option_for(options: &Value, allow: bool) -> Option<String> {
    let wanted: &[&str] = if allow {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    wanted.iter().find_map(|kind| {
        options
            .as_array()?
            .iter()
            .find(|o| o["kind"] == *kind)
            .and_then(|o| o["optionId"].as_str().map(str::to_owned))
    })
}

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
    let approval_id = ApprovalId::new();
    let expires = time::OffsetDateTime::now_utc() + d.gate_timeout;
    let mut events = pre;
    events.push(EventPayload::ApprovalRequested(ApprovalRequested {
        approval_id,
        kind: ApprovalKind::Tool,
        subject: json!({"tool": name, "args": args, "call_id": call_id}),
        options: vec!["allow".into(), "deny".into()],
        expires_at: Timestamp::from(expires),
        on_timeout: TimeoutAction::Deny,
    }));
    send_all(&d.tx, events, raw, turn).await;
    let request = GateRequest {
        turn_id: turn,
        call_id: call_id.clone(),
        tool: name,
        kind: tool_kind(tool["kind"].as_str().unwrap_or("other")).into(),
        args: args.clone(),
    };
    let cancelled = d.cancel.notified();
    let outcome = tokio::select! {
        r = tokio::time::timeout(d.gate_timeout, d.gate.decide(request)) => Some(r),
        () = cancelled => None,
    };
    let (decision, via, comment) = match outcome {
        None => (ApprovalDecision::Deny, ResolvedVia::System, None),
        Some(Ok(GateDecision::Allow { updated_args })) => match updated_args {
            // ACP kann geänderte Argumente nicht übergeben: fail closed.
            Some(changed) if changed != args => (
                ApprovalDecision::Deny,
                ResolvedVia::Policy,
                Some("Der Agent kann geänderte Argumente nicht ausführen".to_owned()),
            ),
            _ => (ApprovalDecision::Allow, ResolvedVia::User, None),
        },
        Some(Ok(GateDecision::Deny { reason })) => {
            (ApprovalDecision::Deny, ResolvedVia::User, reason)
        }
        Some(Err(_)) => (ApprovalDecision::Deny, ResolvedVia::Timeout, None),
    };
    let allow = decision == ApprovalDecision::Allow;
    let option = if outcome_cancelled(via) {
        None
    } else {
        option_for(&params["options"], allow)
    };
    let response = match &option {
        Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    };
    // Ohne passende Erlauben-Option bleibt es eine Ablehnung.
    let allow = allow && option.is_some();
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
    let mut events = vec![EventPayload::ApprovalResolved(ApprovalResolved {
        approval_id,
        decision: if allow {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Deny
        },
        answer: None,
        actor: if via == ResolvedVia::User {
            Actor::User {
                id: PrincipalId::User(UserId::LOCAL),
                device_id: None,
            }
        } else {
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            }
        },
        via,
        remember: None,
        comment,
        on_timeout_applied: (via == ResolvedVia::Timeout).then_some(TimeoutAction::Deny),
    })];
    events.extend(started);
    send_all(&d.tx, events, None, turn).await;
}

fn outcome_cancelled(via: ResolvedVia) -> bool {
    via == ResolvedVia::System
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
}

#[async_trait]
impl HarnessSession for AcpSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let turn = TurnId::new();
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
        let id = self
            .rpc
            .send_request(
                "session/prompt",
                json!({"sessionId": self.session_id, "prompt": [{"type": "text", "text": input.text}]}),
            )
            .await?;
        self.state.lock().await.prompt_request = Some(id);
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
        _model: String,
        _effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        self.caps.check(Action::ModelSwitch)?;
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        let wanted = match mode {
            PermissionMode::Plan => "plan",
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Yolo => "bypassPermissions",
        };
        if !self.modes.iter().any(|m| m == wanted) {
            return Err(HarnessError::Protocol(format!(
                "der Agent bietet den Modus `{wanted}` nicht an"
            )));
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

    #[test]
    fn permission_options_never_pick_always_first() {
        let options = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "o", "kind": "allow_once"},
            {"optionId": "r", "kind": "reject_once"},
        ]);
        assert_eq!(option_for(&options, true).as_deref(), Some("o"));
        assert_eq!(option_for(&options, false).as_deref(), Some("r"));
        assert_eq!(option_for(&json!([]), true), None);
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
