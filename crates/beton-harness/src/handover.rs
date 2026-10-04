//! Handover-Kontext für Fork und Harness-Wechsel (HAR-018) und der Plan, mit dem ein Runner
//! eine abgezweigte Session startet (HAR-019, SES-006, SES-007).
//!
//! Aus dem Event-Log entsteht ein harness-neutraler [`HandoverContext`]. Der Ziel-Adapter
//! bekommt ihn je Capability `fork_history`:
//! - `rebuild`: gleicher Harness, native History (Vendor-Fork oder rekonstruierte
//!   Session-Datei, siehe [`crate::HarnessAdapter::rebuild_history`]),
//! - `preamble`: ein Markdown-Dokument `.beton/handover/<session>.md` im Workspace plus eine
//!   erste Nachricht mit Kurzfassung und Verweis ([`render`]).
//!
//! Das Rendering ist deterministisch und kommt ohne LLM aus: gleicher Log-Ausschnitt →
//! byte-gleiches Markdown (HAR-018 AC2). Es enthält keine Zeitstempel und keine Event-IDs.
//! Budget: höchstens 40 % des Ziel-Kontextfensters (Tokens grob als Bytes / 4). Kürzungsregeln
//! in dieser Reihenfolge, jeweils nur für ältere Turns: (1) Tool-Results über 2 KiB auf Kopf
//! und Ende kürzen, (2) Reasoning entfernen, (3) Turns auf Nutzer-Nachricht und
//! Tool-Call-Einzeiler reduzieren, (4) älteste Turns auslassen
//! (`[… N Turns ausgelassen …]`). Die letzten [`FULL_TURNS`] Turns bleiben vollständig.

use std::fmt::Write as _;
use std::path::PathBuf;

use beton_core::event::{EventPayload, ForkReason, MessageRole, ToolStatus};
use beton_core::id::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capabilities::{Capabilities, ForkHistory};

/// Anteil des Kontextfensters, den die Präambel höchstens belegen darf (Prozent).
pub const BUDGET_PERCENT: u64 = 40;
/// So viele jüngste Turns bleiben immer vollständig.
pub const FULL_TURNS: usize = 3;
/// Tool-Results über dieser Größe werden in älteren Turns gekürzt (Regel 1).
pub const TOOL_RESULT_LIMIT: usize = 2 * 1024;
/// Kopf und Ende eines gekürzten Tool-Results.
const RESULT_HEAD: usize = 1024;
const RESULT_TAIL: usize = 512;
/// Obergrenzen der ersten Nachricht (Kurzfassung).
const BRIEF_GOAL: usize = 500;
const BRIEF_LAST: usize = 600;
const BRIEF_FILES: usize = 8;
const BRIEF_TODOS: usize = 5;

/// Verzeichnis der Handover-Dokumente im Workspace.
pub const HANDOVER_DIR: &str = ".beton/handover";

/// Relativer Pfad des Handover-Dokuments einer Session im Workspace.
pub fn document_path(session: SessionId) -> PathBuf {
    PathBuf::from(HANDOVER_DIR).join(format!("{session}.md"))
}

/// Grobe, deterministische Token-Schätzung (Bytes / 4, aufgerundet).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

/// Lesbarer Name eines Harness für Texte, z. B. „Claude Code“.
pub fn harness_label(harness: &str) -> String {
    match harness {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "fake" => "Fake-Harness".into(),
        other => match other.split_once(':') {
            Some(("acp", agent)) => format!("{agent} (ACP)"),
            Some(("direct", provider)) => format!("{provider} (Direkt-API)"),
            _ => other.to_owned(),
        },
    }
}

/// Gehört ein Event zum Inhalt einer Session, also zu dem, was ein Fork übernimmt (SES-006)?
/// Lebenszyklus-, Runner-, Queue-, Freigabe- und Kosten-Events gehören der Quelle.
pub fn is_content(payload: &EventPayload) -> bool {
    matches!(
        payload,
        EventPayload::MessageCompleted(_)
            | EventPayload::ReasoningCompleted(_)
            | EventPayload::ToolCallRequested(_)
            | EventPayload::ToolCallStarted(_)
            | EventPayload::ToolCallCompleted(_)
            | EventPayload::TurnStarted(_)
            | EventPayload::TurnCompleted(_)
            | EventPayload::TurnFailed(_)
            | EventPayload::TurnInterrupted(_)
            | EventPayload::FsChanged(_)
    )
}

