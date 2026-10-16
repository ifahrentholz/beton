//! Budgets und Modell-Routing (POL-011 `spend_cap`, POL-012 `daily_budget`, POL-013
//! `model_route`): Entzuckerung zu CEL-Regeln beim Laden und Auskünfte für den Runner.
//!
//! Ein Regeltyp wird zu mehreren CEL-Regeln („Teilen“) derselben Regel-ID in den Phasen
//! `session_start` und `model_request`:
//!
//! - **Budget** (`spend_cap`, `daily_budget`): je Schwelle aus `ask_at_usd` ein `ask`, das nur
//!   im Bereich bis zur nächsten Schwelle bzw. zum Limit greift und sich die bestätigte
//!   Schwelle im Zustand merkt (`effects` nur bei `allow`); ab `limit_usd` je nach `on_exceed`
//!   ein `deny`, ein `ask` (jedes Mal) oder ein `modify` auf `fallback_model`.
//! - **Kosten** sind Geld plus Listenpreis-Äquivalent der Subscription-Nutzung (ADR-0034):
//!   `session.cost_usd + session.equivalent_usd` usw. Fehlen die Werte (Server nicht
//!   erreichbar), scheitert die Bedingung und die Regel lehnt ab (fail closed).
//! - **Routing** (`model_route`): je Route ein `modify`, das nur greift, wenn keine frühere
//!   Route der Regel passt (erste passende gewinnt), und für `allowed_models` ein `deny` auf
//!   das (umgeroutete) Modell. Ein unbekanntes Modell ist nicht nachweisbar erlaubt: `deny`.
//!
//! Zustands-Keys der Regeltypen tragen Typ, Ebene, Set und Regel-ID
//! (`spend_cap/<ebene>/<set>/<id>/asked`); handgeschriebene `effects` dürfen Keys mit dem
//! Präfix eines Regeltyps nicht schreiben (#268).

use std::collections::BTreeMap;

use beton_core::event::BudgetAction;
use serde_json::Value;
use sha2::Digest as _;

use crate::Phase;
use crate::cel::{CompileError, compile_bool};
use crate::doc::{
    Action, DefaultAction, EffectDoc, EffectOp, ModifyDoc, OnUnsupported, RuleDoc, RuleType,
    StateScope,
};
use crate::engine::{Decision, EffectiveSet, Verdict};
use crate::load::{RuleKind, RuleRef, Scope};

/// Effort-Stufen (HAR-017).
const EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// Kostenbereich einer Budget-Regel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetScope {
    /// Die Session (`spend_cap`, `scope: session`).
    Session,
    /// Der Run (`spend_cap`, `scope: run`).
    Run,
    /// Die Session mit allen Child-Sessions (`spend_cap`, `scope: subtree`).
    Subtree,
    /// Der User am heutigen Tag (`daily_budget`, `scope: user`).
    User,
}

impl BudgetScope {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetScope::Session => "session",
            BudgetScope::Run => "run",
            BudgetScope::Subtree => "subtree",
            BudgetScope::User => "user",
        }
    }

    /// Kosten in USD als CEL: Geld plus Listenpreis-Äquivalent (ADR-0034).
    pub fn cost_expr(self) -> &'static str {
        match self {
            BudgetScope::Session => "session.cost_usd + session.equivalent_usd",
            BudgetScope::Run => "run.cost_usd + run.equivalent_usd",
            BudgetScope::Subtree => "run.subtree_cost_usd + run.subtree_equivalent_usd",
            BudgetScope::User => "user.daily_cost_usd + user.daily_equivalent_usd",
        }
    }

    /// Modelle ohne Preis in diesem Bereich (CEL-Liste aus `policy.usage`).
    pub fn unpriced_expr(self) -> &'static str {
        match self {
            BudgetScope::Session => "session.unpriced_models",
            BudgetScope::Run => "run.unpriced_models",
            BudgetScope::Subtree => "run.subtree_unpriced_models",
            BudgetScope::User => "user.daily_unpriced_models",
        }
    }

    /// Zustand, in dem die bestätigten Schwellen stehen.
    fn state(self) -> StateScope {
        match self {
            BudgetScope::Session | BudgetScope::Subtree => StateScope::Session,
            BudgetScope::Run => StateScope::Run,
            BudgetScope::User => StateScope::UserDay,
        }
    }

    fn label(self) -> &'static str {
        match self {
            BudgetScope::Session => "der Session",
            BudgetScope::Run => "des Runs",
            BudgetScope::Subtree => "der Session samt Child-Sessions",
            BudgetScope::User => "heute",
        }
    }
}

