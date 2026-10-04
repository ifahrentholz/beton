//! Der Agent-Loop (HAR-010): Model-Request streamen → Tool-Uses einsammeln → je Tool-Call
//! das Gate fragen → über MCP ausführen → Ergebnisse zurück → wiederholen bis `end_turn`
//! oder `max_turns`. Dazu die eigene Compaction (über 80 % Kontext: alte Tool-Ergebnisse
//! kürzen, danach ältere Turns vom selben Modell zusammenfassen lassen).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, AuthSource,
    Compaction, ContextSource, ContextUsage, CostDelta, CostSource, EventPayload,
    HarnessAuthRequired, MessageCompleted, MessageRole, Notice, NoticeLevel, ReasoningCompleted,
    ResolvedVia, SessionSettingsChanged, SettingsMechanism, SystemComponent, TextDelta,
    TimeoutAction, ToolCallCompleted, ToolCallRequested, ToolCallStarted, ToolStatus,
    TurnCompleted, TurnFailed, TurnInterrupted, TurnStarted,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::{
    Action, Capabilities, ExitInfo, Gate, GateDecision, GateRequest, HarnessError, HarnessSession,
    NormalizedEvent, PermissionMode, Shutdown, SwitchOutcome, UserInput,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore, mpsc, watch};

use crate::config::{KeyRef, Provider, WireKind};
use crate::http::{CallError, Client, Retry, StreamSink};
use crate::secret::ApiKey;
use crate::tools::{ToolHost, ToolInfo};
use crate::wire::{
    self, Block, Message, ModelRequest, Role, StopReason, StreamEvent, ToolDef, Usage,
};

/// Präfix eines Tool-Ergebnisses, das beton abgelehnt hat (für das Modell).
pub const DENIED_PREFIX: &str = "Abgelehnt von beton";
/// Präfix eines abgebrochenen Tool-Calls.
pub const CANCELLED_PREFIX: &str = "Abgebrochen";

/// Kontextfenster, wenn `providers.*.models[].context_window` fehlt.
pub fn default_window(wire: WireKind) -> u64 {
    match wire {
        WireKind::Anthropic => 200_000,
        WireKind::Openai => 128_000,
    }
}

/// Einstellungen des Loops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopSettings {
    /// Model-Requests je User-Turn (`executor.max_turns`, Default 200).
    pub max_turns: u32,
    /// `max_tokens` je Request.
    pub max_tokens: u32,
    /// Ab diesem Anteil des Kontextfensters wird vor dem nächsten Request kompaktiert.
    pub compaction_threshold: f64,
    /// Parallele Tool-Calls eines Model-Requests.
    pub parallel_tools: usize,
    /// Höchstens so lange wartet ein Tool-Call auf das Gate; danach `deny`.
    pub gate_timeout: Duration,
}

impl Default for LoopSettings {
    fn default() -> Self {
        Self {
            max_turns: 200,
            max_tokens: 8192,
            compaction_threshold: 0.8,
            parallel_tools: 4,
            gate_timeout: Duration::from_secs(30 * 60),
        }
    }
}

/// System-Prompt des Loops.
pub fn system_prompt(workdir: &str) -> String {
    format!(
        "Du bist ein Coding-Agent, der über beton arbeitet. Arbeitsverzeichnis: {workdir}.\n\
         Nutze die bereitgestellten Tools, um Dateien zu lesen und zu ändern und Kommandos \
         auszuführen. Jeder Tool-Call kann eine Freigabe durch den Menschen erfordern; ein \
         abgelehnter Call liefert eine Begründung. Antworte knapp."
    )
}

/// System-Prompt mit den Instructions des Agents (`instructions_delivery: system_prompt`,
/// AGT-005): einmal am Ende, bei jedem Request derselbe Text.
pub fn system_prompt_with(workdir: &str, instructions: Option<&str>) -> String {
    let base = system_prompt(workdir);
    match instructions {
        Some(i) if !i.trim().is_empty() => format!("{base}\n\n{i}"),
        _ => base,
    }
}

