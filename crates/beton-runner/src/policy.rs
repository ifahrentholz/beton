//! Policy-Gate des Runners (03-policies.md: POL-003, POL-004, POL-006, POL-024, POL-025; #182).
//!
//! Das Gate entscheidet an allen Phasen einer Session: vor dem Start (`session_start`), vor
//! jedem Turn bzw. Model-Request (`model_request`), vor jedem Tool-Call (`tool_call`, auch
//! System-Tools des Servers `beton` mit `tool.kind = "system"`) und vor dem Weiterreichen
//! eines Ergebnisses (`tool_result`). Jede Auswertung erzeugt genau ein `policy.decision`.
//!
//! - **Fail closed:** Ladefehler des Kontexts, Fehler oder Zeitüberschreitung der Engine,
//!   nicht normalisierbare Tool-Calls und fehlende Antworten führen zu `deny`.
//! - **Hot-Reload (POL-001):** Vor jeder Auswertung liest das Gate die Policy-Verzeichnisse
//!   neu (`PolicyStore::refresh`) und schreibt `policy.set_changed`/`policy.set_invalid`.
//! - **Rückfragen des Vendors:** Eine Rückfrage (Claude `can_use_tool`, Codex-Approval,
//!   ACP-Permission) nutzt die Entscheidung des `PreToolUse`-Hooks zum selben Call. Erlaubt die
//!   Policy, fragt beton bei einer Rückfrage den Menschen (Approval-Card, WEB-018) – außer im
//!   Modus `yolo`, bei System-Tools (#151), bei Rückfragen, die der Permission-Mode selbst
//!   beantwortet (`AutoApproval`), und wenn der Mensch schon über eine `ask`-Regel entschieden
//!   hat. `ask` der Policy erzeugt genau eine Approval-Card (`kind: policy`).
//! - Freigaben und Policy-Ereignisse gehen als [`GateEvent`] an die Schleife des Runners; sie
//!   versieht sie mit Session, Reihenfolge und Bezug auf das `tool.call.requested`.

mod dry;

pub use dry::{QueryBudgetSource, QueryTaken, SnapshotSink};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, BudgetExhausted,
    EventPayload, Notice, NoticeLevel, PolicyNotified, PolicyRuleIssue, RawJson, RememberScope,
    ResolvedVia, SystemComponent, TimeoutAction,
};
use beton_core::id::{ApprovalId, PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use beton_harness::{
    Gate, GateDecision, GateOrigin, GateRequest, HarnessId, ModelGateDecision, ModelGateRequest,
    PermissionMode, ResultGateRequest,
};
use beton_policy::Phase;
use beton_policy::approval::{ApprovalPlan, plan};
use beton_policy::audit::{DecisionMeta, decision_event, subject_hash};
use beton_policy::cel::Limits;
use beton_policy::cel::ext::{LexicalResolver, PathResolver};
use beton_policy::context::Activation;
use beton_policy::doc::StateScope;
use beton_policy::enforcement::{Level, Matrix, StartCheck, check_start};
use beton_policy::engine::{
    Decision, EffectiveSet, EvalOptions, Modifications, Outcome, Reason, RuleError, merge_patch,
};
use beton_policy::grants::SessionGrants;
use beton_policy::load::{RuleRef, Scope};
use beton_policy::normalize::{ToolSource, harness_family, normalize_source};
use beton_policy::state::{FinalOutcome, PolicyState, StateChange, StateOp, effects_due};
use beton_policy::store::{PolicyStore, StoreEvent};
use beton_policy::vars::{
    ActorVars, AgentVars, ApprovalsVars, CelTimestamp, HostVars, OrgVars, RequestVars, ResultVars,
    RunVars, SessionVars, StartVars, StateVars, ToolVars, UserVars, Vars,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// Höchstdauer einer Auswertung inklusive Kontextaufbau; danach `deny` (POL-024 AC3).
pub const EVAL_TIMEOUT: Duration = Duration::from_millis(250);
/// Höchstdauer für Verbrauchswerte vom Server (`session.cost_usd`, `user.daily_cost_usd`).
pub const USAGE_TIMEOUT: Duration = Duration::from_secs(2);
/// Wartezeit auf eine menschliche Freigabe (wie das Gate der Adapter, HAR-005 AC4).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Claude-Tool, mit dem das Modell den Plan-Modus verlassen will (HAR-027).
const EXIT_PLAN_MODE: &str = "ExitPlanMode";
/// Begründung, wenn eine Rückfrage andere Argumente trägt als der geprüfte Call (#233).
pub const CHANGED_AFTER_CHECK: &str = "Abgelehnt von beton: Die Argumente wurden nach der Policy-Prüfung verändert (z. B. durch einen weiteren Hook).";
/// Präfix eigener Call-IDs des Relay-Hubs (Aufruf ohne zugeordnetes `tool.call.requested`).
const SYNTHETIC: &str = "mcp_unmatched_";
/// Höchstgröße von `request.prompt` und `result.text` (POL-004).
pub const TEXT_MAX: usize = 64 * 1024;

/// Ein Ereignis des Gates für das Session-Log.
#[derive(Debug, Clone)]
pub struct GateEvent {
    pub payload: EventPayload,
    pub turn: Option<TurnId>,
    /// Original-Zeile der Rückfrage des Vendors (HAR-001 AC3).
    pub raw: Option<RawJson>,
    /// Die Session muss enden (fail closed): z. B. eine nach dem Start hinzugekommene, nicht
    /// durchsetzbare Pflichtregel (#234). Das Event ist dann ein `error`.
    pub fatal: bool,
}

/// Wertet ein Set aus; austauschbar für Fehlerinjektion (POL-024 AC3).
pub trait Evaluator: Send + Sync + 'static {
    fn evaluate(
        &self,
        set: &EffectiveSet,
        ctx: &Activation,
        opts: &EvalOptions,
    ) -> Result<Decision, String>;
}

/// Die Engine aus `beton-policy`; ein Panic gilt als Fehler (fail closed).
#[derive(Debug, Default)]
pub struct EngineEvaluator;

impl Evaluator for EngineEvaluator {
    fn evaluate(
        &self,
        set: &EffectiveSet,
        ctx: &Activation,
        opts: &EvalOptions,
    ) -> Result<Decision, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| set.evaluate(ctx, opts)))
            .map_err(|_| "Policy-Engine abgestürzt".to_owned())
    }
}

/// Verbrauchswerte aus der Usage-Projektion des Servers (POL-004 AC3/AC4). Für Budgets
/// (POL-011, POL-012) zusätzlich das Listenpreis-Äquivalent der Subscription-Nutzung
/// (ADR-0034), der Teilbaum der Session und die Records ohne Preis.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub session_cost_usd: Option<f64>,
    #[serde(default)]
    pub session_equivalent_usd: Option<f64>,
    #[serde(default)]
    pub session_without_price: Option<u64>,
    #[serde(default)]
    pub session_tokens_in: Option<i64>,
    #[serde(default)]
    pub session_tokens_out: Option<i64>,
    #[serde(default)]
    pub subtree_cost_usd: Option<f64>,
    #[serde(default)]
    pub subtree_equivalent_usd: Option<f64>,
    #[serde(default)]
    pub subtree_without_price: Option<u64>,
    #[serde(default)]
    pub user_daily_cost_usd: Option<f64>,
    #[serde(default)]
    pub user_daily_equivalent_usd: Option<f64>,
    #[serde(default)]
    pub user_daily_without_price: Option<u64>,
    #[serde(default)]
    pub user_monthly_cost_usd: Option<f64>,
    #[serde(default)]
    pub timezone: Option<String>,
    /// Modelle ohne Preis je Bereich (`on_unpriced`, POL-011).
    #[serde(default)]
    pub session_unpriced_models: Option<Vec<String>>,
    #[serde(default)]
    pub subtree_unpriced_models: Option<Vec<String>>,
    #[serde(default)]
    pub user_daily_unpriced_models: Option<Vec<String>>,
}

impl Usage {
    /// Records ohne Geld und ohne Listenpreis-Äquivalent im Kostenbereich einer Budget-Regel.
    fn without_price(&self, scope: beton_policy::budget::BudgetScope) -> u64 {
        use beton_policy::budget::BudgetScope;
        match scope {
            BudgetScope::Session | BudgetScope::Run => self.session_without_price,
            BudgetScope::Subtree => self.subtree_without_price,
            BudgetScope::User => self.user_daily_without_price,
        }
        .unwrap_or(0)
    }
}

/// Quelle der Verbrauchswerte (im Runner: System-Call `policy.usage` über den Tunnel).
#[async_trait]
pub trait UsageSource: Send + Sync {
    async fn usage(&self) -> Result<Usage, String>;
}

/// Felder, für die das Gate Verbrauchswerte vom Server holt. `user.timezone` kommt mit ihnen
/// (Einstellung des Daemons, POL-004 AC4); ohne Antwort bleibt es `null` (fail closed).
const USAGE_FIELDS: [&str; 16] = [
    "session.cost_usd",
    "session.equivalent_usd",
    "session.unpriced_models",
    "run.unpriced_models",
    "run.subtree_unpriced_models",
    "user.daily_unpriced_models",
    "session.tokens_in",
    "session.tokens_out",
    "run.cost_usd",
    "run.equivalent_usd",
    "run.subtree_cost_usd",
    "run.subtree_equivalent_usd",
    "user.daily_cost_usd",
    "user.daily_equivalent_usd",
    "user.monthly_cost_usd",
    "user.timezone",
];

/// Fenster von `tool.repeat_count` in Tool-Calls (POL-004, POL-018).
const REPEAT_WINDOW: usize = 10;

/// Startparameter des Gates.
#[derive(Debug, Clone)]
pub struct PolicyOptions {
    pub session_id: Option<String>,
    pub harness: HarnessId,
    pub workdir: PathBuf,
    pub home: Option<PathBuf>,
    /// Projektverzeichnis (`session.project_root`, CLI-008); ohne Angabe aus `workdir`
    /// ermittelt.
    pub project_root: Option<PathBuf>,
    /// Policy-Verzeichnisse je Ebene (User `$BETON_HOME/policies`, Projekt
    /// `<worktree>/.beton/policies`).
    pub dirs: Vec<(Scope, PathBuf)>,
    pub approval_timeout: Duration,
    pub agent: Option<AgentVars>,
}

impl Default for PolicyOptions {
    fn default() -> Self {
        Self {
            session_id: None,
            harness: HarnessId::FAKE.parse().unwrap_or_else(|_| unreachable!()),
            workdir: PathBuf::from("/"),
            home: None,
            project_root: None,
            dirs: Vec::new(),
            approval_timeout: APPROVAL_TIMEOUT,
            agent: None,
        }
    }
}

/// Eine menschliche Entscheidung (`approval.resolve`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Human {
    pub allow: bool,
    /// „Für diese Session erlauben“ (`approve_session`); nur wirksam, wenn die Card die
    /// Option anbietet (POL-010).
    pub remember: bool,
    pub reason: Option<String>,
    pub updated_args: Option<Value>,
    /// Antwort auf eine Frage (`ask_user`, AGT-007 AC4).
    pub answer: Option<String>,
    /// Wer entschieden hat (WEB-007 AC2); ohne Angabe der lokale User.
    pub actor: Option<Actor>,
}

/// Zustand der Policies und gemerkte Freigaben einer Session vom Server (POL-009, POL-010):
/// `session`/`run` aus dem Session-Log, `user_day`/`user` aus dem Store.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LoadedState {
    #[serde(default)]
    pub session: BTreeMap<String, Value>,
    #[serde(default)]
    pub run: BTreeMap<String, Value>,
    #[serde(default)]
    pub user_day: BTreeMap<String, Value>,
    #[serde(default)]
    pub user: BTreeMap<String, Value>,
    #[serde(default)]
    pub grants: Vec<GrantRecord>,
}

/// Ein Grant aus dem Log (`approval.resolved { remember: session }`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GrantRecord {
    pub id: String,
    pub scope: String,
    pub set: String,
    pub rule_id: String,
    pub fingerprint: String,
}

/// Quelle des Zustands (im Runner: System-Calls `policy.state` und `policy.state.apply`).
#[async_trait]
pub trait StateSource: Send + Sync {
    async fn load(&self) -> Result<LoadedState, String>;
    /// Wendet eine Änderung in `user_day`/`user` im Store an.
    async fn apply(&self, op: &StateOp) -> Result<(), String>;
}

fn scope_from(s: &str) -> Option<Scope> {
    Some(match s {
        "org" => Scope::Org,
        "team" => Scope::Team,
        "user" => Scope::User,
        "project" => Scope::Project,
        "agent" => Scope::Agent,
        _ => return None,
    })
}

/// Ergebnis der Prüfung eines Tool-Calls.
#[derive(Debug, Clone, PartialEq)]
struct Verdict {
    allowed: bool,
    /// Ein Mensch hat entschieden (Policy-`ask`).
    human: bool,
    reason: Option<String>,
    /// Vollständige, geänderte Argumente.
    updated_args: Option<Value>,
    /// Hash des freigegebenen Subjekts (Tool und tatsächlich auszuführende Argumente). Eine
    /// Rückfrage des Vendors zum selben Call nutzt den Entscheid nur bei gleichem Hash (#233).
    subject: String,
}

/// `tool.*` aus einem normalisierten Tool-Call (POL-004).
fn tool_vars(n: &beton_policy::normalize::NormalizedTool, call_index: Option<i64>) -> ToolVars {
    let args = match &n.args {
        Value::Object(m) => Some(m.clone().into_iter().collect()),
        _ => None,
    };
    ToolVars {
        name: Some(n.name.clone()),
        native_name: Some(n.native_name.clone()),
        server: Some(n.server.clone()),
        kind: Some(n.kind.clone()),
        args,
        command: n.command.clone(),
        argv: Some(n.argv.clone()),
        cwd: Some(n.cwd.clone()),
        paths: Some(n.paths.clone()),
        write_paths: Some(n.write_paths.clone()),
        urls: Some(n.urls.clone()),
        call_index,
        is_subagent_spawn: Some(n.is_subagent_spawn),
        parse_uncertain: Some(n.parse_uncertain),
        ..ToolVars::default()
    }
}

/// Ergebnis der gemeinsamen Vorprüfungen eines Tool-Calls ([`PolicyGate::precheck`]).
pub(crate) struct Precheck {
    /// Ohne Call-ID teilten sich Calls gespeicherte Entscheide (#234): fail closed.
    pub missing_call_id: bool,
    /// Normalisierter Call (mit Klasse des Adapters) oder warum er nicht prüfbar ist.
    pub tool: Result<beton_policy::normalize::NormalizedTool, String>,
    /// Kanonische Klasse für Durchsetzung und Card.
    pub kind: String,
    /// Pfad bzw. Wort, mit dem ein nicht nur lesender Call Policy-Dateien berührt (#234).
    pub policy_files: Option<String>,
    /// System-Tool eines von beton eingebundenen, reservierten Servers (#151).
    pub beton_system: bool,
    /// `ExitPlanMode`: entscheidet immer ein Mensch (HAR-027).
    pub plan_exit: bool,
}

/// Begründung für Tool-Calls auf Policy-Dateien (Regel `policy-files`, #234).
fn policy_files_reason(path: &str) -> String {
    format!(
        "Policy-Dateien ändert nur der Mensch, nicht ein Agent ({path}; beton-Policy, Regel `policy-files`)"
    )
}

/// Klasse des Adapters, wenn die Abbildungstabelle das Tool nicht kennt (z. B. Fake-Szenario).
fn adapter_kind(n: &mut beton_policy::normalize::NormalizedTool, req: &GateRequest) {
    if n.kind == "other" && beton_policy::enforcement::KINDS.contains(&req.kind.as_str()) {
        n.kind.clone_from(&req.kind);
    }
}

/// Erkennt Bezüge auf Policy-Verzeichnisse in Pfaden und Shell-Wörtern (#234, Re-Review R1).
/// Best effort gegen Verschleierung; Backstop sind die Regeln des Stores (gelöschte Dateien
/// bleiben aktiv, lockernde Änderungen erst in einer neuen Session) und die Sandbox.
struct PolicyPathGuard {
    /// Kanonische Policy-Verzeichnisse.
    dirs: Vec<PathBuf>,
    cwd: String,
    home: Option<String>,
}

impl PolicyPathGuard {
    /// Absoluter Pfad unter einem Policy-Verzeichnis oder das `.beton`-Verzeichnis darüber
    /// (`rm -rf .beton`), nicht aber Arbeitsverzeichnis oder Home.
    fn hits_path(&self, path: &str) -> bool {
        let real = PathBuf::from(resolve_fs(path));
        self.dirs
            .iter()
            .any(|d| real.starts_with(d) || d.parent() == Some(real.as_path()))
    }