fn state_name(s: StateScope) -> &'static str {
    match s {
        StateScope::Session => "session",
        StateScope::Run => "run",
        StateScope::UserDay => "user_day",
        StateScope::User => "user",
    }
}

/// Fehler in den Parametern eines Regeltyps; `path` ist relativ zu `params`.
#[derive(Debug)]
pub(crate) enum ParamError {
    At { path: String, message: String },
    Cel { path: String, error: CompileError },
}

fn at(path: impl Into<String>, message: impl Into<String>) -> ParamError {
    ParamError::At {
        path: path.into(),
        message: message.into(),
    }
}

/// Präfix der Zustands-Keys eines Regeltyps (`spend_cap/`); handgeschriebene `effects` dürfen
/// sie nicht schreiben.
pub fn reserved_prefix(key: &str) -> Option<RuleType> {
    [RuleType::SpendCap, RuleType::DailyBudget]
        .into_iter()
        .find(|t| key.starts_with(&format!("{}/", t.as_str())))
}

/// Präfix der Zustands-Keys einer Budget-Regel: `<typ>/<ebene>/<set>/<id>`. Der Tageszustand
/// (`user_day`) gilt über alle Sessions und Repos; unterhalb der User-Ebene trägt der Key von
/// `daily_budget` deshalb zusätzlich die Herkunft des Sets (Hash der Datei bzw. des Agents),
/// damit ein Repo keine Schwellen eines anderen bestätigt (Security-Review #284).
pub fn key_base(t: RuleType, scope: Scope, set: &str, origin: &str, id: &str) -> String {
    if t == RuleType::DailyBudget && scope > Scope::User {
        let h = hex::encode(sha2::Sha256::digest(origin.as_bytes()));
        format!("{}/{}/{}/{set}/{id}", t.as_str(), scope.as_str(), &h[..12])
    } else {
        format!("{}/{}/{set}/{id}", t.as_str(), scope.as_str())
    }
}

/// Zustands-Key der bestätigten Schwelle einer Budget-Regel.
pub fn asked_key(t: RuleType, scope: Scope, set: &str, origin: &str, id: &str) -> String {
    format!("{}/asked", key_base(t, scope, set, origin, id))
}

/// Zustands-Key der freigegebenen Modelle ohne Preis (`on_unpriced: ask`).
pub fn unpriced_key(t: RuleType, scope: Scope, set: &str, origin: &str, id: &str) -> String {
    format!("{}/unpriced", key_base(t, scope, set, origin, id))
}

/// Gehört `key` zur Regel `id` des Sets `set` (Ebene `scope`, Herkunft `origin`)?
pub fn own_key(t: RuleType, scope: Scope, set: &str, origin: &str, id: &str, key: &str) -> bool {
    key.starts_with(&format!("{}/", key_base(t, scope, set, origin, id)))
}

/// CEL-Literal für eine Zahl (`5.0`).
fn lit(x: f64) -> String {
    format!("{x:?}")
}

/// CEL-Zeichenkettenliteral.
fn cel_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

struct Params<'a> {
    map: &'a BTreeMap<String, Value>,
}

