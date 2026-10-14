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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, EventPayload,
    Notice, NoticeLevel, PolicyNotified, PolicyRuleIssue, RawJson, RememberScope, ResolvedVia,
    SystemComponent, TimeoutAction,
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
    SessionVars, StartVars, StateVars, ToolVars, UserVars, Vars,
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

/// Verbrauchswerte aus der Usage-Projektion des Servers (POL-004 AC3/AC4).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub session_cost_usd: Option<f64>,
    #[serde(default)]
    pub session_tokens_in: Option<i64>,
    #[serde(default)]
    pub session_tokens_out: Option<i64>,
    #[serde(default)]
    pub user_daily_cost_usd: Option<f64>,
    #[serde(default)]
    pub user_monthly_cost_usd: Option<f64>,
    #[serde(default)]
    pub timezone: Option<String>,
}

/// Quelle der Verbrauchswerte (im Runner: System-Call `policy.usage` über den Tunnel).
#[async_trait]
pub trait UsageSource: Send + Sync {
    async fn usage(&self) -> Result<Usage, String>;
}

/// Felder, für die das Gate Verbrauchswerte vom Server holt.
const USAGE_FIELDS: [&str; 5] = [
    "session.cost_usd",
    "session.tokens_in",
    "session.tokens_out",
    "user.daily_cost_usd",
    "user.monthly_cost_usd",
];

