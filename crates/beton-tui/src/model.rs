//! Zustand einer Session aus ihren Events (TUI-001 AC1): dieselbe Ableitung wie die Web-UI
//! (`apps/web/src/store/events.ts`, `lib/timeline.ts`). Reine Funktionen ohne IO; die TUI
//! enthält keine eigene Policy- oder Kostenlogik, sie zeigt nur, was die Events melden.

use std::collections::HashMap;

use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, Event, EventPayload, MessageRole, PlanUpdated,
    RememberScope, ResolvedVia, SessionStatus, TerminalKind, ToolStatus, UsageSubscription,
};
use beton_core::id::{ApprovalId, SessionId};
use beton_core::time::Timestamp;
use serde_json::Value;

use crate::context::{ContextState, context_of};

/// Status einer Session als Zeichen (nie nur Farbe): ◆ wartet, ● läuft, ○ bereit, ✕ Fehler,
/// ■ gestoppt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Waiting,
    Running,
    Idle,
    Failed,
    Stopped,
}

impl Glyph {
    /// Wie `listStatus` der Web-UI.
    pub fn of_status(status: &str) -> Self {
        match status {
            "running" | "starting" => Self::Running,
            "waiting_approval" => Self::Waiting,
            "failed" => Self::Failed,
            "stopped" | "paused" => Self::Stopped,
            _ => Self::Idle,
        }
    }

    pub fn of(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Running | SessionStatus::Starting => Self::Running,
            SessionStatus::WaitingApproval => Self::Waiting,
            SessionStatus::Failed => Self::Failed,
            SessionStatus::Stopped | SessionStatus::Paused => Self::Stopped,
            SessionStatus::Idle => Self::Idle,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Waiting => "◆",
            Self::Running => "●",
            Self::Idle => "○",
            Self::Failed => "✕",
            Self::Stopped => "■",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "wartet auf dich",
            Self::Running => "läuft",
            Self::Idle => "bereit",
            Self::Failed => "Fehler",
            Self::Stopped => "gestoppt",
        }
    }
}

/// Anzeigename eines Harness (wie `harnessName` der Web-UI).
pub fn harness_name(harness: &str) -> String {
    match harness {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "fake" => "Fake-Harness".into(),
        other => other.strip_prefix("acp:").unwrap_or(other).to_owned(),
    }
}

/// Stimme eines Harness (Farbe in Partitur und Badges).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    Claude,
    Codex,
    Acp,
    Direct,
}

impl Voice {
    pub fn of(harness: &str) -> Self {
        match harness {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            h if h.starts_with("acp:") => Self::Acp,
            _ => Self::Direct,
        }
    }
}

/// Zustand eines Tool-Calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    Requested,
    Running,
    Completed,
    Failed,
    Denied,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolItem {
    pub call_id: String,
    pub tool: String,
    pub kind: Option<String>,
    pub args: Value,
    pub state: ToolState,
    pub duration_ms: Option<u64>,
    pub result: Option<Value>,
    /// Gestreamte Ausgabe (`tool.call.output.delta`).
    pub output: String,
    /// Vom Nutzer auf- oder zugeklappt; `None`: Vorgabe (Bearbeiten offen, sonst zu).
    pub expanded: Option<bool>,
}

impl ToolItem {
    pub fn is_edit(&self) -> bool {
        matches!(self.kind.as_deref(), Some("file_edit" | "file_write"))
            || matches!(
                self.tool.as_str(),
                "Edit" | "MultiEdit" | "Write" | "apply_patch"
            )
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded.unwrap_or_else(|| self.is_edit())
    }
}

/// Grund einer Freigabe (`subject.reasons[]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub rule_id: String,
    pub scope: String,
    pub set: String,
    pub reason: String,
}

/// Wie eine Freigabe entschieden wurde.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub decision: ApprovalDecision,
    pub via: ResolvedVia,
    pub remember: bool,
    /// Lokaler Benutzer (`usr_local`) bzw. anderer Akteur.
    pub by_local_user: bool,
    pub at: Timestamp,
    pub comment: Option<String>,
}