const SUMMARY_SYSTEM: &str = "Du fasst den bisherigen Verlauf einer Coding-Session für dich selbst zusammen, damit die Arbeit ohne den vollständigen Verlauf weitergehen kann. Nenne Ziel, getroffene Entscheidungen, geänderte Dateien, wichtige Ergebnisse von Tools und offene Schritte. Keine Einleitung.";
const SUMMARY_REQUEST: &str = "Fasse den bisherigen Verlauf zusammen.";
const SUMMARY_PREFIX: &str = "[Zusammenfassung des bisherigen Verlaufs (Compaction)]";
/// Tool-Ergebnisse älter als die letzten so vielen Runden werden bei Compaction gekürzt.
const KEEP_RECENT_ROUNDS: usize = 2;
const TRUNCATE_ABOVE: usize = 1024;

/// Verlauf einer Session.
#[derive(Debug, Default)]
pub(crate) struct Conversation {
    pub history: Vec<Message>,
    /// Belegter Kontext nach dem letzten Request (Tokens).
    pub last_context: Option<u64>,
}

/// Gemeinsamer Zustand von Session und Turn-Tasks.
pub(crate) struct Shared {
    pub harness: String,
    pub provider: Provider,
    pub key: Option<ApiKey>,
    pub client: Client,
    pub tools: Arc<dyn ToolHost>,
    pub gate: Arc<dyn Gate>,
    pub tx: mpsc::Sender<NormalizedEvent>,
    pub settings: LoopSettings,
    pub system: String,
    pub model: std::sync::Mutex<String>,
    pub conv: Mutex<Conversation>,
    pub running: AtomicBool,
    pub auth_source: AuthSource,
    pub capabilities: Capabilities,
}

impl Shared {
    async fn emit(&self, payload: EventPayload, turn: Option<TurnId>) {
        let _ = self.tx.send(NormalizedEvent::new(payload, turn)).await;
    }

    fn model(&self) -> String {
        self.model.lock().map(|m| m.clone()).unwrap_or_default()
    }

    fn window(&self, model: &str) -> u64 {
        self.provider
            .model(model)
            .and_then(|m| m.context_window)
            .unwrap_or_else(|| default_window(self.provider.wire))
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        self.tools
            .tools()
            .into_iter()
            .map(|t| ToolDef {
                name: t.model_name,
                description: t.description,
                input_schema: t.schema,
            })
            .collect()
    }

    fn auth_hint(&self) -> String {
        match &self.provider.key {
            KeyRef::Env(var) => format!(
                "API-Key in der Umgebungsvariable {var} des Daemons prüfen (providers.{}.api_key_env)",
                self.provider.name
            ),
            KeyRef::None => format!(
                "Der Endpunkt verlangt einen API-Key: providers.{}.api_key_env setzen",
                self.provider.name
            ),
        }
    }

    fn cost(&self, model: &str, usage: Usage, purpose: Option<&str>) -> EventPayload {
        let cost_micro = self.provider.model(model).and_then(|m| m.pricing).map(|p| {
            let input =
                (usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens) as f64;
            (input * p.input_per_mtok + usage.output_tokens as f64 * p.output_per_mtok).round()
                as i64
        });
        EventPayload::CostDelta(CostDelta {
            harness: self.harness.clone(),
            model: model.to_owned(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            cost_micro,
            currency: "USD".into(),
            source: CostSource::Estimated,
            auth_source: self.auth_source,
            purpose: purpose.map(str::to_owned),
        })
    }
}

/// Ergebnis eines Model-Requests.
#[derive(Debug, Default)]
struct Response {
    message_id: Option<String>,
    text: String,
    thinking: Vec<(String, Option<String>)>,
    tool_uses: Vec<(String, String, Value)>,
    usage: Usage,
    stop: Option<StopReason>,
}

/// Sammelt die Antwort und streamt Deltas als Events, wenn `turn` gesetzt ist.
struct Collector<'a> {
    shared: &'a Shared,
    turn: Option<TurnId>,
    resp: Response,
}

impl Collector<'_> {
    fn message_id(&mut self) -> String {
        self.resp
            .message_id
            .get_or_insert_with(|| format!("msg_{}", ulid::Ulid::generate()))
            .clone()
    }
}