    fn hits_word(&self, word: &str, vars: &HashMap<String, String>) -> bool {
        let lower = word.to_lowercase();
        if lower.contains(".beton") {
            return true;
        }
        let (expanded, unknown) = expand_vars(word, vars, self.home.as_deref());
        if unknown && mentions_policies(word) {
            return true;
        }
        let expanded_lower = expanded.to_lowercase();
        if expanded_lower.contains(".beton") {
            return true;
        }
        // Globs wie `.b*` treffen `.beton` (Bash: nur Muster mit führendem Punkt).
        if expanded.contains(['*', '?', '['])
            && expanded
                .split('/')
                .any(|c| c.starts_with('.') && glob_component(c, ".beton"))
        {
            return true;
        }
        if expanded.starts_with('-') || expanded.is_empty() {
            return false;
        }
        let candidate = if let Some(rest) = expanded.strip_prefix("~/") {
            self.home.as_ref().map(|h| format!("{h}/{rest}"))
        } else if expanded.starts_with('/') {
            Some(expanded.clone())
        } else {
            Some(format!("{}/{expanded}", self.cwd))
        };
        candidate
            .and_then(|c| beton_policy::cel::ext::normalize_path(&c))
            .is_some_and(|p| self.hits_path(&p))
    }
}

fn resolve_fs(p: &str) -> String {
    FsResolver.resolve(p).unwrap_or_else(|_| p.to_owned())
}

/// Erwähnt der Text ein Policy-Verzeichnis (für Wörter mit nicht auflösbaren Variablen)?
fn mentions_policies(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("beton") || lower.contains("polic")
}

/// `NAME=wert`-Zuweisungen aller Teilkommandos (auch nach `export`).
fn assignments(argv: &[Vec<String>]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for word in argv.iter().flatten() {
        if let Some((name, value)) = word.split_once('=')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
        {
            out.insert(name.to_owned(), value.to_owned());
        }
    }
    out
}

/// Setzt `$NAME` und `${NAME}` ein (bekannte Zuweisungen, `HOME`); liefert, ob unbekannte
/// Expansionen (auch `$(…)`, Backticks) übrig blieben.
fn expand_vars(word: &str, vars: &HashMap<String, String>, home: Option<&str>) -> (String, bool) {
    let mut out = String::new();
    let mut unknown = word.contains('`');
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let braced = chars.get(i + 1) == Some(&'{');
        let start = if braced { i + 2 } else { i + 1 };
        let mut end = start;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        let name: String = chars[start..end].iter().collect();
        if name.is_empty() || (braced && chars.get(end) != Some(&'}')) {
            unknown = true;
            out.push('$');
            i += 1;
            continue;
        }
        let value = vars
            .get(&name)
            .cloned()
            .or_else(|| (name == "HOME").then(|| home.map(str::to_owned)).flatten());
        match value {
            Some(v) => out.push_str(&v),
            None => unknown = true,
        }
        i = if braced { end + 1 } else { end };
    }
    (out, unknown)
}

/// Passt ein Glob-Muster (`*`, `?`, `[…]` grob als ein Zeichen) auf einen Namen?
fn glob_component(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|k| go(&p[1..], &n[k..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => {
                let close = p.iter().position(|c| *c == ']').unwrap_or(p.len() - 1);
                !n.is_empty() && go(&p[(close + 1).min(p.len())..], &n[1..])
            }
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// Alle Zeichenketten eines JSON-Werts (höchstens 1 000).
fn collect_strings(v: &Value, out: &mut Vec<String>) {
    if out.len() >= 1000 {
        return;
    }
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|i| collect_strings(i, out)),
        Value::Object(map) => map.values().for_each(|i| collect_strings(i, out)),
        _ => {}
    }
}

/// Hash des Subjekts eines Tool-Calls (`tool` und `args`), wie in `policy.decision`.
fn tool_subject(tool: &str, args: &Value) -> String {
    subject_hash(&json!({"tool": tool, "args": args}))
}

/// Ergebnis einer Prüfung von `model_request` bzw. `session_start`.
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    Allow(Modifications),
    Deny {
        reason: String,
        rules: Vec<String>,
    },
    /// `ask`, aber ohne menschliche Rückfrage beantwortet (z. B. Steer).
    AskNeeded,
}

#[derive(Debug, Clone, Default)]
struct Settings {
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    turns: i64,
}

struct Inner {
    opts: PolicyOptions,
    store: Mutex<PolicyStore>,
    matrix: Mutex<Matrix>,
    settings: Mutex<Settings>,
    events: mpsc::UnboundedSender<GateEvent>,
    /// Offene menschliche Freigaben: Antwortkanal und Art (Policy, Rückfrage, Frage).
    pending: Mutex<HashMap<String, (oneshot::Sender<Human>, ApprovalKind)>>,
    decided: Mutex<HashMap<String, Verdict>>,
    /// Von beton eingebundene MCP-Server (#234): nur für sie gelten die Klassen reservierter
    /// Namen (`beton` → `system`). Bis zur MCP-Vorbereitung keiner (fail closed).
    trusted_servers: Mutex<Vec<String>>,
    /// Aufgelöste Policy-Verzeichnisse (Cache für die Regel `policy-files`).
    policy_dirs: Mutex<Option<Vec<PathBuf>>>,
    /// Gemeldete Tool-Calls (Call-ID, MCP-Server, Tool) für die Zuordnung im Relay-Hub.
    requested: Mutex<Vec<(String, Option<String>, String)>>,
    /// Reihenfolge im Log: `policy.decision` erst nach dem `tool.call.requested` (POL-025).
    ordered: std::sync::atomic::AtomicBool,
    /// Reloads werden wie der Start geprüft (#234); vor dem Start nicht (der Runner prüft
    /// dort selbst).
    started: std::sync::atomic::AtomicBool,
    seen: tokio::sync::Notify,
    evaluator: Arc<dyn Evaluator>,
    usage: Option<Arc<dyn UsageSource>>,
    /// Zustand `session`/`run` (POL-009); gültig, sobald `state_ready`.
    state: Mutex<PolicyState>,
    /// Zuletzt geladener Zustand `user_day`/`user`.
    user_state: Mutex<(BTreeMap<String, Value>, BTreeMap<String, Value>)>,
    state_ready: std::sync::atomic::AtomicBool,
    state_source: Option<Arc<dyn StateSource>>,
    /// Gemerkte Freigaben (`approve_session`, POL-010).
    grants: Mutex<SessionGrants>,
    calls: AtomicI64,
    /// Identitäten der letzten Tool-Calls (Call-ID, Identität) für `tool.repeat_count`.
    recent: Mutex<std::collections::VecDeque<(String, String)>>,
    /// Fehlgeschlagene Tool-Calls in Folge (`tool.consecutive_errors`, POL-018).
    errors_in_row: AtomicI64,
    granted: AtomicI64,
    denied: AtomicI64,
    started_at: CelTimestamp,
    /// Set-Snapshots je Version für Explain (POL-027 AC2).
    snapshots: dry::Snapshots,
    /// `policy_query`-Abfragen im laufenden Turn (Limit, Security-Review B1).
    queries: Mutex<dry::QueryBudget>,
    /// Gemeinsames Budget des Session-Baums (Server, #276); ohne zählt das Gate selbst.
    query_source: Option<Arc<dyn dry::QueryBudgetSource>>,
    /// Set-Versionen, deren Lint-Hinweise schon gemeldet sind (#242, #248).
    linted: Mutex<HashSet<String>>,
    /// Budget-Regeln, deren Kosten unbekannt sind (`policy.rule_inapplicable`, einmal je
    /// Regel und Session; POL-011).
    inapplicable: Mutex<BTreeSet<String>>,
    /// Gesetzt, wenn die Sandbox eine Voraussetzung verloren hat (z. B. der Proxy, SBX-006
    /// AC3): Jede weitere Tool-Ausführung wird abgelehnt.
    halted: std::sync::OnceLock<String>,
    /// Nur beobachtete Tool-Calls, die eine `deny`-Regel getroffen hätte (POL-023 AC3): Call-ID →
    /// erste Regel und Begründung.
    observed_denies: Mutex<HashMap<String, (beton_policy::load::RuleRef, String)>>,
}

/// Das Gate einer Session; billig klonbar.
#[derive(Clone)]
pub struct PolicyGate {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PolicyGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyGate")
            .field("harness", &self.inner.opts.harness)
            .finish_non_exhaustive()
    }
}

/// Löst Symlinks über das Dateisystem auf (für `under()`, POL-002, #182): der längste
/// existierende Präfix wird kanonisiert, der Rest lexikalisch angehängt.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsResolver;

impl PathResolver for FsResolver {
    fn resolve(&self, path: &str) -> Result<String, String> {
        let lexical = LexicalResolver.resolve(path)?;
        let mut existing = PathBuf::from(&lexical);
        let mut rest: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if let Ok(real) = std::fs::canonicalize(&existing) {
                let mut out = real;
                for part in rest.iter().rev() {
                    out.push(part);
                }
                return Ok(out.display().to_string());
            }
            match (existing.parent(), existing.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_owned());
                    existing = parent.to_path_buf();
                }
                _ => return Ok(lexical),
            }
        }
    }
}

/// Aktueller Git-Branch von `workdir` (`session.branch`, POL-017) aus `HEAD`, ohne `git`
/// zu starten: `.git` als Verzeichnis oder als Datei mit `gitdir:` (Worktree). `None` bei
/// losgelöstem HEAD oder ohne Repository.
fn current_branch(workdir: &Path) -> Option<String> {
    use std::io::Read as _;
    let read = |path: &Path| -> Option<String> {
        let mut text = String::new();
        std::fs::File::open(path)
            .ok()?
            .take(4096)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    };
    let dot_git = workdir
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|p| p.exists())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = read(&dot_git)?;
        let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
        dot_git.parent()?.join(target)
    };
    let head = read(&git_dir.join("HEAD"))?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty()).then(|| branch.to_owned())
}

fn truncate(text: &str) -> String {
    let mut end = text.len().min(TEXT_MAX);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Begründung für das Modell bzw. den Menschen: primärer Grund zuerst.
fn reason_text(reasons: &[Reason]) -> String {
    match reasons.first() {
        Some(r) => format!("{} (beton-Policy, Regel `{}`)", r.reason, r.rule.rule_id),
        None => "Von der beton-Policy abgelehnt".to_owned(),
    }
}

fn rule_ids(reasons: &[Reason]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in reasons {
        if !out.contains(&r.rule.rule_id) {
            out.push(r.rule.rule_id.clone());
        }
    }
    out
}

/// Nach so langer Wartezeit auf die Freigabe eines Starts wertet das Gate `session_start` auch
/// ohne geänderte Policies erneut aus (Kosten können sich geändert haben, #297).
const START_RECHECK_AFTER: Duration = Duration::from_secs(60);

/// Ablehnung durch eine eingebaute Regel des Gates (ohne Engine, kein Fehler).
fn builtin_deny(phase: Phase, hash: &str, rule_id: &str, message: String) -> Decision {
    let mut d = engine_deny(phase, hash, message);
    d.errors.clear();
    if let Outcome::Deny { reasons } = &mut d.outcome {
        for r in reasons {
            r.rule.rule_id = rule_id.to_owned();
        }
    }
    d
}

/// Tool-Klassen, die Policy-Dateien lesen, aber nicht ändern können (#234).
const READ_ONLY_KINDS: [&str; 2] = ["file_read", "search"];

/// Höchstdauer einer Auswertung: [`EVAL_TIMEOUT`], nie kürzer als das Zeitbudget der Engine.
/// Im Betrieb (10 ms) bleibt es bei `EVAL_TIMEOUT`; nur das großzügige Budget der
/// Integrationstests (Debug-Builds, #298) verlängert den Timeout, damit unter Last nicht er
/// statt des Budgets reißt.
fn eval_timeout(limits: &Limits) -> Duration {
    EVAL_TIMEOUT.max(Duration::from_micros(limits.eval_budget_us))
}

/// Ablehnung ohne Auswertung (Fehler, Zeitüberschreitung): fail closed.
fn engine_deny(phase: Phase, hash: &str, message: String) -> Decision {
    let rule = RuleRef {
        scope: Scope::Org,
        set: String::new(),
        set_version: String::new(),
        rule_id: "engine".into(),
    };
    Decision {
        phase,
        outcome: Outcome::Deny {
            reasons: vec![Reason {
                rule: rule.clone(),
                reason: message.clone(),
                severity: None,
                approval: None,
                fingerprint: None,
            }],
        },
        modified: Modifications::default(),
        matched: Vec::new(),
        would: Vec::new(),
        defaults_applied: Vec::new(),
        conflicts: Vec::new(),
        errors: vec![RuleError {
            rule,
            error: message,
        }],
        grants_used: Vec::new(),
        notifications: Vec::new(),
        effects: Vec::new(),
        warnings: Vec::new(),
        policy_set_hash: hash.to_owned(),
        eval_us: 0,
    }
}

/// Gründe einer Approval-Card (POL-010): Regel, Ebene, Set, Text, Schwere und – für
/// gemerkte Freigaben – der Fingerprint.
fn reasons_json(reasons: &[Reason]) -> Vec<Value> {
    reasons
        .iter()
        .map(|r| {
            let mut v = json!({
                "rule_id": r.rule.rule_id,
                "scope": r.rule.scope.as_str(),
                "set": r.rule.set,
                "reason": r.reason,
            });
            if let Some(sev) = r.severity {
                v["severity"] = serde_json::to_value(sev).unwrap_or(Value::Null);
            }
            if let Some(fp) = &r.fingerprint {
                v["fingerprint"] = Value::String(fp.clone());
            }
            v
        })
        .collect()
}

/// Titel der Card nach Tool-Klasse.
fn tool_title(kind: &str) -> &'static str {
    match kind {
        "shell" => "Shell-Kommando bestätigen",
        "file_write" | "file_edit" => "Datei-Änderung bestätigen",
        "mcp" => "MCP-Tool bestätigen",
        "web_fetch" => "Abruf bestätigen",
        "system" => "System-Tool bestätigen",
        "browser" => "Browser-Aktion bestätigen",
        _ => "Tool-Call bestätigen",
    }
}

/// Subjekt einer Card für einen Tool-Call (Payload `approval.requested`, POL-010).
fn tool_card(
    phase: &str,
    tool: &str,
    kind: &str,
    summary: Option<&str>,
    args: &Value,
    modified: Option<&Value>,
    reasons: &[Reason],
) -> Value {
    let full = summary.unwrap_or(tool);
    // Gekürzt nur mit Kennzeichnung; die vollständigen Argumente stehen in `tool.args`.
    let summary: String = if full.chars().count() > 500 {
        full.chars().take(500).chain(std::iter::once('…')).collect()
    } else {
        full.to_owned()
    };
    json!({
        "phase": phase,
        "title": tool_title(kind),
        "summary": summary,
        "tool": {
            "name": tool,
            "kind": kind,
            "args": args,
            "args_modified": modified,
        },
        "reasons": reasons_json(reasons),
    })
}

/// Ergebnis einer menschlichen Freigabe.
struct Answered {
    human: Human,
    /// Endentscheidung für Effects (POL-009).
    outcome: FinalOutcome,
    /// Abgelaufen mit `on_timeout: abort`: der Turn wird abgebrochen.
    aborted: bool,
}

/// Ergebnis einer Auswertung mit ID des Freigabe-Vorgangs bei `ask`.
struct Evaluated {
    decision: Decision,
    approval: Option<ApprovalId>,
}

