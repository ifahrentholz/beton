//! Claude-Code-Adapter über das stream-json-Protokoll der offiziellen `claude`-CLI
//! (HAR-004, HAR-005, HAR-015, HAR-021).
//!
//! Ein langlebiger Prozess pro Session. Subscriptions laufen ausschließlich über die CLI
//! selbst: beton liest, speichert oder injiziert keine Anthropic-Tokens (ADR-0005) und
//! entfernt bei `auth: subscription` API-Key-Variablen aus der Umgebung.

pub mod mapping;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    EventPayload, HarnessExited, RawJson, ResolvedVia, SessionSettingsChanged, SettingsMechanism,
    TimeoutAction, ToolCallStarted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::process::{LaunchSpec, ProcessHandle, ShutdownTimeouts};
use beton_harness::registry::{VersionProbe, resolve_binary};
use beton_harness::{
    Action, AdapterContext, ApprovalMechanism, AuthStatus, Capabilities, CompactionSupport,
    ExitInfo, ForkHistory, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessError,
    HarnessId, HarnessSession, HostEnv, InstructionsDelivery, Mode, NormalizedEvent,
    PermissionMode, ProbeReport, ResumeSupport, SessionSpec, Shutdown, Subagents, SwitchOutcome,
    SwitchSupport, ToolCallGate, Transport, UsageReporting, UserInput,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use crate::mapping::{MapState, map_line, unmapped};

/// Getestete CLI-Versionen (Golden-Transcripts unter `tests/golden/`).
pub const TESTED_VERSIONS: [&str; 1] = ["2.1.285"];
/// Unterstützter Bereich; die Fake-CLI meldet 2.1.0.
pub const VERSION_RANGE: &str = ">=2.1.0, <3.0.0";
/// Umgebungsvariablen, die bei `auth: subscription` nicht in den Prozess dürfen (HAR-015).
pub const SUBSCRIPTION_ENV_REMOVE: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
/// Höchstens so lange wartet der Adapter auf das Gate; danach `deny` (HAR-005 AC4).
pub const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Der Claude-Code-Adapter.
#[derive(Debug, Clone)]
pub struct ClaudeAdapter {
    pub probe: VersionProbe,
    /// Auth-Herkunft laut `harnesses.claude.auth` (Default: Subscription).
    pub auth: AuthSource,
    /// Benutzer-Anpassungen der CLI (Hooks, Skills, Plugins, MCP) abschalten.
    pub isolated: bool,
    pub gate_timeout: Duration,
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self {
            probe: VersionProbe::default(),
            auth: AuthSource::VendorCli,
            isolated: false,
            gate_timeout: GATE_TIMEOUT,
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
        // Vollständig erst mit dem PreToolUse-Hook (HAR-005, ab M2).
        tool_call_gate: ToolCallGate::ApprovalOnly,
        model_switch: SwitchSupport::Live,
        effort_switch: SwitchSupport::None,
        resume: ResumeSupport::Warm,
        fork_history: ForkHistory::Rebuild,
        interrupt: true,
        steering: false,
        subagents: Subagents::Native,
        usage_reporting: UsageReporting::TokensAndCost,
        // Compaction-Durchreichung folgt mit HAR-022.
        compaction: CompactionSupport::None,
        instructions_delivery: InstructionsDelivery::AppendSystemPrompt,
        mcp_injection: true,
        images: true,
        transcript_import: false,
    }
}

fn harness_id() -> HarnessId {
    HarnessId::CLAUDE.parse().unwrap_or_else(|_| unreachable!())
}

/// Kommandozeile für eine Session (HAR-004, gegen 2.1.285 verifiziert).
pub fn command_args(spec: &SessionSpec, isolated: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(model) = &spec.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(resume) = &spec.resume {
        args.extend(["--resume".into(), resume.clone()]);
    }
    if isolated {
        args.extend(["--safe-mode".into(), "--strict-mcp-config".into()]);
    }
    args
}

#[async_trait]
impl HarnessAdapter for ClaudeAdapter {
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
        let Some(bin) = resolve_binary(&harness_id(), "claude", env) else {
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
            // Der Login-Status kommt erst beim Start (harness.auth_required); Token-Dateien
            // liest beton nie (HAR-015).
            auth_status: AuthStatus::Unknown,
            probe_failed: failed,
        }
    }

    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        // Installiert-Prüfung macht `Registry::start` über den Probe; ohne Auflösung bleibt
        // der Name, damit Replays (Golden-Tests) ohne CLI laufen.
        let (program, mut args) = match resolve_binary(&harness_id(), "claude", &ctx.env) {
            Some(bin) => (bin.program, bin.args),
            None => ("claude".into(), Vec::new()),
        };
        args.extend(command_args(&spec, self.isolated));
        let launch = LaunchSpec {
            program,
            args,
            env: Vec::new(),
            env_remove: if self.auth == AuthSource::VendorCli {
                SUBSCRIPTION_ENV_REMOVE
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            } else {
                Vec::new()
            },
            clear_env: false,
            cwd: Some(spec.workdir.clone()),
        };
        let mut process = ctx.launcher.launch(launch).await?;
        let io = process.take_io().ok_or(HarnessError::Closed)?;
        let stdin: Writer = Arc::new(Mutex::new(Some(io.stdin)));
        let state = Arc::new(Mutex::new(MapState {
            auth_source: Some(self.auth),
            ..MapState::default()
        }));
        let (tx, rx) = mpsc::channel(4096);
        let process = Arc::new(Mutex::new(process));
        let closing = Arc::new(std::sync::atomic::AtomicBool::new(false));

        write_json(
            &stdin,
            &json!({"type": "control_request", "request_id": "beton_init", "request": {"subtype": "initialize"}}),
        )
        .await?;

        tokio::spawn(read_loop(
            io.stdout,
            Reader {
                stdin: stdin.clone(),
                state: state.clone(),
                tx: tx.clone(),
                gate: ctx.gate.clone(),
                gate_timeout: self.gate_timeout,
                process: process.clone(),
                closing: closing.clone(),
            },
        ));
        Ok(Box::new(ClaudeSession {
            stdin,
            state,
            tx,
            rx: Some(rx),
            process,
            closing,
            requests: 0,
        }))
    }
}