#[async_trait]
impl StreamSink for Collector<'_> {
    async fn retry(&mut self, r: &Retry) {
        self.shared
            .emit(
                EventPayload::Notice(Notice {
                    level: NoticeLevel::Info,
                    text: format!(
                        "{}: {} – neuer Versuch {} in {} s",
                        self.shared.harness,
                        r.reason,
                        r.attempt + 1,
                        r.wait.as_secs_f64().ceil()
                    ),
                }),
                self.turn,
            )
            .await;
    }

    async fn event(&mut self, ev: StreamEvent) {
        match ev {
            StreamEvent::MessageId(id) => {
                self.resp.message_id.get_or_insert(id);
            }
            StreamEvent::TextDelta(t) => {
                let id = self.message_id();
                self.resp.text.push_str(&t);
                if self.turn.is_some() {
                    self.shared
                        .emit(
                            EventPayload::MessageDelta(TextDelta {
                                message_id: id,
                                text: t,
                                snapshot: false,
                            }),
                            self.turn,
                        )
                        .await;
                }
            }
            StreamEvent::ThinkingDelta(t) => {
                let id = self.message_id();
                if self.turn.is_some() {
                    self.shared
                        .emit(
                            EventPayload::ReasoningDelta(TextDelta {
                                message_id: id,
                                text: t,
                                snapshot: false,
                            }),
                            self.turn,
                        )
                        .await;
                }
            }
            StreamEvent::Thinking { text, signature } => self.resp.thinking.push((text, signature)),
            StreamEvent::ToolUse { id, name, input } => self.resp.tool_uses.push((id, name, input)),
            StreamEvent::Usage(u) => self.resp.usage = u,
            StreamEvent::Stop(s) => self.resp.stop = Some(s),
        }
    }
}

/// Ein Model-Request; streamt Deltas als Events, wenn `turn` gesetzt ist.
async fn request(
    shared: &Shared,
    turn: Option<TurnId>,
    req: ModelRequest,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Response, CallError> {
    let body = wire::request_body(shared.provider.wire, &req);
    let mut sink = Collector {
        shared,
        turn,
        resp: Response::default(),
    };
    shared
        .client
        .stream(
            &shared.provider,
            shared.key.as_ref(),
            &body,
            cancel,
            &mut sink,
        )
        .await?;
    Ok(sink.resp)
}

fn denied_text(reason: Option<&str>) -> String {
    match reason {
        Some(r) if !r.is_empty() => format!("{DENIED_PREFIX}: {r}"),
        _ => format!("{DENIED_PREFIX}."),
    }
}

async fn wait_cancel(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

async fn complete(
    shared: &Shared,
    turn: TurnId,
    call_id: &str,
    status: ToolStatus,
    text: &str,
    ms: u64,
) {
    shared
        .emit(
            EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id: call_id.to_owned(),
                status,
                result: Some(Value::String(text.to_owned())),
                result_ref: None,
                duration_ms: ms,
            }),
            Some(turn),
        )
        .await;
}

