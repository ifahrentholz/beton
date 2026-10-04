//! Codex-Adapter über `codex app-server` (JSON-RPC 2.0 über stdio, HAR-006, HAR-015,
//! HAR-021).
//!
//! Ein langlebiger Prozess pro Session. Ablauf (verifiziert gegen codex-cli 0.153.2 per
//! `codex app-server generate-json-schema` und Handshake ohne Modellaufruf):
//! `initialize` → `initialized` → `thread/start` bzw. `thread/resume` → je Turn
//! `turn/start`; Abbruch mit `turn/interrupt`, Eingaben in den laufenden Turn mit
//! `turn/steer` (SES-004). Freigaben kommen als Server-Requests
//! `item/commandExecution/requestApproval` und `item/fileChange/requestApproval` und gehen an
//! das Gate. Subscriptions laufen ausschließlich über die CLI selbst: beton liest, speichert
//! oder erneuert keine OpenAI-Tokens (ADR-0005) und entfernt bei `auth: subscription`
//! API-Key-Variablen aus der Umgebung.

pub mod import;
pub mod mapping;
pub mod record;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    EventPayload, HarnessExited, HarnessReady, RawJson, ResolvedVia, SessionSettingsChanged,
    SettingsMechanism, TimeoutAction, TurnFailed, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::jsonrpc::{Incoming, RpcClient, RpcError, codes};
use beton_harness::process::{LaunchSpec, ProcessHandle, ShutdownTimeouts};
use beton_harness::registry::{VersionProbe, resolve_binary};
use beton_harness::{
    Action, AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport,
    ExitInfo, ForkHistory, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessError,
    HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode, NormalizedEvent, OneShotReply,
    OneShotRequest, PermissionMode, ProbeReport, ResumeSupport, SessionSpec, Shutdown, Subagents,
    SwitchOutcome, SwitchSupport, ToolCallGate, Transport, UsageReporting, UserInput,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};

use crate::mapping::{MapState, item_kind, map_notification, requested, started, unmapped};

/// Getestete CLI-Versionen (Golden-Transcripts unter `tests/golden/`).
pub const TESTED_VERSIONS: [&str; 1] = ["0.153.2"];
/// Unterstützter Bereich. Protokollbrüche innerhalb des Bereichs erkennt der Handshake
/// (`harness.incompatible`, HAR-006 AC4).
pub const VERSION_RANGE: &str = ">=0.153.0, <1.0.0";
/// Umgebungsvariablen, die bei `auth: subscription` nicht in den Prozess dürfen (HAR-015).
pub const SUBSCRIPTION_ENV_REMOVE: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];
/// Höchstens so lange wartet der Adapter auf das Gate; danach `decline` (fail closed).
pub const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Frist für `initialize` und `thread/start`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Höchstdauer für `codex login status`.
pub const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// Freigabe-Politik: mindestens `untrusted`, damit Shell- und Schreibaktionen das Gate
/// erreichen (HAR-006, HAR-027).
pub const APPROVAL_POLICY: &str = "untrusted";
/// Codex-eigene Sandbox, solange die beton-Sandbox Stufe 2 fehlt (ab M2 `danger-full-access`).
pub const SANDBOX_MODE: &str = "workspace-write";

/// Wertet `codex login status` aus (Text auf stderr, Exit-Code 1 ohne Login). Kontodaten
/// werden nicht übernommen (HAR-016).
pub fn parse_login_status(success: bool, output: &[u8]) -> AuthStatus {
    let text = String::from_utf8_lossy(output).to_lowercase();
    if text.contains("not logged in") {
        AuthStatus::LoggedOut
    } else if success && text.contains("logged in") {
        AuthStatus::LoggedIn
    } else {
        AuthStatus::Unknown
    }
}

/// Version aus dem `userAgent` der `initialize`-Antwort, z. B. `beton/0.153.2 (Mac OS …)`.
pub fn version_from_user_agent(user_agent: &str) -> Option<String> {
    let (_, rest) = user_agent.split_once('/')?;
    let version = rest.split_whitespace().next()?;
    semver::Version::parse(version).ok()?;
    Some(version.to_owned())
}

