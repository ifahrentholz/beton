//! Fake-Harness `fake` (HAR-026): spielt Szenarien deterministisch im Prozess ab.
//!
//! Alle IDs (Turns, Nachrichten, Tool-Calls) werden aus Zählern abgeleitet, damit dasselbe
//! Szenario mit derselben Eingabe immer dieselbe Event-Folge erzeugt. Nur im Entwicklermodus
//! registriert (siehe [`crate::registry::RegistryOptions`]).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    Compaction, CostDelta, CostSource, EventPayload, HarnessAuthRequired, HarnessExited,
    HarnessReady, MessageCompleted, MessageRole, ReasoningCompleted, ResolvedVia,
    SessionSettingsChanged, SettingsMechanism, TextDelta, TimeoutAction, ToolCallCompleted,
    ToolCallRequested, ToolCallStarted, ToolSource, ToolStatus, TurnCompleted, TurnFailed,
    TurnInterrupted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;

use crate::adapter::{
    AdapterContext, AuthStatus, ExitInfo, Gate, GateDecision, GateRequest, HarnessAdapter,
    HarnessError, HarnessSession, HostEnv, Mode, NormalizedEvent, PermissionMode, ProbeReport,
    SessionSpec, Shutdown, SwitchOutcome, Transport, UserInput,
};
use crate::capabilities::{
    Action, ApprovalMechanism, Capabilities, CompactionSupport, ForkHistory, InstructionsDelivery,
    ResumeSupport, Subagents, SwitchSupport, ToolCallGate, UsageReporting,
};
use crate::id::HarnessId;
use crate::scenario::{Scenario, StartBehavior, Step};

/// Kennung, unter der die Fake-Session sich beim Harness meldet.
pub const FAKE_SESSION_REF: &str = "fake-session";

/// Der Fake-Harness.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeAdapter;

/// Capabilities des Fake-Harness ohne Szenario-Überschreibungen: alles verfügbar.
pub fn default_capabilities() -> Capabilities {
    Capabilities {
        mode: Mode::Native,
        transport: Transport::InProc,
        version_range: None,
        auth_sources: vec![AuthSource::None],
        approval: ApprovalMechanism::NativeRequest,
        tool_call_gate: ToolCallGate::Full,
        model_switch: SwitchSupport::Live,
        effort_switch: SwitchSupport::Live,
        resume: ResumeSupport::Warm,
        fork_history: ForkHistory::Preamble,
        interrupt: true,
        steering: true,
        subagents: Subagents::None,
        usage_reporting: UsageReporting::TokensAndCost,
        compaction: CompactionSupport::Native,
        instructions_delivery: InstructionsDelivery::FirstMessagePrefix,
        mcp_injection: false,
        images: false,
        transcript_import: false,
        models: vec!["fake-small".into(), "fake-large".into()],
        models_stale: false,
        efforts: vec!["low".into(), "medium".into(), "high".into()],
        context_window: Some(crate::capabilities::DEFAULT_CONTEXT_WINDOW),
    }
}

/// Capabilities eines Szenarios: Default plus Überschreibungen aus `capabilities:`.
pub fn scenario_capabilities(scenario: &Scenario) -> Result<Capabilities, HarnessError> {
    let mut value = serde_json::to_value(default_capabilities())
        .map_err(|e| HarnessError::Protocol(e.to_string()))?;
    if let Some(obj) = value.as_object_mut() {
        for (k, v) in &scenario.capabilities {
            obj.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(value)
        .map_err(|e| HarnessError::StartRefused(format!("Szenario-Capabilities ungültig: {e}")))
}

#[async_trait]
impl HarnessAdapter for FakeAdapter {
    fn id(&self) -> HarnessId {
        HarnessId::FAKE.parse().unwrap_or_else(|_| unreachable!())
    }

    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }

    fn capabilities(&self, _mode: Mode, _probe: &ProbeReport) -> Capabilities {
        default_capabilities()
    }

    async fn probe(&self, _env: &HostEnv) -> ProbeReport {
        ProbeReport {
            installed: true,
            path: None,
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            auth_status: AuthStatus::NotApplicable,
            probe_failed: None,
        }
    }

    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let path = spec.scenario.ok_or_else(|| {
            HarnessError::StartRefused("der Fake-Harness braucht --scenario <datei>".into())
        })?;
        let scenario =
            Scenario::load(&path).map_err(|e| HarnessError::StartRefused(e.to_string()))?;
        let mut session = FakeSession::start_with_ref(scenario, spec.model, ctx.gate, spec.resume)?;
        session.workdir = spec.workdir;
        Ok(Box::new(session) as Box<dyn HarnessSession>)
    }
}