/// Wie [`is_content`], nach dem Typnamen (auch für ausgelagerte Nutzlasten).
pub fn is_content_type(name: &str) -> bool {
    matches!(
        name,
        "message.completed"
            | "reasoning.completed"
            | "tool.call.requested"
            | "tool.call.started"
            | "tool.call.completed"
            | "turn.started"
            | "turn.completed"
            | "turn.failed"
            | "turn.interrupted"
            | "fs.changed"
    )
}

/// Normalisiert einen Fork-Punkt auf das letzte vollständige Turn-Ende bis einschließlich
/// `at_seq` (SES-006 AC2). Ein Turn beginnt mit der Nutzer-Nachricht bzw. `turn.started` und
/// endet mit `turn.completed`, `turn.failed` oder `turn.interrupted`. Erwartet das Log in
/// `seq`-Reihenfolge; vor dem ersten Turn ist jede Position eine Grenze.
pub fn normalize_fork_point<'a>(
    log: impl IntoIterator<Item = (u64, &'a EventPayload)>,
    at_seq: u64,
) -> u64 {
    let mut open = false;
    let mut boundary = 0;
    for (seq, payload) in log {
        if seq > at_seq {
            break;
        }
        match payload {
            EventPayload::MessageCompleted(m) if m.role == MessageRole::User => open = true,
            EventPayload::TurnStarted(_) => open = true,
            EventPayload::TurnCompleted(_)
            | EventPayload::TurnFailed(_)
            | EventPayload::TurnInterrupted(_) => open = false,
            _ => {}
        }
        if !open {
            boundary = seq;
        }
    }
    boundary
}

/// Ein Inhalts-Event des Verlaufs mit aufgelöster Nutzlast und, falls gespeichert, dem
/// Original-Payload des Harness (`raw`, verlustfreier Rebuild nach HAR-019).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub payload: EventPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Wofür der Plan gilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    /// Neue Session ab einem Event einer anderen (SES-006).
    Fork,
    /// Fortsetzen einer Session ohne native Referenz, z. B. eines Imports (HAR-019 AC2).
    Resume,
}

/// Was ein Runner braucht, um eine Session mit übernommenem Verlauf zu starten. Der Server
/// schreibt ihn beim Fork; der Runner wählt daraus `native`, `rebuild` oder `preamble`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForkPlan {
    pub kind: PlanKind,
    pub from_session: SessionId,
    pub from_harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    /// Effektiver Fork-Punkt (letztes vollständiges Turn-Ende).
    pub at_seq: u64,
    pub reason: ForkReason,
    /// Native Referenz der Quelle, nur wenn die Quelle nach `at_seq` nichts mehr enthält: Dann
    /// kann der Vendor-Mechanismus den Verlauf exakt übernehmen (z. B. `--fork-session`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
    /// Inhalts-Events bis einschließlich `at_seq`.
    pub history: Vec<HistoryEvent>,
}

/// Wie der Verlauf beim Ziel-Harness ankommt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryChoice {
    /// Vendor-Mechanismus mit der nativen Referenz der Quelle (Claude `--resume --fork-session`).
    Native { reference: String },
    /// Native Session aus dem Event-Log rekonstruieren (HAR-019).
    Rebuild,
    /// Handover-Dokument und erste Nachricht (HAR-018).
    Preamble,
}

impl ForkPlan {
    /// Wahl nach Capability `fork_history` (HAR-018): nur derselbe Harness mit `rebuild`
    /// übernimmt den Verlauf nativ; sonst Präambel.
    pub fn choose(&self, target_harness: &str, caps: &Capabilities) -> HistoryChoice {
        if caps.fork_history != ForkHistory::Rebuild || self.from_harness != target_harness {
            return HistoryChoice::Preamble;
        }
        match (&self.native_ref, self.kind) {
            (Some(reference), PlanKind::Fork) => HistoryChoice::Native {
                reference: reference.clone(),
            },
            _ => HistoryChoice::Rebuild,
        }
    }