/// Eine Freigabe (`approval.requested`) mit dem, was die Karte zeigt.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalCard {
    pub approval_id: ApprovalId,
    pub kind: ApprovalKind,
    pub title: String,
    pub call_id: Option<String>,
    pub tool: String,
    pub tool_kind: Option<String>,
    /// Befehl bzw. Pfad (`subject.summary`).
    pub summary: String,
    pub args: Value,
    pub reasons: Vec<Reason>,
    pub options: Vec<String>,
    pub expires_at: Timestamp,
    /// Frage des Agents (`kind: question`).
    pub question: Option<String>,
    pub resolution: Option<Resolution>,
}

impl ApprovalCard {
    /// „Für diese Session erlauben“ nur, wenn die Karte es anbietet (POL-010).
    pub fn allows_session(&self) -> bool {
        self.options.iter().any(|o| o == "approve_session")
    }

    fn from_event(p: &beton_core::event::ApprovalRequested) -> Self {
        let s = &p.subject;
        let tool_name = s["tool"]["name"]
            .as_str()
            .or_else(|| s["tool"].as_str())
            .unwrap_or_default()
            .to_owned();
        let args = s["tool"]["args_modified"]
            .as_object()
            .map(|_| s["tool"]["args_modified"].clone())
            .unwrap_or_else(|| s["tool"]["args"].clone());
        let summary = s["summary"]
            .as_str()
            .map(str::to_owned)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| arg_summary(&args));
        let reasons = s["reasons"]
            .as_array()
            .map(|rs| {
                rs.iter()
                    .map(|r| Reason {
                        rule_id: text(&r["rule_id"]),
                        scope: text(&r["scope"]),
                        set: text(&r["set"]),
                        reason: text(&r["reason"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let question = (p.kind == ApprovalKind::Question).then(|| {
            s["question"]
                .as_str()
                .or_else(|| s["text"].as_str())
                .or_else(|| s["summary"].as_str())
                .unwrap_or_default()
                .to_owned()
        });
        Self {
            approval_id: p.approval_id,
            kind: p.kind,
            title: text(&s["title"]),
            call_id: s["call_id"].as_str().map(str::to_owned),
            tool: tool_name,
            tool_kind: s["tool"]["kind"].as_str().map(str::to_owned),
            summary,
            args,
            reasons,
            options: p.options.clone(),
            expires_at: p.expires_at,
            question,
            resolution: None,
        }
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// Ton einer Notiz im Verlauf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Ok,
    Warn,
}

/// Ein Eintrag im Verlauf.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User {
        text: String,
    },
    Assistant {
        message_id: String,
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool(ToolItem),
    Approval(ApprovalCard),
    Note {
        text: String,
        tone: Tone,
    },
    Error {
        title: String,
        detail: Option<String>,
    },
}

/// Ein Terminal der Session (aus `terminal.opened`/`renamed`/`closed`).
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalInfo {
    pub terminal_id: String,
    pub name: String,
    pub cmd: String,
    pub kind: TerminalKind,
    pub opened_at: Timestamp,
}

/// Abgeleiteter Zustand einer Session.
#[derive(Debug, Clone)]
pub struct SessionModel {
    pub id: SessionId,
    pub harness: String,
    pub model: Option<String>,
    pub title: String,
    pub cwd: Option<String>,
    pub status: SessionStatus,
    pub items: Vec<Item>,
    /// Laufende Nachrichten (`message.delta`) in Ankunftsreihenfolge.
    pub streaming: Vec<(String, String)>,
    /// Laufende Überlegungen (`reasoning.delta`).
    pub reasoning: Vec<(String, String)>,
    tools: HashMap<String, usize>,
    approvals: HashMap<ApprovalId, usize>,
    pending_output: HashMap<String, String>,
    /// Eingereihte Eingaben (`queue.updated`, SES-004).
    pub queue_len: usize,
    pub context: Option<ContextState>,
    /// Letzter Stand je Abo-Fenster (`usage.subscription`, USE-004).
    pub subscription: Vec<UsageSubscription>,
    pub terminals: Vec<TerminalInfo>,
    /// Letzter Plan des Agents (`plan.updated`).
    pub plan: Option<PlanUpdated>,
    /// Höchste gesehene dauerhafte `seq`.
    pub last_seq: u64,
    /// Zählt jede Änderung (für Caches der Darstellung).
    pub version: u64,
    /// Seq des letzten `fs.changed`/`turn.*`/`git.*`: Anlass, die Änderungen neu zu laden.
    pub workspace_version: u64,
}

impl SessionModel {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            harness: String::new(),
            model: None,
            title: String::new(),
            cwd: None,
            status: SessionStatus::Idle,
            items: Vec::new(),
            streaming: Vec::new(),
            reasoning: Vec::new(),
            tools: HashMap::new(),
            approvals: HashMap::new(),
            pending_output: HashMap::new(),
            queue_len: 0,
            context: None,
            subscription: Vec::new(),
            terminals: Vec::new(),
            plan: None,
            last_seq: 0,
            version: 0,
            workspace_version: 0,
        }
    }

    /// Zustand verwerfen (z. B. nach `seq_ahead`); Kopf (Titel, Harness) bleibt.
    pub fn reset(&mut self) {
        let mut fresh = Self::new(self.id);
        fresh.title = std::mem::take(&mut self.title);
        fresh.version = self.version + 1;
        *self = fresh;
    }

    /// Offene Freigaben in Reihenfolge ihres Eintreffens.
    pub fn pending_approvals(&self) -> impl Iterator<Item = &ApprovalCard> {
        self.items.iter().filter_map(|i| match i {
            Item::Approval(a) if a.resolution.is_none() => Some(a),
            _ => None,
        })
    }

    pub fn pending_count(&self) -> usize {
        self.pending_approvals().count()
    }

    /// Wartet ein Tool-Call auf eine Freigabe?
    pub fn waiting_for_approval(&self, call_id: &str) -> bool {
        self.pending_approvals()
            .any(|a| a.call_id.as_deref() == Some(call_id))
    }

    /// Glyph des Kopfes: offene Freigabe heißt immer „wartet auf dich“.
    pub fn glyph(&self) -> Glyph {
        if self.pending_count() > 0 {
            Glyph::Waiting
        } else {
            Glyph::of(self.status)
        }
    }

    /// Läuft gerade ein Turn (dann reiht `Senden` ein)?
    pub fn busy(&self) -> bool {
        matches!(
            self.status,
            SessionStatus::Running | SessionStatus::Starting | SessionStatus::WaitingApproval
        )
    }

    /// Mehrere Events anwenden.
    pub fn apply_all<'a>(&mut self, events: impl IntoIterator<Item = &'a Event>) {
        for e in events {
            self.apply(e);
        }
    }

    /// Ein Event anwenden (dauerhafte Duplikate werden übergangen).
    pub fn apply(&mut self, e: &Event) {
        if e.transient {
            self.apply_transient(e);
            return;
        }
        if e.seq <= self.last_seq && self.last_seq > 0 {
            return;
        }
        self.last_seq = e.seq;
        self.version += 1;
        let Some(p) = e.payload() else {
            return;
        };
        match p {
            EventPayload::SessionCreated(c) => {
                self.harness = c.harness.clone();
                self.model = c.model.clone();
                self.cwd = Some(c.cwd.clone());
            }
            EventPayload::SessionStatus(s) => self.status = s.status,
            EventPayload::SessionTitleChanged(t) => self.title = t.title.clone(),
            EventPayload::SessionSettingsChanged(s) => {
                if let Some(m) = &s.model {
                    self.model = Some(m.clone());
                }
            }
            EventPayload::SessionResumed(_) => self.note("Session fortgesetzt", Tone::Ok),
            EventPayload::MessageCompleted(m) => {
                self.streaming.retain(|(id, _)| id != &m.message_id);
                let text = text_of(&m.content);
                match m.role {
                    MessageRole::User => self.items.push(Item::User { text }),
                    MessageRole::Assistant if !text.trim().is_empty() => {
                        self.items.push(Item::Assistant {
                            message_id: m.message_id.clone(),
                            text,
                        });
                    }
                    MessageRole::Assistant => {}
                }
            }
            EventPayload::ReasoningCompleted(r) => {
                let streamed = self
                    .reasoning
                    .iter()
                    .position(|(id, _)| id == &r.message_id)
                    .map(|i| self.reasoning.remove(i).1);
                let text = if r.redacted {
                    "(redigiert)".to_owned()
                } else {
                    r.summary.clone().or(streamed).unwrap_or_default()
                };
                if !text.trim().is_empty() {
                    self.items.push(Item::Reasoning { text });
                }
            }
            EventPayload::ToolCallRequested(t) => {
                let output = self.pending_output.remove(&t.call_id).unwrap_or_default();
                self.tools.insert(t.call_id.clone(), self.items.len());
                self.items.push(Item::Tool(ToolItem {
                    call_id: t.call_id.clone(),
                    tool: t.tool.clone(),
                    kind: t.kind.clone(),
                    args: t.args.clone(),
                    state: ToolState::Requested,
                    duration_ms: None,
                    result: None,
                    output,
                    expanded: None,
                }));
            }
            EventPayload::ToolCallStarted(t) => {
                if let Some(tool) = self.tool_mut(&t.call_id)
                    && tool.state == ToolState::Requested
                {
                    tool.state = ToolState::Running;
                }
            }
            EventPayload::ToolCallCompleted(t) => {
                if let Some(tool) = self.tool_mut(&t.call_id) {
                    tool.state = match t.status {
                        ToolStatus::Ok => ToolState::Completed,
                        ToolStatus::Denied => ToolState::Denied,
                        ToolStatus::Error | ToolStatus::Cancelled => ToolState::Failed,
                    };
                    tool.duration_ms = Some(t.duration_ms);
                    tool.result.clone_from(&t.result);
                }
            }
            EventPayload::ApprovalRequested(a) => {
                self.approvals.insert(a.approval_id, self.items.len());
                self.items.push(Item::Approval(ApprovalCard::from_event(a)));
            }
            EventPayload::ApprovalResolved(r) => {
                if let Some(&i) = self.approvals.get(&r.approval_id)
                    && let Some(Item::Approval(card)) = self.items.get_mut(i)
                {
                    card.resolution = Some(Resolution {
                        decision: r.decision,
                        via: r.via,
                        remember: r.remember == Some(RememberScope::Session),
                        by_local_user: matches!(&r.actor, Actor::User { id, .. } if id.to_string() == "usr_local"),
                        at: e.ts,
                        comment: r.comment.clone(),
                    });
                }
            }
            EventPayload::PolicyDecision(d)
                if d.outcome == beton_core::event::PolicyOutcome::Deny =>
            {
                let reason = d
                    .matched
                    .first()
                    .and_then(|m| m["reason"].as_str().or_else(|| m["rule_id"].as_str()))
                    .unwrap_or("Policy")
                    .to_owned();
                self.items.push(Item::Note {
                    text: format!("Von Policy abgelehnt: {reason}"),
                    tone: Tone::Warn,
                });
            }
            EventPayload::TurnStarted(_) => {}
            EventPayload::TurnCompleted(_) => {
                self.end_turn();
                self.workspace_version = e.seq;
            }
            EventPayload::TurnFailed(t) => {
                self.end_turn();
                self.workspace_version = e.seq;
                self.error(&t.problem);
            }
            EventPayload::Error(err) => self.error(&err.problem),
            EventPayload::TurnInterrupted(_) => {
                self.end_turn();
                self.workspace_version = e.seq;
                self.note("Turn unterbrochen", Tone::Plain);
            }
            EventPayload::Notice(n) => self.note(&n.text, Tone::Plain),
            EventPayload::HarnessExited(h) => self.items.push(Item::Error {
                title: "Harness beendet".into(),
                detail: (!h.stderr_tail.is_empty()).then(|| h.stderr_tail.clone()),
            }),
            EventPayload::HarnessAuthRequired(h) => self.items.push(Item::Error {
                title: format!("{} braucht eine Anmeldung", harness_name(&h.harness)),
                detail: Some(h.hint.clone()),
            }),
            EventPayload::HarnessIncompatible(h) => self.items.push(Item::Error {
                title: "Harness-Version nicht unterstützt".into(),
                detail: Some(format!(
                    "{} (erwartet {})",
                    h.detected_version, h.expected_range
                )),
            }),
            EventPayload::EventRejected(r) => self.items.push(Item::Error {
                title: "Event abgelehnt".into(),
                detail: Some(format!("{} ({})", r.original_type, r.reason)),
            }),
            EventPayload::CompactionCompleted(c) => {
                let text = match c.after_tokens {
                    Some(after) => format!(
                        "Kontext kompaktiert · {} → {} Tokens",
                        c.before_tokens, after
                    ),
                    None => "Kontext kompaktiert".into(),
                };
                self.note(&text, Tone::Plain);
            }
            EventPayload::QueueUpdated(q) => self.queue_len = q.items.len(),
            EventPayload::ContextUsage(_) => self.context = context_of(e),
            EventPayload::UsageSubscription(u) => {
                self.subscription.retain(|s| s.window != u.window);
                self.subscription.push(u.clone());
            }
            EventPayload::PlanUpdated(p) => self.plan = Some(p.clone()),
            EventPayload::TerminalOpened(t) => {
                self.terminals.retain(|x| x.terminal_id != t.terminal_id);
                self.terminals.push(TerminalInfo {
                    terminal_id: t.terminal_id.clone(),
                    name: t.name.clone().unwrap_or_else(|| t.cmd.clone()),
                    cmd: t.cmd.clone(),
                    kind: t.kind,
                    opened_at: e.ts,
                });
            }
            EventPayload::TerminalRenamed(t) => {
                if let Some(x) = self
                    .terminals
                    .iter_mut()
                    .find(|x| x.terminal_id == t.terminal_id)
                {
                    x.name.clone_from(&t.name);
                }
            }
            EventPayload::TerminalClosed(t) => {
                self.terminals.retain(|x| x.terminal_id != t.terminal_id);
            }
            EventPayload::FsChanged(_)
            | EventPayload::GitCommitCreated(_)
            | EventPayload::GitWorktreeCreated(_) => self.workspace_version = e.seq,
            _ => {}
        }
    }

    fn apply_transient(&mut self, e: &Event) {
        let Some(p) = e.payload() else {
            return;
        };
        match p {
            EventPayload::MessageDelta(d) => {
                upsert(&mut self.streaming, &d.message_id, &d.text, d.snapshot)
            }
            EventPayload::ReasoningDelta(d) => {
                upsert(&mut self.reasoning, &d.message_id, &d.text, d.snapshot)
            }
            EventPayload::ToolCallOutputDelta(d) => match self.tools.get(&d.call_id) {
                Some(&i) => {
                    if let Some(Item::Tool(t)) = self.items.get_mut(i) {
                        if d.snapshot {
                            t.output.clear();
                        }
                        t.output.push_str(&d.text);
                    }
                }
                None => {
                    let buf = self.pending_output.entry(d.call_id.clone()).or_default();
                    if d.snapshot {
                        buf.clear();
                    }
                    buf.push_str(&d.text);
                }
            },
            _ => return,
        }
        self.version += 1;
    }

    fn tool_mut(&mut self, call_id: &str) -> Option<&mut ToolItem> {
        let i = *self.tools.get(call_id)?;
        match self.items.get_mut(i) {
            Some(Item::Tool(t)) => Some(t),
            _ => None,
        }
    }

    /// Tool-Card an Position `index` auf- bzw. zuklappen.
    pub fn toggle_tool(&mut self, index: usize) -> bool {
        if let Some(Item::Tool(t)) = self.items.get_mut(index) {
            t.expanded = Some(!t.is_expanded());
            self.version += 1;
            true
        } else {
            false
        }
    }

    fn end_turn(&mut self) {
        self.streaming.clear();
        self.reasoning.clear();
    }

    fn note(&mut self, text: &str, tone: Tone) {
        self.items.push(Item::Note {
            text: text.to_owned(),
            tone,
        });
    }

    fn error(&mut self, problem: &Value) {
        self.items.push(Item::Error {
            title: problem["title"].as_str().unwrap_or("Fehler").to_owned(),
            detail: problem["detail"].as_str().map(str::to_owned),
        });
    }
}

fn upsert(list: &mut Vec<(String, String)>, id: &str, text: &str, snapshot: bool) {
    match list.iter_mut().find(|(k, _)| k == id) {
        Some((_, buf)) => {
            if snapshot {
                buf.clear();
            }
            buf.push_str(text);
        }
        None => list.push((id.to_owned(), text.to_owned())),
    }
}

/// Text der Inhaltsblöcke einer Nachricht (wie `textOf` der Web-UI).
pub fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .map(|b| match b {
            Value::String(s) => s.as_str(),
            b if b["type"] == "text" => b["text"].as_str().unwrap_or_default(),
            _ => "",
        })
        .collect()
}

/// Ziel eines Tool-Calls (wie `argSummary` der Web-UI): erste Zeile des ersten Textfelds aus
/// `command`, `file_path`, `path`, `url`, `pattern`, `query`, sonst kompaktes JSON.
pub fn arg_summary(args: &Value) -> String {
    for key in ["command", "file_path", "path", "url", "pattern", "query"] {
        if let Some(s) = args[key].as_str() {
            return s.lines().next().unwrap_or_default().to_owned();
        }
        if key == "command"
            && let Some(parts) = args[key].as_array()
        {
            let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            if !words.is_empty() {
                return words.join(" ");
            }
        }
    }
    match args {
        Value::Null => String::new(),
        Value::String(s) => s.lines().next().unwrap_or_default().to_owned(),
        Value::Object(m) if m.is_empty() => String::new(),
        other => other.to_string().chars().take(120).collect(),
    }
}

/// Deutsche Bezeichnung eines Tools nach dem Prototyp: Suche, Lesen, Bearbeiten, Shell.
pub fn tool_label(tool: &str, kind: Option<&str>) -> String {
    let by_kind = match kind {
        Some("shell") => Some("Shell"),
        Some("file_read") => Some("Lesen"),
        Some("file_edit" | "file_write") => Some("Bearbeiten"),
        Some("search") => Some("Suche"),
        Some("web_fetch" | "web_search") => Some("Web"),
        _ => None,
    };
    if let Some(l) = by_kind {
        return l.into();
    }
    match tool {
        "Bash" | "shell" | "exec_command" => "Shell".into(),
        "Read" => "Lesen".into(),
        "Edit" | "MultiEdit" | "Write" | "apply_patch" => "Bearbeiten".into(),
        "Grep" | "Glob" => "Suche".into(),
        "WebFetch" | "WebSearch" => "Web".into(),
        other => other.to_owned(),
    }
}

/// Zeilen eines Edits (wie `editPreview` der Web-UI): `(entfernt, hinzugefügt)`.
pub fn edit_lines(args: &Value) -> Option<(Vec<String>, Vec<String>)> {
    let lines = |v: &Value| -> Vec<String> {
        v.as_str()
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    };
    if args["old_string"].is_string() || args["new_string"].is_string() {
        return Some((lines(&args["old_string"]), lines(&args["new_string"])));
    }
    if let Some(edits) = args["edits"].as_array() {
        let mut del = Vec::new();
        let mut add = Vec::new();
        for e in edits {
            del.extend(lines(&e["old_string"]));
            add.extend(lines(&e["new_string"]));
        }
        return Some((del, add));
    }
    if args["content"].is_string() {
        return Some((Vec::new(), lines(&args["content"])));
    }
    let patch = args["input"].as_str().or_else(|| args["patch"].as_str())?;
    if !patch.contains("*** Begin Patch") {
        return None;
    }
    let mut del = Vec::new();
    let mut add = Vec::new();
    for l in patch.lines() {
        if l.starts_with("***") || l.starts_with("@@") {
            continue;
        }
        if let Some(r) = l.strip_prefix('+') {
            add.push(r.to_owned());
        } else if let Some(r) = l.strip_prefix('-') {
            del.push(r.to_owned());
        }
    }
    Some((del, add))
}

/// Dauer wie in der Web-UI: `0,2 s`.
pub fn seconds(ms: u64) -> String {
    let tenths = (ms + 50) / 100;
    format!("{},{} s", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beton_core::event::{
        ApprovalRequested, ApprovalResolved, MessageCompleted, TextDelta, ToolCallCompleted,
        ToolCallRequested,
    };
    use serde_json::json;

    fn sid() -> SessionId {
        "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap()
    }

    fn ev(seq: u64, p: EventPayload) -> Event {
        let mut e = Event::new(sid(), seq, Actor::default(), p);
        e.transient =
            e.body.event_type().persistence() == beton_core::event::Persistence::Transient;
        e
    }

    #[test]
    fn tui_001_deltas_stream_until_the_message_completes() {
        let mut m = SessionModel::new(sid());
        let delta = |t: &str| {
            ev(
                0,
                EventPayload::MessageDelta(TextDelta {
                    message_id: "m1".into(),
                    text: t.into(),
                    snapshot: false,
                }),
            )
        };
        m.apply(&delta("Hal"));
        m.apply(&delta("lo"));
        assert_eq!(m.streaming, [("m1".to_owned(), "Hallo".to_owned())]);
        m.apply(&ev(
            1,
            EventPayload::MessageCompleted(MessageCompleted {
                message_id: "m1".into(),
                role: MessageRole::Assistant,
                content: vec![json!({"type": "text", "text": "Hallo"})],
                author: None,
            }),
        ));
        assert!(m.streaming.is_empty());
        assert!(matches!(&m.items[..], [Item::Assistant { text, .. }] if text == "Hallo"));
        // Doppelt zugestellte dauerhafte Events zählen nicht doppelt.
        m.apply(&ev(
            1,
            EventPayload::Notice(beton_core::event::Notice::default()),
        ));
        assert_eq!(m.items.len(), 1);
    }

    #[test]
    fn tui_004_approval_card_reads_the_subject_like_the_web_ui() {
        let mut m = SessionModel::new(sid());
        m.apply(&ev(
            1,
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: "c1".into(),
                tool: "Bash".into(),
                kind: Some("shell".into()),
                args: json!({"command": "git push -u origin x"}),
                ..Default::default()
            }),
        ));
        let id = ApprovalId::new();
        m.apply(&ev(
            2,
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: id,
                kind: ApprovalKind::Policy,
                subject: json!({
                    "title": "Shell-Kommando bestätigen", "summary": "git push -u origin x",
                    "tool": {"name": "Bash", "kind": "shell", "args": {"command": "git push -u origin x"}},
                    "reasons": [{"rule_id": "git-push-fragen", "scope": "project", "set": "shop", "reason": "Pushes verlassen den Rechner."}],
                    "call_id": "c1"
                }),
                options: vec!["approve".into(), "approve_session".into(), "deny".into()],
                ..Default::default()
            }),
        ));
        let card = m.pending_approvals().next().unwrap().clone();
        assert_eq!(card.tool, "Bash");
        assert_eq!(card.summary, "git push -u origin x");
        assert_eq!(card.reasons[0].rule_id, "git-push-fragen");
        assert!(card.allows_session());
        assert!(m.waiting_for_approval("c1"));
        assert_eq!(m.glyph(), Glyph::Waiting);
        m.apply(&ev(
            3,
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: id,
                decision: ApprovalDecision::Allow,
                actor: Actor::User {
                    id: "usr_local".parse().unwrap(),
                    device_id: None,
                },
                ..Default::default()
            }),
        ));
        assert_eq!(m.pending_count(), 0);
        let Item::Approval(card) = &m.items[1] else {
            panic!()
        };
        assert!(card.resolution.as_ref().unwrap().by_local_user);
        m.apply(&ev(
            4,
            EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id: "c1".into(),
                status: ToolStatus::Ok,
                duration_ms: 4_800,
                ..Default::default()
            }),
        ));
        let Item::Tool(t) = &m.items[0] else { panic!() };
        assert_eq!(t.state, ToolState::Completed);
        assert_eq!(seconds(t.duration_ms.unwrap()), "4,8 s");
    }

    #[test]
    fn tui_001_tool_labels_and_targets_follow_the_prototype() {
        assert_eq!(tool_label("Grep", None), "Suche");
        assert_eq!(tool_label("Read", Some("file_read")), "Lesen");
        assert_eq!(tool_label("Edit", Some("file_edit")), "Bearbeiten");
        assert_eq!(tool_label("Bash", Some("shell")), "Shell");
        assert_eq!(tool_label("mcp__x__y", Some("mcp")), "mcp__x__y");
        assert_eq!(
            arg_summary(&json!({"command": "pnpm vitest\nrun"})),
            "pnpm vitest"
        );
        assert_eq!(arg_summary(&json!({"file_path": "src/a.ts"})), "src/a.ts");
        assert_eq!(arg_summary(&json!({})), "");
        assert_eq!(seconds(200), "0,2 s");
        assert_eq!(seconds(0), "0,0 s");
        let (del, add) = edit_lines(&json!({"old_string": "a", "new_string": "b\nc"})).unwrap();
        assert_eq!((del.len(), add.len()), (1, 2));
    }
}