/// Der Codex-Adapter.
#[derive(Debug, Clone)]
pub struct CodexAdapter {
    pub probe: VersionProbe,
    /// Auth-Herkunft laut `harnesses.codex.auth` (Default: Subscription).
    pub auth: AuthSource,
    pub gate_timeout: Duration,
    pub handshake_timeout: Duration,
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self {
            probe: VersionProbe::default(),
            auth: AuthSource::VendorCli,
            gate_timeout: GATE_TIMEOUT,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        }
    }
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        mode: Mode::Native,
        transport: Transport::Native,
        version_range: Some(VERSION_RANGE.into()),
        auth_sources: vec![AuthSource::VendorCli, AuthSource::ApiKey],
        approval: ApprovalMechanism::NativeRequest,
        // Lesende Aktionen führt Codex ohne Rückfrage aus.
        tool_call_gate: ToolCallGate::ApprovalOnly,
        // `turn/start` nimmt Modell und Effort je Turn.
        model_switch: SwitchSupport::Live,
        effort_switch: SwitchSupport::Live,
        resume: ResumeSupport::Warm,
        fork_history: ForkHistory::Preamble,
        interrupt: true,
        // `turn/steer` speist Eingaben in den laufenden Turn (SES-004 AC3).
        steering: true,
        subagents: Subagents::None,
        usage_reporting: UsageReporting::Tokens,
        // `thread/compact/start` folgt mit HAR-022.
        compaction: CompactionSupport::None,
        instructions_delivery: InstructionsDelivery::DeveloperInstructions,
        // MCP-Server über `thread/start.config` (HAR-009).
        mcp_injection: true,
        images: false,
        // Import vorhandener Chats aus `$CODEX_HOME/sessions` (HAR-024).
        transcript_import: true,
        models: Vec::new(),
        models_stale: false,
        efforts: vec!["low".into(), "medium".into(), "high".into()],
        // Eingabefenster der GPT-5-Codex-Modelle (Handover-Budget, HAR-018).
        context_window: Some(272_000),
        // Codex liest `AGENTS.md` selbst, `CLAUDE.md` nicht (AGT-005).
        native_project_files: vec!["AGENTS.md".into()],
    }
}

fn harness_id() -> HarnessId {
    HarnessId::CODEX.parse().unwrap_or_else(|_| unreachable!())
}

fn incompatible(detected: Option<String>, why: &str) -> HarnessError {
    tracing::warn!(reason = why, "codex app-server: Protokoll passt nicht");
    HarnessError::Incompatible {
        detected: detected.unwrap_or_else(|| "unbekannt".into()),
        expected: VERSION_RANGE.into(),
    }
}

impl CodexAdapter {
    /// Variablen, die bei `auth: subscription` nicht in den CLI-Prozess dürfen (HAR-015).
    pub fn env_remove(&self) -> Vec<String> {
        if self.auth == AuthSource::VendorCli {
            SUBSCRIPTION_ENV_REMOVE
                .iter()
                .map(|s| (*s).to_owned())
                .collect()
        } else {
            Vec::new()
        }
    }
}

/// Kommandozeile des Einmal-Modus (SES-010, Flags gegen codex-cli 0.153.2 verifiziert):
/// JSONL-Ereignisse, keine gespeicherte Session, nur lesende Sandbox. Der Inhalt kommt über
/// stdin (`-`), nicht über argv. Ohne Modell gilt der Default der CLI-Konfiguration.
pub fn one_shot_args(req: &OneShotRequest) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(model) = &req.model {
        args.extend(["-m".into(), model.clone()]);
    }
    args.push("-".into());
    args
}