    /// Der harness-neutrale Handover-Kontext dieses Plans.
    pub fn context(&self) -> HandoverContext {
        HandoverContext::from_history(
            &self.history,
            Source {
                session: self.from_session,
                title: self.source_title.clone(),
                harness: self.from_harness.clone(),
                up_to_seq: self.at_seq,
                worktree: self.worktree.clone(),
                branch: self.branch.clone(),
                agent_ref: self.agent_ref.clone(),
            },
        )
    }
}

/// Herkunft eines Handover-Kontexts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub session: SessionId,
    pub title: Option<String>,
    pub harness: String,
    pub up_to_seq: u64,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub agent_ref: Option<String>,
}

/// Harness-neutraler Übergabe-Kontext (HAR-018).
#[derive(Debug, Clone, PartialEq)]
pub struct HandoverContext {
    pub source_session: SessionId,
    pub source_title: Option<String>,
    pub source_harness: String,
    pub up_to_seq: u64,
    pub transcript: Vec<HandoverTurn>,
    pub changed_files: Vec<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub plan: Vec<PlanItem>,
    pub open_todos: Vec<String>,
    pub agent_ref: Option<String>,
}

/// Ein Schritt aus dem letzten Plan bzw. der Todo-Liste des Agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub text: String,
    pub done: bool,
}