impl<'a> Params<'a> {
    fn new(
        rule: &'a RuleDoc,
        allowed: &[&str],
        empty: &'a BTreeMap<String, Value>,
    ) -> Result<Self, ParamError> {
        let map = rule.params.as_ref().unwrap_or(empty);
        if let Some(k) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(at(
                k.as_str(),
                format!(
                    "unbekannter Parameter `{k}` (erlaubt: {})",
                    allowed.join(", ")
                ),
            ));
        }
        Ok(Self { map })
    }

    fn text(&self, name: &str) -> Result<Option<String>, ParamError> {
        match self.map.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(at(name, "Text erwartet")),
        }
    }

    fn amount(v: &Value, path: &str) -> Result<f64, ParamError> {
        match v.as_f64() {
            Some(x) if x.is_finite() && x > 0.0 => Ok(x),
            Some(_) => Err(at(path, "Betrag in USD muss positiv sein")),
            None => Err(at(path, "Betrag in USD (Zahl) erwartet")),
        }
    }

    fn limit(&self) -> Result<f64, ParamError> {
        match self.map.get("limit_usd") {
            None => Err(at("limit_usd", "`limit_usd` fehlt")),
            Some(v) => Self::amount(v, "limit_usd"),
        }
    }

    /// Schwellen aufsteigend, jede kleiner als das Limit, keine doppelt.
    fn thresholds(&self, limit: f64) -> Result<Vec<f64>, ParamError> {
        let items = match self.map.get("ask_at_usd") {
            None => return Ok(Vec::new()),
            Some(Value::Array(items)) => items,
            Some(_) => return Err(at("ask_at_usd", "Liste von Beträgen erwartet")),
        };
        let mut out = Vec::with_capacity(items.len());
        for (i, v) in items.iter().enumerate() {
            let path = format!("ask_at_usd[{i}]");
            let x = Self::amount(v, &path)?;
            if x >= limit {
                return Err(at(path, "Schwelle muss kleiner als `limit_usd` sein"));
            }
            if out.contains(&x) {
                return Err(at(path, "Schwelle doppelt"));
            }
            out.push(x);
        }
        out.sort_by(f64::total_cmp);
        Ok(out)
    }
}

/// Was beim Erreichen des Limits passiert.
enum OnExceed {
    Deny,
    Ask,
    Route {
        model: String,
        effort: Option<String>,
    },
}

fn effort(v: Option<String>, path: &str) -> Result<Option<String>, ParamError> {
    match v {
        Some(e) if !EFFORTS.contains(&e.as_str()) => Err(at(
            path,
            format!("`{e}` ist keine Effort-Stufe ({})", EFFORTS.join(", ")),
        )),
        other => Ok(other),
    }
}

fn on_exceed(p: &Params, with_effort: bool) -> Result<OnExceed, ParamError> {
    let fallback = p.text("fallback_model")?;
    let fallback_effort = if with_effort {
        effort(p.text("fallback_effort")?, "fallback_effort")?
    } else {
        None
    };
    let mode = p.text("on_exceed")?;
    let route = mode.as_deref() == Some("route");
    if !route && fallback.is_some() {
        return Err(at(
            "fallback_model",
            "`fallback_model` nur mit `on_exceed: route`",
        ));
    }
    if !route && fallback_effort.is_some() {
        return Err(at(
            "fallback_effort",
            "`fallback_effort` nur mit `on_exceed: route`",
        ));
    }
    Ok(match mode.as_deref() {
        None | Some("deny") => OnExceed::Deny,
        Some("ask") => OnExceed::Ask,
        Some("route") => OnExceed::Route {
            model: fallback.filter(|m| !m.is_empty()).ok_or_else(|| {
                at(
                    "fallback_model",
                    "`on_exceed: route` braucht `fallback_model`",
                )
            })?,
            effort: fallback_effort,
        },
        Some(other) => {
            return Err(at(
                "on_exceed",
                format!("`on_exceed: {other}` ist ungültig (deny, ask, route)"),
            ));
        }
    })
}