/// Wertet die JSONL-Ausgabe von `codex exec --json` aus (SES-010): letzte
/// `agent_message`, Tokens aus `turn.completed`; `turn.failed` bzw. `error` sind Fehler.
pub fn parse_one_shot(
    stdout: &[u8],
    model: Option<&str>,
    auth: AuthSource,
) -> Result<OneShotReply, HarnessError> {
    let mut text = None;
    let mut usage = None;
    let mut failure = None;
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"] == "agent_message" => {
                text = v["item"]["text"].as_str().map(str::to_owned);
            }
            Some("turn.completed") => usage = Some(v["usage"].clone()),
            Some("turn.failed") => failure = Some("turn.failed".to_owned()),
            Some("error") => failure = Some("error".to_owned()),
            _ => {}
        }
    }
    if let Some(kind) = failure.filter(|_| text.is_none()) {
        return Err(HarnessError::Protocol(format!("codex exec meldet {kind}")));
    }
    let text = text.ok_or_else(|| HarnessError::Protocol("codex exec ohne Antwort".into()))?;
    let model = model.unwrap_or_default().to_owned();
    let cost = usage.map(|u| {
        let tok = |k: &str| u[k].as_u64().unwrap_or(0);
        beton_core::event::CostDelta {
            harness: "codex".into(),
            model: model.clone(),
            input_tokens: tok("input_tokens").saturating_sub(tok("cached_input_tokens")),
            output_tokens: tok("output_tokens"),
            cache_read_tokens: tok("cached_input_tokens"),
            cache_write_tokens: 0,
            // Codex meldet nur Tokens; Preise folgen mit dem Katalog (USE-002, ab M2).
            cost_micro: None,
            currency: "USD".into(),
            source: if auth == AuthSource::VendorCli {
                beton_core::event::CostSource::Subscription
            } else {
                beton_core::event::CostSource::Estimated
            },
            auth_source: auth,
            purpose: None,
        }
    });
    Ok(OneShotReply { text, model, cost })
}