/// Eine laufende Fake-Session.
pub struct FakeSession {
    scenario: Arc<Scenario>,
    capabilities: Capabilities,
    model: String,
    gate: Arc<dyn Gate>,
    tx: Option<mpsc::Sender<NormalizedEvent>>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    next_turn: usize,
    running: Option<JoinHandle<()>>,
    interrupt: Arc<Notify>,
    crashed: Arc<Mutex<bool>>,
    session_ref: String,
    /// Steer-Eingaben in den laufenden Turn (Schritt `await_steer`, SES-004).
    steer: Option<mpsc::UnboundedSender<String>>,
    /// Arbeitsverzeichnis für `write_file`-Schritte.
    workdir: std::path::PathBuf,
}

impl std::fmt::Debug for FakeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSession")
            .field("next_turn", &self.next_turn)
            .finish_non_exhaustive()
    }
}

/// Deterministische Turn-ID: `trn_000…0<n>`.
fn turn_id(n: usize) -> TurnId {
    TurnId::from_ulid(ulid::Ulid(n as u128 + 1))
}

fn local_user() -> PrincipalId {
    PrincipalId::User(UserId::LOCAL)
}

impl FakeSession {
    /// Startet eine Session direkt aus einem Szenario (ohne Datei).
    pub fn start(
        scenario: Scenario,
        model: Option<String>,
        gate: Arc<dyn Gate>,
    ) -> Result<Self, HarnessError> {
        Self::start_with_ref(scenario, model, gate, None)
    }