/// Grundgerüst eines Teils: Regel-ID, Darstellung und Betriebsart der Regel.
fn part(rule: &RuleDoc, phase: Phase, when: Option<String>, action: Action) -> RuleDoc {
    RuleDoc {
        id: rule.id.clone(),
        on: Some(phase),
        when,
        action: Some(action),
        rule_type: None,
        params: None,
        severity: rule.severity,
        reason: None,
        approval: None,
        modify: None,
        on_unsupported: None,
        notify: None,
        effects: Vec::new(),
        effects_on: None,
        dry_run: rule.dry_run,
        enforcement: rule.enforcement,
        kinds: None,
        remember_key: None,
    }
}

fn usd(x: f64) -> String {
    format!("{x} USD")
}

/// `spend_cap` (POL-011) und `daily_budget` (POL-012).
fn budget(
    t: RuleType,
    rule: &RuleDoc,
    scope: Scope,
    set: &str,
    origin: &str,
) -> Result<Vec<RuleDoc>, ParamError> {
    let empty = BTreeMap::new();
    let allowed: &[&str] = match t {
        RuleType::SpendCap => &[
            "scope",
            "limit_usd",
            "ask_at_usd",
            "on_exceed",
            "on_unpriced",
            "fallback_model",
            "fallback_effort",
        ],
        _ => &[
            "scope",
            "limit_usd",
            "ask_at_usd",
            "on_exceed",
            "on_unpriced",
            "fallback_model",
        ],
    };
    let p = Params::new(rule, allowed, &empty)?;
    let budget_scope = match (t, p.text("scope")?.as_deref()) {
        (RuleType::SpendCap, None | Some("session")) => BudgetScope::Session,
        (RuleType::SpendCap, Some("run")) => BudgetScope::Run,
        (RuleType::SpendCap, Some("subtree")) => BudgetScope::Subtree,
        (RuleType::SpendCap, Some(other)) => {
            return Err(at(
                "scope",
                format!("`scope: {other}` ist ungültig (session, run, subtree)"),
            ));
        }
        (_, None | Some("user")) => BudgetScope::User,
        (_, Some("team")) => {
            return Err(at(
                "scope",
                "`scope: team` folgt mit dem Team-Betrieb (M4, POL-008)",
            ));
        }
        (_, Some(other)) => {
            return Err(at(
                "scope",
                format!("`scope: {other}` ist ungültig (user; team ab M4)"),
            ));
        }
    };
    let limit = p.limit()?;
    let thresholds = p.thresholds(limit)?;
    let exceed = on_exceed(&p, t == RuleType::SpendCap)?;
    let unpriced = match p.text("on_unpriced")?.as_deref() {
        None | Some("ask") => Some(Action::Ask),
        Some("deny") => Some(Action::Deny),
        Some("ignore") => None,
        Some(other) => {
            return Err(at(
                "on_unpriced",
                format!("`on_unpriced: {other}` ist ungültig (ask, deny, ignore)"),
            ));
        }
    };
    let cost = budget_scope.cost_expr();
    let state = budget_scope.state();
    let key = asked_key(t, scope, set, origin, &rule.id);
    let state_ref = format!("state.{}", state_name(state));
    let key_lit = cel_string(&key);
    let unpriced_lit = cel_string(&unpriced_key(t, scope, set, origin, &rule.id));
    let mut parts = Vec::new();
    for phase in [Phase::SessionStart, Phase::ModelRequest] {
        let model = match phase {
            Phase::SessionStart => "start.model",
            _ => "request.model",
        };
        // Startmodell eines Repo-Agents bzw. eines Spawns ohne Katalogpreis: Das Budget
        // könnte seine Kosten nicht zählen, also lehnt es ab (Security-Review #284, M1).
        if phase == Phase::SessionStart {
            let when = "has(start.model) && !priced(start.model) && \
                 ((has(start.trigger) && start.trigger == \"spawn\") || \
                 (has(agent.source) && !(agent.source in [\"user\", \"builtin\"])))";
            let mut d = part(rule, phase, Some(when.to_owned()), Action::Deny);
            d.reason = Some(format!(
                "Startmodell ohne Katalogpreis von einem Agent bzw. Spawn unterhalb der \
                 User-Ebene: {} `{}` kann seine Kosten nicht zählen",
                t.as_str(),
                rule.id
            ));
            parts.push(d);
        }
        // Nutzung ohne Preis (`on_unpriced`): das nächste Modell bzw. schon genutzte Modelle,
        // die noch nicht freigegeben sind (je Modell einmal im Bereich des Budgets).
        if let Some(action) = unpriced {
            let acked = |x: &str| {
                format!("({unpriced_lit} in {state_ref} && {x} in {state_ref}[{unpriced_lit}])")
            };
            let list = budget_scope.unpriced_expr();
            let when = format!(
                "(has({model}) && !priced({model}) && !{}) || {list}.exists(m, !{})",
                acked(model),
                acked("m")
            );
            let mut u = part(rule, phase, Some(when), action);
            u.reason = Some(format!(
                "Modell ohne Katalogpreis: {} `{}` kann die Kosten {} nicht zählen (on_unpriced)",
                t.as_str(),
                rule.id,
                budget_scope.label()
            ));
            if action == Action::Ask {
                u.effects = vec![EffectDoc {
                    op: EffectOp::Set,
                    scope: state,
                    key: unpriced_key(t, scope, set, origin, &rule.id),
                    value: Some(format!(
                        "({unpriced_lit} in {state_ref} ? {state_ref}[{unpriced_lit}] : []) + {list}.filter(m, !{}) + (has({model}) && !priced({model}) && !{} ? [{model}] : [])",
                        acked("m"),
                        acked(model)
                    )),
                }];
                u.effects_on = Some(vec![DefaultAction::Allow]);
            }
            parts.push(u);
        }
        for (i, t_i) in thresholds.iter().enumerate() {
            let next = thresholds.get(i + 1).copied().unwrap_or(limit);
            let when = format!(
                "{cost} >= {} && {cost} < {} && !({key_lit} in {state_ref} && double({state_ref}[{key_lit}]) >= {})",
                lit(*t_i),
                lit(next),
                lit(*t_i)
            );
            let mut ask = part(rule, phase, Some(when), Action::Ask);
            ask.reason = Some(format!(
                "Kosten {} erreichen {} ({} `{}`, Limit {})",
                budget_scope.label(),
                usd(*t_i),
                t.as_str(),
                rule.id,
                usd(limit)
            ));
            ask.effects = vec![EffectDoc {
                op: EffectOp::Set,
                scope: state,
                key: key.clone(),
                value: Some(lit(*t_i)),
            }];
            // Gemerkt wird nur eine bestätigte Schwelle.
            ask.effects_on = Some(vec![DefaultAction::Allow]);
            parts.push(ask);
        }
        let when = Some(format!("{cost} >= {}", lit(limit)));
        let exceeded = format!(
            "Kostenlimit {} von {} erreicht ({} `{}`)",
            budget_scope.label(),
            usd(limit),
            t.as_str(),
            rule.id
        );
        let limit_part = match &exceed {
            OnExceed::Deny => {
                let mut d = part(rule, phase, when, Action::Deny);
                d.reason = Some(rule.reason.clone().unwrap_or(exceeded));
                d
            }
            OnExceed::Ask => {
                let mut a = part(rule, phase, when, Action::Ask);
                a.reason = Some(rule.reason.clone().unwrap_or(exceeded));
                a
            }
            OnExceed::Route { model, effort } => {
                // Ohne eigene Begründung erklärt die Engine `on_unsupported` selbst.
                let mut m = part(rule, phase, when, Action::Modify);
                m.reason = rule.reason.clone();
                m.modify = Some(ModifyDoc {
                    model: Some(model.clone()),
                    reasoning_effort: effort.clone(),
                    permission_mode: None,
                    args: None,
                    redact: None,
                });
                m
            }
        };
        parts.push(limit_part);
    }
    Ok(parts)
}