type Writer = Arc<Mutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>>;

async fn write_json(stdin: &Writer, v: &Value) -> Result<(), HarnessError> {
    let mut guard = stdin.lock().await;
    let w = guard.as_mut().ok_or(HarnessError::Closed)?;
    w.write_all(format!("{v}\n").as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

struct Reader {
    stdin: Writer,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    gate: Arc<dyn Gate>,
    gate_timeout: Duration,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<std::sync::atomic::AtomicBool>,
}

async fn read_loop(stdout: Box<dyn tokio::io::AsyncRead + Send + Unpin>, r: Reader) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let raw = RawJson::from_string(line.clone()).ok();
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!("claude: ungültige stdout-Zeile");
            send(&r.tx, unmapped(&Value::String(line)), None, None).await;
            continue;
        };
        match v["type"].as_str() {
            Some("control_request") => handle_control(&r, &v, raw).await,
            Some("control_response") => {}
            _ => {
                let (events, turn) = {
                    let mut st = r.state.lock().await;
                    let turn = st.turn;
                    (map_line(&v, &mut st), turn)
                };
                if events
                    .iter()
                    .any(|e| matches!(e, EventPayload::HarnessUnmapped(_)))
                {
                    tracing::warn!(kind = ?v["type"], "claude: unbekannte Nachricht");
                }
                let mut raw = raw;
                for e in events {
                    // `raw` hängt am ersten Event der Zeile (HAR-001 AC3).
                    send(&r.tx, e, raw.take(), turn).await;
                }
            }
        }
    }
    // stdout zu: Prozess endet. Unerwartet → harness.exited (HAR-001 AC1).
    if !r.closing.load(std::sync::atomic::Ordering::SeqCst) {
        let exit = r.process.lock().await.wait().await.unwrap_or_default();
        send(
            &r.tx,
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

/// Permission-Bridge (HAR-005): `can_use_tool` → Gate → `control_response`.
async fn handle_control(r: &Reader, v: &Value, raw: Option<RawJson>) {
    let req = &v["request"];
    let request_id = v["request_id"].clone();
    let turn = r.state.lock().await.turn;
    if req["subtype"] != "can_use_tool" {
        send(&r.tx, unmapped(v), raw, turn).await;
        let _ = write_json(
            &r.stdin,
            &json!({"type": "control_response", "response": {"subtype": "error", "request_id": request_id, "error": "nicht unterstützt"}}),
        )
        .await;
        return;
    }
    let tool = req["tool_name"].as_str().unwrap_or_default().to_owned();
    let input = req["input"].clone();
    let call_id = req["tool_use_id"].as_str().unwrap_or_default().to_owned();
    let approval_id = ApprovalId::new();
    let expires = time::OffsetDateTime::now_utc() + r.gate_timeout;
    send(
        &r.tx,
        EventPayload::ApprovalRequested(ApprovalRequested {
            approval_id,
            kind: ApprovalKind::Tool,
            subject: json!({"tool": tool, "args": input, "call_id": call_id}),
            options: vec!["allow".into(), "deny".into()],
            expires_at: Timestamp::from(expires),
            on_timeout: TimeoutAction::Deny,
        }),
        raw,
        turn,
    )
    .await;
    let decision = tokio::time::timeout(
        r.gate_timeout,
        r.gate.decide(GateRequest {
            turn_id: turn,
            call_id: call_id.clone(),
            tool: tool.clone(),
            kind: tool_kind(&tool).into(),
            args: input.clone(),
        }),
    )
    .await;
    let (response, decision, via, modified) = match decision {
        Ok(GateDecision::Allow { updated_args }) => {
            let args = updated_args.clone().unwrap_or_else(|| input.clone());
            (
                json!({"behavior": "allow", "updatedInput": args}),
                ApprovalDecision::Allow,
                ResolvedVia::User,
                updated_args,
            )
        }
        Ok(GateDecision::Deny { reason }) => (
            json!({"behavior": "deny", "message": reason.unwrap_or_else(|| "Von beton abgelehnt.".into())}),
            ApprovalDecision::Deny,
            ResolvedVia::User,
            None,
        ),
        // Fail closed (HAR-005 AC4).
        Err(_) => (
            json!({"behavior": "deny", "message": "Keine Entscheidung erhalten; aus Sicherheitsgründen abgelehnt."}),
            ApprovalDecision::Deny,
            ResolvedVia::Timeout,
            None,
        ),
    };
    if decision == ApprovalDecision::Deny {
        r.state.lock().await.denied.insert(call_id.clone());
    }
    let _ = write_json(
        &r.stdin,
        &json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}),
    )
    .await;
    send(
        &r.tx,
        EventPayload::ApprovalResolved(ApprovalResolved {
            approval_id,
            decision,
            answer: None,
            actor: if via == ResolvedVia::Timeout {
                Actor::System {
                    component: beton_core::event::SystemComponent::Runner,
                }
            } else {
                Actor::User {
                    id: PrincipalId::User(UserId::LOCAL),
                    device_id: None,
                }
            },
            via,
            remember: None,
            comment: None,
            on_timeout_applied: (via == ResolvedVia::Timeout).then_some(TimeoutAction::Deny),
        }),
        None,
        turn,
    )
    .await;
    if decision == ApprovalDecision::Allow {
        send(
            &r.tx,
            EventPayload::ToolCallStarted(ToolCallStarted {
                call_id,
                sandbox_stage: None,
                args: modified,
            }),
            None,
            turn,
        )
        .await;
    }
}