    /// Wie [`Self::start`]; mit `resume` meldet sich der Fake unter dieser Referenz (SES-003).
    pub fn start_with_ref(
        scenario: Scenario,
        model: Option<String>,
        gate: Arc<dyn Gate>,
        resume: Option<String>,
    ) -> Result<Self, HarnessError> {
        let session_ref = resume.unwrap_or_else(|| FAKE_SESSION_REF.into());
        if let Some(StartBehavior {
            refuse: Some(reason),
        }) = &scenario.start
        {
            return Err(HarnessError::StartRefused(reason.clone()));
        }
        let capabilities = scenario_capabilities(&scenario)?;
        let (tx, rx) = mpsc::channel(4096);
        tx.try_send(NormalizedEvent::new(
            EventPayload::HarnessReady(HarnessReady {
                harness_session_ref: Some(session_ref.clone()),
                tools: vec!["Bash".into(), "Read".into(), "Edit".into()],
                mcp_servers: Vec::new(),
            }),
            None,
        ))
        .map_err(|_| HarnessError::Closed)?;
        Ok(Self {
            scenario: Arc::new(scenario),
            capabilities,
            model: model.unwrap_or_else(|| "fake-model".into()),
            gate,
            tx: Some(tx),
            rx: Some(rx),
            next_turn: 0,
            running: None,
            interrupt: Arc::new(Notify::new()),
            crashed: Arc::new(Mutex::new(false)),
            workdir: std::env::current_dir().unwrap_or_default(),
            session_ref,
            steer: None,
        })
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn sender(&self) -> Result<mpsc::Sender<NormalizedEvent>, HarnessError> {
        self.tx.clone().ok_or(HarnessError::Closed)
    }

    async fn emit(&self, payload: EventPayload) -> Result<(), HarnessError> {
        self.sender()?
            .send(NormalizedEvent::new(payload, None))
            .await
            .map_err(|_| HarnessError::Closed)
    }
}

#[async_trait]
impl HarnessSession for FakeSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        if *self.crashed.lock().await {
            return Err(HarnessError::Closed);
        }
        if self.running.as_ref().is_some_and(|h| !h.is_finished()) {
            return Err(HarnessError::Protocol("es läuft bereits ein Turn".into()));
        }
        let index = self.next_turn;
        let turn = self.scenario.turns.get(index).cloned().ok_or_else(|| {
            HarnessError::UnexpectedInput {
                expected: "<Szenario-Ende>".into(),
                got: input.text.clone(),
            }
        })?;
        if let Some(expected) = &turn.expect_input
            && *expected != input.text
        {
            return Err(HarnessError::UnexpectedInput {
                expected: expected.clone(),
                got: input.text,
            });
        }
        self.next_turn += 1;
        let id = turn_id(index);
        let (steer_tx, steer_rx) = mpsc::unbounded_channel();
        self.steer = Some(steer_tx);
        let player = Player {
            tx: self.sender()?,
            gate: self.gate.clone(),
            interrupt: self.interrupt.clone(),
            crashed: self.crashed.clone(),
            turn: id,
            turn_index: index,
            model: self.model.clone(),
            messages: 0,
            calls: 0,
            text: String::new(),
            last_call: None,
            last_decision: None,
            steer: steer_rx,
            workdir: self.workdir.clone(),
            input: input.text,
        };
        self.running = Some(tokio::spawn(player.play(turn.emit)));
        Ok(id)
    }

    async fn steer(&mut self, input: UserInput) -> Result<(), HarnessError> {
        self.capabilities.check(Action::Steer)?;
        let running = self.running.as_ref().is_some_and(|h| !h.is_finished());
        match &self.steer {
            Some(tx) if running => tx.send(input.text).map_err(|_| HarnessError::Closed),
            _ => Err(HarnessError::Protocol("kein laufender Turn".into())),
        }
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        self.capabilities.check(Action::Interrupt)?;
        self.interrupt.notify_waiters();
        if let Some(running) = self.running.take() {
            let _ = running.await;
        }
        Ok(())
    }

    async fn set_model(
        &mut self,
        model: String,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        self.capabilities.check(Action::ModelSwitch)?;
        let live = self.capabilities.model_switch == SwitchSupport::Live;
        self.model.clone_from(&model);
        self.emit(EventPayload::SessionSettingsChanged(
            SessionSettingsChanged {
                model: Some(model),
                effort,
                mechanism: Some(if live {
                    SettingsMechanism::Live
                } else {
                    SettingsMechanism::Restart
                }),
                ..SessionSettingsChanged::default()
            },
        ))
        .await?;
        Ok(if live {
            SwitchOutcome::Live
        } else {
            SwitchOutcome::Restarted
        })
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError> {
        let name = match mode {
            PermissionMode::Plan => "plan",
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Yolo => "yolo",
        };
        self.emit(EventPayload::SessionSettingsChanged(
            SessionSettingsChanged {
                permission_mode: Some(name.into()),
                mechanism: Some(SettingsMechanism::Live),
                ..SessionSettingsChanged::default()
            },
        ))
        .await
    }

    async fn compact(&mut self) -> Result<(), HarnessError> {
        self.capabilities.check(Action::Compact)?;
        self.emit(EventPayload::CompactionStarted(Compaction {
            before_tokens: 1000,
            after_tokens: None,
        }))
        .await?;
        self.emit(EventPayload::CompactionCompleted(Compaction {
            before_tokens: 1000,
            after_tokens: Some(200),
        }))
        .await
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        Some(self.session_ref.clone())
    }

    fn capabilities(&self) -> Option<Capabilities> {
        Some(self.capabilities.clone())
    }

    async fn shutdown(mut self: Box<Self>, _how: Shutdown) -> Result<ExitInfo, HarnessError> {
        if let Some(running) = self.running.take() {
            running.abort();
        }
        self.tx = None;
        Ok(ExitInfo {
            code: Some(0),
            ..ExitInfo::default()
        })
    }
}