/// Ein Tool-Call: Gate (mit `approval.*`), Ausführung, `tool.call.*`.
async fn one_call(
    shared: Arc<Shared>,
    turn: TurnId,
    call_id: String,
    info: Option<ToolInfo>,
    name: String,
    input: Value,
    mut cancel: watch::Receiver<bool>,
) -> Block {
    let result = |content: String, is_error: bool| Block::ToolResult {
        id: call_id.clone(),
        content,
        is_error,
    };
    let Some(info) = info else {
        let text = format!("tool_not_enabled: {name} gibt es in dieser Session nicht");
        complete(&shared, turn, &call_id, ToolStatus::Error, &text, 0).await;
        return result(text, true);
    };
    let approval_id = ApprovalId::new();
    let expires = time::OffsetDateTime::now_utc() + shared.settings.gate_timeout;
    shared
        .emit(
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id,
                kind: ApprovalKind::Tool,
                subject: json!({"tool": name, "args": input, "call_id": call_id}),
                options: vec!["allow".into(), "deny".into()],
                expires_at: Timestamp::from(expires),
                on_timeout: TimeoutAction::Deny,
            }),
            Some(turn),
        )
        .await;
    let decide = tokio::time::timeout(
        shared.settings.gate_timeout,
        shared.gate.decide(GateRequest {
            turn_id: Some(turn),
            call_id: call_id.clone(),
            tool: name.clone(),
            kind: info.kind.clone(),
            args: input.clone(),
        }),
    );
    let system = Actor::System {
        component: SystemComponent::Runner,
    };
    let user = Actor::User {
        id: PrincipalId::User(UserId::LOCAL),
        device_id: None,
    };
    let decision = tokio::select! {
        d = decide => d,
        () = wait_cancel(&mut cancel) => {
            shared.emit(EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id, decision: ApprovalDecision::Abort, answer: None, actor: system,
                via: ResolvedVia::System, remember: None, comment: None, on_timeout_applied: None,
            }), Some(turn)).await;
            let text = format!("{CANCELLED_PREFIX}: Turn unterbrochen");
            complete(&shared, turn, &call_id, ToolStatus::Cancelled, &text, 0).await;
            return result(text, true);
        }
    };
    // Fail closed: ohne Entscheidung innerhalb der Frist gilt `deny`.
    let (decision, via, actor, comment, timed_out) = match decision {
        Ok(d) => {
            let comment = match &d {
                GateDecision::Deny { reason } => reason.clone(),
                GateDecision::Allow { .. } => None,
            };
            (d, ResolvedVia::User, user, comment, false)
        }
        Err(_) => (
            GateDecision::Deny {
                reason: Some(
                    "Keine Entscheidung erhalten; aus Sicherheitsgründen abgelehnt.".into(),
                ),
            },
            ResolvedVia::Timeout,
            system,
            None,
            true,
        ),
    };
    shared
        .emit(
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id,
                decision: match decision {
                    GateDecision::Allow { .. } => ApprovalDecision::Allow,
                    GateDecision::Deny { .. } => ApprovalDecision::Deny,
                },
                answer: None,
                actor,
                via,
                remember: None,
                comment,
                on_timeout_applied: timed_out.then_some(TimeoutAction::Deny),
            }),
            Some(turn),
        )
        .await;
    let updated = match decision {
        GateDecision::Deny { reason } => {
            let text = denied_text(reason.as_deref());
            complete(&shared, turn, &call_id, ToolStatus::Denied, &text, 0).await;
            return result(text, true);
        }
        // `null` heißt „unverändert“ (so kommt es aus der Freigabe-API).
        GateDecision::Allow { updated_args } => updated_args.filter(|a| !a.is_null()),
    };
    shared
        .emit(
            EventPayload::ToolCallStarted(ToolCallStarted {
                call_id: call_id.clone(),
                sandbox_stage: None,
                args: updated.clone(),
            }),
            Some(turn),
        )
        .await;
    let started = Instant::now();
    let args = updated.unwrap_or(input);
    let outcome = tokio::select! {
        o = shared.tools.call(&info.model_name, args) => o,
        () = wait_cancel(&mut cancel) => {
            let text = format!("{CANCELLED_PREFIX}: Turn unterbrochen");
            complete(&shared, turn, &call_id, ToolStatus::Cancelled, &text, 0).await;
            return result(text, true);
        }
    };
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    complete(
        &shared,
        turn,
        &call_id,
        if outcome.ok {
            ToolStatus::Ok
        } else {
            ToolStatus::Error
        },
        &outcome.text,
        ms,
    )
    .await;
    result(outcome.text, !outcome.ok)
}