#[async_trait]
impl HarnessAdapter for CodexAdapter {
    fn id(&self) -> HarnessId {
        harness_id()
    }

    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }

    fn capabilities(&self, _mode: Mode, _probe: &ProbeReport) -> Capabilities {
        capabilities()
    }

    async fn probe(&self, env: &HostEnv) -> ProbeReport {
        let Some(bin) = resolve_binary(&harness_id(), "codex", env) else {
            return ProbeReport::default();
        };
        let (version, failed) = match self.probe.version(&bin.program).await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        ProbeReport {
            installed: true,
            path: Some(bin.program.display().to_string()),
            version,
            auth_status: AuthStatus::Unknown,
            probe_failed: failed,
        }
    }

    async fn auth_status(&self, env: &HostEnv) -> AuthStatus {
        let Some(bin) = resolve_binary(&harness_id(), "codex", env) else {
            return AuthStatus::Unknown;
        };
        let remove: &[&str] = if self.auth == AuthSource::VendorCli {
            &SUBSCRIPTION_ENV_REMOVE
        } else {
            &[]
        };
        match beton_harness::registry::run_status(
            &bin.program,
            &["login", "status"],
            remove,
            AUTH_STATUS_TIMEOUT,
        )
        .await
        {
            Ok(out) => {
                let mut text = out.stdout;
                text.extend_from_slice(&out.stderr);
                parse_login_status(out.success, &text)
            }
            Err(_) => AuthStatus::Unknown,
        }
    }

    fn transcript_importer(&self) -> Option<Arc<dyn beton_harness::import::TranscriptImporter>> {
        Some(Arc::new(import::CodexImporter::default()))
    }

    /// `codex exec` mit der Anmeldung der CLI (SES-010, ADR-0034).
    async fn one_shot(
        &self,
        request: &OneShotRequest,
        ctx: &AdapterContext,
    ) -> Result<OneShotReply, HarnessError> {
        let (program, mut args) = match resolve_binary(&harness_id(), "codex", &ctx.env) {
            Some(bin) => (bin.program, bin.args),
            None => ("codex".into(), Vec::new()),
        };
        args.extend(one_shot_args(request));
        let launch = LaunchSpec {
            program,
            args,
            env: Vec::new(),
            env_remove: self.env_remove(),
            clear_env: false,
            cwd: Some(request.workdir.clone()),
        };
        let input = if request.instructions.is_empty() {
            request.prompt.clone()
        } else {
            format!("{}\n\n{}", request.instructions, request.prompt)
        };
        let out = beton_harness::process::run_once(
            ctx.launcher.as_ref(),
            launch,
            input.as_bytes(),
            request.timeout,
        )
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::TimedOut => HarnessError::Timeout(e.to_string()),
            _ => HarnessError::Io(e),
        })?;
        parse_one_shot(&out.stdout, request.model.as_deref(), self.auth)
    }

    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let (program, mut args) = match resolve_binary(&harness_id(), "codex", &ctx.env) {
            Some(bin) => (bin.program, bin.args),
            None => ("codex".into(), Vec::new()),
        };
        args.push("app-server".into());
        let launch = LaunchSpec {
            program,
            args,
            env: Vec::new(),
            env_remove: self.env_remove(),
            clear_env: false,
            cwd: Some(spec.workdir.clone()),
        };
        let mut process = ctx.launcher.launch(launch).await?;
        let io = process.take_io().ok_or(HarnessError::Closed)?;
        let process = Arc::new(Mutex::new(process));
        let (rpc, incoming) = RpcClient::spawn(io.stdin, io.stdout);
        match handshake(&rpc, &spec, self.handshake_timeout).await {
            Ok(thread) => {
                let thread_id = thread.id.clone();
                let state = Arc::new(Mutex::new(MapState {
                    thread_id: Some(thread.id.clone()),
                    model: thread.model,
                    auth_source: Some(self.auth),
                    ..MapState::default()
                }));
                let (tx, rx) = mpsc::channel(4096);
                let _ = tx
                    .send(NormalizedEvent::new(
                        EventPayload::HarnessReady(HarnessReady {
                            harness_session_ref: Some(thread.id),
                            tools: Vec::new(),
                            mcp_servers: spec.mcp.names(),
                        }),
                        None,
                    ))
                    .await;
                let closing = Arc::new(AtomicBool::new(false));
                let cancel = Arc::new(Notify::new());
                tokio::spawn(dispatch(
                    incoming,
                    Dispatcher {
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
                Ok(Box::new(CodexSession {
                    rpc,
                    state,
                    tx,
                    rx: Some(rx),
                    process,
                    closing,
                    cancel,
                    thread_id,
                    next: NextTurn::default(),
                }))
            }
            Err(e) => {
                rpc.close().await;
                let _ = process
                    .lock()
                    .await
                    .terminate(ShutdownTimeouts {
                        graceful: Duration::from_secs(2),
                        ..ShutdownTimeouts::default()
                    })
                    .await;
                Err(e)
            }
        }
    }
}

struct Thread {
    id: String,
    model: String,
}

/// MCP-Server für `thread/start.config` (HAR-009): `mcp_servers.<name>` mit dem
/// Relay-Kommando. Codex ergänzt sie zu den eigenen Servern des Nutzers (verifiziert gegen
/// 0.153.2: Config-Overrides werden je Schlüssel zusammengeführt).
pub fn mcp_config(spec: &SessionSpec) -> Option<Value> {
    if spec.mcp.servers.is_empty() {
        return None;
    }
    let servers: serde_json::Map<String, Value> = spec
        .mcp
        .servers
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                json!({"command": s.command, "args": s.args}),
            )
        })
        .collect();
    Some(json!({"mcp_servers": servers}))
}