/// Spielt die Schritte eines Turns ab.
struct Player {
    tx: mpsc::Sender<NormalizedEvent>,
    gate: Arc<dyn Gate>,
    interrupt: Arc<Notify>,
    crashed: Arc<Mutex<bool>>,
    turn: TurnId,
    turn_index: usize,
    model: String,
    messages: usize,
    calls: usize,
    /// Bisher gestreamter Text der aktuellen Nachricht.
    text: String,
    last_call: Option<String>,
    last_decision: Option<bool>,
    steer: mpsc::UnboundedReceiver<String>,
    workdir: std::path::PathBuf,
    /// Eingabe dieses Turns (für `echo_input`).
    input: String,
}

enum Outcome {
    Continue,
    /// Turn endet (Fehler, Absturz, Unterbrechung) – Events sind schon gesendet.
    Stop,
}

impl Player {
    async fn send(&self, payload: EventPayload) {
        let _ = self
            .tx
            .send(NormalizedEvent::new(payload, Some(self.turn)))
            .await;
    }

    fn message_id(&self) -> String {
        format!("msg_{}_{}", self.turn_index + 1, self.messages + 1)
    }

    async fn play(mut self, steps: Vec<Step>) {
        self.send(EventPayload::TurnStarted(TurnStarted {
            turn_id: self.turn,
            input_id: None,
            author: local_user(),
        }))
        .await;
        let interrupt = self.interrupt.clone();
        let finished = tokio::select! {
            outcome = self.steps(steps) => matches!(outcome, Outcome::Continue),
            () = interrupt.notified() => {
                self.flush_message().await;
                self.send(EventPayload::TurnInterrupted(TurnInterrupted {
                    turn_id: self.turn,
                    by: local_user(),
                }))
                .await;
                false
            }
        };
        if finished {
            self.flush_message().await;
            self.send(EventPayload::TurnCompleted(TurnCompleted {
                turn_id: self.turn,
                stop_reason: "end_turn".into(),
                usage_summary: json!({}),
            }))
            .await;
        }
    }