impl PolicyGate {
    pub fn new(mut opts: PolicyOptions, events: mpsc::UnboundedSender<GateEvent>) -> Self {
        if opts.project_root.is_none() {
            opts.project_root = Some(beton_agents::resolve::project_root(
                &opts.workdir,
                Path::new(""),
            ));
        }
        let mut store = PolicyStore::new();
        for (scope, dir) in &opts.dirs {
            store.add_dir(*scope, dir);
        }
        // Projekt-Policies liegen in der Hand des Agents bzw. Repos, User-Policies ohne Sandbox
        // in Reichweite seiner Tools: während der Session nur verschärfen (#253).
        store.tighten_only(Scope::Project);
        store.tighten_only(Scope::User);
        Self {
            inner: Arc::new(Inner {
                opts,
                store: Mutex::new(store),
                matrix: Mutex::new(Matrix::full()),
                settings: Mutex::new(Settings::default()),
                events,
                pending: Mutex::new(HashMap::new()),
                decided: Mutex::new(HashMap::new()),
                trusted_servers: Mutex::new(Vec::new()),
                policy_dirs: Mutex::new(None),
                requested: Mutex::new(Vec::new()),
                ordered: std::sync::atomic::AtomicBool::new(false),
                started: std::sync::atomic::AtomicBool::new(true),
                seen: tokio::sync::Notify::new(),
                evaluator: Arc::new(EngineEvaluator),
                usage: None,
                state: Mutex::new(PolicyState::default()),
                user_state: Mutex::new((BTreeMap::new(), BTreeMap::new())),
                // Ohne Quelle beginnt die Session leer (neue Session, Tests).
                state_ready: std::sync::atomic::AtomicBool::new(true),
                state_source: None,
                grants: Mutex::new(SessionGrants::default()),
                calls: AtomicI64::new(0),
                recent: Mutex::new(std::collections::VecDeque::new()),
                errors_in_row: AtomicI64::new(0),
                granted: AtomicI64::new(0),
                denied: AtomicI64::new(0),
                started_at: CelTimestamp::now(),
                snapshots: dry::Snapshots::default(),
                queries: Mutex::new(dry::QueryBudget::default()),
                query_source: None,
                linted: Mutex::new(HashSet::new()),
                inapplicable: Mutex::new(BTreeSet::new()),
                halted: std::sync::OnceLock::new(),
                observed_denies: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn rebuild(self, f: impl FnOnce(&mut Inner)) -> Self {
        match Arc::try_unwrap(self.inner) {
            Ok(mut inner) => {
                f(&mut inner);
                Self {
                    inner: Arc::new(inner),
                }
            }
            Err(inner) => Self { inner },
        }
    }

    /// Ersetzt die Engine (Tests: Fehlerinjektion).
    pub fn with_evaluator(self, evaluator: Arc<dyn Evaluator>) -> Self {
        self.rebuild(|i| i.evaluator = evaluator)
    }

    /// Quelle für Verbrauchswerte (`session.cost_usd`, `user.daily_cost_usd`).
    pub fn with_usage(self, usage: Arc<dyn UsageSource>) -> Self {
        self.rebuild(|i| i.usage = Some(usage))
    }

    /// Quelle für Zustand und Grants (POL-009, POL-010); geladen vor der ersten Auswertung.
    pub fn with_state(self, source: Arc<dyn StateSource>) -> Self {
        self.rebuild(|i| {
            i.state_source = Some(source);
            i.state_ready = std::sync::atomic::AtomicBool::new(false);
        })
    }

    /// Lädt Zustand und Grants einmal vom Server. `false`, solange das nicht gelungen ist.
    async fn ensure_state(&self) -> bool {
        if self.inner.state_ready.load(Ordering::SeqCst) {
            return true;
        }
        let Some(source) = &self.inner.state_source else {
            return true;
        };
        let loaded = match tokio::time::timeout(USAGE_TIMEOUT, source.load()).await {
            Ok(Ok(l)) => l,
            _ => {
                tracing::warn!("Policy: Zustand nicht verfügbar");
                return false;
            }
        };
        if self.inner.state_ready.swap(true, Ordering::SeqCst) {
            return true;
        }
        if let Ok(mut st) = self.inner.state.lock() {
            let mut fresh = PolicyState::default();
            for (scope, map) in [
                (StateScope::Session, &loaded.session),
                (StateScope::Run, &loaded.run),
            ] {
                for (key, value) in map {
                    fresh.restore(&StateChange {
                        scope,
                        key: key.clone(),
                        op: beton_policy::doc::EffectOp::Set,
                        value: Some(value.clone()),
                    });
                }
            }
            *st = fresh;
        }
        if let Ok(mut u) = self.inner.user_state.lock() {
            *u = (loaded.user_day.clone(), loaded.user.clone());
        }
        if let Ok(mut g) = self.inner.grants.lock() {
            for r in &loaded.grants {
                if let Some(scope) = scope_from(&r.scope) {
                    let rule = RuleRef {
                        scope,
                        set: r.set.clone(),
                        set_version: String::new(),
                        rule_id: r.rule_id.clone(),
                    };
                    g.add_with_id(&rule, &r.fingerprint, &r.id);
                }
            }
        }
        true
    }

    /// `state.*` für den Kontext; `user_day`/`user` frisch, wenn eine Regel sie liest. Geladen
    /// wird nur, wenn eine Regel der Phase `state` liest oder – in `tool_call` – Grants greifen
    /// können; vor dem Start braucht das Gate sonst keinen Server.
    async fn with_state_values(&self, set: &EffectiveSet, phase: Phase, vars: &mut Vars) {
        let used: BTreeSet<String> = set.programs(phase).flat_map(|p| p.fields()).collect();
        let reads_state = used.iter().any(|f| f == "state" || f.starts_with("state."));
        let grants_matter = phase == Phase::ToolCall && set.has_phase(Phase::ToolCall);
        if !(reads_state || grants_matter) || !self.ensure_state().await {
            return;
        }
        let user_scopes = used.iter().any(|f| f.starts_with("state.user"));
        // Scheitert das Neuladen von `user_day`/`user`, gilt der Zustand als nicht verfügbar
        // statt still der alte Stand (POL-009, Security-Review #284): Regeln darauf scheitern.
        let mut user_available = true;
        if user_scopes && let Some(source) = &self.inner.state_source {
            match tokio::time::timeout(USAGE_TIMEOUT, source.load()).await {
                Ok(Ok(fresh)) => {
                    if let Ok(mut u) = self.inner.user_state.lock() {
                        *u = (fresh.user_day, fresh.user);
                    }
                }
                _ => {
                    tracing::warn!("Policy: Zustand user_day/user nicht verfügbar");
                    user_available = false;
                }
            }
        }
        let (session, run) = self
            .inner
            .state
            .lock()
            .map(|s| (s.snapshot(StateScope::Session), s.snapshot(StateScope::Run)))
            .unwrap_or_default();
        let (user_day, user) = self
            .inner
            .user_state
            .lock()
            .map(|u| u.clone())
            .unwrap_or_default();
        vars.state = Some(StateVars {
            session: Some(session),
            run: Some(run),
            user_day: user_available.then_some(user_day),
            user: user_available.then_some(user),
        });
    }

    /// Wendet die Effects einer Entscheidung an, deren `effects_on` zur Endentscheidung passt
    /// (POL-009): `session`/`run` als `policy.state_changed` ins Log, `user_day`/`user` in den
    /// Store. Ohne geladenen Zustand unterbleiben sie mit Warnung.
    async fn apply_effects(
        &self,
        decision: &Decision,
        outcome: FinalOutcome,
        turn: Option<TurnId>,
    ) {
        let due: Vec<_> = decision
            .effects
            .iter()
            .filter(|e| effects_due(&e.effects_on, outcome))
            .collect();
        if due.is_empty() {
            return;
        }
        if !self.ensure_state().await {
            let rules: Vec<_> = due.iter().map(|e| e.rule.rule_id.as_str()).collect();
            self.emit(
                EventPayload::Notice(Notice {
                    level: NoticeLevel::Warn,
                    text: format!(
                        "Policy-Zustand nicht verfügbar: Effects von {} nicht angewendet",
                        rules.join(", ")
                    ),
                }),
                turn,
                None,
            );
            return;
        }
        for pending in due {
            for op in &pending.ops {
                let failed = match op.scope {
                    StateScope::Session | StateScope::Run => {
                        let applied = self
                            .inner
                            .state
                            .lock()
                            .map_err(|_| "interner Fehler".to_owned())
                            .and_then(|mut st| {
                                st.apply(op.scope, &op.key, op.op, op.value.clone())
                                    .map_err(|e| e.to_string())
                            });
                        match applied {
                            Ok(change) => {
                                self.emit(
                                    EventPayload::PolicyStateChanged(change.to_event()),
                                    turn,
                                    None,
                                );
                                None
                            }
                            Err(e) => Some(e),
                        }
                    }
                    StateScope::UserDay | StateScope::User => match &self.inner.state_source {
                        None => None,
                        Some(source) => {
                            match tokio::time::timeout(USAGE_TIMEOUT, source.apply(op)).await {
                                Ok(Ok(())) => None,
                                Ok(Err(e)) => Some(e),
                                Err(_) => Some("keine Antwort".into()),
                            }
                        }
                    },
                };
                if let Some(e) = failed {
                    self.emit(
                        EventPayload::Notice(Notice {
                            level: NoticeLevel::Warn,
                            text: format!(
                                "Effect `{}` von Regel `{}` nicht angewendet: {e}",
                                op.key, pending.rule.rule_id
                            ),
                        }),
                        turn,
                        None,
                    );
                }
            }
        }
    }

    /// MCP-Server, die beton selbst eingebunden hat (Relay-Hub, Workspace-Tools des
    /// Direkt-Harness; #234).
    /// Inline-Sets einer Ebene, z. B. das Session-Budget (API-006, POL-011).
    pub fn set_inline(&self, scope: Scope, sets: Vec<beton_policy::load::LoadedSet>) {
        if let Ok(mut store) = self.inner.store.lock() {
            store.set_inline(scope, sets);
        }
    }

    pub fn set_trusted_servers(&self, servers: Vec<String>) {
        if let Ok(mut t) = self.inner.trusted_servers.lock() {
            *t = servers;
        }
    }

    /// Enforcement-Matrix des Harness der Session (POL-024).
    pub fn set_matrix(&self, matrix: Matrix) {
        match self.inner.matrix.lock() {
            Ok(mut m) => *m = matrix,
            Err(p) => *p.into_inner() = matrix,
        }
    }

    fn matrix(&self) -> Matrix {
        // Vergifteter Mutex: die zuletzt gesetzte Matrix, nie pauschal `full` (#234).
        self.inner
            .matrix
            .lock()
            .map(|m| m.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    /// Modell, Effort und Permission-Mode der Session (für `session.*`).
    pub fn set_settings(
        &self,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) {
        if let Ok(mut s) = self.inner.settings.lock() {
            s.model = model;
            s.effort = effort;
            s.permission_mode = permission_mode;
        }
    }

    pub fn set_permission_mode(&self, mode: PermissionMode) {
        if let Ok(mut s) = self.inner.settings.lock() {
            s.permission_mode = Some(mode);
        }
    }

    /// Ein Turn hat begonnen (`session.turns`).
    pub fn note_turn(&self) {
        if let Ok(mut s) = self.inner.settings.lock() {
            s.turns += 1;
        }
    }

    /// Ein `tool.call.completed` aus dem Event-Strom: zählt fehlgeschlagene Calls in Folge
    /// (`tool.consecutive_errors`, POL-018). Ein erfolgreicher Call setzt zurück; abgelehnte
    /// und abgebrochene ändern nichts.
    pub fn note_completed(&self, status: beton_core::event::ToolStatus) {
        use beton_core::event::ToolStatus;
        match status {
            ToolStatus::Error => {
                self.inner.errors_in_row.fetch_add(1, Ordering::SeqCst);
            }
            ToolStatus::Ok => self.inner.errors_in_row.store(0, Ordering::SeqCst),
            _ => {}
        }
    }

    /// Merkt sich einen Tool-Call im Fenster von `tool.repeat_count` (POL-018) und liefert die
    /// Zahl identischer Calls darin (einschließlich dieses). Dieselbe Call-ID zählt einmal.
    fn note_call(&self, call_id: &str, n: &beton_policy::normalize::NormalizedTool) -> i64 {
        let id = beton_policy::normalize::call_identity(&n.kind, &n.name, &n.args);
        let Ok(mut recent) = self.inner.recent.lock() else {
            return 1;
        };
        match recent.iter_mut().find(|(c, _)| c == call_id) {
            Some(entry) => entry.1 = id.clone(),
            None => {
                recent.push_back((call_id.to_owned(), id.clone()));
                while recent.len() > REPEAT_WINDOW {
                    recent.pop_front();
                }
            }
        }
        i64::try_from(recent.iter().filter(|(_, i)| *i == id).count()).unwrap_or(i64::MAX)
    }

    /// Ein `tool.call.requested` aus dem Event-Strom (Zuordnung im Relay-Hub, Reihenfolge).
    pub fn note_requested(&self, call_id: &str, mcp_server: Option<&str>, tool: &str) {
        if let Ok(mut r) = self.inner.requested.lock() {
            r.push((
                call_id.to_owned(),
                mcp_server.map(str::to_owned),
                tool.to_owned(),
            ));
            if r.len() > 256 {
                r.remove(0);
            }
        }
        self.inner.seen.notify_waiters();
    }

    /// Der Runner meldet `tool.call.requested` über [`PolicyGate::note_requested`]; danach
    /// wartet ein `policy.decision` kurz auf sein Subjekt, damit es im Log dahinter steht.
    pub fn order_after_requests(&self) {
        self.inner.ordered.store(true, Ordering::SeqCst);
    }

    fn seen(&self, call_id: &str) -> bool {
        self.inner
            .requested
            .lock()
            .map(|r| r.iter().any(|(id, _, _)| id == call_id))
            .unwrap_or(true)
    }

    async fn wait_seen(&self, call_id: &str) {
        // Eigene IDs des Relay-Hubs (ohne gemeldeten Tool-Call) warten nicht.
        if !self.inner.ordered.load(Ordering::SeqCst) || call_id.starts_with(SYNTHETIC) {
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        while !self.seen(call_id) {
            let notified = self.inner.seen.notified();
            if self.seen(call_id) || tokio::time::timeout_at(deadline, notified).await.is_err() {
                return;
            }
        }
    }

    /// Gibt es für den Call schon ein `policy.decision`?
    pub fn decided(&self, call_id: &str) -> bool {
        self.inner
            .decided
            .lock()
            .map(|d| d.contains_key(call_id))
            .unwrap_or(false)
    }

    fn emit(&self, payload: EventPayload, turn: Option<TurnId>, raw: Option<RawJson>) {
        let _ = self.inner.events.send(GateEvent {
            payload,
            turn,
            raw,
            fatal: false,
        });
    }

    /// Liest die Policy-Verzeichnisse neu (POL-001, #182) und liefert das effektive Set.
    /// Änderungen gehen als `policy.set_changed`/`policy.set_invalid` ins Log.
    pub fn refresh(&self, turn: Option<TurnId>) -> Arc<EffectiveSet> {
        let Ok(mut store) = self.inner.store.lock() else {
            // Vergifteter Zustand: ein leeres Set mit einer nie gültigen Datei verweigert den
            // Start; Auswertungen scheitern ohnehin (fail closed).
            return Arc::new(EffectiveSet::new(Vec::new()).with_never_valid(vec![
                beton_core::event::PolicySetInvalid {
                    file: "<policy-store>".into(),
                    line: 0,
                    error: "interner Fehler".into(),
                },
            ]));
        };
        let events = store.refresh();
        let changed = !events.is_empty();
        for e in events {
            let payload = match e {
                StoreEvent::Changed(c) => EventPayload::PolicySetChanged(c),
                StoreEvent::Invalid(i) => EventPayload::PolicySetInvalid(i),
            };
            self.emit(payload, turn, None);
        }
        let set = store.effective();
        drop(store);
        self.report_lints(&set, turn);
        if changed && self.inner.started.load(Ordering::SeqCst) {
            self.recheck(&set, turn);
        }
        set
    }

    /// Lint-Hinweise jeder Set-Version einmal als Warnung (`notice`, #242, #248).
    fn report_lints(&self, set: &EffectiveSet, turn: Option<TurnId>) {
        for s in set.sets() {
            let fresh = self
                .inner
                .linted
                .lock()
                .map(|mut l| l.insert(s.version.clone()))
                .unwrap_or(false);
            if !fresh {
                continue;
            }
            for (rule, text) in s.lints() {
                self.emit(
                    EventPayload::Notice(Notice {
                        level: NoticeLevel::Warn,
                        text: format!(
                            "Policy `{}` ({}), Regel `{rule}`: {text}",
                            s.name,
                            s.scope.as_str()
                        ),
                    }),
                    turn,
                    None,
                );
            }
        }
    }

    /// Nach dem Start bei jeder Änderung dieselbe Prüfung wie vor dem Start (#234): Nie gültige
    /// Dateien und nicht durchsetzbare Pflichtregeln beenden die Session (fail closed).
    fn recheck(&self, set: &EffectiveSet, turn: Option<TurnId>) {
        let (code, detail, rules) = if set.never_valid().is_empty() {
            let check = self.start_check(set);
            if check.refused.is_empty() {
                return;
            }
            (
                "policy_enforcement_unavailable",
                check
                    .refused
                    .iter()
                    .map(beton_policy::enforcement::Gap::describe)
                    .collect::<Vec<_>>()
                    .join("; "),
                check.refused_rules(),
            )
        } else {
            (
                "policy_invalid",
                set.never_valid()
                    .iter()
                    .map(|f| {
                        format!(
                            "Policy-Datei {} ist ungültig (Zeile {}): {}",
                            f.file, f.line, f.error
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
                Vec::new(),
            )
        };
        let _ = self.inner.events.send(GateEvent {
            payload: EventPayload::Error(beton_core::event::ErrorEvent {
                problem: json!({
                    "type": format!("urn:beton:problem:{code}"),
                    "code": code,
                    "title": "Geänderte Policies sind hier nicht durchsetzbar; Session aus Sicherheitsgründen beendet",
                    "detail": detail,
                    "rules": rules,
                }),
            }),
            turn,
            raw: None,
            fatal: true,
        });
    }

    /// Vor dem Start prüft der Runner selbst (Start verweigert statt Session beendet); bis
    /// [`PolicyGate::mark_started`] lösen Reloads keine Prüfung aus.
    pub fn defer_rechecks(&self) {
        self.inner.started.store(false, Ordering::SeqCst);
    }

    /// Die Session ist gestartet: ab jetzt prüft jeder Reload wie der Start (#234). Prüft
    /// sofort einmal, damit eine Änderung zwischen Startprüfung und Start nicht durchrutscht.
    pub fn mark_started(&self) {
        self.inner.started.store(true, Ordering::SeqCst);
        let before = self.inner.store.lock().ok().map(|mut s| s.effective());
        let set = self.refresh(None);
        // Hat `refresh` eine Änderung gefunden, hat es schon geprüft.
        if before.is_some_and(|b| Arc::ptr_eq(&b, &set)) {
            self.recheck(&set, None);
        }
    }

    /// Prüft die Regeln gegen die Matrix (POL-024); Warnungen gehen sofort ins Log.
    pub fn start_check(&self, set: &EffectiveSet) -> StartCheck {
        let check = check_start(set, &self.matrix());
        if !check.degraded.is_empty() {
            self.emit(
                EventPayload::PolicyEnforcementDegraded(PolicyRuleIssue {
                    rule_ids: check.degraded_rules(),
                    reason: check
                        .degraded
                        .iter()
                        .map(beton_policy::enforcement::Gap::describe)
                        .collect::<Vec<_>>()
                        .join("; "),
                }),
                None,
                None,
            );
        }
        check
    }

    fn base_vars(&self, actor: &str) -> Vars {
        let s = self
            .inner
            .settings
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        Vars {
            now: Some(CelTimestamp::now()),
            session: Some(SessionVars {
                id: self.inner.opts.session_id.clone(),
                harness: Some(self.inner.opts.harness.to_string()),
                mode: Some("native".into()),
                model: s.model,
                reasoning_effort: s.effort,
                permission_mode: s.permission_mode.map(|m| m.as_str().to_owned()),
                turns: Some(s.turns),
                tool_calls: Some(self.inner.calls.load(Ordering::SeqCst)),
                started_at: Some(self.inner.started_at.clone()),
                worktree: Some(self.inner.opts.workdir.display().to_string()),
                // Bei jeder Auswertung neu: der Agent kann den Branch wechseln (POL-017).
                branch: current_branch(&self.inner.opts.workdir),
                project_root: self
                    .inner
                    .opts
                    .project_root
                    .as_ref()
                    .map(|p| p.display().to_string()),
                ..SessionVars::default()
            }),
            user: Some(UserVars {
                id: Some(UserId::LOCAL.to_string()),
                ..UserVars::default()
            }),
            org: Some(OrgVars {
                id: Some(beton_core::id::OrgId::LOCAL.to_string()),
            }),
            host: Some(HostVars {
                id: Some(beton_core::id::HostId::LOCAL.to_string()),
                os: Some(std::env::consts::OS.into()),
                arch: Some(std::env::consts::ARCH.into()),
                local: Some(true),
                home: self
                    .inner
                    .opts
                    .home
                    .as_ref()
                    .map(|h| h.display().to_string()),
                ..HostVars::default()
            }),
            actor: Some(ActorVars {
                kind: Some(actor.into()),
                id: None,
            }),
            agent: self.inner.opts.agent.clone(),
            approvals: Some(ApprovalsVars {
                granted_in_session: Some(self.inner.granted.load(Ordering::SeqCst)),
                denied_in_session: Some(self.inner.denied.load(Ordering::SeqCst)),
            }),
            ..Vars::default()
        }
    }

    /// Holt Verbrauchswerte, wenn eine Regel der Phase sie benutzt. Ohne Antwort bleiben sie
    /// `null`; Regeln darauf scheitern dann und lehnen ab (fail closed).
    async fn with_usage_values(&self, set: &EffectiveSet, phase: Phase, vars: &mut Vars) {
        let used: BTreeSet<String> = set.programs(phase).flat_map(|p| p.fields()).collect();
        if !USAGE_FIELDS.iter().any(|f| used.contains(*f)) {
            return;
        }
        let Some(source) = &self.inner.usage else {
            return;
        };
        let Ok(Ok(u)) = tokio::time::timeout(USAGE_TIMEOUT, source.usage()).await else {
            tracing::warn!("Policy: Verbrauchswerte nicht verfügbar");
            return;
        };
        if let Some(s) = vars.session.as_mut() {
            s.cost_usd = u.session_cost_usd;
            s.equivalent_usd = u.session_equivalent_usd;
            s.unpriced_models.clone_from(&u.session_unpriced_models);
            s.tokens_in = u.session_tokens_in;
            s.tokens_out = u.session_tokens_out;
        }
        // Bis es Runs gibt (ASY, M5), ist der Run einer Session die Session selbst (POL-004).
        let run = vars.run.get_or_insert_with(RunVars::default);
        run.cost_usd = u.session_cost_usd;
        run.equivalent_usd = u.session_equivalent_usd;
        run.subtree_cost_usd = u.subtree_cost_usd;
        run.subtree_equivalent_usd = u.subtree_equivalent_usd;
        run.unpriced_models.clone_from(&u.session_unpriced_models);
        run.subtree_unpriced_models
            .clone_from(&u.subtree_unpriced_models);
        if let Some(user) = vars.user.as_mut() {
            user.daily_cost_usd = u.user_daily_cost_usd;
            user.daily_equivalent_usd = u.user_daily_equivalent_usd;
            user.daily_unpriced_models
                .clone_from(&u.user_daily_unpriced_models);
            user.monthly_cost_usd = u.user_monthly_cost_usd;
            user.timezone = u.timezone.clone();
        }
        self.report_inapplicable(set, &u);
    }

    /// Budget-Regeln, deren Kosten unbekannt sind (Nutzung ohne Geld und ohne Katalogpreis,
    /// POL-011): Die Regel rechnet nur mit den bekannten Beträgen und greift deshalb unter
    /// Umständen nicht; `policy.rule_inapplicable` macht das einmal je Regel sichtbar.
    fn report_inapplicable(&self, set: &EffectiveSet, usage: &Usage) {
        let mut fresh = Vec::new();
        if let Ok(mut seen) = self.inner.inapplicable.lock() {
            for b in beton_policy::budget::budget_rules(set) {
                if usage.without_price(b.scope) == 0 {
                    continue;
                }
                let key = format!(
                    "{}/{}/{}",
                    b.rule.scope.as_str(),
                    b.rule.set,
                    b.rule.rule_id
                );
                if seen.insert(key) {
                    fresh.push(b);
                }
            }
        }
        if fresh.is_empty() {
            return;
        }
        let scopes: BTreeSet<&str> = fresh.iter().map(|b| b.scope.as_str()).collect();
        self.emit(
            EventPayload::PolicyRuleInapplicable(PolicyRuleIssue {
                rule_ids: fresh.iter().map(|b| b.rule.rule_id.clone()).collect(),
                reason: format!(
                    "Nutzung ohne Preis (weder Geld noch Listenpreis-Äquivalent im Katalog) im \
                     Bereich {}: das Budget zählt nur bekannte Kosten",
                    scopes.into_iter().collect::<Vec<_>>().join(", ")
                ),
            }),
            None,
            None,
        );
    }

    /// Wertet das Set auf dem Kontext aus (Engine in eigenem Thread, mit Zeitbudget); Fehler,
    /// Absturz und Zeitüberschreitung ergeben `deny` (POL-024 AC3). Ohne Seiteneffekte.
    async fn run_engine(
        &self,
        set: &Arc<EffectiveSet>,
        phase: Phase,
        vars: Vars,
        source: Option<Arc<ToolSource>>,
    ) -> Decision {
        let capabilities = self.matrix().capabilities(phase);
        let grants = self
            .inner
            .grants
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        match Activation::new(phase, vars.to_json()) {
            Err(e) => engine_deny(phase, set.hash(), format!("Kontext ungültig: {e}")),
            Ok(act) => {
                let mut act = act.with_resolver(Arc::new(FsResolver));
                if let Some(s) = source {
                    act = act.with_tool_source(s);
                }
                let evaluator = self.inner.evaluator.clone();
                let set2 = set.clone();
                let limits = Limits::runtime();
                let timeout = eval_timeout(&limits);
                let job = tokio::task::spawn_blocking(move || {
                    let opts = EvalOptions {
                        capabilities,
                        grants: Some(&grants),
                        limits,
                        ..EvalOptions::default()
                    };
                    evaluator.evaluate(&set2, &act, &opts)
                });
                match tokio::time::timeout(timeout, job).await {
                    Ok(Ok(Ok(d))) => d,
                    Ok(Ok(Err(e))) => engine_deny(phase, set.hash(), format!("Engine-Fehler: {e}")),
                    Ok(Err(_)) => engine_deny(phase, set.hash(), "Engine abgestürzt".into()),
                    Err(_) => engine_deny(
                        phase,
                        set.hash(),
                        "Policy-Auswertung überschritt das Zeitbudget (fail closed)".into(),
                    ),
                }
            }
        }
    }

    /// Eine Auswertung samt `policy.decision` (POL-025).
    #[allow(clippy::too_many_arguments)]
    async fn evaluate(
        &self,
        phase: Phase,
        mut vars: Vars,
        source: Option<Arc<ToolSource>>,
        level: Level,
        subject_ref: Value,
        subject: Option<&Value>,
        turn: Option<TurnId>,
        raw: Option<RawJson>,
    ) -> Evaluated {
        let set = self.refresh(turn);
        self.ensure_snapshot(&set, phase);
        self.with_usage_values(&set, phase, &mut vars).await;
        self.with_state_values(&set, phase, &mut vars).await;
        let decision = self.run_engine(&set, phase, vars, source).await;
        let approval = matches!(decision.outcome, Outcome::Ask { .. }).then(ApprovalId::new);
        let event = decision_event(
            &decision,
            DecisionMeta {
                decision_id: beton_policy::audit::new_decision_id(),
                enforcement: level,
                subject_ref,
                subject_hash: subject.map(subject_hash),
                approval_id: approval,
            },
        );
        // Die Original-Zeile einer Rückfrage enthält die Argumente im Klartext; sie gehört nicht
        // an `policy.decision` (POL-025 AC2, #234).
        let _ = raw;
        self.emit(EventPayload::PolicyDecision(event), turn, None);
        // Erreichte Budget-Limits (POL-011, POL-012): `deny` bzw. `ask` ab `limit_usd`.
        for e in beton_policy::budget::exhausted(&decision, &set) {
            self.emit(
                EventPayload::BudgetExhausted(BudgetExhausted {
                    scope: e.scope.as_str().to_owned(),
                    lease_id: None,
                    action: e.action,
                }),
                turn,
                None,
            );
        }
        // `notify` blockiert nie (POL-006 AC2): `inbox` (und bis Web-Push bzw.
        // Desktop-Notifications auch `push`) wird zum Inbox-Eintrag (`policy.notified`, UX-001),
        // `log` zum Hinweis im Verlauf.
        for n in &decision.notifications {
            use beton_policy::doc::Channel;
            let channels: Vec<String> = n
                .channels
                .iter()
                .map(|c| match c {
                    Channel::Inbox => "inbox",
                    Channel::Push => "push",
                    Channel::Log => "log",
                })
                .map(str::to_owned)
                .collect();
            if n.channels
                .iter()
                .any(|c| matches!(c, Channel::Inbox | Channel::Push))
            {
                self.emit(
                    EventPayload::PolicyNotified(PolicyNotified {
                        rule_id: n.rule.rule_id.clone(),
                        scope: n.rule.scope.as_str().to_owned(),
                        set: n.rule.set.clone(),
                        channels: channels.clone(),
                        message: n.message.clone(),
                    }),
                    turn,
                    None,
                );
            }
            if n.channels.contains(&Channel::Log) {
                self.emit(
                    EventPayload::Notice(Notice {
                        level: NoticeLevel::Info,
                        text: format!(
                            "Policy-Hinweis ({}): {}",
                            n.rule.rule_id,
                            n.message.as_deref().unwrap_or("Regel hat gegriffen")
                        ),
                    }),
                    turn,
                    None,
                );
            }
        }
        Evaluated { decision, approval }
    }

    /// Wer fragt und in welchem Kontext (Approval-Card, POL-010).
    fn card_context(&self, subject: &mut Value) {
        subject["requested_by"] = json!({
            "actor": "agent",
            "harness": self.inner.opts.harness.to_string(),
            "agent": self.inner.opts.agent.as_ref().and_then(|a| a.name.clone()),
        });
        subject["context"] = json!({
            "worktree": self.inner.opts.workdir.display().to_string(),
        });
    }

    /// Menschliche Freigabe: `approval.requested`, warten, `approval.resolved` (POL-010).
    /// Frist, `on_timeout` und Optionen kommen aus dem Plan der Card; ohne Plan (Rückfrage des
    /// Vendors) gilt die Frist des Gates und `deny`. Merken (`approve_session`) wirkt nur, wenn
    /// der Plan es anbietet; dann erfüllen Grants künftig dieselben Regeln für denselben
    /// Fingerprint.
    #[allow(clippy::too_many_arguments)]
    async fn ask_human(
        &self,
        kind: ApprovalKind,
        key: String,
        approval_id: ApprovalId,
        mut subject: Value,
        turn: Option<TurnId>,
        plan: Option<&ApprovalPlan>,
        reasons: &[Reason],
    ) -> Answered {
        let timeout = plan.map_or(self.inner.opts.approval_timeout, |p| p.timeout);
        let on_timeout = plan.map_or(beton_policy::doc::TimeoutAction::Deny, |p| p.on_timeout);
        let options: Vec<String> = match (kind, plan) {
            (ApprovalKind::Question, _) => vec!["answer".into()],
            (_, Some(p)) => p.options().into_iter().map(str::to_owned).collect(),
            (_, None) => vec!["approve".into(), "deny".into()],
        };
        subject["call_id"] = Value::String(key.clone());
        if subject.get("requested_by").is_none() {
            self.card_context(&mut subject);
        }
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.inner.pending.lock() {
            p.insert(key.clone(), (tx, kind));
        }
        let expires = time::OffsetDateTime::now_utc()
            + time::Duration::try_from(timeout).unwrap_or(time::Duration::HOUR);
        let wire_timeout = match on_timeout {
            beton_policy::doc::TimeoutAction::Allow => TimeoutAction::Allow,
            beton_policy::doc::TimeoutAction::Deny => TimeoutAction::Deny,
            beton_policy::doc::TimeoutAction::Abort => TimeoutAction::Abort,
        };
        self.emit(
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id,
                kind,
                subject,
                options,
                expires_at: Timestamp::from(expires),
                on_timeout: wire_timeout,
            }),
            turn,
            None,
        );
        let answer = tokio::time::timeout(timeout, rx).await;
        if let Ok(mut p) = self.inner.pending.lock() {
            p.remove(&key);
        }
        let (mut human, via) = match answer {
            Ok(Ok(h)) => (h, ResolvedVia::User),
            Ok(Err(_)) => (
                Human {
                    reason: Some("Runner beendet".into()),
                    ..Human::default()
                },
                ResolvedVia::System,
            ),
            Err(_) => (
                Human {
                    allow: kind != ApprovalKind::Question && wire_timeout == TimeoutAction::Allow,
                    ..Human::default()
                },
                ResolvedVia::Timeout,
            ),
        };
        let remember = via == ResolvedVia::User
            && human.allow
            && human.remember
            && plan.is_some_and(|p| p.approve_session);
        human.remember = remember;
        if remember && let Ok(mut g) = self.inner.grants.lock() {
            for r in reasons {
                if let Some(fp) = &r.fingerprint {
                    g.add(&r.rule, fp);
                }
            }
        }
        let answer_text = human.answer.clone().filter(|a| {
            kind == ApprovalKind::Question && !a.trim().is_empty() && via == ResolvedVia::User
        });
        let decision = if answer_text.is_some() {
            ApprovalDecision::Answer
        } else if via == ResolvedVia::Timeout && wire_timeout == TimeoutAction::Abort {
            ApprovalDecision::Abort
        } else if human.allow && kind != ApprovalKind::Question {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Deny
        };
        if decision == ApprovalDecision::Allow || decision == ApprovalDecision::Answer {
            self.inner.granted.fetch_add(1, Ordering::SeqCst);
        } else {
            self.inner.denied.fetch_add(1, Ordering::SeqCst);
        }
        let actor = match via {
            ResolvedVia::User => human.actor.clone().unwrap_or(Actor::User {
                id: PrincipalId::User(UserId::LOCAL),
                device_id: None,
            }),
            _ => Actor::System {
                component: SystemComponent::Runner,
            },
        };
        self.emit(
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id,
                decision,
                answer: answer_text.clone(),
                actor,
                via,
                remember: remember.then_some(RememberScope::Session),
                comment: human.reason.clone(),
                on_timeout_applied: (via == ResolvedVia::Timeout).then_some(wire_timeout),
            }),
            turn,
            None,
        );
        let outcome = match (via, decision) {
            (ResolvedVia::Timeout | ResolvedVia::System, _) => FinalOutcome::Expired,
            (_, ApprovalDecision::Allow | ApprovalDecision::Answer) => FinalOutcome::Allow,
            _ => FinalOutcome::Deny,
        };
        human.answer = answer_text;
        Answered {
            human,
            outcome,
            aborted: decision == ApprovalDecision::Abort,
        }
    }

    /// Frage an den Menschen (`ask_user`, AGT-007 AC4): eine Inbox-Frage
    /// (`approval.requested { kind: question }`); die Antwort kommt zurück, ohne Antwort binnen
    /// `timeout` `None`.
    pub async fn ask_question(
        &self,
        key: &str,
        question: String,
        choices: Vec<String>,
        timeout: Duration,
        turn: Option<TurnId>,
    ) -> Option<String> {
        let summary: String = question.chars().take(200).collect();
        let subject = json!({
            "title": "Frage des Agents",
            "summary": summary,
            "question": question,
            "choices": choices,
        });
        let plan = ApprovalPlan {
            timeout,
            on_timeout: beton_policy::doc::TimeoutAction::Deny,
            approve_session: false,
        };
        let answered = self
            .ask_human(
                ApprovalKind::Question,
                key.to_owned(),
                ApprovalId::new(),
                subject,
                turn,
                Some(&plan),
                &[],
            )
            .await;
        answered.human.answer
    }

    /// Wartet eine Freigabe auf einen Menschen? (POL-010 AC4: dann überdauert der Runner den
    /// Daemon.)
    pub fn has_pending(&self) -> bool {
        self.inner
            .pending
            .lock()
            .map(|p| !p.is_empty())
            .unwrap_or(true)
    }

    /// Wartet eine **Policy-Freigabe**? Nur sie hält den Runner über einen geordnet
    /// abgekoppelten Daemon hinaus am Leben (POL-010 AC4); Fragen (`ask_user`) nicht.
    pub fn has_pending_policy(&self) -> bool {
        self.inner
            .pending
            .lock()
            .map(|p| p.values().any(|(_, k)| *k == ApprovalKind::Policy))
            .unwrap_or(false)
    }

    /// Entscheidung aus `approval.resolve` (Schlüssel: `subject.call_id`). `false`, wenn
    /// nichts offen ist.
    pub fn resolve(&self, key: &str, human: Human) -> bool {
        self.inner
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(key))
            .is_some_and(|(tx, _)| tx.send(human).is_ok())
    }

    fn tool_source(&self, req: &GateRequest) -> Arc<ToolSource> {
        // Der Fake-Harness spielt Tool-Namen wie Claude Code ab (HAR-026).
        let family = match harness_family(self.inner.opts.harness.as_str()) {
            "fake" => "claude",
            other => other,
        };
        Arc::new(ToolSource {
            harness: family.to_owned(),
            native_name: req.tool.clone(),
            acp_kind: req.acp_kind.clone(),
            mcp_server: req.mcp_server.clone(),
            cwd: self.inner.opts.workdir.display().to_string(),
            home: self
                .inner
                .opts
                .home
                .as_ref()
                .map(|h| h.display().to_string()),
            // Vergifteter Zustand: niemandem vertrauen (fail closed).
            trusted_servers: Some(
                self.inner
                    .trusted_servers
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default(),
            ),
        })
    }

    /// Phase `tool_call`: wertet aus und fragt bei `ask` (außer `observe_only`) den Menschen.
    async fn tool_verdict(&self, req: &GateRequest, level: Level) -> Verdict {
        let source = self.tool_source(req);
        let call_index = self.inner.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let subject = json!({"tool": req.tool, "args": req.args});
        let subject_ref = json!({"call_id": req.call_id});
        let mut vars = self.base_vars("agent");
        self.wait_seen(&req.call_id).await;
        let pre = self.precheck(req);
        let verdict = match pre.tool {
            Err(e) => {
                // Nicht normalisierbar: ablehnen und auditieren (fail closed).
                let set = self.refresh(req.turn_id);
                let decision = engine_deny(Phase::ToolCall, set.hash(), e.to_string());
                let event = decision_event(
                    &decision,
                    DecisionMeta {
                        subject_ref,
                        subject_hash: Some(subject_hash(&subject)),
                        ..DecisionMeta::new(level)
                    },
                );
                self.emit(EventPayload::PolicyDecision(event), req.turn_id, None);
                Verdict {
                    allowed: false,
                    human: false,
                    reason: Some(format!("Tool-Call nicht prüfbar: {e}")),
                    updated_args: None,
                    subject: tool_subject(&req.tool, &req.args),
                }
            }
            Ok(n) => {
                if let Some(reason) = self.inner.halted.get() {
                    return self.builtin_denied(
                        req,
                        &subject,
                        subject_ref,
                        level,
                        "sandbox-halted",
                        reason.clone(),
                    );
                }
                if let Some(path) = pre.policy_files {
                    return self.policy_files_denied(req, &subject, subject_ref, level, &path);
                }
                let mut tool = tool_vars(&n, Some(call_index));
                tool.repeat_count = Some(self.note_call(&req.call_id, &n));
                tool.consecutive_errors = Some(self.inner.errors_in_row.load(Ordering::SeqCst));
                vars.tool = Some(tool);
                let ev = self
                    .evaluate(
                        Phase::ToolCall,
                        vars,
                        Some(source),
                        level,
                        subject_ref,
                        Some(&subject),
                        req.turn_id,
                        req.raw.clone(),
                    )
                    .await;
                let updated = ev.decision.modified.args.as_ref().map(|patch| {
                    let mut a = req.args.clone();
                    merge_patch(&mut a, patch);
                    a
                });
                match (&ev.decision.outcome, level) {
                    // Nur beobachtbar: protokolliert, aber nicht durchgesetzt.
                    (outcome, Level::ObserveOnly) => {
                        let fin = match outcome {
                            Outcome::Allow => FinalOutcome::Allow,
                            Outcome::Deny { reasons } => {
                                if let Some(first) = reasons.first()
                                    && let Ok(mut m) = self.inner.observed_denies.lock()
                                {
                                    m.insert(
                                        req.call_id.clone(),
                                        (first.rule.clone(), reason_text(reasons)),
                                    );
                                }
                                FinalOutcome::Deny
                            }
                            Outcome::Ask { .. } => FinalOutcome::Expired,
                        };
                        self.apply_effects(&ev.decision, fin, req.turn_id).await;
                        Verdict {
                            allowed: true,
                            human: false,
                            reason: None,
                            subject: tool_subject(&req.tool, updated.as_ref().unwrap_or(&req.args)),
                            updated_args: updated,
                        }
                    }
                    (Outcome::Allow, _) => {
                        self.apply_effects(&ev.decision, FinalOutcome::Allow, req.turn_id)
                            .await;
                        Verdict {
                            allowed: true,
                            human: false,
                            reason: None,
                            subject: tool_subject(&req.tool, updated.as_ref().unwrap_or(&req.args)),
                            updated_args: updated,
                        }
                    }
                    (Outcome::Deny { reasons }, _) => {
                        self.apply_effects(&ev.decision, FinalOutcome::Deny, req.turn_id)
                            .await;
                        Verdict {
                            allowed: false,
                            human: false,
                            reason: Some(reason_text(reasons)),
                            updated_args: None,
                            subject: tool_subject(&req.tool, &req.args),
                        }
                    }
                    (Outcome::Ask { reasons }, _) => {
                        let card = plan(reasons, None);
                        let subject = tool_card(
                            "tool_call",
                            &req.tool,
                            &n.kind,
                            n.command
                                .as_deref()
                                .or_else(|| n.paths.first().map(String::as_str)),
                            &req.args,
                            updated.as_ref(),
                            reasons,
                        );
                        let answered = self
                            .ask_human(
                                ApprovalKind::Policy,
                                req.call_id.clone(),
                                ev.approval.unwrap_or_default(),
                                subject,
                                req.turn_id,
                                Some(&card),
                                reasons,
                            )
                            .await;
                        self.apply_effects(&ev.decision, answered.outcome, req.turn_id)
                            .await;
                        let human = answered.human;
                        // Vom Menschen geänderte Argumente prüft die Policy erneut (#234).
                        let denied = match &human.updated_args {
                            Some(new) if human.allow => {
                                self.recheck_human_args(req, level, new).await
                            }
                            _ => None,
                        };
                        let updated_args = human.updated_args.or(updated);
                        Verdict {
                            allowed: human.allow && denied.is_none(),
                            human: true,
                            reason: denied.or(human.reason).or_else(|| {
                                Some(if answered.aborted {
                                    "Freigabe abgelaufen; der Turn wird abgebrochen (beton-Policy)"
                                        .to_owned()
                                } else {
                                    reason_text(reasons)
                                })
                            }),
                            subject: tool_subject(
                                &req.tool,
                                updated_args.as_ref().unwrap_or(&req.args),
                            ),
                            updated_args,
                        }
                    }
                }
            }
        };
        if let Ok(mut d) = self.inner.decided.lock() {
            d.insert(req.call_id.clone(), verdict.clone());
        }
        verdict
    }

    /// Integritätsprüfung (HAR-005, #233): Meldet die Matrix für die Klasse des Calls `full`,
    /// muss er vor der Ausführung das Gate passiert haben. Ein ausgeführter Call ohne
    /// Entscheidung heißt, dass der Hook umgangen wurde; die Begründung nennt nur Tool und
    /// Klasse, keine Argumente.
    pub fn integrity_violation(&self, req: &GateRequest) -> Option<String> {
        if self.decided(&req.call_id) {
            return None;
        }
        let kind = self.kind_of(req);
        (self.matrix().level(Phase::ToolCall, Some(&kind)) == Level::Full).then(|| {
            format!(
                "Tool-Call `{}` ({kind}) lief ohne Prüfung durch das Policy-Gate, obwohl der Harness volle Durchsetzung meldet; Session aus Sicherheitsgründen beendet",
                req.tool
            )
        })
    }

    /// Kanonische Policy-Verzeichnisse (einmal aufgelöst; das Ziel eines Symlinks kann sich
    /// während der Session ändern, deshalb nur, solange es das Verzeichnis gibt).
    fn policy_dirs(&self) -> Vec<PathBuf> {
        if let Ok(cache) = self.inner.policy_dirs.lock()
            && let Some(dirs) = cache.as_ref()
        {
            return dirs.clone();
        }
        let dirs: Vec<PathBuf> = self
            .inner
            .opts
            .dirs
            .iter()
            .map(|(_, d)| PathBuf::from(resolve_fs(&d.display().to_string())))
            .collect();
        if self.inner.opts.dirs.iter().all(|(_, d)| d.exists())
            && let Ok(mut cache) = self.inner.policy_dirs.lock()
        {
            *cache = Some(dirs.clone());
        }
        dirs
    }

    /// Gemeinsame Vorprüfungen eines Tool-Calls (#253, Security-Review B2 zu #257): Echte
    /// Calls (`decide`, `tool_verdict`, Nachprüfung geänderter Argumente) und `policy_query`
    /// rufen genau diese Funktion auf, damit die Abfrage dieselbe Entscheidung liefert.
    pub(crate) fn precheck(&self, req: &GateRequest) -> Precheck {
        self.precheck_args(req, &req.args)
    }

    /// Wie [`PolicyGate::precheck`] mit anderen Argumenten (vom Menschen geändert).
    fn precheck_args(&self, req: &GateRequest, args: &Value) -> Precheck {
        let normalized = normalize_source(&self.tool_source(req), args);
        // Für MCP-Tools entscheidet die Normalisierung mit der Herkunft des Servers (#234):
        // Ein fremder Server namens `beton` ist kein System-Tool, auch wenn der Adapter das meint.
        let kind = match &normalized {
            Ok(n) if n.server != "builtin" => n.kind.clone(),
            _ if req.kind.is_empty() => beton_harness::tool_kind(
                self.inner.opts.harness.as_str(),
                &req.tool,
                req.acp_kind.as_deref(),
                req.mcp_server.as_deref(),
            ),
            _ => req.kind.clone(),
        };
        // #151 gilt nur für System-Tools, die beton selbst einbindet (reservierter Server),
        // nicht für Claude-Builtins der Klasse `system` (Re-Review R3 zu #253, HAR-027).
        let beton_system =
            kind == "system" && normalized.as_ref().is_ok_and(|n| n.server != "builtin");
        let tool = normalized
            .map(|mut n| {
                adapter_kind(&mut n, req);
                n
            })
            .map_err(|e| e.to_string());
        let policy_files = tool
            .as_ref()
            .ok()
            .and_then(|n| self.touches_policy_files(n));
        Precheck {
            missing_call_id: req.call_id.trim().is_empty(),
            tool,
            kind,
            policy_files,
            beton_system,
            plan_exit: req.tool == EXIT_PLAN_MODE,
        }
    }

    /// Kanonische Klasse einer Anfrage (aus [`PolicyGate::precheck`]).
    fn kind_of(&self, req: &GateRequest) -> String {
        self.precheck(req).kind
    }

    /// Prüft vom Menschen geänderte Argumente erneut (eigenes `policy.decision` mit
    /// `subject_ref.recheck`); `Some(Begründung)` bei `deny`. Ein `ask` gilt als beantwortet:
    /// der Mensch hat genau diese Argumente freigegeben (#234).
    async fn recheck_human_args(
        &self,
        req: &GateRequest,
        level: Level,
        args: &Value,
    ) -> Option<String> {
        let source = self.tool_source(req);
        let pre = self.precheck_args(req, args);
        let n = match pre.tool {
            Ok(n) => n,
            Err(e) => return Some(format!("Geänderte Argumente nicht prüfbar: {e}")),
        };
        if let Some(path) = pre.policy_files {
            return Some(policy_files_reason(&path));
        }
        let mut vars = self.base_vars("agent");
        vars.tool = Some(tool_vars(&n, None));
        let subject = json!({"tool": req.tool, "args": args});
        let ev = self
            .evaluate(
                Phase::ToolCall,
                vars,
                Some(source),
                level,
                json!({"call_id": req.call_id, "recheck": true}),
                Some(&subject),
                req.turn_id,
                None,
            )
            .await;
        match ev.decision.outcome {
            Outcome::Deny { reasons } => Some(reason_text(&reasons)),
            Outcome::Allow | Outcome::Ask { .. } => None,
        }
    }

    /// Pfad unter einem Policy-Verzeichnis, den ein nicht nur lesender Tool-Call berührt
    /// (#234): Policies ändert nur der Mensch, nie ein Agent. Symlinks und Groß-/Kleinschreibung
    /// löst die Pfadauflösung des Dateisystems auf.
    fn touches_policy_files(&self, n: &beton_policy::normalize::NormalizedTool) -> Option<String> {
        if READ_ONLY_KINDS.contains(&n.kind.as_str()) {
            return None;
        }
        let guard = PolicyPathGuard {
            dirs: self.policy_dirs(),
            cwd: n.cwd.clone(),
            home: self
                .inner
                .opts
                .home
                .as_ref()
                .map(|h| h.display().to_string()),
        };
        if let Some(p) = n.paths.iter().find(|p| guard.hits_path(p)) {
            return Some(p.clone());
        }
        // Shell (Re-Review R1, #253): jedes Wort aller Teilkommandos, nicht nur solche, die
        // wie ein Pfad aussehen; `$`-Variablen aus Zuweisungen im selben Kommando eingesetzt.
        if !n.argv.is_empty() {
            // Der Parser entfernt Zuweisungen aus `argv`; sie stehen im Kommando-Text.
            let words: Vec<String> = n
                .command
                .as_deref()
                .unwrap_or_default()
                .split(|c: char| c.is_whitespace() || ";&|()".contains(c))
                .map(|w| w.trim_matches(|c| c == '"' || c == '\'').to_owned())
                .collect();
            let vars = assignments(std::slice::from_ref(&words));
            for sub in &n.argv {
                for (i, word) in sub.iter().enumerate() {
                    // Der Programmname ohne `/` ist kein Pfad (nur auf `.beton` prüfen).
                    let program = i == 0 && !word.contains('/');
                    if (program && word.to_lowercase().contains(".beton"))
                        || (!program && guard.hits_word(word, &vars))
                    {
                        return Some(word.clone());
                    }
                }
            }
            if n.parse_uncertain
                && let Some(cmd) = &n.command
                && mentions_policies(cmd)
            {
                return Some(cmd.clone());
            }
        }
        // MCP- und unbekannte Tools: alle Zeichenketten der Argumente (ohne Pfad-Abbildung).
        if matches!(n.kind.as_str(), "mcp" | "other") {
            let mut strings = Vec::new();
            collect_strings(&n.args, &mut strings);
            for word in strings.iter().flat_map(|s| s.split_whitespace()) {
                if guard.hits_word(word, &HashMap::new()) {
                    return Some(word.to_owned());
                }
            }
        }
        None
    }

    /// `deny` eines Tool-Calls auf Policy-Dateien samt `policy.decision` (#234).
    fn policy_files_denied(
        &self,
        req: &GateRequest,
        subject: &Value,
        subject_ref: Value,
        level: Level,
        path: &str,
    ) -> Verdict {
        let reason = policy_files_reason(path);
        self.builtin_denied(req, subject, subject_ref, level, "policy-files", reason)
    }

    /// Hält das Gate an (SBX-006 AC3): Jede weitere Tool-Ausführung wird mit `reason`
    /// abgelehnt (eingebaute Regel `sandbox-halted`), bis die Session endet.
    pub fn halt_tools(&self, reason: &str) {
        let _ = self.inner.halted.set(reason.to_owned());
    }

    /// Ablehnung durch eine eingebaute Regel, ohne Auswertung der Policies (fail closed).
    fn builtin_denied(
        &self,
        req: &GateRequest,
        subject: &Value,
        subject_ref: Value,
        level: Level,
        rule_id: &str,
        reason: String,
    ) -> Verdict {
        let set = self.refresh(req.turn_id);
        let decision = builtin_deny(Phase::ToolCall, set.hash(), rule_id, reason.clone());
        let event = decision_event(
            &decision,
            DecisionMeta {
                subject_ref,
                subject_hash: Some(subject_hash(subject)),
                ..DecisionMeta::new(level)
            },
        );
        self.emit(EventPayload::PolicyDecision(event), req.turn_id, None);
        let verdict = Verdict {
            allowed: level == Level::ObserveOnly,
            human: false,
            reason: Some(reason),
            updated_args: None,
            subject: tool_subject(&req.tool, &req.args),
        };
        if let Ok(mut d) = self.inner.decided.lock() {
            d.insert(req.call_id.clone(), verdict.clone());
        }
        verdict
    }

    /// Tool-Call ohne Gate-Anfrage des Harness (nur beobachtbar, POL-025 AC1): genau ein
    /// `policy.decision` mit `enforcement: observe_only`.
    /// Trifft eine `deny`-Regel, liefert es die erste samt Begründung (POL-023 AC3: dann folgen
    /// `policy.notified` und der Abbruch des Turns).
    pub async fn observe(&self, req: GateRequest) -> Option<(beton_policy::load::RuleRef, String)> {
        if self.decided(&req.call_id) {
            return None;
        }
        let call = req.call_id.clone();
        let _ = self.tool_verdict(&req, Level::ObserveOnly).await;
        self.inner
            .observed_denies
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&call))
    }

    /// Phase `model_request` (Turn oder Call). Mit `ask_human = false` liefert `ask`
    /// [`Checked::AskNeeded`], sonst entscheidet ein Mensch; `waiting` erfährt die ID der
    /// Approval-Card, sobald gewartet wird.
    pub async fn check_model_request(
        &self,
        req: ModelGateRequest,
        granularity: &str,
        ask_human: bool,
        waiting: Option<oneshot::Sender<ApprovalId>>,
    ) -> Checked {
        let mut vars = self.base_vars(if granularity == "turn" {
            "user"
        } else {
            "agent"
        });
        vars.request = Some(RequestVars {
            model: req.model.clone(),
            provider: req.provider.clone(),
            reasoning_effort: req.reasoning_effort.clone(),
            granularity: Some(granularity.into()),
            prompt: req.prompt.as_deref().map(truncate),
            messages: req.messages.clone(),
            tools: Some(req.tools.clone()),
            estimated_input_tokens: req
                .estimated_input_tokens
                .map(|t| i64::try_from(t).unwrap_or(i64::MAX)),
            context_used_pct: None,
        });
        let key = format!("model_request:{}", ApprovalId::new());
        let subject = json!({"model": req.model, "prompt": req.prompt});
        let ev = self
            .evaluate(
                Phase::ModelRequest,
                vars,
                None,
                self.matrix().level(Phase::ModelRequest, None),
                json!({"turn_id": req.turn_id, "granularity": granularity}),
                Some(&subject),
                req.turn_id,
                None,
            )
            .await;
        match ev.decision.outcome.clone() {
            Outcome::Allow => {
                self.apply_effects(&ev.decision, FinalOutcome::Allow, req.turn_id)
                    .await;
                Checked::Allow(ev.decision.modified)
            }
            Outcome::Deny { reasons } => {
                self.apply_effects(&ev.decision, FinalOutcome::Deny, req.turn_id)
                    .await;
                Checked::Deny {
                    reason: reason_text(&reasons),
                    rules: rule_ids(&reasons),
                }
            }
            Outcome::Ask { .. } if !ask_human => Checked::AskNeeded,
            Outcome::Ask { reasons } => {
                let approval = ev.approval.unwrap_or_default();
                if let Some(w) = waiting {
                    let _ = w.send(approval);
                }
                let card = plan(&reasons, None);
                let model = req.model.clone().unwrap_or_default();
                let answered = self
                    .ask_human(
                        ApprovalKind::Policy,
                        key,
                        approval,
                        json!({"phase": "model_request", "title": "Modell-Anfrage bestätigen",
                               "summary": if model.is_empty() { "Nächster Turn".to_owned() } else { format!("Modell {model}") },
                               "model": req.model,
                               "reasons": reasons_json(&reasons)}),
                        req.turn_id,
                        Some(&card),
                        &reasons,
                    )
                    .await;
                self.apply_effects(&ev.decision, answered.outcome, req.turn_id)
                    .await;
                let human = answered.human;
                if human.allow {
                    Checked::Allow(ev.decision.modified)
                } else {
                    Checked::Deny {
                        reason: human.reason.unwrap_or_else(|| reason_text(&reasons)),
                        rules: rule_ids(&reasons),
                    }
                }
            }
        }
    }

    /// Phase `session_start` (POL-003): vor dem Start des Harness. `modify` darf den
    /// Permission-Mode nur verschärfen.
    pub async fn check_start(
        &self,
        start: StartVars,
        waiting: Option<oneshot::Sender<ApprovalId>>,
    ) -> Checked {
        let requested_mode = start
            .permission_mode
            .as_deref()
            .and_then(|m| m.parse::<PermissionMode>().ok());
        let mut vars = self.base_vars("user");
        vars.start = Some(start.clone());
        let key = format!("session_start:{}", ApprovalId::new());
        let ev = self
            .evaluate(
                Phase::SessionStart,
                vars,
                None,
                Level::Full,
                json!({"session_id": self.inner.opts.session_id}),
                None,
                None,
                None,
            )
            .await;
        // Nur strenger (HAR-027, #231): ohne angefragten Modus ist `default` die Basis; ein
        // lockernder oder unbekannter Wert wird verworfen.
        let base = requested_mode.unwrap_or(PermissionMode::Default);
        let tightened = |mut modified: Modifications| {
            let tighter = modified
                .permission_mode
                .as_deref()
                .and_then(|m| m.parse::<PermissionMode>().ok())
                .is_some_and(|new| {
                    crate::settings::strictness(new) <= crate::settings::strictness(base)
                });
            if !tighter {
                modified.permission_mode = None;
            }
            modified
        };
        let modified = tightened(ev.decision.modified.clone());
        match ev.decision.outcome.clone() {
            Outcome::Allow => {
                self.apply_effects(&ev.decision, FinalOutcome::Allow, None)
                    .await;
                Checked::Allow(modified)
            }
            Outcome::Deny { reasons } => {
                self.apply_effects(&ev.decision, FinalOutcome::Deny, None)
                    .await;
                Checked::Deny {
                    reason: reason_text(&reasons),
                    rules: rule_ids(&reasons),
                }
            }
            Outcome::Ask { reasons } => {
                let approval = ev.approval.unwrap_or_default();
                if let Some(w) = waiting {
                    let _ = w.send(approval);
                }
                let card = plan(&reasons, None);
                let asked_at = std::time::Instant::now();
                let answered = self
                    .ask_human(
                        ApprovalKind::Policy,
                        key,
                        approval,
                        json!({"phase": "session_start", "title": "Session-Start bestätigen",
                               "summary": format!("Harness {}", self.inner.opts.harness),
                               "reasons": reasons_json(&reasons)}),
                        None,
                        Some(&card),
                        &reasons,
                    )
                    .await;
                self.apply_effects(&ev.decision, answered.outcome, None)
                    .await;
                let human = answered.human;
                if human.allow {
                    // Während der Wartezeit können sich Policies und Kosten ändern (#297):
                    // erneut auswerten, wenn sich das Set geändert hat oder lange gewartet wurde.
                    let changed = self.refresh(None).hash() != ev.decision.policy_set_hash;
                    if changed || asked_at.elapsed() >= START_RECHECK_AFTER {
                        return self.recheck_start(start, &reasons, tightened).await;
                    }
                    Checked::Allow(modified)
                } else {
                    Checked::Deny {
                        reason: human.reason.unwrap_or_else(|| reason_text(&reasons)),
                        rules: rule_ids(&reasons),
                    }
                }
            }
        }
    }

    /// Erneute Auswertung von `session_start` nach einer Freigabe (#297): `deny` verweigert den
    /// Start; neue `ask`-Gründe, die die Freigabe nicht abdeckt, ebenfalls (fail closed, die
    /// Session muss neu gestartet werden). Effects der erneuten Auswertung gelten nur bei
    /// `deny`; die der freigegebenen Auswertung sind schon angewendet.
    async fn recheck_start(
        &self,
        start: StartVars,
        approved: &[Reason],
        tightened: impl Fn(Modifications) -> Modifications,
    ) -> Checked {
        let mut vars = self.base_vars("user");
        vars.start = Some(start);
        let ev = self
            .evaluate(
                Phase::SessionStart,
                vars,
                None,
                Level::Full,
                json!({"session_id": self.inner.opts.session_id, "recheck": true}),
                None,
                None,
                None,
            )
            .await;
        let same = |a: &Reason, b: &Reason| {
            a.rule.scope == b.rule.scope
                && a.rule.set == b.rule.set
                && a.rule.rule_id == b.rule.rule_id
        };
        match ev.decision.outcome.clone() {
            Outcome::Allow => Checked::Allow(tightened(ev.decision.modified)),
            Outcome::Ask { reasons } if reasons.iter().all(|r| approved.iter().any(|a| same(a, r))) => {
                Checked::Allow(tightened(ev.decision.modified))
            }
            Outcome::Ask { reasons } => Checked::Deny {
                reason: format!(
                    "Policies haben sich während der Freigabe geändert; der Start braucht eine neue Freigabe: {}",
                    reason_text(&reasons)
                ),
                rules: rule_ids(&reasons),
            },
            Outcome::Deny { reasons } => {
                self.apply_effects(&ev.decision, FinalOutcome::Deny, None)
                    .await;
                Checked::Deny {
                    reason: reason_text(&reasons),
                    rules: rule_ids(&reasons),
                }
            }
        }
    }

    /// Ordnet einen Aufruf im Relay-Hub dem jüngsten passenden, noch unentschiedenen
    /// `tool.call.requested` zu; sonst eine eigene ID.
    fn correlate(&self, server: &str, tool: &str) -> String {
        let found = self.inner.requested.lock().ok().and_then(|r| {
            r.iter()
                .rev()
                .find(|(id, s, t)| s.as_deref() == Some(server) && t == tool && !self.decided(id))
                .map(|(id, _, _)| id.clone())
        });
        found.unwrap_or_else(|| format!("{SYNTHETIC}{}", ApprovalId::new()))
    }
}

#[async_trait]
impl Gate for PolicyGate {
    async fn decide(&self, req: GateRequest) -> GateDecision {
        let pre = self.precheck(&req);
        // Ohne Call-ID teilten sich Calls gespeicherte Entscheide (#234): fail closed.
        if pre.missing_call_id {
            tracing::warn!("Policy: Tool-Call ohne Call-ID abgelehnt");
            return GateDecision::Deny {
                reason: Some("Abgelehnt von beton: Tool-Call ohne ID.".into()),
            };
        }
        let kind = pre.kind.clone();
        match req.origin {
            GateOrigin::PreTool => {
                let v = self.tool_verdict(&req, Level::Full).await;
                if v.allowed {
                    GateDecision::Allow {
                        updated_args: v.updated_args,
                    }
                } else {
                    GateDecision::Deny { reason: v.reason }
                }
            }
            GateOrigin::VendorAsk | GateOrigin::AutoApproval => {
                let prior = self
                    .inner
                    .decided
                    .lock()
                    .ok()
                    .and_then(|d| d.get(&req.call_id).cloned());
                let had_prior = prior.is_some();
                // Angehaltene Sandbox (SBX-006 AC3): Ein gespeicherter Entscheid gilt nicht
                // mehr; die Prüfung lehnt mit `sandbox-halted` ab (Security-Review #327). Mit
                // gespeichertem Entscheid gibt es schon ein `policy.decision` (POL-025 AC1);
                // die Ablehnung steht dann als `policy.notified` im Log (#330).
                if let Some(reason) = self.inner.halted.get().cloned() {
                    if !had_prior {
                        let v = self.tool_verdict(&req, Level::ApprovalOnly).await;
                        return GateDecision::Deny { reason: v.reason };
                    }
                    if let Ok(mut d) = self.inner.decided.lock()
                        && let Some(entry) = d.get_mut(&req.call_id)
                    {
                        entry.allowed = false;
                    }
                    self.emit(
                        EventPayload::PolicyNotified(PolicyNotified {
                            rule_id: "sandbox-halted".into(),
                            scope: Scope::Org.as_str().to_owned(),
                            set: String::new(),
                            channels: vec!["ui".into()],
                            message: Some(format!(
                                "Freigabe für `{}` nach der Policy-Prüfung abgelehnt: {reason}",
                                req.tool
                            )),
                        }),
                        req.turn_id,
                        None,
                    );
                    return GateDecision::Deny {
                        reason: Some(reason),
                    };
                }
                let v = match prior {
                    // Gespeicherter Entscheid nur für dasselbe Subjekt (#233): Hat etwas die
                    // Argumente nach der Prüfung geändert (z. B. ein weiterer Hook mit
                    // `updatedInput`), gilt er nicht (fail closed).
                    Some(v) if v.subject != tool_subject(&req.tool, &req.args) => {
                        tracing::warn!(
                            call_id = %req.call_id,
                            "Policy: Rückfrage mit anderen Argumenten als geprüft; abgelehnt"
                        );
                        return GateDecision::Deny {
                            reason: Some(CHANGED_AFTER_CHECK.into()),
                        };
                    }
                    Some(v) => v,
                    None => self.tool_verdict(&req, Level::ApprovalOnly).await,
                };
                if !v.allowed {
                    return GateDecision::Deny { reason: v.reason };
                }
                // Mit Hook sind die Argumente der Rückfrage schon die geänderten.
                let updated = if had_prior { None } else { v.updated_args };
                let yolo = self
                    .inner
                    .settings
                    .lock()
                    .map(|s| s.permission_mode == Some(PermissionMode::Yolo))
                    .unwrap_or(false);
                // #151 gilt nur für System-Tools, die beton selbst einbindet (reservierter
                // Server), nicht für Claude-Builtins der Klasse `system`; `ExitPlanMode`
                // entscheidet immer ein Mensch (Re-Review R3 zu #253, HAR-027).
                let (beton_system, plan_exit) = (pre.beton_system, pre.plan_exit);
                if v.human
                    || (!plan_exit
                        && (req.origin == GateOrigin::AutoApproval || beton_system || yolo))
                {
                    return GateDecision::Allow {
                        updated_args: updated,
                    };
                }
                // Rückfrage des Vendors: der Mensch entscheidet (Approval-Card, WEB-007).
                let args = updated.clone().unwrap_or_else(|| req.args.clone());
                let summary = args["command"]
                    .as_str()
                    .or_else(|| args["file_path"].as_str())
                    .or_else(|| args["path"].as_str())
                    .map(str::to_owned);
                let subject = tool_card(
                    "tool_call",
                    &req.tool,
                    &kind,
                    summary.as_deref(),
                    &args,
                    None,
                    &[],
                );
                let human = self
                    .ask_human(
                        ApprovalKind::Tool,
                        req.call_id.clone(),
                        ApprovalId::new(),
                        subject,
                        req.turn_id,
                        None,
                        &[],
                    )
                    .await
                    .human;
                let denied = match &human.updated_args {
                    Some(new) if human.allow => {
                        let level = if had_prior {
                            Level::Full
                        } else {
                            Level::ApprovalOnly
                        };
                        self.recheck_human_args(&req, level, new).await
                    }
                    _ => None,
                };
                // Während der Rückfrage angehalten (SBX-006 AC3): Die Freigabe gilt nicht.
                let denied = denied.or_else(|| {
                    human
                        .allow
                        .then(|| self.inner.halted.get().cloned())
                        .flatten()
                });
                if let Ok(mut d) = self.inner.decided.lock()
                    && let Some(entry) = d.get_mut(&req.call_id)
                {
                    entry.human = true;
                    entry.allowed = human.allow && denied.is_none();
                }
                if let Some(reason) = denied {
                    return GateDecision::Deny {
                        reason: Some(reason),
                    };
                }
                if human.allow {
                    GateDecision::Allow {
                        updated_args: human.updated_args.or(updated),
                    }
                } else {
                    GateDecision::Deny {
                        reason: human.reason,
                    }
                }
            }
        }
    }

    async fn model_request(&self, req: ModelGateRequest) -> ModelGateDecision {
        match self.check_model_request(req, "call", true, None).await {
            Checked::Allow(m) => ModelGateDecision::Allow {
                model: m.model,
                reasoning_effort: m.reasoning_effort,
            },
            Checked::Deny { reason, .. } => ModelGateDecision::Deny {
                reason: Some(reason),
            },
            Checked::AskNeeded => ModelGateDecision::Deny { reason: None },
        }
    }

    async fn tool_result(&self, req: ResultGateRequest) -> GateDecision {
        let set = self.refresh(req.call.turn_id);
        // Ohne Regeln in dieser Phase keine Auswertung (und kein Event).
        if !set.has_phase(Phase::ToolResult) {
            return GateDecision::Allow { updated_args: None };
        }
        let source = self.tool_source(&req.call);
        let mut vars = self.base_vars("agent");
        match normalize_source(&source, &req.call.args) {
            // Alle `tool.*` wie in `tool_call` (POL-004, #234).
            Ok(mut n) => {
                adapter_kind(&mut n, &req.call);
                vars.tool = Some(tool_vars(&n, None));
            }
            Err(e) => {
                return GateDecision::Deny {
                    reason: Some(format!("Ergebnis nicht prüfbar: {e}")),
                };
            }
        }
        vars.result = Some(ResultVars {
            ok: Some(req.ok),
            exit_code: req.exit_code,
            error: req.error.as_deref().map(truncate),
            text: Some(truncate(&req.text)),
            bytes: Some(i64::try_from(req.bytes).unwrap_or(i64::MAX)),
            duration_ms: Some(i64::try_from(req.duration_ms).unwrap_or(i64::MAX)),
            truncated: Some(req.truncated || req.text.len() > TEXT_MAX),
        });
        let subject = json!({"tool": req.call.tool, "result": req.text});
        let ev = self
            .evaluate(
                Phase::ToolResult,
                vars,
                Some(source),
                Level::Full,
                json!({"call_id": req.call.call_id}),
                Some(&subject),
                req.call.turn_id,
                None,
            )
            .await;
        let turn = req.call.turn_id;
        match ev.decision.outcome.clone() {
            Outcome::Allow => {
                self.apply_effects(&ev.decision, FinalOutcome::Allow, turn)
                    .await;
                GateDecision::Allow { updated_args: None }
            }
            Outcome::Deny { reasons } => {
                self.apply_effects(&ev.decision, FinalOutcome::Deny, turn)
                    .await;
                GateDecision::Deny {
                    reason: Some(reason_text(&reasons)),
                }
            }
            Outcome::Ask { reasons } => {
                let card = plan(&reasons, None);
                let answered = self
                    .ask_human(
                        ApprovalKind::Policy,
                        format!("tool_result:{}", req.call.call_id),
                        ev.approval.unwrap_or_default(),
                        json!({"phase": "tool_result", "title": "Tool-Ergebnis freigeben",
                               "summary": req.call.tool,
                               "tool": {"name": req.call.tool, "kind": req.call.kind},
                               "reasons": reasons_json(&reasons)}),
                        turn,
                        Some(&card),
                        &reasons,
                    )
                    .await;
                self.apply_effects(&ev.decision, answered.outcome, turn)
                    .await;
                let human = answered.human;
                if human.allow {
                    GateDecision::Allow { updated_args: None }
                } else {
                    GateDecision::Deny {
                        reason: human.reason.or_else(|| Some(reason_text(&reasons))),
                    }
                }
            }
        }
    }
}

#[async_trait]
impl crate::mcp::Asker for PolicyGate {
    async fn ask(&self, question: &str, choices: &[String], timeout: Duration) -> Option<String> {
        let key = format!("ask_user:{}", ApprovalId::new());
        self.ask_question(&key, question.to_owned(), choices.to_vec(), timeout, None)
            .await
    }
}

#[async_trait]
impl beton_mcp::hub::McpGate for PolicyGate {
    async fn check(&self, server: &str, tool: &str, args: &Value) -> Result<Option<Value>, String> {
        let call_id = self.correlate(server, tool);
        let req = GateRequest {
            call_id,
            tool: tool.to_owned(),
            kind: beton_harness::tool_kind(
                self.inner.opts.harness.as_str(),
                tool,
                None,
                Some(server),
            ),
            args: args.clone(),
            mcp_server: Some(server.to_owned()),
            origin: GateOrigin::PreTool,
            ..GateRequest::default()
        };
        let v = self.tool_verdict(&req, Level::Full).await;
        if v.allowed {
            Ok(v.updated_args)
        } else {
            Err(v
                .reason
                .unwrap_or_else(|| "Von der beton-Policy abgelehnt".into()))
        }
    }
}

/// Policy-Verzeichnisse einer Session: User `$BETON_HOME/policies`, Projekt
/// `<worktree>/.beton/policies` (POL-001).
pub fn policy_dirs(beton_home: &Path, workdir: &Path) -> Vec<(Scope, PathBuf)> {
    vec![
        (Scope::User, beton_policy::store::policies_dir(beton_home)),
        (
            Scope::Project,
            beton_policy::store::policies_dir(&workdir.join(".beton")),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use beton_core::event::RememberScope;

    fn gate() -> (PolicyGate, mpsc::UnboundedReceiver<GateEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (PolicyGate::new(PolicyOptions::default(), tx), rx)
    }

    /// POL-024 AC3 / #298: Im Betrieb bleibt der Timeout bei 250 ms; nur ein längeres
    /// Zeitbudget der Engine (Integrationstests) verlängert ihn.
    #[test]
    fn pol_024_eval_timeout_is_never_shorter_than_the_engine_budget() {
        let real = Limits::default();
        assert_eq!(eval_timeout(&real), EVAL_TIMEOUT);
        let generous = Limits {
            eval_budget_us: 500_000,
            ..real
        };
        assert_eq!(eval_timeout(&generous), Duration::from_millis(500));
    }

    fn ask(call: &str) -> GateRequest {
        GateRequest {
            call_id: call.into(),
            tool: "Bash".into(),
            kind: "shell".into(),
            args: json!({"command": "git push"}),
            origin: GateOrigin::VendorAsk,
            ..GateRequest::default()
        }
    }

    #[tokio::test]
    async fn web_018_vendor_ask_waits_for_the_human_and_fails_closed() {
        let (g, mut rx) = gate();
        let g2 = g.clone();
        let waiting = tokio::spawn(async move { g2.decide(ask("c1")).await });
        // Erst die Policy (allow), dann die Approval-Card.
        let mut seen = Vec::new();
        while !seen
            .iter()
            .any(|e: &EventPayload| matches!(e, EventPayload::ApprovalRequested(_)))
        {
            seen.push(rx.recv().await.unwrap().payload);
        }
        assert!(
            matches!(seen[0], EventPayload::PolicyDecision(_)),
            "{seen:?}"
        );
        assert!(g.resolve(
            "c1",
            Human {
                allow: true,
                ..Human::default()
            }
        ));
        assert_eq!(
            waiting.await.unwrap(),
            GateDecision::Allow { updated_args: None }
        );
        assert!(!g.resolve("c1", Human::default()), "nichts mehr offen");

        // Runner endet (offene Freigaben verworfen): deny.
        let g2 = g.clone();
        let waiting = tokio::spawn(async move { g2.decide(ask("c2")).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        g.inner.pending.lock().unwrap().clear();
        assert!(matches!(waiting.await.unwrap(), GateDecision::Deny { .. }));
    }

    /// Security-Review #327: Ist die Sandbox angehalten (Proxy ausgefallen, SBX-006 AC3), gilt
    /// ein vorher gespeicherter Entscheid des Hooks nicht mehr: Auch die Rückfrage des Vendors
    /// und die automatische Freigabe zum selben Call werden abgelehnt (`sandbox-halted`).
    #[tokio::test]
    async fn sbx_006_ac3_halted_sandbox_denies_even_with_a_stored_hook_decision() {
        for origin in [GateOrigin::VendorAsk, GateOrigin::AutoApproval] {
            let (g, _rx) = gate();
            let pre = GateRequest {
                origin: GateOrigin::PreTool,
                ..ask("c1")
            };
            assert!(matches!(g.decide(pre).await, GateDecision::Allow { .. }));
            g.halt_tools("Egress-Proxy ausgefallen");
            let decision = tokio::time::timeout(
                Duration::from_secs(2),
                g.decide(GateRequest {
                    origin,
                    ..ask("c1")
                }),
            )
            .await
            .expect("keine Approval-Card bei angehaltener Sandbox");
            assert!(
                matches!(decision, GateDecision::Deny { .. }),
                "{origin:?}: {decision:?}"
            );
        }
    }

    /// #330 (Re-Review #327): Lehnt die angehaltene Sandbox einen Call mit gespeichertem
    /// Hook-Entscheid ab, entsteht kein zweites `policy.decision` (POL-025 AC1: genau eines je
    /// Call); die Ablehnung steht als `policy.notified` der Regel `sandbox-halted` im Log.
    #[tokio::test]
    async fn pol_025_ac1_halted_sandbox_adds_no_second_decision_for_a_decided_call() {
        for origin in [GateOrigin::VendorAsk, GateOrigin::AutoApproval] {
            let (g, mut rx) = gate();
            let pre = GateRequest {
                origin: GateOrigin::PreTool,
                ..ask("c1")
            };
            assert!(matches!(g.decide(pre).await, GateDecision::Allow { .. }));
            g.halt_tools("Egress-Proxy ausgefallen");
            let decision = g
                .decide(GateRequest {
                    origin,
                    ..ask("c1")
                })
                .await;
            assert!(matches!(decision, GateDecision::Deny { .. }), "{origin:?}");
            let mut seen = Vec::new();
            while let Ok(e) = rx.try_recv() {
                seen.push(e.payload);
            }
            assert_eq!(
                seen.iter()
                    .filter(|e| matches!(e, EventPayload::PolicyDecision(_)))
                    .count(),
                1,
                "{origin:?}: {seen:?}"
            );
            assert!(
                seen.iter().any(|e| matches!(e, EventPayload::PolicyNotified(n)
                    if n.rule_id == "sandbox-halted" && n.channels == ["ui"])),
                "{origin:?}: {seen:?}"
            );
            assert!(!g.inner.decided.lock().unwrap()["c1"].allowed, "{origin:?}");
        }
    }

    #[tokio::test]
    async fn pol_003_system_tools_pass_the_policy_without_a_human_card() {
        // #151: Fragt der Vendor (z. B. Claude im Plan-Modus) nach einem System-Tool von
        // beton, entscheidet allein die Policy; eine Approval-Card entsteht nicht.
        let (g, mut rx) = gate();
        g.set_trusted_servers(vec!["beton".into()]);
        let req = GateRequest {
            call_id: "c1".into(),
            tool: "mcp__beton__session_list".into(),
            kind: "system".into(),
            mcp_server: Some("beton".into()),
            args: json!({}),
            origin: GateOrigin::VendorAsk,
            ..GateRequest::default()
        };
        let decision = tokio::time::timeout(Duration::from_secs(2), g.decide(req))
            .await
            .expect("keine Wartezeit auf einen Menschen");
        assert_eq!(decision, GateDecision::Allow { updated_args: None });
        let mut seen = Vec::new();
        while let Ok(e) = rx.try_recv() {
            seen.push(e.payload);
        }
        assert_eq!(
            seen.iter()
                .filter(|e| matches!(e, EventPayload::PolicyDecision(_)))
                .count(),
            1,
            "{seen:?}"
        );
        assert!(
            !seen
                .iter()
                .any(|e| matches!(e, EventPayload::ApprovalRequested(_))),
            "{seen:?}"
        );
    }

    // ------------------------------------------------------------------ WP-37 (POL-009/010)

    struct Policy {
        gate: PolicyGate,
        rx: mpsc::UnboundedReceiver<GateEvent>,
        _dir: tempfile::TempDir,
    }

    fn with_policy(body: &str) -> Policy {
        with_policy_state(body, None)
    }

    fn with_policy_state(body: &str, state: Option<Arc<dyn StateSource>>) -> Policy {
        with_policy_at(Scope::Project, body, state)
    }

    /// Policy auf `scope`; Effects auf fremden Zustand (`user_day`, Keys ohne Präfix) darf nur
    /// die User-Ebene schreiben (POL-009, #268).
    fn with_policy_at(scope: Scope, body: &str, state: Option<Arc<dyn StateSource>>) -> Policy {
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("policies");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(
            pdir.join("p.yaml"),
            format!("spec_version: 1\nname: p\n{body}"),
        )
        .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let opts = PolicyOptions {
            dirs: vec![(scope, pdir)],
            workdir: dir.path().to_path_buf(),
            ..PolicyOptions::default()
        };
        let mut gate = PolicyGate::new(opts, tx);
        if let Some(source) = state {
            gate = gate.with_state(source);
        }
        Policy {
            gate,
            rx,
            _dir: dir,
        }
    }

    fn pre(call: &str, command: &str) -> GateRequest {
        GateRequest {
            call_id: call.into(),
            tool: "Bash".into(),
            kind: "shell".into(),
            args: json!({ "command": command }),
            origin: GateOrigin::PreTool,
            ..GateRequest::default()
        }
    }

    async fn next_request(rx: &mut mpsc::UnboundedReceiver<GateEvent>) -> ApprovalRequested {
        loop {
            let e = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("approval.requested kommt")
                .expect("Kanal offen");
            if let EventPayload::ApprovalRequested(r) = e.payload {
                return r;
            }
        }
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<GateEvent>) -> Vec<EventPayload> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e.payload);
        }
        out
    }

    fn allow() -> Human {
        Human {
            allow: true,
            ..Human::default()
        }
    }

    const TWO_ASKS: &str = "rules:\n  - id: confirm-destructive-shell\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: ask\n    severity: high\n    reason: \"Destruktives Kommando\"\n    approval: { timeout: 30m, on_timeout: deny }\n  - id: git-guard\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: ask\n    reason: \"Push verlässt den Rechner\"\n    approval: { timeout: 2h, on_timeout: abort }\n";

    #[tokio::test]
    async fn pol_010_ac1_one_card_with_all_reasons_and_the_combined_plan() {
        let mut p = with_policy(TWO_ASKS);
        let g = p.gate.clone();
        let call =
            tokio::spawn(async move { g.decide(pre("c1", "git push origin feature/x")).await });
        let req = next_request(&mut p.rx).await;
        assert_eq!(req.kind, ApprovalKind::Policy);
        let reasons = req.subject["reasons"].as_array().unwrap();
        assert_eq!(reasons.len(), 2, "{}", req.subject);
        assert_eq!(reasons[0]["rule_id"], "confirm-destructive-shell");
        assert_eq!(reasons[0]["scope"], "project");
        assert_eq!(reasons[0]["set"], "p");
        assert_eq!(reasons[0]["severity"], "high");
        assert_eq!(req.subject["tool"]["name"], "Bash");
        assert_eq!(req.subject["tool"]["kind"], "shell");
        assert_eq!(
            req.subject["tool"]["args"]["command"],
            "git push origin feature/x"
        );
        assert_eq!(req.subject["summary"], "git push origin feature/x");
        assert_eq!(req.subject["call_id"], "c1");
        assert_eq!(req.options, ["approve", "approve_session", "deny"]);
        // Kürzeste Frist (30 min), strengstes on_timeout (abort).
        assert_eq!(req.on_timeout, TimeoutAction::Abort);
        let left = req.expires_at.as_offset_date_time() - time::OffsetDateTime::now_utc();
        assert!(
            left > time::Duration::minutes(29) && left <= time::Duration::minutes(30),
            "{left}"
        );
        assert!(p.gate.resolve("c1", allow()));
        assert_eq!(
            call.await.unwrap(),
            GateDecision::Allow { updated_args: None }
        );
        let rest = drain(&mut p.rx);
        assert_eq!(
            rest.iter()
                .filter(|e| matches!(e, EventPayload::ApprovalRequested(_)))
                .count(),
            0,
            "genau eine Card: {rest:?}"
        );
    }

    const ASK_PUSH: &str = "rules:\n  - id: confirm\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: ask\n    reason: \"Push bestätigen\"\n    approval: { remember: session }\n";

    #[tokio::test]
    async fn pol_010_ac2_approve_session_creates_a_grant_for_the_same_command() {
        let mut p = with_policy(ASK_PUSH);
        let g = p.gate.clone();
        let call = tokio::spawn(async move { g.decide(pre("c1", "git push origin x")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.resolve(
            "c1",
            Human {
                allow: true,
                remember: true,
                ..Human::default()
            }
        ));
        assert!(matches!(call.await.unwrap(), GateDecision::Allow { .. }));
        let resolved = drain(&mut p.rx)
            .into_iter()
            .find_map(|e| match e {
                EventPayload::ApprovalResolved(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.remember, Some(RememberScope::Session));

        // Derselbe Befehl: ohne Card erlaubt, das policy.decision nennt den Grant.
        let d = tokio::time::timeout(
            Duration::from_secs(5),
            p.gate.decide(pre("c2", "git push origin x")),
        )
        .await
        .expect("ohne Rückfrage");
        assert!(matches!(d, GateDecision::Allow { .. }));
        let events = drain(&mut p.rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, EventPayload::ApprovalRequested(_)))
        );
        let decision = events
            .iter()
            .find_map(|e| match e {
                EventPayload::PolicyDecision(d) => Some(d),
                _ => None,
            })
            .unwrap();
        assert_eq!(decision.outcome, beton_core::event::PolicyOutcome::Allow);
        assert_eq!(decision.grants_used.len(), 1, "{decision:?}");

        // Ein anderer Befehl erzeugt wieder eine Card.
        let g = p.gate.clone();
        let other =
            tokio::spawn(async move { g.decide(pre("c3", "git push --tags origin")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.resolve("c3", Human::default()));
        assert!(matches!(other.await.unwrap(), GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn pol_010_remember_without_the_option_creates_no_grant() {
        let mut p = with_policy(
            "rules:\n  - id: confirm\n    on: tool_call\n    action: ask\n    approval: { remember: none }\n",
        );
        let g = p.gate.clone();
        let call = tokio::spawn(async move { g.decide(pre("c1", "ls")).await });
        let req = next_request(&mut p.rx).await;
        assert_eq!(req.options, ["approve", "deny"]);
        assert!(p.gate.resolve(
            "c1",
            Human {
                allow: true,
                remember: true,
                ..Human::default()
            }
        ));
        assert!(matches!(call.await.unwrap(), GateDecision::Allow { .. }));
        // Wieder eine Card für denselben Befehl.
        let g = p.gate.clone();
        let again = tokio::spawn(async move { g.decide(pre("c2", "ls")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.resolve("c2", allow()));
        assert!(matches!(again.await.unwrap(), GateDecision::Allow { .. }));
    }

    #[tokio::test]
    async fn pol_010_ac5_timeout_applies_the_strictest_action() {
        let mut p = with_policy(
            "rules:\n  - id: a\n    on: tool_call\n    action: ask\n    approval: { timeout: 1s, on_timeout: deny }\n  - id: b\n    on: tool_call\n    action: ask\n    approval: { timeout: 1h, on_timeout: abort }\n",
        );
        let started = std::time::Instant::now();
        let d = p.gate.decide(pre("c1", "rm -rf dist")).await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(900) && waited < Duration::from_secs(10),
            "{waited:?}"
        );
        let resolved = drain(&mut p.rx)
            .into_iter()
            .find_map(|e| match e {
                EventPayload::ApprovalResolved(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.via, ResolvedVia::Timeout);
        assert_eq!(resolved.decision, ApprovalDecision::Abort);
        assert_eq!(resolved.on_timeout_applied, Some(TimeoutAction::Abort));
    }

    #[tokio::test]
    async fn pol_010_on_timeout_allow_lets_the_call_through() {
        let mut p = with_policy(
            "rules:\n  - id: a\n    on: tool_call\n    action: ask\n    approval: { timeout: 1s, on_timeout: allow }\n",
        );
        let d = p.gate.decide(pre("c1", "npm test")).await;
        assert!(matches!(d, GateDecision::Allow { .. }), "{d:?}");
        let resolved = drain(&mut p.rx)
            .into_iter()
            .find_map(|e| match e {
                EventPayload::ApprovalResolved(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.decision, ApprovalDecision::Allow);
        assert_eq!(resolved.via, ResolvedVia::Timeout);
        assert_eq!(resolved.on_timeout_applied, Some(TimeoutAction::Allow));
    }

    const COUNT_ASKED: &str = "rules:\n  - id: count\n    on: tool_call\n    action: ask\n    approval: { timeout: 1s, on_timeout: deny }\n    effects:\n      - { op: increment, scope: session, key: asked }\n";

    fn state_changes(events: &[EventPayload]) -> Vec<(String, Value)> {
        events
            .iter()
            .filter_map(|e| match e {
                EventPayload::PolicyStateChanged(c) => Some((c.key.clone(), c.value.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn pol_009_ac2_effects_follow_the_resolution() {
        let mut p = with_policy_at(Scope::User, COUNT_ASKED, None);
        // Freigabe: angewendet.
        let g = p.gate.clone();
        let call = tokio::spawn(async move { g.decide(pre("c1", "ls")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.resolve("c1", allow()));
        call.await.unwrap();
        assert_eq!(
            state_changes(&drain(&mut p.rx)),
            [("asked".to_owned(), json!(1))]
        );
        // Ablehnung (Default effects_on): angewendet.
        let g = p.gate.clone();
        let call = tokio::spawn(async move { g.decide(pre("c2", "ls")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.resolve("c2", Human::default()));
        call.await.unwrap();
        assert_eq!(
            state_changes(&drain(&mut p.rx)),
            [("asked".to_owned(), json!(2))]
        );
        // Ablauf: nicht angewendet.
        p.gate.decide(pre("c3", "ls")).await;
        assert_eq!(state_changes(&drain(&mut p.rx)), []);
    }

    #[derive(Default)]
    struct FakeState {
        loaded: LoadedState,
        applied: Mutex<Vec<beton_policy::state::StateOp>>,
    }

    #[async_trait]
    impl StateSource for FakeState {
        async fn load(&self) -> Result<LoadedState, String> {
            Ok(self.loaded.clone())
        }
        async fn apply(&self, op: &beton_policy::state::StateOp) -> Result<(), String> {
            self.applied.lock().unwrap().push(op.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn pol_009_state_comes_from_the_server_and_user_scopes_go_back() {
        let source = Arc::new(FakeState {
            loaded: LoadedState {
                session: [("pushes".to_owned(), json!(2))].into(),
                user_day: [("pushes".to_owned(), json!(7))].into(),
                ..LoadedState::default()
            },
            ..FakeState::default()
        });
        let mut p = with_policy_at(
            Scope::User,
            "rules:\n  - id: count-pushes\n    on: tool_call\n    action: notify\n    notify: { channels: [inbox], message: 'Push Nr. {{ state.session.pushes + 1 }} heute {{ state.user_day.pushes }}' }\n    effects:\n      - { op: increment, scope: session, key: pushes }\n      - { op: increment, scope: user_day, key: pushes }\n",
            Some(source.clone()),
        );
        let d = p.gate.decide(pre("c1", "git push")).await;
        assert!(matches!(d, GateDecision::Allow { .. }));
        let events = drain(&mut p.rx);
        let notified = events
            .iter()
            .find_map(|e| match e {
                EventPayload::PolicyNotified(n) => Some(n),
                _ => None,
            })
            .expect("policy.notified");
        assert_eq!(notified.message.as_deref(), Some("Push Nr. 3 heute 7"));
        assert_eq!(notified.channels, ["inbox"]);
        assert_eq!(notified.rule_id, "count-pushes");
        assert_eq!(state_changes(&events), [("pushes".to_owned(), json!(3))]);
        let applied = source.applied.lock().unwrap().clone();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].scope, beton_policy::doc::StateScope::UserDay);
    }

    #[tokio::test]
    async fn pol_009_without_state_effects_are_not_applied() {
        struct Broken;
        #[async_trait]
        impl StateSource for Broken {
            async fn load(&self) -> Result<LoadedState, String> {
                Err("weg".into())
            }
            async fn apply(&self, _: &beton_policy::state::StateOp) -> Result<(), String> {
                Err("weg".into())
            }
        }
        let mut p = with_policy_at(
            Scope::User,
            "rules:\n  - id: count\n    on: tool_call\n    action: allow\n    effects:\n      - { op: increment, scope: session, key: n }\n",
            Some(Arc::new(Broken)),
        );
        assert!(matches!(
            p.gate.decide(pre("c1", "ls")).await,
            GateDecision::Allow { .. }
        ));
        let events = drain(&mut p.rx);
        assert_eq!(state_changes(&events), []);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EventPayload::Notice(n) if n.level == NoticeLevel::Warn)),
            "{events:?}"
        );
    }

    /// #242 N3, #248: Lint-Hinweise einer Set-Version erscheinen einmal als Warnung im Verlauf
    /// (`notice`), nicht bei jeder Auswertung.
    #[tokio::test]
    async fn pol_002_lints_of_a_set_version_become_one_warning() {
        let mut p = with_policy(
            "rules:\n  - id: raw\n    on: tool_call\n    when: 'tool.paths.exists(x, glob(x, \"/etc/**\"))'\n    action: deny\n",
        );
        let _ = p.gate.decide(pre("c1", "ls")).await;
        let _ = p.gate.decide(pre("c2", "ls")).await;
        let warnings: Vec<String> = drain(&mut p.rx)
            .into_iter()
            .filter_map(|e| match e {
                EventPayload::Notice(n) if n.level == NoticeLevel::Warn => Some(n.text),
                _ => None,
            })
            .collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("raw") && warnings[0].contains("resolve("),
            "{warnings:?}"
        );
    }

    #[tokio::test]
    async fn pol_006_notify_log_stays_in_the_stream() {
        let mut p = with_policy(
            "rules:\n  - id: note\n    on: tool_call\n    action: notify\n    notify: { channels: [log], message: 'nur Verlauf' }\n",
        );
        p.gate.decide(pre("c1", "ls")).await;
        let events = drain(&mut p.rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, EventPayload::PolicyNotified(_)))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EventPayload::Notice(n) if n.text.contains("nur Verlauf")))
        );
    }

    #[tokio::test]
    async fn agt_007_ac4_ask_user_waits_for_the_answer() {
        let (g, mut rx) = gate();
        let g2 = g.clone();
        let asked = tokio::spawn(async move {
            g2.ask_question(
                "q1",
                "Pro IP oder pro Account?".into(),
                vec!["Pro IP".into(), "Pro Account".into()],
                Duration::from_secs(30),
                None,
            )
            .await
        });
        let req = next_request(&mut rx).await;
        assert_eq!(req.kind, ApprovalKind::Question);
        assert_eq!(req.subject["question"], "Pro IP oder pro Account?");
        assert_eq!(req.subject["choices"], json!(["Pro IP", "Pro Account"]));
        assert_eq!(req.options, ["answer"]);
        assert!(g.resolve(
            "q1",
            Human {
                answer: Some("Pro IP".into()),
                ..Human::default()
            }
        ));
        assert_eq!(asked.await.unwrap(), Some("Pro IP".to_owned()));
        let resolved = drain(&mut rx)
            .into_iter()
            .find_map(|e| match e {
                EventPayload::ApprovalResolved(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.decision, ApprovalDecision::Answer);
        assert_eq!(resolved.answer.as_deref(), Some("Pro IP"));
        // Ohne Antwort binnen Frist: keine Antwort.
        let none = g
            .ask_question(
                "q2",
                "Noch da?".into(),
                Vec::new(),
                Duration::from_millis(200),
                None,
            )
            .await;
        assert_eq!(none, None);
    }

    #[tokio::test]
    async fn web_007_vendor_ask_card_names_tool_and_options() {
        let (g, mut rx) = gate();
        let g2 = g.clone();
        let call = tokio::spawn(async move { g2.decide(ask("c1")).await });
        let req = next_request(&mut rx).await;
        assert_eq!(req.kind, ApprovalKind::Tool);
        assert_eq!(req.subject["tool"]["name"], "Bash");
        assert_eq!(req.subject["tool"]["args"]["command"], "git push");
        assert_eq!(req.options, ["approve", "deny"]);
        assert!(g.resolve(
            "c1",
            Human {
                allow: true,
                actor: Some(Actor::User {
                    id: PrincipalId::User(UserId::LOCAL),
                    device_id: None,
                }),
                ..Human::default()
            }
        ));
        call.await.unwrap();
    }

    #[tokio::test]
    async fn pol_010_ac4_only_policy_approvals_count_as_pending_for_survival() {
        let mut p = with_policy("rules:\n  - id: c\n    on: tool_call\n    action: ask\n");
        let g = p.gate.clone();
        let q = tokio::spawn(async move {
            g.ask_question(
                "q",
                "Frage?".into(),
                Vec::new(),
                Duration::from_secs(30),
                None,
            )
            .await
        });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.has_pending());
        assert!(!p.gate.has_pending_policy(), "eine Frage zählt nicht");
        let g = p.gate.clone();
        let call = tokio::spawn(async move { g.decide(pre("c1", "ls")).await });
        let _ = next_request(&mut p.rx).await;
        assert!(p.gate.has_pending_policy());
        assert!(p.gate.resolve("c1", allow()));
        call.await.unwrap();
        assert!(p.gate.resolve("q", Human::default()));
        q.await.unwrap();
        assert!(!p.gate.has_pending());
    }

    #[tokio::test]
    async fn pol_009_rules_reading_state_deny_without_state() {
        struct Broken;
        #[async_trait]
        impl StateSource for Broken {
            async fn load(&self) -> Result<LoadedState, String> {
                Err("weg".into())
            }
            async fn apply(&self, _: &beton_policy::state::StateOp) -> Result<(), String> {
                Err("weg".into())
            }
        }
        for when in ["state.session.pushes > 3", "has(state.session)"] {
            let p = with_policy_state(
                &format!(
                    "rules:\n  - id: limit\n    on: tool_call\n    when: '{when}'\n    action: deny\n"
                ),
                Some(Arc::new(Broken)),
            );
            assert!(
                matches!(
                    p.gate.decide(pre("c1", "ls")).await,
                    GateDecision::Deny { .. }
                ),
                "{when}: ohne Zustand fail closed"
            );
        }
    }

    /// Security-Review #284 (Minor 2): Scheitert das Neuladen von `user_day`/`user`, nimmt das
    /// Gate nicht still den alten Stand, sondern behandelt ihn als nicht verfügbar (POL-009).
    #[tokio::test]
    async fn pol_009_a_failed_reload_of_user_state_fails_closed() {
        struct Once(std::sync::atomic::AtomicUsize);
        #[async_trait]
        impl StateSource for Once {
            async fn load(&self) -> Result<LoadedState, String> {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(LoadedState::default())
                } else {
                    Err("weg".into())
                }
            }
            async fn apply(&self, _: &beton_policy::state::StateOp) -> Result<(), String> {
                Ok(())
            }
        }
        let p = with_policy_state(
            "rules:\n  - id: daily\n    on: tool_call\n    when: 'has(state.user_day.pushes)'\n    action: deny\n",
            Some(Arc::new(Once(std::sync::atomic::AtomicUsize::new(0)))),
        );
        // Erster Load (Zustand der Session) gelingt, das Neuladen von `user_day` nicht.
        assert!(
            matches!(
                p.gate.decide(pre("c1", "ls")).await,
                GateDecision::Deny { .. }
            ),
            "ohne frischen user_day fail closed"
        );
    }

    #[test]
    fn pol_010_long_summaries_are_marked_as_shortened() {
        let long = "x".repeat(600);
        let card = tool_card(
            "tool_call",
            "Bash",
            "shell",
            Some(&long),
            &json!({}),
            None,
            &[],
        );
        let summary = card["summary"].as_str().unwrap();
        assert!(summary.ends_with('…'), "{summary}");
        assert!(summary.chars().count() <= 501);
        let short = tool_card(
            "tool_call",
            "Bash",
            "shell",
            Some("ls"),
            &json!({}),
            None,
            &[],
        );
        assert_eq!(short["summary"], "ls");
    }

    /// #248: Auf einem case-insensitiven Dateisystem (macOS) übernimmt die Pfadauflösung für
    /// existierende Teile die Schreibweise des Dateisystems; `resolve()`/`under()` vergleichen
    /// damit dieselbe Datei unabhängig von der Schreibweise der Anfrage. Auf case-sensitiven
    /// Dateisystemen (Linux) sind andere Schreibweisen andere Dateien.
    #[test]
    fn pol_002_fs_resolver_uses_the_case_of_the_file_system() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap().join("Repo");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join(".env"), "x").unwrap();
        let other = real.parent().unwrap().join("REPO/.ENV");
        if !other.exists() {
            return;
        }
        let resolved = FsResolver.resolve(&other.display().to_string()).unwrap();
        assert_eq!(resolved, real.join(".env").display().to_string());
    }

    #[test]
    fn pol_002_fs_resolver_follows_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let resolved = FsResolver
                .resolve(&link.join("datei.txt").display().to_string())
                .unwrap();
            let canonical = real.canonicalize().unwrap().join("datei.txt");
            assert_eq!(resolved, canonical.display().to_string());
        }
        assert!(FsResolver.resolve("relativ").is_err());
    }
}