/// Eine Route von `model_route`.
struct Route {
    when: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    /// Phasen, in denen `when` gültig ist.
    phases: Vec<Phase>,
}

const ROUTE_KEYS: [&str; 3] = ["when", "model", "reasoning_effort"];

fn routes(p: &Params) -> Result<Vec<Route>, ParamError> {
    let items = match p.map.get("routes") {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(at("routes", "Liste von Routen erwartet")),
    };
    let mut out: Vec<Route> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let path = format!("routes[{i}]");
        let Value::Object(map) = item else {
            return Err(at(path, "Route (Objekt) erwartet"));
        };
        if let Some(k) = map.keys().find(|k| !ROUTE_KEYS.contains(&k.as_str())) {
            return Err(at(
                format!("{path}.{k}"),
                format!(
                    "unbekanntes Feld `{k}` (erlaubt: {})",
                    ROUTE_KEYS.join(", ")
                ),
            ));
        }
        let text = |k: &str| -> Result<Option<String>, ParamError> {
            match map.get(k) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(at(format!("{path}.{k}"), "Text erwartet")),
            }
        };
        let model = text("model")?.filter(|m| !m.is_empty());
        let effort = effort(
            text("reasoning_effort")?,
            &format!("{path}.reasoning_effort"),
        )?;
        if model.is_none() && effort.is_none() {
            return Err(at(
                path,
                "Route setzt weder `model` noch `reasoning_effort`",
            ));
        }
        if out.iter().any(|r| r.when.is_none()) {
            return Err(at(
                path,
                "Route wird nie erreicht: eine frühere Route ohne `when` passt immer",
            ));
        }
        let when = text("when")?;
        let mut phases = vec![Phase::SessionStart, Phase::ModelRequest];
        if let Some(w) = &when {
            // `request.*` gibt es nur in `model_request`: die Route gilt dann nur dort.
            let checked: Vec<(Phase, Result<_, CompileError>)> = phases
                .iter()
                .map(|ph| (*ph, compile_bool(w, *ph)))
                .collect();
            phases = checked
                .iter()
                .filter(|(_, r)| r.is_ok())
                .map(|(ph, _)| *ph)
                .collect();
            if phases.is_empty() {
                let error = checked
                    .into_iter()
                    .find_map(|(ph, r)| (ph == Phase::ModelRequest).then(|| r.err()).flatten());
                return Err(match error {
                    Some(error) => ParamError::Cel {
                        path: format!("{path}.when"),
                        error,
                    },
                    None => at(format!("{path}.when"), "Bedingung ungültig"),
                });
            }
        }
        out.push(Route {
            when,
            model,
            effort,
            phases,
        });
    }
    Ok(out)
}