    fn steps(&mut self, steps: Vec<Step>) -> futures_lite::BoxedSteps<'_> {
        Box::pin(async move {
            for step in steps {
                if let Outcome::Stop = self.step(step).await {
                    return Outcome::Stop;
                }
            }
            Outcome::Continue
        })
    }

    /// Schließt eine gestreamte Nachricht mit `message.completed` ab.
    async fn flush_message(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.text);
        self.send(EventPayload::MessageCompleted(MessageCompleted {
            message_id: self.message_id(),
            role: MessageRole::Assistant,
            content: vec![json!({"type": "text", "text": text})],
            author: None,
        }))
        .await;
        self.messages += 1;
    }

    async fn step(&mut self, step: Step) -> Outcome {
        if let Some(ms) = step.delay_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        if step.message_delta.is_none() {
            self.flush_message().await;
        }
        if let Some(text) = step.message_delta {
            let chunk = step.chunk.unwrap_or(usize::MAX);
            let chars: Vec<char> = text.chars().collect();
            for (i, piece) in chars.chunks(chunk.min(chars.len().max(1))).enumerate() {
                if i > 0
                    && let Some(ms) = step.chunk_delay_ms
                {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                let piece: String = piece.iter().collect();
                self.text.push_str(&piece);
                self.send(EventPayload::MessageDelta(TextDelta {
                    message_id: self.message_id(),
                    text: piece,
                    snapshot: false,
                }))
                .await;
            }
        } else if let Some(text) = step.message {
            self.text = text;
            self.flush_message().await;
        } else if let Some(text) = step.reasoning {
            self.send(EventPayload::ReasoningCompleted(ReasoningCompleted {
                message_id: format!("rsn_{}_{}", self.turn_index + 1, self.messages + 1),
                summary: Some(text),
                redacted: false,
            }))
            .await;
        } else if let Some(call) = step.tool_call {
            self.calls += 1;
            let call_id = format!("call_{}_{}", self.turn_index + 1, self.calls);
            self.send(EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: call_id.clone(),
                tool: call.name.clone(),
                mcp_server: None,
                args: call.args.clone(),
                source: ToolSource::Harness,
                parent_call_id: None,
            }))
            .await;
            let allowed = if step.gate {
                // Wie die echten Adapter: Anfrage und Entscheidung als Events (HAR-005).
                let approval_id = ApprovalId::from_ulid(ulid::Ulid(
                    (self.turn_index as u128 + 1) * 1000 + self.calls as u128,
                ));
                self.send(EventPayload::ApprovalRequested(ApprovalRequested {
                    approval_id,
                    kind: ApprovalKind::Tool,
                    subject: json!({"tool": call.name, "args": call.args, "call_id": call_id}),
                    options: vec!["allow".into(), "deny".into()],
                    expires_at: Timestamp::default(),
                    on_timeout: TimeoutAction::Deny,
                }))
                .await;
                let decision = self
                    .gate
                    .decide(GateRequest {
                        turn_id: Some(self.turn),
                        call_id: call_id.clone(),
                        tool: call.name,
                        kind: call.kind,
                        args: call.args,
                    })
                    .await;
                let allowed = matches!(decision, GateDecision::Allow { .. });
                let comment = match &decision {
                    GateDecision::Deny { reason } => reason.clone(),
                    GateDecision::Allow { .. } => None,
                };
                self.send(EventPayload::ApprovalResolved(ApprovalResolved {
                    approval_id,
                    decision: if allowed {
                        ApprovalDecision::Allow
                    } else {
                        ApprovalDecision::Deny
                    },
                    answer: None,
                    actor: Actor::User {
                        id: local_user(),
                        device_id: None,
                    },
                    via: ResolvedVia::User,
                    remember: None,
                    comment,
                    on_timeout_applied: None,
                }))
                .await;
                allowed
            } else {
                true
            };
            if !allowed {
                self.send(EventPayload::ToolCallCompleted(ToolCallCompleted {
                    call_id: call_id.clone(),
                    status: ToolStatus::Denied,
                    result: None,
                    result_ref: None,
                    duration_ms: 0,
                }))
                .await;
            }
            self.last_call = Some(call_id);
            self.last_decision = Some(allowed);
        } else if let Some(call) = step.mcp_call {
            // Der Fake-Harness hat keine MCP-Injektion (Capability `mcp_injection: false`);
            // echte MCP-Aufrufe spielt die Fake-CLI ab (HAR-009).
            self.calls += 1;
            let call_id = format!("call_{}_{}", self.turn_index + 1, self.calls);
            self.send(EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: call_id.clone(),
                tool: call.tool,
                mcp_server: Some(call.server),
                args: call.args,
                source: ToolSource::Harness,
                parent_call_id: None,
            }))
            .await;
            self.last_call = Some(call_id);
            self.complete_call(
                ToolStatus::Error,
                Value::String("mcp_injection: der Fake-Harness hat keine MCP-Server".into()),
            )
            .await;
        } else if let Some(on_gate) = step.on_gate {
            let branch = if self.last_decision.unwrap_or(true) {
                on_gate.allow
            } else {
                on_gate.deny
            };
            if let Outcome::Stop = self.steps(branch).await {
                return Outcome::Stop;
            }
        } else if let Some(result) = step.tool_result {
            self.complete_call(ToolStatus::Ok, result).await;
        } else if let Some(error) = step.tool_error {
            self.complete_call(ToolStatus::Error, Value::String(error))
                .await;
        } else if let Some(usage) = step.usage {
            self.send(EventPayload::CostDelta(CostDelta {
                harness: HarnessId::FAKE.into(),
                model: usage.model.unwrap_or_else(|| self.model.clone()),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_read_tokens: usage.cache_read_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                cost_micro: usage.cost_usd.map(|usd| (usd * 1_000_000.0).round() as i64),
                currency: "USD".into(),
                source: CostSource::Reported,
                auth_source: AuthSource::None,
                purpose: None,
            }))
            .await;
        } else if let Some(message) = step.error {
            self.send(EventPayload::TurnFailed(TurnFailed {
                turn_id: self.turn,
                problem: json!({"type": "harness_error", "title": message}),
            }))
            .await;
            return Outcome::Stop;
        } else if let Some(code) = step.crash {
            *self.crashed.lock().await = true;
            self.send(EventPayload::HarnessExited(HarnessExited {
                code: Some(code),
                signal: None,
                stderr_tail: format!("fake: Absturz laut Szenario (Exit-Code {code})\n"),
            }))
            .await;
            return Outcome::Stop;
        } else if let Some(hint) = step.auth_expired {
            self.send(EventPayload::HarnessAuthRequired(HarnessAuthRequired {
                harness: HarnessId::FAKE.into(),
                hint,
            }))
            .await;
            self.send(EventPayload::TurnFailed(TurnFailed {
                turn_id: self.turn,
                problem: json!({"type": "auth_required"}),
            }))
            .await;
            return Outcome::Stop;
        } else if step.hang {
            std::future::pending::<()>().await;
        } else if let Some(expected) = step.await_steer {
            // SES-004 AC3: Die Eingabe fließt in den laufenden Turn, kein neuer Turn.
            let got = self.steer.recv().await.unwrap_or_default();
            if !expected.is_empty() && got != expected {
                self.send(EventPayload::TurnFailed(TurnFailed {
                    turn_id: self.turn,
                    problem: json!({"type": "unexpected_input", "title": format!("Steer: erwartet `{expected}`, erhalten `{got}`")}),
                }))
                .await;
                return Outcome::Stop;
            }
        } else if step.echo_input {
            self.text = self.input.clone();
            self.flush_message().await;
        } else if step.echo_history {
            // Der Fake-Harness hat keinen nativen Verlauf (Capability `fork_history: preamble`).
            self.text = String::from("(kein Verlauf)");
            self.flush_message().await;
        } else if let Some(write) = step.write_file {
            let workdir = self.workdir.clone();
            let _ = tokio::task::spawn_blocking(move || write.apply(&workdir)).await;
        }
        Outcome::Continue
    }

    async fn complete_call(&mut self, status: ToolStatus, result: Value) {
        let Some(call_id) = self.last_call.take() else {
            return;
        };
        self.send(EventPayload::ToolCallStarted(ToolCallStarted {
            call_id: call_id.clone(),
            sandbox_stage: None,
            args: None,
        }))
        .await;
        self.send(EventPayload::ToolCallCompleted(ToolCallCompleted {
            call_id,
            status,
            result: Some(result),
            result_ref: None,
            duration_ms: 0,
        }))
        .await;
    }
}