/// Führt die Tool-Calls einer Antwort aus (höchstens `parallel_tools` gleichzeitig);
/// Ergebnisse in der Reihenfolge der Calls.
async fn run_tools(
    shared: &Arc<Shared>,
    turn: TurnId,
    calls: &[(String, String, Value)],
    cancel: &watch::Receiver<bool>,
) -> Vec<Block> {
    let tools = shared.tools.tools();
    let mut infos = Vec::new();
    for (id, name, input) in calls {
        let info = tools.iter().find(|t| t.model_name == *name).cloned();
        let (tool, mcp_server, source) = match &info {
            Some(i) => (i.tool.clone(), Some(i.server.clone()), i.source),
            None => (name.clone(), None, beton_core::event::ToolSource::Harness),
        };
        shared
            .emit(
                EventPayload::ToolCallRequested(ToolCallRequested {
                    call_id: id.clone(),
                    tool,
                    mcp_server,
                    args: input.clone(),
                    source,
                    // Der Direkt-API-Harness hat keine Vendor-Sub-Agents.
                    parent_call_id: None,
                }),
                Some(turn),
            )
            .await;
        infos.push(info);
    }
    let sem = Arc::new(Semaphore::new(shared.settings.parallel_tools.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for (i, ((id, name, input), info)) in calls.iter().zip(infos).enumerate() {
        let (shared, sem, cancel) = (shared.clone(), sem.clone(), cancel.clone());
        let (id, name, input) = (id.clone(), name.clone(), input.clone());
        set.spawn(async move {
            let _permit = sem.acquire_owned().await;
            (
                i,
                one_call(shared, turn, id, info, name, input, cancel).await,
            )
        });
    }
    let mut out: Vec<Option<Block>> = vec![None; calls.len()];
    while let Some(joined) = set.join_next().await {
        if let Ok((i, block)) = joined {
            out[i] = Some(block);
        }
    }
    out.into_iter()
        .zip(calls)
        .map(|(b, (id, _, _))| {
            b.unwrap_or_else(|| Block::ToolResult {
                id: id.clone(),
                content: format!("{CANCELLED_PREFIX}: interner Fehler"),
                is_error: true,
            })
        })
        .collect()
}

/// Wie ein Turn endete.
enum TurnEnd {
    Completed(String),
    Interrupted,
    Failed(CallError),
}

fn assistant_blocks(resp: &Response) -> Vec<Block> {
    let mut blocks: Vec<Block> = resp
        .thinking
        .iter()
        .map(|(text, signature)| Block::Thinking {
            text: text.clone(),
            signature: signature.clone(),
        })
        .collect();
    if !resp.text.is_empty() {
        blocks.push(Block::Text(resp.text.clone()));
    }
    blocks.extend(
        resp.tool_uses
            .iter()
            .map(|(id, name, input)| Block::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
    );
    blocks
}

async fn emit_completed_messages(shared: &Shared, turn: TurnId, resp: &Response) {
    let id = resp
        .message_id
        .clone()
        .unwrap_or_else(|| format!("msg_{}", ulid::Ulid::generate()));
    for (text, _) in &resp.thinking {
        shared
            .emit(
                EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id.clone(),
                    summary: (!text.is_empty()).then(|| text.clone()),
                    redacted: text.is_empty(),
                }),
                Some(turn),
            )
            .await;
    }
    if !resp.text.is_empty() {
        shared
            .emit(
                EventPayload::MessageCompleted(MessageCompleted {
                    message_id: id,
                    role: MessageRole::Assistant,
                    content: vec![json!({"type": "text", "text": resp.text})],
                    author: None,
                }),
                Some(turn),
            )
            .await;
    }
}

/// Kürzt ein langes Tool-Ergebnis auf Kopf und Ende.
fn truncate_result(content: &str) -> Option<String> {
    if content.len() <= TRUNCATE_ABOVE {
        return None;
    }
    let head: String = content.chars().take(512).collect();
    let tail: String = {
        let rev: String = content.chars().rev().take(256).collect();
        rev.chars().rev().collect()
    };
    let cut = content.chars().count().saturating_sub(768);
    Some(format!(
        "{head}\n[… {cut} Zeichen gekürzt (Compaction) …]\n{tail}"
    ))
}

fn estimate(shared: &Shared, history: &[Message]) -> u64 {
    let chars = shared.system.len() + history.iter().map(Message::chars).sum::<usize>();
    wire::estimate_tokens(chars)
}

/// Compaction (HAR-010, SES-011): Stufe 1 kürzt Tool-Ergebnisse außer in den letzten
/// Runden, Stufe 2 lässt ältere Turns vom selben Modell zusammenfassen. `forced` (manuell)
/// fasst immer zusammen, sofern es Älteres gibt. Liefert den Verbrauch der Zusammenfassung.
async fn compact(
    shared: &Shared,
    conv: &mut Conversation,
    turn: Option<TurnId>,
    forced: bool,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Usage, CallError> {
    let model = shared.model();
    let window = shared.window(&model);
    let before = conv
        .last_context
        .unwrap_or_else(|| estimate(shared, &conv.history));
    shared
        .emit(
            EventPayload::CompactionStarted(Compaction {
                before_tokens: before,
                after_tokens: None,
            }),
            turn,
        )
        .await;
    // Stufe 1: alte Tool-Ergebnisse kürzen.
    let assistant_idx: Vec<usize> = conv
        .history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Assistant)
        .map(|(i, _)| i)
        .collect();
    let protect_from = assistant_idx
        .len()
        .checked_sub(KEEP_RECENT_ROUNDS)
        .and_then(|k| assistant_idx.get(k).copied())
        .unwrap_or(0);
    for m in conv.history.iter_mut().take(protect_from) {
        for b in &mut m.blocks {
            if let Block::ToolResult { content, .. } = b
                && let Some(short) = truncate_result(content)
            {
                *content = short;
            }
        }
    }
    let mut usage = Usage::default();
    let target = (window as f64 * 0.5) as u64;
    if forced || estimate(shared, &conv.history) > target {
        // Stufe 2: alles vor dem aktuellen Turn (bzw. vor den letzten Runden) zusammenfassen.
        let last_user_text = conv.history.iter().rposition(|m| {
            m.role == Role::User && m.blocks.iter().any(|b| matches!(b, Block::Text(_)))
        });
        let cut = match last_user_text {
            Some(i) if i > 0 && turn.is_some() => i,
            _ if turn.is_none() => conv.history.len(),
            _ => assistant_idx
                .len()
                .checked_sub(KEEP_RECENT_ROUNDS)
                .and_then(|k| assistant_idx.get(k).copied())
                .unwrap_or(0),
        };
        if cut > 0 {
            let mut messages: Vec<Message> = conv.history[..cut].to_vec();
            messages.push(Message::user_text(SUMMARY_REQUEST));
            let resp = request(
                shared,
                None,
                ModelRequest {
                    model: model.clone(),
                    system: SUMMARY_SYSTEM.into(),
                    messages,
                    tools: Vec::new(),
                    max_tokens: shared.settings.max_tokens.min(4096),
                    prompt_caching: false,
                },
                cancel,
            )
            .await?;
            usage = resp.usage;
            let mut rest = conv.history.split_off(cut);
            conv.history.clear();
            conv.history.push(Message::user_text(format!(
                "{SUMMARY_PREFIX}\n{}",
                resp.text
            )));
            if rest.first().is_some_and(|m| m.role == Role::User) || rest.is_empty() {
                conv.history.push(Message {
                    role: Role::Assistant,
                    blocks: vec![Block::Text("Verstanden.".into())],
                });
            }
            conv.history.append(&mut rest);
        }
    }
    let after = estimate(shared, &conv.history);
    conv.last_context = Some(after);
    shared
        .emit(
            EventPayload::CompactionCompleted(Compaction {
                before_tokens: before,
                after_tokens: Some(after),
            }),
            turn,
        )
        .await;
    shared
        .emit(
            EventPayload::ContextUsage(ContextUsage {
                used_tokens: after,
                window_tokens: window,
                source: ContextSource::Estimated,
            }),
            turn,
        )
        .await;
    Ok(usage)
}

async fn run_turn(
    shared: Arc<Shared>,
    turn: TurnId,
    text: String,
    mut cancel: watch::Receiver<bool>,
) {
    let mut conv = shared.conv.lock().await;
    conv.history.push(Message::user_text(text));
    let mut total = Usage::default();
    let mut requests: u32 = 0;
    let mut last_model = shared.model();
    let end = loop {
        if *cancel.borrow() {
            break TurnEnd::Interrupted;
        }
        let model = shared.model();
        let window = shared.window(&model);
        if conv
            .last_context
            .is_some_and(|used| used as f64 > window as f64 * shared.settings.compaction_threshold)
        {
            match compact(&shared, &mut conv, Some(turn), false, &mut cancel).await {
                Ok(u) => total.add(u),
                Err(CallError::Cancelled) => break TurnEnd::Interrupted,
                Err(e) => break TurnEnd::Failed(e),
            }
        }
        let req = ModelRequest {
            model: model.clone(),
            system: shared.system.clone(),
            messages: conv.history.clone(),
            tools: shared.tool_defs(),
            max_tokens: shared.settings.max_tokens,
            prompt_caching: shared.provider.prompt_caching,
        };
        let resp = match request(&shared, Some(turn), req, &mut cancel).await {
            Ok(r) => r,
            Err(CallError::Cancelled) => break TurnEnd::Interrupted,
            Err(e) => break TurnEnd::Failed(e),
        };
        last_model = model;
        requests += 1;
        total.add(resp.usage);
        conv.last_context = Some(resp.usage.context());
        emit_completed_messages(&shared, turn, &resp).await;
        let blocks = assistant_blocks(&resp);
        if !blocks.is_empty() {
            conv.history.push(Message {
                role: Role::Assistant,
                blocks,
            });
        }
        if resp.tool_uses.is_empty() {
            break TurnEnd::Completed(resp.stop.unwrap_or(StopReason::EndTurn).as_str().to_owned());
        }
        if requests >= shared.settings.max_turns {
            // Limit erreicht: Calls nicht mehr ausführen, Verlauf gültig halten.
            let mut results = Vec::new();
            for (id, _, _) in &resp.tool_uses {
                let text = format!("{CANCELLED_PREFIX}: max_turns erreicht");
                shared
                    .emit(
                        EventPayload::ToolCallCompleted(ToolCallCompleted {
                            call_id: id.clone(),
                            status: ToolStatus::Cancelled,
                            result: Some(Value::String(text.clone())),
                            result_ref: None,
                            duration_ms: 0,
                        }),
                        Some(turn),
                    )
                    .await;
                results.push(Block::ToolResult {
                    id: id.clone(),
                    content: text,
                    is_error: true,
                });
            }
            conv.history.push(Message {
                role: Role::User,
                blocks: results,
            });
            break TurnEnd::Completed("max_turns".into());
        }
        let results = run_tools(&shared, turn, &resp.tool_uses, &cancel).await;
        conv.history.push(Message {
            role: Role::User,
            blocks: results,
        });
    };
    let window = shared.window(&last_model);
    let context = conv.last_context;
    drop(conv);
    if requests > 0 {
        shared
            .emit(shared.cost(&last_model, total, None), Some(turn))
            .await;
    }
    if let (Some(used), Some(_)) = (context, (requests > 0).then_some(())) {
        shared
            .emit(
                EventPayload::ContextUsage(ContextUsage {
                    used_tokens: used,
                    window_tokens: window,
                    source: ContextSource::Harness,
                }),
                Some(turn),
            )
            .await;
    }
    // Erst freigeben, dann melden: Wer auf das Turn-Ende wartet, darf sofort senden.
    shared.running.store(false, Ordering::SeqCst);
    match end {
        TurnEnd::Completed(stop_reason) => {
            shared
                .emit(
                    EventPayload::TurnCompleted(TurnCompleted {
                        turn_id: turn,
                        stop_reason,
                        usage_summary: json!({
                            "input_tokens": total.input_tokens,
                            "output_tokens": total.output_tokens,
                            "requests": requests,
                        }),
                    }),
                    Some(turn),
                )
                .await;
        }
        TurnEnd::Interrupted => {
            shared
                .emit(
                    EventPayload::TurnInterrupted(TurnInterrupted {
                        turn_id: turn,
                        by: PrincipalId::User(UserId::LOCAL),
                        reason: None,
                    }),
                    Some(turn),
                )
                .await;
        }
        TurnEnd::Failed(e) => {
            if e.is_auth() {
                shared
                    .emit(
                        EventPayload::HarnessAuthRequired(HarnessAuthRequired {
                            harness: shared.harness.clone(),
                            hint: shared.auth_hint(),
                        }),
                        Some(turn),
                    )
                    .await;
            }
            let detail = match &shared.key {
                Some(k) => k.scrub(&e.to_string()),
                None => e.to_string(),
            };
            shared
                .emit(
                    EventPayload::TurnFailed(TurnFailed {
                        turn_id: turn,
                        problem: json!({
                            "type": format!("urn:beton:problem:{}", e.code()),
                            "code": e.code(),
                            "title": format!("{} meldet einen Fehler", shared.harness),
                            "detail": detail,
                        }),
                    }),
                    Some(turn),
                )
                .await;
        }
    }
}

/// Eine laufende Direkt-API-Session.
pub struct DirectSession {
    pub(crate) shared: Arc<Shared>,
    pub(crate) rx: Option<mpsc::Receiver<NormalizedEvent>>,
    pub(crate) cancel: Option<watch::Sender<bool>>,
}

#[async_trait]
impl HarnessSession for DirectSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        if self.shared.running.swap(true, Ordering::SeqCst) {
            return Err(HarnessError::Busy("ein Turn läuft bereits".into()));
        }
        let turn = TurnId::new();
        self.shared
            .emit(
                EventPayload::TurnStarted(TurnStarted {
                    turn_id: turn,
                    input_id: None,
                    author: PrincipalId::User(UserId::LOCAL),
                }),
                Some(turn),
            )
            .await;
        let (tx, rx) = watch::channel(false);
        self.cancel = Some(tx);
        tokio::spawn(run_turn(self.shared.clone(), turn, input.text, rx));
        Ok(turn)
    }

    async fn steer(&mut self, _input: UserInput) -> Result<(), HarnessError> {
        self.shared.capabilities.check(Action::Steer)?;
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        if let Some(c) = &self.cancel {
            let _ = c.send(true);
        }
        Ok(())
    }

    async fn set_model(
        &mut self,
        model: String,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        if effort.is_some() {
            self.shared.capabilities.check(Action::EffortSwitch)?;
        }
        if let Ok(mut m) = self.shared.model.lock() {
            m.clone_from(&model);
        }
        self.shared
            .emit(
                EventPayload::SessionSettingsChanged(SessionSettingsChanged {
                    model: Some(model),
                    mechanism: Some(SettingsMechanism::Live),
                    ..SessionSettingsChanged::default()
                }),
                None,
            )
            .await;
        Ok(SwitchOutcome::Live)
    }

    async fn set_permission_mode(&mut self, _mode: PermissionMode) -> Result<(), HarnessError> {
        // Jeder Tool-Call geht ohnehin durch das Gate; Modi wirken über die Policies (M2).
        Ok(())
    }

    async fn compact(&mut self) -> Result<(), HarnessError> {
        self.shared.capabilities.check(Action::Compact)?;
        if self.shared.running.load(Ordering::SeqCst) {
            return Err(HarnessError::Busy(
                "Compaction erst nach dem laufenden Turn".into(),
            ));
        }
        let shared = self.shared.clone();
        let (_tx, mut rx) = watch::channel(false);
        tokio::spawn(async move {
            let _keep = _tx;
            let mut conv = shared.conv.lock().await;
            match compact(&shared, &mut conv, None, true, &mut rx).await {
                Ok(u) if u != Usage::default() => {
                    let model = shared.model();
                    drop(conv);
                    shared
                        .emit(shared.cost(&model, u, Some("compaction")), None)
                        .await;
                }
                Ok(_) => {}
                Err(e) => {
                    drop(conv);
                    let detail = match &shared.key {
                        Some(k) => k.scrub(&e.to_string()),
                        None => e.to_string(),
                    };
                    shared
                        .emit(
                            EventPayload::Notice(Notice {
                                level: NoticeLevel::Warn,
                                text: format!("Compaction fehlgeschlagen: {detail}"),
                            }),
                            None,
                        )
                        .await;
                }
            }
        });
        Ok(())
    }

    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }

    fn native_session_ref(&self) -> Option<String> {
        None
    }

    fn capabilities(&self) -> Option<Capabilities> {
        Some(self.shared.capabilities.clone())
    }

    async fn shutdown(self: Box<Self>, _how: Shutdown) -> Result<ExitInfo, HarnessError> {
        if let Some(c) = &self.cancel {
            let _ = c.send(true);
        }
        self.shared.tools.shutdown().await;
        Ok(ExitInfo::default())
    }
}