/// Startparameter des Gates.
#[derive(Debug, Clone)]
pub struct PolicyOptions {
    pub session_id: Option<String>,
    pub harness: HarnessId,
    pub workdir: PathBuf,
    pub home: Option<PathBuf>,
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
    pending: Mutex<HashMap<String, oneshot::Sender<Human>>>,
    decided: Mutex<HashMap<String, Verdict>>,
    /// Gemeldete Tool-Calls (Call-ID, MCP-Server, Tool) für die Zuordnung im Relay-Hub.
    requested: Mutex<Vec<(String, Option<String>, String)>>,
    /// Reihenfolge im Log: `policy.decision` erst nach dem `tool.call.requested` (POL-025).
    ordered: std::sync::atomic::AtomicBool,
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
    granted: AtomicI64,
    denied: AtomicI64,
    started_at: CelTimestamp,
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

/// Wie streng ein Permission-Mode ist (für `modify` in `session_start`: nur strenger).
fn strictness(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::AcceptEdits => 2,
        PermissionMode::Yolo => 3,
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
    let summary: String = summary.unwrap_or(tool).chars().take(500).collect();
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
    pub fn new(opts: PolicyOptions, events: mpsc::UnboundedSender<GateEvent>) -> Self {
        let mut store = PolicyStore::new();
        for (scope, dir) in &opts.dirs {
            store.add_dir(*scope, dir);
        }
        Self {
            inner: Arc::new(Inner {
                opts,
                store: Mutex::new(store),
                matrix: Mutex::new(Matrix::full()),
                settings: Mutex::new(Settings::default()),
                events,
                pending: Mutex::new(HashMap::new()),
                decided: Mutex::new(HashMap::new()),
                requested: Mutex::new(Vec::new()),
                ordered: std::sync::atomic::AtomicBool::new(false),
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
                granted: AtomicI64::new(0),
                denied: AtomicI64::new(0),
                started_at: CelTimestamp::now(),
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

    /// `state.*` für den Kontext; `user_day`/`user` frisch, wenn eine Regel sie liest.
    async fn with_state_values(&self, set: &EffectiveSet, phase: Phase, vars: &mut Vars) {
        if !self.ensure_state().await {
            return;
        }
        let used: BTreeSet<String> = set.programs(phase).flat_map(|p| p.fields()).collect();
        let user_scopes = used.iter().any(|f| f.starts_with("state.user"));
        if user_scopes
            && let Some(source) = &self.inner.state_source
            && let Ok(Ok(fresh)) = tokio::time::timeout(USAGE_TIMEOUT, source.load()).await
            && let Ok(mut u) = self.inner.user_state.lock()
        {
            *u = (fresh.user_day, fresh.user);
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
            user_day: Some(user_day),
            user: Some(user),
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

    /// Enforcement-Matrix des Harness der Session (POL-024).
    pub fn set_matrix(&self, matrix: Matrix) {
        if let Ok(mut m) = self.inner.matrix.lock() {
            *m = matrix;
        }
    }

    fn matrix(&self) -> Matrix {
        self.inner
            .matrix
            .lock()
            .map(|m| m.clone())
            .unwrap_or_else(|_| Matrix::full())
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
        let _ = self.inner.events.send(GateEvent { payload, turn, raw });
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
        for e in store.refresh() {
            let payload = match e {
                StoreEvent::Changed(c) => EventPayload::PolicySetChanged(c),
                StoreEvent::Invalid(i) => EventPayload::PolicySetInvalid(i),
            };
            self.emit(payload, turn, None);
        }
        store.effective()
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
                ..SessionVars::default()
            }),
            user: Some(UserVars {
                id: Some(UserId::LOCAL.to_string()),
                timezone: Some("UTC".into()),
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
            s.tokens_in = u.session_tokens_in;
            s.tokens_out = u.session_tokens_out;
        }
        if let Some(user) = vars.user.as_mut() {
            user.daily_cost_usd = u.user_daily_cost_usd;
            user.monthly_cost_usd = u.user_monthly_cost_usd;
            if let Some(tz) = u.timezone {
                user.timezone = Some(tz);
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
        self.with_usage_values(&set, phase, &mut vars).await;
        self.with_state_values(&set, phase, &mut vars).await;
        let capabilities = self.matrix().capabilities(phase);
        let grants = self
            .inner
            .grants
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let decision = match Activation::new(phase, vars.to_json()) {
            Err(e) => engine_deny(phase, set.hash(), format!("Kontext ungültig: {e}")),
            Ok(act) => {
                let mut act = act.with_resolver(Arc::new(FsResolver));
                if let Some(s) = source {
                    act = act.with_tool_source(s);
                }
                let evaluator = self.inner.evaluator.clone();
                let set2 = set.clone();
                let job = tokio::task::spawn_blocking(move || {
                    let opts = EvalOptions {
                        capabilities,
                        grants: Some(&grants),
                        ..EvalOptions::default()
                    };
                    evaluator.evaluate(&set2, &act, &opts)
                });
                match tokio::time::timeout(EVAL_TIMEOUT, job).await {
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
        };
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
        self.emit(EventPayload::PolicyDecision(event), turn, raw);
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
            p.insert(key.clone(), tx);
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

    /// Entscheidung aus `approval.resolve` (Schlüssel: `subject.call_id`). `false`, wenn
    /// nichts offen ist.
    pub fn resolve(&self, key: &str, human: Human) -> bool {
        self.inner
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(key))
            .is_some_and(|tx| tx.send(human).is_ok())
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
        let tool = normalize_source(&source, &req.args);
        let verdict = match tool {
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
                self.emit(
                    EventPayload::PolicyDecision(event),
                    req.turn_id,
                    req.raw.clone(),
                );
                Verdict {
                    allowed: false,
                    human: false,
                    reason: Some(format!("Tool-Call nicht prüfbar: {e}")),
                    updated_args: None,
                }
            }
            Ok(mut n) => {
                // Unbekannt in der Tabelle, aber vom Adapter klassifiziert (z. B. Fake-Szenario).
                if n.kind == "other"
                    && beton_policy::enforcement::KINDS.contains(&req.kind.as_str())
                {
                    n.kind.clone_from(&req.kind);
                }
                let args = match &n.args {
                    Value::Object(m) => Some(m.clone().into_iter().collect()),
                    _ => None,
                };
                vars.tool = Some(ToolVars {
                    name: Some(n.name.clone()),
                    native_name: Some(n.native_name.clone()),
                    server: Some(n.server.clone()),
                    kind: Some(n.kind.clone()),
                    args,
                    command: n.command.clone(),
                    argv: Some(n.argv.clone()),
                    cwd: Some(n.cwd.clone()),
                    paths: Some(n.paths.clone()),
                    urls: Some(n.urls.clone()),
                    call_index: Some(call_index),
                    is_subagent_spawn: Some(n.is_subagent_spawn),
                    parse_uncertain: Some(n.parse_uncertain),
                    ..ToolVars::default()
                });
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
                            Outcome::Deny { .. } => FinalOutcome::Deny,
                            Outcome::Ask { .. } => FinalOutcome::Expired,
                        };
                        self.apply_effects(&ev.decision, fin, req.turn_id).await;
                        Verdict {
                            allowed: true,
                            human: false,
                            reason: None,
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
                        Verdict {
                            allowed: human.allow,
                            human: true,
                            reason: human.reason.or_else(|| {
                                Some(if answered.aborted {
                                    "Freigabe abgelaufen; der Turn wird abgebrochen (beton-Policy)"
                                        .to_owned()
                                } else {
                                    reason_text(reasons)
                                })
                            }),
                            updated_args: human.updated_args.or(updated),
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

    /// Tool-Call ohne Gate-Anfrage des Harness (nur beobachtbar, POL-025 AC1): genau ein
    /// `policy.decision` mit `enforcement: observe_only`.
    pub async fn observe(&self, req: GateRequest) {
        if self.decided(&req.call_id) {
            return;
        }
        let v = self.tool_verdict(&req, Level::ObserveOnly).await;
        let _ = v;
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
        vars.start = Some(start);
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
        let mut modified = ev.decision.modified.clone();
        if let (Some(new), Some(old)) = (
            modified
                .permission_mode
                .as_deref()
                .and_then(|m| m.parse::<PermissionMode>().ok()),
            requested_mode,
        ) && strictness(new) > strictness(old)
        {
            // Lockern ist nicht erlaubt: Änderung verwerfen.
            modified.permission_mode = None;
        }
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
        let kind = if req.kind.is_empty() {
            beton_harness::tool_kind(
                self.inner.opts.harness.as_str(),
                &req.tool,
                req.acp_kind.as_deref(),
                req.mcp_server.as_deref(),
            )
        } else {
            req.kind.clone()
        };
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
                let v = match prior {
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
                if req.origin == GateOrigin::AutoApproval || v.human || kind == "system" || yolo {
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
                if let Ok(mut d) = self.inner.decided.lock()
                    && let Some(entry) = d.get_mut(&req.call_id)
                {
                    entry.human = true;
                    entry.allowed = human.allow;
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
            Ok(n) => {
                vars.tool = Some(ToolVars {
                    name: Some(n.name.clone()),
                    native_name: Some(n.native_name.clone()),
                    server: Some(n.server.clone()),
                    kind: Some(n.kind.clone()),
                    command: n.command.clone(),
                    argv: Some(n.argv.clone()),
                    paths: Some(n.paths.clone()),
                    urls: Some(n.urls.clone()),
                    ..ToolVars::default()
                });
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

    #[tokio::test]
    async fn pol_003_system_tools_pass_the_policy_without_a_human_card() {
        // #151: Fragt der Vendor (z. B. Claude im Plan-Modus) nach einem System-Tool von
        // beton, entscheidet allein die Policy; eine Approval-Card entsteht nicht.
        let (g, mut rx) = gate();
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
            dirs: vec![(Scope::Project, pdir)],
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
        let mut p = with_policy(COUNT_ASKED);
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
        let mut p = with_policy_state(
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
        let mut p = with_policy_state(
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