/// `model_route` (POL-013).
fn model_route(rule: &RuleDoc) -> Result<Vec<RuleDoc>, ParamError> {
    let empty = BTreeMap::new();
    let p = Params::new(
        rule,
        &["allowed_models", "routes", "on_unsupported"],
        &empty,
    )?;
    let allowed = match p.map.get("allowed_models") {
        None => None,
        Some(Value::Array(items)) if items.is_empty() => {
            return Err(at(
                "allowed_models",
                "mindestens ein Modell-Muster erwartet",
            ));
        }
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| at(format!("allowed_models[{i}]"), "Muster (Text) erwartet"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(at("allowed_models", "Liste von Modell-Mustern erwartet")),
    };
    let on_unsupported = match p.text("on_unsupported")?.as_deref() {
        None => None,
        Some("deny") => Some(OnUnsupported::Deny),
        Some("ask") => Some(OnUnsupported::Ask),
        Some("skip") => Some(OnUnsupported::Skip),
        Some(other) => {
            return Err(at(
                "on_unsupported",
                format!("`on_unsupported: {other}` ist ungültig (deny, ask, skip)"),
            ));
        }
    };
    let routes = routes(&p)?;
    if allowed.is_none() && routes.is_empty() {
        return Err(at("", "model_route braucht `allowed_models` oder `routes`"));
    }
    let mut parts = Vec::new();
    for phase in [Phase::SessionStart, Phase::ModelRequest] {
        // Erste passende Route gewinnt: jede Route schließt die früheren (dieser Phase) aus.
        let mut earlier: Vec<String> = Vec::new();
        for route in routes.iter().filter(|r| r.phases.contains(&phase)) {
            let own = route.when.as_ref().map(|w| format!("({w})"));
            let not_earlier = (!earlier.is_empty()).then(|| format!("!({})", earlier.join(" || ")));
            let when = match (own, not_earlier) {
                (Some(o), Some(n)) => Some(format!("{o} && {n}")),
                (Some(o), None) => Some(o),
                (None, Some(n)) => Some(n),
                (None, None) => None,
            };
            if let Some(w) = &route.when {
                earlier.push(format!("({w})"));
            }
            let mut m = part(rule, phase, when, Action::Modify);
            m.reason = rule.reason.clone();
            m.on_unsupported = on_unsupported;
            m.modify = Some(ModifyDoc {
                model: route.model.clone(),
                reasoning_effort: route.effort.clone(),
                permission_mode: None,
                args: None,
                redact: None,
            });
            parts.push(m);
        }
        if let Some(patterns) = &allowed {
            let field = match phase {
                Phase::SessionStart => "start.model",
                _ => "request.model",
            };
            let globs: Vec<String> = patterns
                .iter()
                .map(|g| format!("glob({field}, {})", cel_string(g)))
                .collect();
            // Ein unbekanntes Modell (Default des Harness) ist nicht nachweisbar erlaubt.
            let when = format!("!has({field}) || !({})", globs.join(" || "));
            let mut d = part(rule, phase, Some(when), Action::Deny);
            d.reason = Some(rule.reason.clone().unwrap_or_else(|| {
                format!(
                    "Modell nicht erlaubt (model_route `{}`); erlaubt: {}",
                    rule.id,
                    patterns.join(", ")
                )
            }));
            parts.push(d);
        }
    }
    Ok(parts)
}

/// Entzuckert einen Regeltyp dieses Moduls zu CEL-Regeln; `None` für andere Typen.
pub(crate) fn desugar(
    t: RuleType,
    rule: &RuleDoc,
    scope: Scope,
    set: &str,
    origin: &str,
) -> Option<Result<Vec<RuleDoc>, ParamError>> {
    Some(match t {
        RuleType::SpendCap | RuleType::DailyBudget => budget(t, rule, scope, set, origin),
        RuleType::ModelRoute => model_route(rule),
        _ => return None,
    })
}

/// Name des Sets mit dem Session-Budget (`beton run --max-cost`, API-006).
pub const SESSION_BUDGET_SET: &str = "session-budget";
/// Regel-ID des Session-Budgets.
pub const SESSION_BUDGET_RULE: &str = "max-cost";

/// Das Session-Budget aus `session.created.max_cost_usd` (API-006 AC4) als Inline-Set der
/// User-Ebene: eine `spend_cap`-Regel `max-cost` mit `scope: session` und `on_exceed: deny`.
/// Ein ungültiger Betrag ist ein Ladefehler (der Runner startet dann nicht, fail closed).
pub fn session_budget(max_cost_usd: f64) -> Result<crate::load::LoadedSet, crate::load::LoadError> {
    // Der Lauf samt Sub-Agents (Security-Review #284, M2): Teilbaum der Session.
    let rule = serde_json::json!({
        "id": SESSION_BUDGET_RULE,
        "type": "spend_cap",
        "params": { "scope": "subtree", "limit_usd": max_cost_usd, "on_exceed": "deny" },
        "reason": format!("Session-Budget von {} erreicht (--max-cost)", usd(max_cost_usd)),
    });
    crate::load::inline_set(
        Scope::User,
        SESSION_BUDGET_SET,
        "session.created (max_cost_usd)",
        &[rule],
    )
}

/// Eine Budget-Regel des effektiven Sets.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetRule {
    pub rule: RuleRef,
    pub rule_type: RuleType,
    pub scope: BudgetScope,
}