/// Ein Turn des Verlaufs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HandoverTurn {
    /// Nutzer-Nachrichten (die erste startet den Turn, weitere kamen per Steer).
    pub user: Vec<String>,
    pub items: Vec<TurnItem>,
    pub outcome: TurnOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TurnOutcome {
    /// Kein Turn-Ende im Ausschnitt.
    #[default]
    Open,
    Completed,
    Failed(String),
    Interrupted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnItem {
    Assistant(String),
    Reasoning(String),
    Tool(ToolItem),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolItem {
    pub call_id: String,
    pub tool: String,
    pub args: Value,
    pub status: Option<ToolStatus>,
    pub result: Option<String>,
}

fn content_text(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn result_text(result: &Value) -> String {
    match result {
        Value::String(s) => s.clone(),
        Value::Array(items) if items.iter().all(|i| i["text"].is_string()) => items
            .iter()
            .filter_map(|i| i["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Datei-Pfade aus Edit-artigen Tool-Calls (ergänzt `fs.changed`).
fn edited_path(tool: &str, args: &Value) -> Option<String> {
    let edit = matches!(
        tool,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "write_file" | "edit_file"
    );
    if !edit {
        return None;
    }
    ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|k| args[*k].as_str())
        .map(str::to_owned)
}

/// Plan aus `TodoWrite` (Claude) bzw. `update_plan` (Codex).
fn plan_from(tool: &str, args: &Value) -> Option<Vec<PlanItem>> {
    let (list, text_key) = match tool {
        "TodoWrite" => (args["todos"].as_array()?, "content"),
        "update_plan" => (args["plan"].as_array()?, "step"),
        _ => return None,
    };
    Some(
        list.iter()
            .filter_map(|i| {
                Some(PlanItem {
                    text: i[text_key].as_str()?.to_owned(),
                    done: i["status"] == "completed",
                })
            })
            .collect(),
    )
}

impl HandoverContext {
    /// Baut den Kontext aus Inhalts-Events (in `seq`-Reihenfolge).
    pub fn from_history(history: &[HistoryEvent], source: Source) -> Self {
        let mut turns: Vec<HandoverTurn> = Vec::new();
        let mut ended = true;
        let mut changed: Vec<String> = Vec::new();
        let mut plan: Vec<PlanItem> = Vec::new();
        fn touch(path: &str, changed: &mut Vec<String>) {
            if !changed.iter().any(|p| p == path) {
                changed.push(path.to_owned());
            }
        }
        for e in history {
            let start_new = |turns: &mut Vec<HandoverTurn>, ended: &mut bool| {
                if *ended || turns.is_empty() {
                    turns.push(HandoverTurn::default());
                    *ended = false;
                }
            };
            match &e.payload {
                EventPayload::MessageCompleted(m) if m.role == MessageRole::User => {
                    start_new(&mut turns, &mut ended);
                    if let Some(t) = turns.last_mut() {
                        t.user.push(content_text(&m.content));
                    }
                }
                EventPayload::TurnStarted(_) => start_new(&mut turns, &mut ended),
                EventPayload::MessageCompleted(m) => {
                    start_new(&mut turns, &mut ended);
                    let text = content_text(&m.content);
                    if let Some(t) = turns.last_mut()
                        && !text.is_empty()
                    {
                        t.items.push(TurnItem::Assistant(text));
                    }
                }
                EventPayload::ReasoningCompleted(r) => {
                    if let (Some(t), Some(s)) = (turns.last_mut(), &r.summary)
                        && !s.is_empty()
                    {
                        t.items.push(TurnItem::Reasoning(s.clone()));
                    }
                }
                EventPayload::ToolCallRequested(c) => {
                    start_new(&mut turns, &mut ended);
                    if let Some(p) = edited_path(&c.tool, &c.args) {
                        touch(&p, &mut changed);
                    }
                    if let Some(p) = plan_from(&c.tool, &c.args) {
                        plan = p;
                    }
                    if let Some(t) = turns.last_mut() {
                        t.items.push(TurnItem::Tool(ToolItem {
                            call_id: c.call_id.clone(),
                            tool: c.tool.clone(),
                            args: c.args.clone(),
                            status: None,
                            result: None,
                        }));
                    }
                }
                EventPayload::ToolCallCompleted(c) => {
                    let found = turns.iter_mut().rev().find_map(|t| {
                        t.items.iter_mut().rev().find_map(|i| match i {
                            TurnItem::Tool(tool) if tool.call_id == c.call_id => Some(tool),
                            _ => None,
                        })
                    });
                    if let Some(tool) = found {
                        tool.status = Some(c.status);
                        tool.result = c.result.as_ref().map(result_text);
                    }
                }
                EventPayload::TurnCompleted(_) => {
                    if let Some(t) = turns.last_mut() {
                        t.outcome = TurnOutcome::Completed;
                    }
                    ended = true;
                }
                EventPayload::TurnFailed(f) => {
                    if let Some(t) = turns.last_mut() {
                        let why = f.problem["title"]
                            .as_str()
                            .or_else(|| f.problem["detail"].as_str())
                            .unwrap_or("Fehler")
                            .to_owned();
                        t.outcome = TurnOutcome::Failed(why);
                    }
                    ended = true;
                }
                EventPayload::TurnInterrupted(_) => {
                    if let Some(t) = turns.last_mut() {
                        t.outcome = TurnOutcome::Interrupted;
                    }
                    ended = true;
                }
                EventPayload::FsChanged(f) => {
                    for c in &f.changes {
                        touch(&c.path, &mut changed);
                    }
                }
                _ => {}
            }
        }
        let open_todos = plan
            .iter()
            .filter(|p| !p.done)
            .map(|p| p.text.clone())
            .collect();
        Self {
            source_session: source.session,
            source_title: source.title,
            source_harness: source.harness,
            up_to_seq: source.up_to_seq,
            transcript: turns,
            changed_files: changed,
            worktree: source.worktree,
            branch: source.branch,
            plan,
            open_todos,
            agent_ref: source.agent_ref,
        }
    }

    /// Ziel der Session: die erste Nutzer-Nachricht.
    pub fn goal(&self) -> Option<&str> {
        self.transcript
            .iter()
            .flat_map(|t| t.user.iter())
            .map(String::as_str)
            .find(|u| !u.trim().is_empty())
    }

    /// Letzte Antwort des Agents.
    pub fn last_answer(&self) -> Option<&str> {
        self.transcript.iter().rev().find_map(|t| {
            t.items.iter().rev().find_map(|i| match i {
                TurnItem::Assistant(a) => Some(a.as_str()),
                _ => None,
            })
        })
    }

    fn title(&self) -> String {
        self.source_title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.source_session.to_string())
    }
}

/// Ergebnis des Präambel-Renderings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preamble {
    /// Inhalt von `.beton/handover/<session>.md`.
    pub document: String,
    /// Erste Nachricht an den Ziel-Harness (Kurzfassung und Verweis), ohne die Eingabe des
    /// Nutzers.
    pub brief: String,
    /// Geschätzte Tokens des Dokuments.
    pub tokens: u64,
    /// Budget in Tokens (40 % des Kontextfensters).
    pub budget: u64,
    pub turns: usize,
    /// Davon unverändert enthalten.
    pub full_turns: usize,
    pub omitted_turns: usize,
}

impl Preamble {
    /// Die erste Nachricht an den Ziel-Harness: Kurzfassung, Trenner, Eingabe des Nutzers.
    pub fn first_message(brief: &str, input: &str) -> String {
        format!("{brief}\n\n---\n\n{input}")
    }
}

/// Kürzungsstufe eines Turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Full,
    /// Regel 1: Tool-Results gekürzt.
    TrimmedResults,
    /// Regel 2: zusätzlich ohne Reasoning.
    NoReasoning,
    /// Regel 3: nur Nutzer-Nachricht und Tool-Call-Einzeiler.
    Condensed,
    /// Regel 4: ausgelassen.
    Omitted,
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Kopf und Ende eines langen Textes mit Marker dazwischen.
fn head_tail(text: &str, head: usize, tail: usize) -> String {
    if text.len() <= head + tail {
        return text.to_owned();
    }
    let h = floor_boundary(text, head);
    let t = ceil_boundary(text, text.len() - tail);
    format!(
        "{}\n[… {} Bytes gekürzt …]\n{}",
        &text[..h],
        t - h,
        &text[t..]
    )
}

/// Auf eine Zeile mit höchstens `max` Bytes kürzen.
fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() <= max {
        return flat;
    }
    format!("{} …", &flat[..floor_boundary(&flat, max)])
}

/// Zaun für Code-Blöcke, der im Inhalt nicht vorkommt.
fn fence(text: &str) -> String {
    let mut f = "```".to_owned();
    while text.contains(&f) {
        f.push('`');
    }
    f
}

/// Einzeiler eines Tool-Calls, z. B. `` `Bash` cargo test → ok ``.
fn tool_line(t: &ToolItem) -> String {
    let arg = ["command", "file_path", "path", "pattern", "url", "query"]
        .iter()
        .find_map(|k| t.args[*k].as_str().map(str::to_owned))
        .unwrap_or_else(|| {
            if t.args.is_null() {
                String::new()
            } else {
                t.args.to_string()
            }
        });
    let status = match t.status {
        Some(ToolStatus::Ok) => "ok",
        Some(ToolStatus::Error) => "Fehler",
        Some(ToolStatus::Denied) => "abgelehnt",
        Some(_) => "beendet",
        None => "ohne Ergebnis",
    };
    let arg = one_line(&arg, 160);
    if arg.is_empty() {
        format!("- `{}` → {status}", t.tool)
    } else {
        format!("- `{}` {arg} → {status}", t.tool)
    }
}

fn render_turn(n: usize, turn: &HandoverTurn, level: Level) -> String {
    let mut out = String::new();
    if level == Level::Omitted {
        return out;
    }
    let condensed = level == Level::Condensed;
    let _ = writeln!(
        out,
        "### Turn {n}{}\n",
        if condensed { " (gekürzt)" } else { "" }
    );
    for (i, u) in turn.user.iter().enumerate() {
        let label = if i == 0 {
            "Nutzer"
        } else {
            "Nutzer (nachgereicht)"
        };
        let text = if condensed {
            head_tail(u, RESULT_HEAD, RESULT_TAIL)
        } else {
            u.clone()
        };
        let _ = writeln!(out, "**{label}:** {}\n", text.trim_end());
    }
    for item in &turn.items {
        match item {
            TurnItem::Assistant(a) if !condensed => {
                let _ = writeln!(out, "**Agent:** {}\n", a.trim_end());
            }
            TurnItem::Reasoning(r) if level < Level::NoReasoning => {
                let _ = writeln!(out, "**Überlegung:** {}\n", r.trim_end());
            }
            TurnItem::Tool(t) => {
                let _ = writeln!(out, "{}", tool_line(t));
                if !condensed && let Some(result) = t.result.as_deref().filter(|r| !r.is_empty()) {
                    let shown =
                        if level >= Level::TrimmedResults && result.len() > TOOL_RESULT_LIMIT {
                            head_tail(result, RESULT_HEAD, RESULT_TAIL)
                        } else {
                            result.to_owned()
                        };
                    let f = fence(&shown);
                    let _ = writeln!(out, "\n{f}text\n{}\n{f}\n", shown.trim_end());
                }
            }
            _ => {}
        }
    }
    let mut out = out.trim_end().to_owned();
    out.push_str("\n\n");
    match &turn.outcome {
        TurnOutcome::Failed(why) => {
            let _ = writeln!(out, "_Turn fehlgeschlagen: {}_\n", one_line(why, 300));
        }
        TurnOutcome::Interrupted => out.push_str("_Turn abgebrochen._\n\n"),
        TurnOutcome::Open => out.push_str("_Turn ohne Abschluss._\n\n"),
        TurnOutcome::Completed => {}
    }
    out
}

fn render_header(ctx: &HandoverContext, target: &str, full: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Übergabe: {}\n", one_line(&ctx.title(), 200));
    let _ = writeln!(
        out,
        "Diese Session setzt `{}` ({}) ab Ereignis {} auf {} fort. beton hat das Dokument ohne \
         LLM aus dem Verlauf erzeugt; die letzten {full} Turns sind vollständig, ältere Turns \
         können gekürzt sein.\n",
        ctx.source_session,
        harness_label(&ctx.source_harness),
        ctx.up_to_seq,
        harness_label(target),
    );
    let _ = writeln!(out, "## Überblick\n");
    if let Some(goal) = ctx.goal() {
        let _ = writeln!(out, "- Ziel: {}", one_line(&head_tail(goal, 1000, 0), 1000));
    }
    if let Some(wt) = &ctx.worktree {
        let _ = writeln!(out, "- Worktree: `{wt}`");
    }
    if let Some(b) = &ctx.branch {
        let _ = writeln!(out, "- Branch: `{b}`");
    }
    if let Some(a) = &ctx.agent_ref {
        let _ = writeln!(out, "- Agent: `{a}`");
    }
    let _ = writeln!(out, "- Turns im Verlauf: {}", ctx.transcript.len());
    out.push('\n');
    if !ctx.plan.is_empty() {
        let _ = writeln!(out, "## Plan\n");
        for p in &ctx.plan {
            let _ = writeln!(
                out,
                "- [{}] {}",
                if p.done { "x" } else { " " },
                one_line(&p.text, 300)
            );
        }
        out.push('\n');
    }
    if !ctx.open_todos.is_empty() {
        let _ = writeln!(out, "## Offene Aufgaben\n");
        for t in &ctx.open_todos {
            let _ = writeln!(out, "- {}", one_line(t, 300));
        }
        out.push('\n');
    }
    if !ctx.changed_files.is_empty() {
        let _ = writeln!(out, "## Geänderte Dateien\n");
        for f in &ctx.changed_files {
            let _ = writeln!(out, "- `{f}`");
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Verlauf\n");
    out
}

fn omitted_marker(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("[… {n} Turns ausgelassen …]\n\n")
    }
}

/// Rendert Dokument und erste Nachricht für den Ziel-Harness (HAR-018).
///
/// `context_window` ist das Kontextfenster des Ziels in Tokens, `document` der Pfad des
/// Dokuments im Workspace (für den Verweis in der ersten Nachricht).
pub fn render(
    ctx: &HandoverContext,
    target_harness: &str,
    context_window: u64,
    document: &str,
) -> Preamble {
    let budget = context_window.saturating_mul(BUDGET_PERCENT) / 100;
    let n = ctx.transcript.len();
    let older = n.saturating_sub(FULL_TURNS);
    let full = n.min(FULL_TURNS);
    let header = render_header(ctx, target_harness, full);
    let mut levels = vec![Level::Full; n];
    let mut parts: Vec<String> = ctx
        .transcript
        .iter()
        .enumerate()
        .map(|(i, t)| render_turn(i + 1, t, Level::Full))
        .collect();
    let total = |parts: &[String], levels: &[Level]| -> u64 {
        let omitted = levels.iter().filter(|l| **l == Level::Omitted).count();
        let bytes = header.len()
            + omitted_marker(omitted).len()
            + parts.iter().map(String::len).sum::<usize>();
        (bytes as u64).div_ceil(4)
    };
    let fits = |parts: &[String], levels: &[Level]| total(parts, levels) <= budget;
    // Regeln 1 und 2 gelten für alle älteren Turns auf einmal, 3 und 4 vom ältesten an.
    'rules: for rule in [Level::TrimmedResults, Level::NoReasoning] {
        if fits(&parts, &levels) {
            break 'rules;
        }
        for i in 0..older {
            levels[i] = rule;
            parts[i] = render_turn(i + 1, &ctx.transcript[i], rule);
        }
    }
    for rule in [Level::Condensed, Level::Omitted] {
        for i in 0..older {
            if fits(&parts, &levels) {
                break;
            }
            levels[i] = rule;
            parts[i] = render_turn(i + 1, &ctx.transcript[i], rule);
        }
    }
    let omitted = levels.iter().filter(|l| **l == Level::Omitted).count();
    let mut document_text = header;
    document_text.push_str(&omitted_marker(omitted));
    for p in &parts {
        document_text.push_str(p);
    }
    if n == 0 {
        document_text.push_str("Der Verlauf enthält noch keine Turns.\n");
    }
    let tokens = estimate_tokens(&document_text);
    let full_turns = levels.iter().filter(|l| **l == Level::Full).count();
    let brief = render_brief(
        ctx,
        target_harness,
        document,
        n,
        full_turns,
        tokens,
        context_window,
    );
    Preamble {
        document: document_text,
        brief,
        tokens,
        budget,
        turns: n,
        full_turns,
        omitted_turns: omitted,
    }
}

fn render_brief(
    ctx: &HandoverContext,
    target: &str,
    document: &str,
    turns: usize,
    full: usize,
    tokens: u64,
    context_window: u64,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "[beton · Übergabe] Du setzt die Session „{}“ fort, die bisher mit {} lief (bis \
         Ereignis {}). Du arbeitest jetzt als {}.",
        one_line(&ctx.title(), 200),
        harness_label(&ctx.source_harness),
        ctx.up_to_seq,
        harness_label(target),
    );
    if let Some(goal) = ctx.goal() {
        let _ = writeln!(out, "Ziel: {}", one_line(goal, BRIEF_GOAL));
    }
    if !ctx.changed_files.is_empty() {
        let shown: Vec<String> = ctx
            .changed_files
            .iter()
            .rev()
            .take(BRIEF_FILES)
            .map(|f| format!("`{f}`"))
            .collect();
        let more = ctx.changed_files.len().saturating_sub(BRIEF_FILES);
        let _ = writeln!(
            out,
            "Zuletzt geändert: {}{}",
            shown.join(", "),
            if more > 0 {
                format!(" (+{more} weitere)")
            } else {
                String::new()
            }
        );
    }
    if let Some(b) = &ctx.branch {
        let _ = writeln!(out, "Branch: `{b}`");
    }
    if !ctx.open_todos.is_empty() {
        let shown: Vec<String> = ctx
            .open_todos
            .iter()
            .take(BRIEF_TODOS)
            .map(|t| one_line(t, 120))
            .collect();
        let more = ctx.open_todos.len().saturating_sub(BRIEF_TODOS);
        let _ = writeln!(
            out,
            "Offen: {}{}",
            shown.join("; "),
            if more > 0 {
                format!(" (+{more})")
            } else {
                String::new()
            }
        );
    }
    if let Some(last) = ctx.last_answer() {
        let _ = writeln!(out, "Letzter Stand: {}", one_line(last, BRIEF_LAST));
    }
    let percent = (tokens * 100).div_ceil(context_window.max(1));
    let _ = write!(
        out,
        "Den Verlauf ({turns} Turns, davon {full} vollständig, ca. {tokens} Tokens, {percent} % \
         des Kontextfensters) findest du in `{document}`. Lies die Datei, bevor du weitermachst."
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use beton_core::event::{MessageCompleted, TurnCompleted, TurnStarted};
    use beton_core::id::{PrincipalId, UserId};
    use serde_json::json;

    fn user(text: &str) -> EventPayload {
        EventPayload::MessageCompleted(MessageCompleted {
            message_id: "u".into(),
            role: MessageRole::User,
            content: vec![json!({"type": "text", "text": text})],
            author: Some(PrincipalId::User(UserId::LOCAL)),
        })
    }

    fn started() -> EventPayload {
        EventPayload::TurnStarted(TurnStarted {
            turn_id: TurnId::new(),
            input_id: None,
            author: PrincipalId::User(UserId::LOCAL),
        })
    }

    fn done() -> EventPayload {
        EventPayload::TurnCompleted(TurnCompleted {
            turn_id: TurnId::new(),
            stop_reason: "end_turn".into(),
            usage_summary: json!({}),
        })
    }

    #[test]
    fn ses_006_ac2_mid_turn_fork_point_moves_to_previous_turn_end() {
        let other = EventPayload::default();
        // 1 created, 2 user, 3 started, 4 msg, 5 completed, 6 status, 7 user, 8 started, 9 msg
        let log = [
            other.clone(),
            user("a"),
            started(),
            other.clone(),
            done(),
            other.clone(),
            user("b"),
            started(),
            other,
        ];
        let seq =
            |at| normalize_fork_point(log.iter().enumerate().map(|(i, p)| (i as u64 + 1, p)), at);
        assert_eq!(seq(9), 6);
        assert_eq!(seq(8), 6);
        assert_eq!(seq(7), 6, "eine Nutzer-Nachricht eröffnet den Turn");
        assert_eq!(seq(6), 6);
        assert_eq!(seq(5), 5);
        assert_eq!(seq(4), 1);
        assert_eq!(seq(1), 1);
    }

    #[test]
    fn head_tail_respects_char_boundaries() {
        let text = "ä".repeat(3000);
        let cut = head_tail(&text, RESULT_HEAD, RESULT_TAIL);
        assert!(cut.contains("Bytes gekürzt"));
        assert!(cut.len() < text.len());
    }

    #[test]
    fn choice_follows_fork_history_capability() {
        let mut caps = crate::fake::default_capabilities();
        let plan = ForkPlan {
            kind: PlanKind::Fork,
            from_session: SessionId::new(),
            from_harness: "claude".into(),
            source_title: None,
            at_seq: 5,
            reason: ForkReason::User,
            native_ref: Some("abc".into()),
            worktree: None,
            branch: None,
            agent_ref: None,
            history: Vec::new(),
        };
        caps.fork_history = ForkHistory::Rebuild;
        assert_eq!(
            plan.choose("claude", &caps),
            HistoryChoice::Native {
                reference: "abc".into()
            }
        );
        assert_eq!(plan.choose("codex", &caps), HistoryChoice::Preamble);
        let rebuilt = ForkPlan {
            native_ref: None,
            ..plan.clone()
        };
        assert_eq!(rebuilt.choose("claude", &caps), HistoryChoice::Rebuild);
        caps.fork_history = ForkHistory::Preamble;
        assert_eq!(plan.choose("claude", &caps), HistoryChoice::Preamble);
    }
}