mod futures_lite {
    use std::future::Future;
    use std::pin::Pin;

    /// Rekursive Schrittfolge (`on_gate` enthält wieder Schritte).
    pub(super) type BoxedSteps<'a> = Pin<Box<dyn Future<Output = super::Outcome> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AllowAll, DenyAll};
    use crate::registry::{Registry, RegistryOptions};

    const PUSH_ASK: &str = r#"
capabilities: { approval: native_request, model_switch: live, fork_history: preamble }
turns:
  - expect_input: "Bitte pushen"
    emit:
      - { message_delta: "Ich pushe jetzt.", chunk: 4, delay_ms: 10 }
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "ok" }], deny: [{ message: "Push abgelehnt." }] } }
      - { usage: { input_tokens: 1200, output_tokens: 80, cost_usd: 0.01 } }
"#;

    async fn run(yaml: &str, gate: Arc<dyn Gate>, inputs: &[&str]) -> Vec<NormalizedEvent> {
        let mut s = FakeSession::start(Scenario::from_yaml(yaml).unwrap(), None, gate).unwrap();
        let mut rx = s.events().unwrap();
        for input in inputs {
            s.send((*input).into()).await.unwrap();
            if let Some(h) = s.running.take() {
                h.await.unwrap();
            }
        }
        Box::new(s).shutdown(Shutdown::Kill).await.unwrap();
        let mut out = Vec::new();
        while let Some(e) = rx.recv().await {
            out.push(e);
        }
        out
    }

    fn types(events: &[NormalizedEvent]) -> Vec<&'static str> {
        events.iter().map(|e| e.payload.type_name()).collect()
    }

    #[tokio::test]
    async fn har_026_ac1_same_scenario_and_input_give_identical_events() {
        let a = run(PUSH_ASK, Arc::new(AllowAll), &["Bitte pushen"]).await;
        let b = run(PUSH_ASK, Arc::new(AllowAll), &["Bitte pushen"]).await;
        assert_eq!(a, b);
        assert_eq!(
            types(&a),
            [
                "harness.ready",
                "turn.started",
                "message.delta",
                "message.delta",
                "message.delta",
                "message.delta",
                "message.completed",
                "tool.call.requested",
                "approval.requested",
                "approval.resolved",
                "tool.call.started",
                "tool.call.completed",
                "cost.delta",
                "turn.completed",
            ]
        );
        let EventPayload::CostDelta(cost) = &a[12].payload else {
            panic!()
        };
        assert_eq!(cost.cost_micro, Some(10_000));
        assert_eq!(
            a[1].turn_id.unwrap().to_string(),
            "trn_00000000000000000000000001"
        );
    }

    #[tokio::test]
    async fn har_026_gate_deny_follows_deny_branch() {
        let events = run(PUSH_ASK, Arc::new(DenyAll), &["Bitte pushen"]).await;
        let completed = events
            .iter()
            .find_map(|e| match &e.payload {
                EventPayload::ToolCallCompleted(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert_eq!(completed.status, ToolStatus::Denied);
        assert!(events.iter().any(|e| matches!(&e.payload,
            EventPayload::MessageCompleted(m) if m.content[0]["text"] == "Push abgelehnt.")));
    }

    #[tokio::test]
    async fn har_026_ac2_start_can_be_refused() {
        let s = Scenario::from_yaml("start: { refuse: \"Policy nicht durchsetzbar\" }").unwrap();
        let err = FakeSession::start(s, None, Arc::new(AllowAll)).unwrap_err();
        assert_eq!(err.code(), "start_refused");
    }

    #[tokio::test]
    async fn har_026_ac2_capability_unsupported_is_reported() {
        let s =
            Scenario::from_yaml("capabilities: { model_switch: none, interrupt: false }").unwrap();
        let mut s = FakeSession::start(s, None, Arc::new(AllowAll)).unwrap();
        let err = s.set_model("opus".into(), None).await.unwrap_err();
        assert_eq!(err.code(), "capability_unsupported");
        assert_eq!(
            s.interrupt().await.unwrap_err().code(),
            "capability_unsupported"
        );
    }

    #[tokio::test]
    async fn har_026_ac2_crash_and_auth_expiry_are_testable() {
        let crash = "turns: [{ emit: [{ message: Hallo }, { crash: 137 }] }, { emit: [] }]";
        let mut s = FakeSession::start(
            Scenario::from_yaml(crash).unwrap(),
            None,
            Arc::new(AllowAll),
        )
        .unwrap();
        let mut rx = s.events().unwrap();
        s.send("x".into()).await.unwrap();
        s.running.take().unwrap().await.unwrap();
        assert_eq!(
            s.send("y".into()).await.unwrap_err().code(),
            "session_closed"
        );
        drop(s);
        let mut exited = None;
        while let Some(e) = rx.recv().await {
            if let EventPayload::HarnessExited(x) = e.payload {
                exited = Some(x);
            }
        }
        let exited = exited.unwrap();
        assert_eq!(exited.code, Some(137));
        assert!(!exited.stderr_tail.is_empty());

        let auth = run(
            "turns: [{ emit: [{ auth_expired: \"claude login ausführen\" }] }]",
            Arc::new(AllowAll),
            &["x"],
        )
        .await;
        assert_eq!(
            types(&auth),
            [
                "harness.ready",
                "turn.started",
                "harness.auth_required",
                "turn.failed"
            ]
        );
    }

    #[tokio::test]
    async fn interrupt_ends_a_hanging_turn() {
        let yaml = "turns: [{ emit: [{ message_delta: \"Ich denke\" }, { hang: true }] }]";
        let mut s =
            FakeSession::start(Scenario::from_yaml(yaml).unwrap(), None, Arc::new(AllowAll))
                .unwrap();
        let mut rx = s.events().unwrap();
        s.send("los".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        s.interrupt().await.unwrap();
        drop(s);
        let mut seen = Vec::new();
        while let Some(e) = rx.recv().await {
            seen.push(e.payload.type_name());
        }
        assert_eq!(seen.last(), Some(&"turn.interrupted"));
        assert!(seen.contains(&"message.completed"));
    }

    #[tokio::test]
    async fn unexpected_input_is_rejected() {
        let mut s = FakeSession::start(
            Scenario::from_yaml(PUSH_ASK).unwrap(),
            None,
            Arc::new(AllowAll),
        )
        .unwrap();
        let err = s.send("Etwas anderes".into()).await.unwrap_err();
        assert_eq!(err.code(), "unexpected_input");
    }

    #[test]
    fn har_026_ac3_fake_only_in_dev_mode() {
        let fake: HarnessId = "fake".parse().unwrap();
        assert!(
            Registry::new(RegistryOptions { dev: false })
                .get(&fake)
                .is_none()
        );
        assert!(
            Registry::new(RegistryOptions { dev: true })
                .get(&fake)
                .is_some()
        );
    }

    #[tokio::test]
    async fn har_001_ac4_trait_objects_work_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.yaml");
        std::fs::write(&path, PUSH_ASK).unwrap();
        let registry = Registry::new(RegistryOptions { dev: true });
        let ctx = AdapterContext {
            gate: Arc::new(AllowAll),
            launcher: Arc::new(crate::process::RealLauncher),
            env: HostEnv::default(),
        };
        let mut session: Box<dyn HarnessSession> = registry
            .start(
                &"fake".parse().unwrap(),
                SessionSpec {
                    scenario: Some(path),
                    ..SessionSpec::default()
                },
                ctx,
            )
            .await
            .unwrap();
        let mut rx = session.events().unwrap();
        session.send("Bitte pushen".into()).await.unwrap();
        let mut last = None;
        while let Some(e) = rx.recv().await {
            let done = matches!(e.payload, EventPayload::TurnCompleted(_));
            last = Some(e);
            if done {
                break;
            }
        }
        assert!(matches!(
            last.unwrap().payload,
            EventPayload::TurnCompleted(_)
        ));
        session.shutdown(Shutdown::default()).await.unwrap();

        let catalog = registry.catalog(&HostEnv::default()).await;
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id.as_str(), "fake");
        assert!(!catalog[0].incompatible);
    }

    #[tokio::test]
    async fn har_026_chunk_delay_paces_the_stream() {
        let start = std::time::Instant::now();
        let events = run(
            "turns:\n  - emit: [{ message_delta: \"abcde\", chunk: 1, chunk_delay_ms: 25 }]\n",
            Arc::new(AllowAll),
            &["x"],
        )
        .await;
        let deltas = events
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::MessageDelta(_)))
            .count();
        assert_eq!(deltas, 5);
        assert!(
            start.elapsed() >= Duration::from_millis(100),
            "{:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn ses_004_steer_flows_into_running_turn() {
        let yaml = "turns:\n  - emit:\n      - { message: \"Ich lege los.\" }\n      - { await_steer: \"Bitte auch X\" }\n      - { message: \"Mache X mit.\" }\n";
        let mut s =
            FakeSession::start(Scenario::from_yaml(yaml).unwrap(), None, Arc::new(AllowAll))
                .unwrap();
        let mut rx = s.events().unwrap();
        // Ohne laufenden Turn kein Steer.
        assert!(s.steer("zu früh".into()).await.is_err());
        s.send("los".into()).await.unwrap();
        s.steer("Bitte auch X".into()).await.unwrap();
        s.running.take().unwrap().await.unwrap();
        Box::new(s).shutdown(Shutdown::Kill).await.unwrap();
        let mut out = Vec::new();
        while let Some(e) = rx.recv().await {
            out.push(e);
        }
        let t = types(&out);
        assert_eq!(t.iter().filter(|x| **x == "turn.started").count(), 1);
        assert!(!t.contains(&"turn.interrupted"));
        assert_eq!(t.last(), Some(&"turn.completed"));

        let mut s = FakeSession::start(
            Scenario::from_yaml(
                "capabilities: { steering: false }\nturns: [{ emit: [{ hang: true }] }]",
            )
            .unwrap(),
            None,
            Arc::new(AllowAll),
        )
        .unwrap();
        s.send("los".into()).await.unwrap();
        let err = s.steer("x".into()).await.unwrap_err();
        assert_eq!(err.code(), "capability_unsupported");
        Box::new(s).shutdown(Shutdown::Kill).await.unwrap();
    }
}