/// `initialize`, `initialized` und `thread/start` bzw. `thread/resume` (HAR-006 AC4).
async fn handshake(
    rpc: &RpcClient,
    spec: &SessionSpec,
    timeout: Duration,
) -> Result<Thread, HarnessError> {
    let init = rpc
        .request_timeout(
            "initialize",
            json!({"clientInfo": {"name": "beton", "title": "beton", "version": env!("CARGO_PKG_VERSION")}}),
            timeout,
        )
        .await;
    let init = match init {
        Ok(v) => v,
        // Schema-Fehler beim Handshake: anderes Protokoll.
        Err(RpcError::Remote { message, .. }) => {
            return Err(incompatible(
                None,
                &format!("initialize abgelehnt: {message}"),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let Some(user_agent) = init["userAgent"].as_str() else {
        return Err(incompatible(None, "initialize-Antwort ohne userAgent"));
    };
    let detected = version_from_user_agent(user_agent);
    if let Some(v) = &detected
        && let (Ok(version), Ok(range)) = (
            semver::Version::parse(v),
            semver::VersionReq::parse(VERSION_RANGE),
        )
        && !range.matches(&version)
    {
        return Err(incompatible(detected, "Version außerhalb des Bereichs"));
    }
    rpc.notify("initialized", json!({})).await?;
    let mut params = json!({
        "cwd": spec.workdir.display().to_string(),
        "approvalPolicy": APPROVAL_POLICY,
        "sandbox": SANDBOX_MODE,
    });
    if let Some(model) = &spec.model {
        params["model"] = json!(model);
    }
    if let Some(config) = mcp_config(spec) {
        params["config"] = config;
    }
    let method = match &spec.resume {
        Some(thread) => {
            params["threadId"] = json!(thread);
            "thread/resume"
        }
        None => {
            // Agent-Instructions als Developer-Instructions des Threads (AGT-005); ein
            // fortgesetzter Thread hat sie schon in seinem Verlauf.
            if let Some(text) = &spec.instructions {
                params["developerInstructions"] = json!(text);
            }
            "thread/start"
        }
    };
    let result = match rpc.request_timeout(method, params, timeout).await {
        Ok(v) => v,
        Err(RpcError::Remote { message, .. }) if spec.resume.is_some() => {
            return Err(HarnessError::StartRefused(format!(
                "Codex-Thread lässt sich nicht fortsetzen: {message}"
            )));
        }
        Err(RpcError::Remote { message, .. }) => {
            return Err(incompatible(
                detected,
                &format!("{method} abgelehnt: {message}"),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let Some(id) = result["thread"]["id"].as_str() else {
        return Err(incompatible(detected, "thread ohne id"));
    };
    // Session-Skills als zusätzliche Skill-Wurzel (AGT-008, gegen 0.153.2 verifiziert).
    if let Some(dir) = &spec.mcp.skills_dir {
        let roots = json!({"extraRoots": [dir.join("skills").display().to_string()]});
        if let Err(e) = rpc
            .request_timeout("skills/extraRoots/set", roots, timeout)
            .await
        {
            tracing::warn!("codex: Session-Skills nicht gesetzt: {e}");
        }
    }
    Ok(Thread {
        id: id.to_owned(),
        model: result["model"].as_str().unwrap_or_default().to_owned(),
    })
}

struct Dispatcher {
    rpc: RpcClient,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    gate: Arc<dyn Gate>,
    gate_timeout: Duration,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<AtomicBool>,
    cancel: Arc<Notify>,
}

async fn send(
    tx: &mpsc::Sender<NormalizedEvent>,
    payload: EventPayload,
    raw: Option<RawJson>,
    turn: Option<TurnId>,
) {
    let _ = tx
        .send(NormalizedEvent {
            payload,
            raw,
            turn_id: turn,
        })
        .await;
}

async fn send_all(
    tx: &mpsc::Sender<NormalizedEvent>,
    events: Vec<EventPayload>,
    mut raw: Option<RawJson>,
    turn: Option<TurnId>,
) {
    for e in events {
        // `raw` hängt am ersten Event der Zeile (HAR-001 AC3).
        send(tx, e, raw.take(), turn).await;
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
                    let turn = st.turn;
                    (map_notification(&method, &params, &mut st), turn)
                };
                if events
                    .iter()
                    .any(|e| matches!(e, EventPayload::HarnessUnmapped(_)))
                {
                    tracing::warn!(method, "codex: unbekannte Nachricht");
                }
                send_all(&d.tx, events, raw, turn).await;
            }
            Incoming::Request {
                id,
                method,
                params,
                raw,
            } => server_request(&d, &id, &method, &params, raw).await,
            Incoming::Response { result, raw, .. } => {
                // Antwort auf `turn/start`.
                let mut st = d.state.lock().await;
                match result {
                    Ok(v) => {
                        if let Some(t) = v["turn"]["id"].as_str() {
                            st.codex_turn = Some(t.to_owned());
                        }
                    }
                    Err(e) => {
                        if let Some(turn_id) = st.turn.take() {
                            let failed = EventPayload::TurnFailed(TurnFailed {
                                turn_id,
                                problem: json!({"type": "urn:beton:problem:harness_error", "title": e.to_string()}),
                            });
                            drop(st);
                            send(&d.tx, failed, raw, Some(turn_id)).await;
                        }
                    }
                }
            }
            Incoming::Invalid { line } => {
                tracing::warn!("codex: ungültige stdout-Zeile");
                let turn = d.state.lock().await.turn;
                send(&d.tx, unmapped(&Value::String(line)), None, turn).await;
            }
        }
    }
    // stdout zu: Prozess endet. Unerwartet → turn.failed und harness.exited (HAR-001 AC1).
    if !d.closing.load(Ordering::SeqCst) {
        let exit = d.process.lock().await.wait().await.unwrap_or_default();
        if let Some(turn_id) = d.state.lock().await.turn.take() {
            send(
                &d.tx,
                EventPayload::TurnFailed(TurnFailed {
                    turn_id,
                    problem: json!({"type": "urn:beton:problem:harness_exited", "title": "codex app-server ist beendet"}),
                }),
                None,
                Some(turn_id),
            )
            .await;
        }
        send(
            &d.tx,
            EventPayload::HarnessExited(HarnessExited {
                code: exit.code,
                signal: exit.signal,
                stderr_tail: exit.stderr_tail,
            }),
            None,
            None,
        )
        .await;
    }
}

/// Server-Requests: Freigaben gehen an das Gate, alles andere lehnt beton ab (fail closed).
async fn server_request(
    d: &Dispatcher,
    id: &Value,
    method: &str,
    params: &Value,
    raw: Option<RawJson>,
) {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            approval(d, id, method, params, raw).await;
        }
        _ => {
            // Token-Erneuerung (`account/chatgptAuthTokens/refresh`) übernimmt nie beton
            // (ADR-0005); Rückfragen, Elicitations und dynamische Tools folgen später.
            let turn = d.state.lock().await.turn;
            send(
                &d.tx,
                unmapped(&json!({"method": method, "params": params})),
                raw,
                turn,
            )
            .await;
            let _ = d
                .rpc
                .respond_error(id, codes::METHOD_NOT_FOUND, "von beton nicht unterstützt")
                .await;
        }
    }
}

async fn approval(d: &Dispatcher, id: &Value, method: &str, params: &Value, raw: Option<RawJson>) {
    let call_id = params["itemId"].as_str().unwrap_or_default().to_owned();
    let command = method == "item/commandExecution/requestApproval";
    let item_type = if command {
        "commandExecution"
    } else {
        "fileChange"
    };
    let (turn, pre) = {
        let mut st = d.state.lock().await;
        // Kam das Item noch nicht an, entsteht `tool.call.requested` aus der Anfrage.
        let pre = requested(
            &json!({"type": item_type, "id": call_id, "command": params["command"], "cwd": params["cwd"], "changes": params["changes"]}),
            &mut st,
        );
        (st.turn, pre)
    };
    let mut raw = raw;
    if let Some(e) = pre {
        send(&d.tx, e, raw.take(), turn).await;
    }
    let args = if command {
        json!({"command": params["command"], "cwd": params["cwd"], "reason": params["reason"]})
    } else {
        json!({"reason": params["reason"], "grant_root": params["grantRoot"]})
    };
    let approval_id = ApprovalId::new();
    let expires = time::OffsetDateTime::now_utc() + d.gate_timeout;
    send(
        &d.tx,
        EventPayload::ApprovalRequested(ApprovalRequested {
            approval_id,
            kind: ApprovalKind::Tool,
            subject: json!({"tool": item_type, "args": args, "call_id": call_id}),
            options: vec!["allow".into(), "deny".into()],
            expires_at: Timestamp::from(expires),
            on_timeout: TimeoutAction::Deny,
        }),
        raw,
        turn,
    )
    .await;
    let request = GateRequest {
        turn_id: turn,
        call_id: call_id.clone(),
        tool: item_type.into(),
        kind: item_kind(item_type).into(),
        args: args.clone(),
    };
    let cancelled = d.cancel.notified();
    let outcome = tokio::select! {
        r = tokio::time::timeout(d.gate_timeout, d.gate.decide(request)) => Some(r),
        () = cancelled => None,
    };
    let (decision, via, comment, answer) = match outcome {
        // Interrupt während der Rückfrage: Codex bricht den Turn mit `cancel` ab.
        None => (ApprovalDecision::Deny, ResolvedVia::System, None, "cancel"),
        Some(Ok(GateDecision::Allow { updated_args })) => match updated_args {
            // Codex kann geänderte Argumente nicht ausführen: fail closed.
            Some(changed) if changed != args => (
                ApprovalDecision::Deny,
                ResolvedVia::Policy,
                Some("Codex kann geänderte Argumente nicht ausführen".to_owned()),
                "decline",
            ),
            _ => (ApprovalDecision::Allow, ResolvedVia::User, None, "accept"),
        },
        Some(Ok(GateDecision::Deny { reason })) => {
            (ApprovalDecision::Deny, ResolvedVia::User, reason, "decline")
        }
        Some(Err(_)) => (
            ApprovalDecision::Deny,
            ResolvedVia::Timeout,
            None,
            "decline",
        ),
    };
    if decision == ApprovalDecision::Deny {
        d.state.lock().await.denied.insert(call_id.clone());
    }
    let _ = d.rpc.respond(id, json!({"decision": answer})).await;
    send(
        &d.tx,
        EventPayload::ApprovalResolved(ApprovalResolved {
            approval_id,
            decision,
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
        }),
        None,
        turn,
    )
    .await;
    if decision == ApprovalDecision::Allow {
        let started = started(&call_id, &mut *d.state.lock().await);
        if let Some(e) = started {
            send(&d.tx, e, None, turn).await;
        }
    }
}

/// Einstellungen, die mit dem nächsten `turn/start` wirken.
#[derive(Debug, Default)]
struct NextTurn {
    model: Option<String>,
    effort: Option<String>,
    sandbox_policy: Option<Value>,
}

/// Warum es keine Codex-Turn-ID gibt.
#[derive(Debug)]
enum NoTurn {
    /// Kein laufender Turn.
    Idle,
    /// `turn/start` blieb unbeantwortet.
    Unanswered,
}

impl From<NoTurn> for HarnessError {
    fn from(e: NoTurn) -> Self {
        match e {
            NoTurn::Idle => HarnessError::Protocol("kein laufender Turn".into()),
            NoTurn::Unanswered => {
                HarnessError::Protocol("Turn-ID von Codex fehlt; turn/start unbeantwortet".into())
            }
        }
    }
}

/// Eine laufende Codex-Session.
pub struct CodexSession {
    rpc: RpcClient,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<AtomicBool>,
    cancel: Arc<Notify>,
    thread_id: String,
    next: NextTurn,
}

impl CodexSession {
    /// Turn-ID des laufenden Turns laut Codex; wartet kurz auf die Antwort von `turn/start`.
    async fn codex_turn(&self) -> Result<String, NoTurn> {
        for _ in 0..100 {
            let st = self.state.lock().await;
            if st.turn.is_none() {
                return Err(NoTurn::Idle);
            }
            if let Some(t) = &st.codex_turn {
                return Ok(t.clone());
            }
            drop(st);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err(NoTurn::Unanswered)
    }
}

#[async_trait]
impl HarnessSession for CodexSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let turn = TurnId::new();
        {
            let mut st = self.state.lock().await;
            if st.turn.is_some() {
                return Err(HarnessError::Protocol("es läuft bereits ein Turn".into()));
            }
            st.turn = Some(turn);
            st.codex_turn = None;
            st.turn_start_total = st.total;
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
        let mut params = json!({
            "threadId": self.thread_id,
            "input": [{"type": "text", "text": input.text}],
        });
        if let Some(model) = &self.next.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &self.next.effort {
            params["effort"] = json!(effort);
        }
        if let Some(policy) = &self.next.sandbox_policy {
            params["sandboxPolicy"] = policy.clone();
        }
        self.rpc.send_request("turn/start", params).await?;
        Ok(turn)
    }

    async fn steer(&mut self, input: UserInput) -> Result<(), HarnessError> {
        capabilities().check(Action::Steer)?;
        let turn_id = self.codex_turn().await?;
        // `expectedTurnId` verhindert, dass die Eingabe in einem anderen Turn landet.
        self.rpc
            .request_timeout(
                "turn/steer",
                json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "input": [{"type": "text", "text": input.text}],
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        // Offene Rückfragen beantwortet der Dispatcher mit `cancel`.
        self.cancel.notify_waiters();
        let turn_id = match self.codex_turn().await {
            Ok(t) => t,
            Err(NoTurn::Idle) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        self.rpc
            .request_timeout(
                "turn/interrupt",
                json!({"threadId": self.thread_id, "turnId": turn_id}),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    async fn set_model(
        &mut self,
        model: String,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        self.next.model = Some(model.clone());
        if effort.is_some() {
            self.next.effort.clone_from(&effort);
        }
        self.state.lock().await.model.clone_from(&model);
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::SessionSettingsChanged(SessionSettingsChanged {
                    model: Some(model),
                    effort,
                    mechanism: Some(SettingsMechanism::Live),
                    ..SessionSettingsChanged::default()
                }),
                None,
            ))
            .await;
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        // Die Freigabe-Politik bleibt `untrusted`; `plan` macht die Codex-Sandbox
        // schreibgeschützt (HAR-027).
        let (name, policy) = match mode {
            PermissionMode::Plan => ("plan", json!({"type": "readOnly"})),
            PermissionMode::Default => ("default", json!({"type": "workspaceWrite"})),
            PermissionMode::AcceptEdits => ("accept_edits", json!({"type": "workspaceWrite"})),
            PermissionMode::Yolo => ("yolo", json!({"type": "workspaceWrite"})),
        };
        self.next.sandbox_policy = Some(policy);
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::SessionSettingsChanged(SessionSettingsChanged {
                    permission_mode: Some(name.into()),
                    mechanism: Some(SettingsMechanism::Live),
                    ..SessionSettingsChanged::default()
                }),
                None,
            ))
            .await;
        Ok(())
    }

    async fn compact(&mut self) -> Result<(), HarnessError> {
        capabilities().check(Action::Compact)?;
        Ok(())
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError> {
        self.closing.store(true, Ordering::SeqCst);
        // stdin schließen ist das protokolleigene Ende.
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
    fn har_016_login_status_reads_only_the_state() {
        assert_eq!(
            parse_login_status(true, b"Logged in using ChatGPT"),
            AuthStatus::LoggedIn
        );
        assert_eq!(
            parse_login_status(false, b"Not logged in"),
            AuthStatus::LoggedOut
        );
        assert_eq!(parse_login_status(false, b"Fehler"), AuthStatus::Unknown);
    }

    #[test]
    fn har_006_version_comes_from_the_user_agent() {
        // Format der echten CLI (0.153.2): `<clientInfo.name>/<version> (<OS>; <arch>) …`.
        assert_eq!(
            version_from_user_agent("beton/0.153.2 (Mac OS 26.6.2; arm64) unknown (beton; 0.0.0)")
                .as_deref(),
            Some("0.153.2")
        );
        assert_eq!(version_from_user_agent("ohne Version"), None);
    }
}

#[cfg(test)]
mod mcp_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use beton_harness::{McpInjection, McpLaunch};

    #[test]
    fn har_009_codex_gets_relays_via_thread_config() {
        assert_eq!(mcp_config(&SessionSpec::default()), None);
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
            mcp_config(&spec).unwrap(),
            json!({"mcp_servers": {"beton": {"command": "/bin/beton", "args": ["mcp", "serve"]}}})
        );
        assert!(capabilities().mcp_injection);
    }

    #[test]
    fn agt_007_codex_marks_beton_mcp_calls_as_system_tools() {
        let mut st = mapping::MapState::default();
        let item = json!({"id": "i1", "type": "mcpToolCall", "server": "beton", "tool": "policy_query", "arguments": {}, "status": "inProgress"});
        let Some(EventPayload::ToolCallRequested(r)) = requested(&item, &mut st) else {
            panic!()
        };
        assert_eq!(r.source, beton_core::event::ToolSource::BetonMcp);
        assert_eq!(r.tool, "policy_query");
        let item = json!({"id": "i2", "type": "mcpToolCall", "server": "gh", "tool": "get", "arguments": {}, "status": "inProgress"});
        let Some(EventPayload::ToolCallRequested(r)) = requested(&item, &mut st) else {
            panic!()
        };
        assert_eq!(r.source, beton_core::event::ToolSource::Harness);
    }
}