/// Kanonische Tool-Klasse (POL-005) für die bekannten Claude-Tools.
pub fn tool_kind(tool: &str) -> &'static str {
    match tool {
        "Bash" | "BashOutput" | "KillShell" => "shell",
        "Read" | "NotebookRead" => "file_read",
        "Write" => "file_write",
        "Edit" | "MultiEdit" | "NotebookEdit" => "file_edit",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "web_fetch",
        t if t.starts_with("mcp__") => "mcp",
        _ => "other",
    }
}

/// Eine laufende Claude-Code-Session.
pub struct ClaudeSession {
    stdin: Writer,
    state: Arc<Mutex<MapState>>,
    tx: mpsc::Sender<NormalizedEvent>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    process: Arc<Mutex<Box<dyn ProcessHandle>>>,
    closing: Arc<std::sync::atomic::AtomicBool>,
    requests: u64,
}

impl ClaudeSession {
    async fn control(&mut self, request: Value) -> Result<(), HarnessError> {
        self.requests += 1;
        write_json(
            &self.stdin,
            &json!({"type": "control_request", "request_id": format!("beton_{}", self.requests), "request": request}),
        )
        .await
    }
}

#[async_trait]
impl HarnessSession for ClaudeSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let turn = TurnId::new();
        self.state.lock().await.turn = Some(turn);
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
        write_json(
            &self.stdin,
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": input.text}]}}),
        )
        .await?;
        Ok(turn)
    }

    async fn steer(&mut self, _input: UserInput) -> Result<(), HarnessError> {
        capabilities().check(Action::Steer)?;
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        self.state.lock().await.interrupt_requested = true;
        self.control(json!({"subtype": "interrupt"})).await
    }

    async fn set_model(
        &mut self,
        model: String,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        if effort.is_some() {
            capabilities().check(Action::EffortSwitch)?;
        }
        self.control(json!({"subtype": "set_model", "model": model}))
            .await?;
        let _ = self
            .tx
            .send(NormalizedEvent::new(
                EventPayload::SessionSettingsChanged(SessionSettingsChanged {
                    model: Some(model),
                    mechanism: Some(SettingsMechanism::Live),
                    ..SessionSettingsChanged::default()
                }),
                None,
            ))
            .await;
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        let mode = match mode {
            PermissionMode::Plan => "plan",
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Yolo => "bypassPermissions",
        };
        self.control(json!({"subtype": "set_permission_mode", "mode": mode}))
            .await
    }

    async fn compact(&mut self) -> Result<(), HarnessError> {
        capabilities().check(Action::Compact)?;
        Ok(())
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        self.state
            .try_lock()
            .ok()
            .and_then(|s| s.session_ref.clone())
    }

    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError> {
        self.closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // stdin schließen ist das protokolleigene Ende.
        self.stdin.lock().await.take();
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