/// Kostenbereich einer geladenen Budget-Regel aus ihren Parametern.
fn scope_of(t: RuleType, params: Option<&BTreeMap<String, Value>>) -> Option<BudgetScope> {
    let scope = params.and_then(|p| p.get("scope")).and_then(Value::as_str);
    match (t, scope) {
        (RuleType::SpendCap, None | Some("session")) => Some(BudgetScope::Session),
        (RuleType::SpendCap, Some("run")) => Some(BudgetScope::Run),
        (RuleType::SpendCap, Some("subtree")) => Some(BudgetScope::Subtree),
        (RuleType::DailyBudget, None | Some("user")) => Some(BudgetScope::User),
        _ => None,
    }
}

/// Budget-Regeln (`spend_cap`, `daily_budget`) des Sets in kanonischer Reihenfolge, je Regel
/// einmal; Dry-Run-Regeln zählen nicht.
pub fn budget_rules(set: &EffectiveSet) -> Vec<BudgetRule> {
    let mut out: Vec<BudgetRule> = Vec::new();
    for s in set.sets() {
        if s.mode == crate::doc::Mode::DryRun {
            continue;
        }
        for r in &s.rules {
            if r.doc.dry_run {
                continue;
            }
            let Some(t) = r.doc.rule_type else { continue };
            let Some(scope) = scope_of(t, r.doc.params.as_ref()) else {
                continue;
            };
            let rule = s.rule_ref(&r.id);
            if !out.iter().any(|b| b.rule == rule) {
                out.push(BudgetRule {
                    rule,
                    rule_type: t,
                    scope,
                });
            }
        }
    }
    out
}

/// Ein erreichtes Limit in einer Entscheidung.
#[derive(Debug, Clone, PartialEq)]
pub struct Exhausted {
    pub rule: RuleRef,
    pub scope: BudgetScope,
    pub action: BudgetAction,
}

/// Limits, die eine Entscheidung erreicht hat (`budget.exhausted`): gegriffene `deny`- bzw.
/// `ask`-Teile einer Budget-Regel ab `limit_usd` (nicht die Schwellen aus `ask_at_usd`).
pub fn exhausted(decision: &Decision, set: &EffectiveSet) -> Vec<Exhausted> {
    let mut out: Vec<Exhausted> = Vec::new();
    for m in &decision.matched {
        if m.dry_run {
            continue;
        }
        let (action, wanted) = match m.verdict {
            Verdict::Deny => (BudgetAction::Deny, Action::Deny),
            Verdict::Ask => (BudgetAction::Ask, Action::Ask),
            _ => continue,
        };
        let Some(loaded) = set.sets().iter().find(|s| {
            s.scope == m.rule.scope && s.name == m.rule.set && s.version == m.rule.set_version
        }) else {
            continue;
        };
        // Die Schwellen merken sich ihre Bestätigung (`effects`); der Teil ab dem Limit nicht.
        let limit_part = loaded.rules.iter().find(|r| {
            r.id == m.rule.rule_id
                && r.doc.effects.is_empty()
                && r.doc.reason == m.reason
                && matches!(&r.kind, RuleKind::Cel { phase, action, .. }
                    if *phase == decision.phase && *action == wanted)
        });
        let Some(r) = limit_part else { continue };
        let Some(t) = r.doc.rule_type else { continue };
        let Some(scope) = scope_of(t, r.doc.params.as_ref()) else {
            continue;
        };
        if !out.iter().any(|e| e.rule == m.rule) {
            out.push(Exhausted {
                rule: m.rule.clone(),
                scope,
                action,
            });
        }
    }
    out
}
