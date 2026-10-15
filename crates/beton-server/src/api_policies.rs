//! Policies: Explain, Dry-Run und Shadow-Ansicht (POL-027, API-002).
//!
//! - `GET /v1/policies` – das effektive Set (`beton policy show --effective`), mit Herkunft je
//!   Regel; mit `session_id` inklusive der Projekt-Policies der Session.
//! - `POST /v1/policies/evaluate` – wertet eine hypothetische Anfrage aus (`beton policy eval`):
//!   im Kontext einer Session oder mit Fixtures. Ohne Events, ohne Approvals, ohne
//!   Zustandsänderungen (AC3).
//! - `GET /v1/sessions/{id}/policy-decisions/{seq}` – Explain einer Entscheidung aus dem
//!   `policy.decision`-Event und dem Set-Snapshot ihrer Version (AC1, AC2).
//! - `GET /v1/policies/shadow` – Treffer der Regeln im Shadow-Betrieb je Regel und Tag (AC4).
//!
//! Den Snapshot je Set-Version legt der Runner über den System-Call `policy.snapshot` ab.

use std::path::Path;

use axum::Extension;
use axum::extract::{Path as UrlPath, State};
use beton_core::event::{EventPayload, PolicyDecision};
use beton_core::id::SessionId;
use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event};
use beton_policy::enforcement::Matrix;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::explain::{context_values, explain, render};
use beton_policy::grants::SessionGrants;
use beton_policy::load::{RuleRef, Scope};
use beton_policy::snapshot::{self, Manifest};
use beton_policy::store::{PolicyStore, policies_dir};
use beton_store::SessionRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::{ApiJson, ApiQuery};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Höchstens so viele Tage zeigt die Shadow-Ansicht.
pub const MAX_SHADOW_DAYS: u32 = 31;
/// Events, die nach einem `tool.call.requested` nach seinem `policy.decision` durchsucht werden.
const DECISION_SEARCH: u32 = 1000;

fn invalid(detail: impl Into<String>) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail)
}

fn not_found(detail: impl Into<String>) -> Problem {
    Problem::new(ProblemCode::NotFound).detail(detail)
}

/// Das effektive Set (POL-027 `show --effective`).
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct EffectivePolicies {
    pub policy_set_hash: String,
    /// Sets mit Ebene, Herkunft, Version, Modus und Regeln (`id`, `phases`, `action`, `when`).
    #[schema(value_type = Vec<Object>)]
    #[ts(type = "unknown[]")]
    pub sets: Vec<Value>,
    /// Dateien, die gerade nicht laden (Pfad).
    pub invalid: Vec<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PoliciesParams {
    /// Session, deren Projekt-Policies mitzählen; ohne Angabe nur die User-Ebene.
    pub session_id: Option<String>,
}

/// Anfrage für `POST /v1/policies/evaluate`.
#[derive(Debug, Deserialize, ToSchema, TS)]
pub struct EvaluateRequest {
    /// Phase: `session_start`, `model_request`, `tool_call`, `tool_result`, …
    pub phase: String,
    /// Phasenspezifische Eingabe wie in Policy-Tests, z. B.
    /// `{ "tool": { "name": "Bash", "args": { "command": "git push" } } }`.
    #[schema(value_type = Object)]
    #[ts(type = "Record<string, unknown>")]
    pub input: Value,
    /// Kontext dieser Session (Policies, Kosten, Zustand, Grants).
    #[serde(default)]
    #[ts(optional)]
    pub session_id: Option<String>,
    /// Harness für Normalisierung und Durchsetzung; Default: der der Session bzw. `claude`.
    #[serde(default)]
    #[ts(optional)]
    pub harness: Option<String>,
    /// Weitere Kontextwerte (wie `fixtures` in Policy-Tests); gelten vor denen der Session.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub fixtures: Option<Value>,
}

/// Ergebnis einer seiteneffektfreien Auswertung.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct PolicyEvaluation {
    /// `allow`, `deny` oder `ask`.
    pub outcome: String,
    /// Durchsetzung, die der Harness in dieser Phase erreicht.
    pub enforcement: String,
    /// Die Entscheidung in der Form von `policy.decision` (ohne `decision_id`).
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub decision: Value,
    /// Strukturierte Erklärung samt benutzter Kontextwerte.
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub explanation: Value,
    /// Erklärung als Text.
    pub explain: String,
}

/// Explain einer gespeicherten Entscheidung.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct DecisionExplain {
    pub session_id: String,
    /// Seq des `policy.decision`.
    pub seq: u64,
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub decision: Value,
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub explanation: Value,
    pub explain: String,
    /// Warum der Snapshot nicht nutzbar war (fehlt oder beschädigt).
    #[ts(optional)]
    pub snapshot_error: Option<String>,
}

/// Letzter Treffer einer Shadow-Regel.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ShadowLastHit {
    pub session_id: String,
    pub session_title: String,
    pub seq: u64,
    /// RFC 3339.
    pub at: String,
    /// Kurzform der Aktion (z. B. Tool und Kommando), falls bekannt.
    #[ts(optional)]
    pub summary: Option<String>,
}

/// Eine Regel im Shadow-Betrieb mit Treffern je Tag.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ShadowRule {
    pub rule_id: String,
    pub scope: String,
    pub set: String,
    /// Verdikt ohne Shadow-Betrieb: `deny` (würde ablehnen), `ask`, `modify`, …
    pub verdict: String,
    /// Treffer je Tag in der Reihenfolge von `days`.
    pub counts: Vec<u64>,
    pub total: u64,
    #[ts(optional)]
    pub last: Option<ShadowLastHit>,
}

/// Shadow-Treffer der letzten Tage (POL-027 AC4).
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ShadowReport {
    /// Tage (UTC, `YYYY-MM-DD`), ältester zuerst, heute zuletzt.
    pub days: Vec<String>,
    pub rules: Vec<ShadowRule>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ShadowParams {
    /// Anzahl Tage bis heute (1–31, Default 7).
    pub days: Option<u32>,
}

/// Session laden und Leserecht prüfen.
async fn readable(
    state: &AppState,
    auth: Authenticated,
    id: &str,
) -> Result<SessionRecord, Problem> {
    let session: SessionId = id.parse().map_err(|_| not_found(format!("Session {id}")))?;
    let record = state.store.session(state.local.org, session).await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    Ok(record)
}

/// Effektives Set für User-Ebene und (optional) das Projekt in `workdir`.
fn effective(beton_home: &Path, workdir: Option<&Path>) -> std::sync::Arc<EffectiveSet> {
    let mut store = PolicyStore::new();
    store.add_dir(Scope::User, policies_dir(beton_home));
    if let Some(w) = workdir {
        store.add_dir(Scope::Project, policies_dir(&w.join(".beton")));
    }
    let _ = store.refresh();
    store.effective()
}

fn sets_json(set: &EffectiveSet) -> Vec<Value> {
    let event = PolicyDecision {
        policy_set_hash: set.hash().to_owned(),
        ..PolicyDecision::default()
    };
    explain(&event, Some(set))
        .sets
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
        .collect()
}

/// Das effektive Set: User-Ebene und mit `session_id` die Projekt-Ebene der Session.
#[utoipa::path(get, path = "/v1/policies", tag = "policies",
    params(PoliciesParams),
    responses((status = 200, description = "Geltende Regeln mit Ebene und Herkunft", body = EffectivePolicies)))]
pub async fn list_policies(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    ApiQuery(p): ApiQuery<PoliciesParams>,
) -> ApiResult<axum::Json<EffectivePolicies>> {
    let workdir = match &p.session_id {
        Some(id) => {
            let record = readable(&state, auth, id).await?;
            Some(state.sessions().workspace_root(&record).await?)
        }
        None => None,
    };
    let home = state.runtime.sessions.beton_home.clone();
    let set = tokio::task::spawn_blocking(move || effective(&home, workdir.as_deref()))
        .await
        .map_err(|e| Problem::internal(&e))?;
    Ok(axum::Json(EffectivePolicies {
        policy_set_hash: set.hash().to_owned(),
        sets: sets_json(&set),
        invalid: set.invalid().to_vec(),
    }))
}

/// Kontext einer Session für eine hypothetische Auswertung: Kosten, Zustand und Grants wie
/// beim Runner, aber nur lesend.
async fn session_fixtures(
    state: &AppState,
    record: &SessionRecord,
) -> Result<(Value, SessionGrants), Problem> {
    let created = state.sessions().created(record.id).await?;
    let usage = crate::usage_policy::policy_usage(state, record.id).await?;
    let policy = crate::policy_state::policy_state(state, record.id).await?;
    let worktree = state.sessions().workspace_root(record).await?;
    let mut grants = SessionGrants::default();
    for g in policy["grants"].as_array().into_iter().flatten() {
        let Some(scope) = g["scope"].as_str().and_then(Scope::parse) else {
            continue;
        };
        let rule = RuleRef {
            scope,
            set: g["set"].as_str().unwrap_or_default().to_owned(),
            set_version: String::new(),
            rule_id: g["rule_id"].as_str().unwrap_or_default().to_owned(),
        };
        grants.add_with_id(
            &rule,
            g["fingerprint"].as_str().unwrap_or_default(),
            g["id"].as_str().unwrap_or_default(),
        );
    }
    let fixtures = json!({
        "session": {
            "id": record.id.to_string(),
            "harness": record.harness,
            "model": created.model,
            "permission_mode": created.permission_mode,
            "cost_usd": usage["session_cost_usd"],
            "tokens_in": usage["session_tokens_in"],
            "tokens_out": usage["session_tokens_out"],
            "worktree": worktree.display().to_string(),
            "started_at": record.created_at.to_string(),
        },
        "user": {
            "id": record.owner.to_string(),
            "timezone": usage["timezone"],
            "daily_cost_usd": usage["user_daily_cost_usd"],
            "monthly_cost_usd": usage["user_monthly_cost_usd"],
        },
        "state": {
            "session": policy["session"],
            "run": policy["run"],
            "user_day": policy["user_day"],
            "user": policy["user"],
        },
    });
    Ok((fixtures, grants))
}

fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Wertet eine hypothetische Anfrage aus – ohne Events, Approvals oder Zustandsänderungen.
#[utoipa::path(post, path = "/v1/policies/evaluate", tag = "policies",
    request_body = EvaluateRequest,
    responses((status = 200, description = "Entscheidung mit Erklärung", body = PolicyEvaluation)))]
pub async fn evaluate(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    ApiJson(req): ApiJson<EvaluateRequest>,
) -> ApiResult<axum::Json<PolicyEvaluation>> {
    let phase: Phase = serde_json::from_value(Value::String(req.phase.clone()))
        .map_err(|_| invalid(format!("Phase `{}` ist unbekannt", req.phase)))?;
    let Value::Object(input) = &req.input else {
        return Err(invalid("`input` muss ein Objekt sein"));
    };
    let mut fixtures = Value::Object(Map::new());
    let mut grants = SessionGrants::default();
    let mut harness = req.harness.clone();
    let mut workdir = None;
    if let Some(id) = &req.session_id {
        let record = readable(&state, auth, id).await?;
        let (f, g) = session_fixtures(&state, &record).await?;
        fixtures = f;
        grants = g;
        harness.get_or_insert_with(|| record.harness.clone());
        workdir = Some(state.sessions().workspace_root(&record).await?);
    }
    if let Some(extra) = &req.fixtures {
        if !extra.is_object() {
            return Err(invalid("`fixtures` muss ein Objekt sein"));
        }
        merge(&mut fixtures, extra);
    }
    let harness = harness.unwrap_or_else(|| beton_policy::policytest::DEFAULT_HARNESS.to_owned());
    let matrix = match harness.parse::<beton_harness::HarnessId>() {
        Ok(id) => state
            .sessions()
            .harness_capabilities(&id, workdir.as_deref().unwrap_or(Path::new("/")))
            .map(|c| c.enforcement())
            .unwrap_or_else(Matrix::full),
        Err(_) => return Err(invalid(format!("Harness `{harness}` ist ungültig"))),
    };
    let home = state.runtime.sessions.beton_home.clone();
    let input = input.clone();
    let result = tokio::task::spawn_blocking(move || {
        let set = effective(&home, workdir.as_deref());
        let act =
            beton_policy::policytest::activation(phase, &fixtures, &input, Some(&harness), None)?;
        let decision = set.evaluate(
            &act,
            &EvalOptions {
                capabilities: matrix.capabilities(phase),
                grants: Some(&grants),
                ..EvalOptions::default()
            },
        );
        let kind = match act.field("tool", "kind") {
            Some(beton_policy::value::Value::String(k)) => Some(k.to_string()),
            _ => None,
        };
        let level = matrix.level(phase, kind.as_deref());
        let mut event = decision_event(&decision, DecisionMeta::new(level));
        event.decision_id.clear();
        let mut ex = explain(&event, Some(&set));
        ex.context = context_values(&set, phase, &act);
        Ok::<_, String>((event, ex, level))
    })
    .await
    .map_err(|e| Problem::internal(&e))?
    .map_err(invalid)?;
    let (event, ex, level) = result;
    Ok(axum::Json(PolicyEvaluation {
        outcome: ex.outcome.clone(),
        enforcement: level.as_str().to_owned(),
        decision: serde_json::to_value(&event).unwrap_or(Value::Null),
        explain: render(&ex),
        explanation: serde_json::to_value(&ex).unwrap_or(Value::Null),
    }))
}

/// Die Entscheidung zu `seq`: das `policy.decision` selbst oder das zum Tool-Call an `seq`.
async fn find_decision(
    state: &AppState,
    session: SessionId,
    seq: u64,
) -> Result<(u64, PolicyDecision), Problem> {
    let org = state.local.org;
    let events = state
        .store
        .events(org, session, seq.saturating_sub(1), 1)
        .await?;
    let event = events
        .into_iter()
        .find(|e| e.seq == seq)
        .ok_or_else(|| not_found(format!("Event {seq}")))?;
    let call_id = match state.store.resolve_payload(org, &event).await? {
        EventPayload::PolicyDecision(d) => return Ok((seq, d)),
        EventPayload::ToolCallRequested(t) => t.call_id,
        _ => {
            return Err(not_found(format!(
                "Event {seq} ist weder `policy.decision` noch ein Tool-Call"
            )));
        }
    };
    for e in state
        .store
        .events(org, session, seq, DECISION_SEARCH)
        .await?
    {
        if e.type_name() != "policy.decision" {
            continue;
        }
        if let EventPayload::PolicyDecision(d) = state.store.resolve_payload(org, &e).await?
            && d.subject_ref["call_id"].as_str() == Some(call_id.as_str())
        {
            return Ok((e.seq, d));
        }
    }
    Err(not_found(format!(
        "Kein `policy.decision` zum Tool-Call an {seq}"
    )))
}

/// Explain einer Entscheidung der Session (POL-027 AC1/AC2): `seq` eines `policy.decision`
/// oder eines `tool.call.requested`.
#[utoipa::path(get, path = "/v1/sessions/{id}/policy-decisions/{seq}", tag = "policies",
    params(("id" = String, Path), ("seq" = u64, Path)),
    responses((status = 200, description = "Entscheidung mit Erklärung", body = DecisionExplain)))]
pub async fn explain_decision(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    UrlPath((id, seq)): UrlPath<(String, u64)>,
) -> ApiResult<axum::Json<DecisionExplain>> {
    let record = readable(&state, auth, &id).await?;
    let (seq, decision) = find_decision(&state, record.id, seq).await?;
    let org = state.local.org;
    let (set, snapshot_error) = match state
        .store
        .policy_snapshot(org, record.id, &decision.policy_set_hash)
        .await
    {
        Ok(Some((manifest, sources))) => {
            match serde_json::from_value::<Manifest>(manifest)
                .map_err(|e| e.to_string())
                .and_then(|m| {
                    snapshot::rebuild(&m, |v| {
                        sources
                            .iter()
                            .find(|(version, _)| version == v)
                            .map(|(_, b)| b.clone())
                    })
                    .map_err(|e| e.to_string())
                }) {
                Ok(set) => (Some(set), None),
                Err(e) => (None, Some(e)),
            }
        }
        Ok(None) => (
            None,
            Some("Für diese Set-Version ist kein Snapshot gespeichert".to_owned()),
        ),
        Err(e) => (None, Some(Problem::from(e).detail.unwrap_or_default())),
    };
    let ex = explain(&decision, set.as_ref());
    Ok(axum::Json(DecisionExplain {
        session_id: record.id.to_string(),
        seq,
        decision: serde_json::to_value(&decision).unwrap_or(Value::Null),
        explain: render(&ex),
        explanation: serde_json::to_value(&ex).unwrap_or(Value::Null),
        snapshot_error,
    }))
}

fn day(t: OffsetDateTime) -> String {
    let d = t.date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Kurzform der Aktion hinter einem `policy.decision` (Tool und Kommando bzw. Pfad).
async fn summary(state: &AppState, session: SessionId, seq: u64) -> Option<String> {
    let org = state.local.org;
    let (_, decision) = find_decision(state, session, seq).await.ok()?;
    let call_id = decision.subject_ref["call_id"].as_str()?.to_owned();
    let from = seq.saturating_sub(u64::from(DECISION_SEARCH));
    let events = state
        .store
        .events(org, session, from, DECISION_SEARCH)
        .await
        .ok()?;
    for e in events.iter().rev().filter(|e| e.seq <= seq) {
        if e.type_name() != "tool.call.requested" {
            continue;
        }
        if let Ok(EventPayload::ToolCallRequested(t)) = state.store.resolve_payload(org, e).await
            && t.call_id == call_id
        {
            let detail = ["command", "file_path", "path", "url"]
                .iter()
                .find_map(|k| t.args[*k].as_str().map(str::to_owned));
            let mut text = match detail {
                Some(d) => format!("{}: {d}", t.tool),
                None => t.tool.clone(),
            };
            if text.chars().count() > 120 {
                text = text.chars().take(119).collect::<String>() + "…";
            }
            return Some(text);
        }
    }
    None
}

/// Treffer der Regeln im Shadow-Betrieb je Regel und Tag (POL-027 AC4).
#[utoipa::path(get, path = "/v1/policies/shadow", tag = "policies",
    params(ShadowParams),
    responses((status = 200, description = "Treffer je Regel und Tag", body = ShadowReport)))]
pub async fn shadow(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    ApiQuery(p): ApiQuery<ShadowParams>,
) -> ApiResult<axum::Json<ShadowReport>> {
    let n = p.days.unwrap_or(7);
    if n == 0 || n > MAX_SHADOW_DAYS {
        return Err(invalid(format!(
            "days muss zwischen 1 und {MAX_SHADOW_DAYS} liegen"
        )));
    }
    // Tage in UTC, bis die Zeitzone des Users einstellbar ist (#195).
    let today = OffsetDateTime::now_utc();
    let days: Vec<String> = (0..n)
        .rev()
        .map(|i| day(today - time::Duration::days(i64::from(i))))
        .collect();
    let org = state.local.org;
    let rows = state
        .store
        .policy_shadow(org, &days[0], &days[days.len() - 1])
        .await?;
    let mut rules: Vec<ShadowRule> = Vec::new();
    let mut last: Vec<Option<(String, SessionId, u64)>> = Vec::new();
    for r in rows {
        let idx = match rules.iter().position(|x| {
            x.scope == r.scope
                && x.set == r.set_name
                && x.rule_id == r.rule_id
                && x.verdict == r.verdict
        }) {
            Some(i) => i,
            None => {
                rules.push(ShadowRule {
                    rule_id: r.rule_id.clone(),
                    scope: r.scope.clone(),
                    set: r.set_name.clone(),
                    verdict: r.verdict.clone(),
                    counts: vec![0; days.len()],
                    total: 0,
                    last: None,
                });
                last.push(None);
                rules.len() - 1
            }
        };
        if let Some(d) = days.iter().position(|d| *d == r.day) {
            rules[idx].counts[d] += r.hits;
            rules[idx].total += r.hits;
        }
        let at = r.last_at.to_string();
        if last[idx].as_ref().is_none_or(|(a, _, _)| at > *a) {
            last[idx] = Some((at, r.last_session_id, r.last_seq));
        }
    }
    for (rule, last) in rules.iter_mut().zip(last) {
        let Some((at, session, seq)) = last else {
            continue;
        };
        let Ok(record) = state.store.session(org, session).await else {
            continue;
        };
        if !state.runtime.authorizer.can_read(auth, &record) {
            continue;
        }
        rule.last = Some(ShadowLastHit {
            session_id: session.to_string(),
            session_title: record.title.clone(),
            seq,
            at,
            summary: summary(&state, session, seq).await,
        });
    }
    rules.sort_by(|a, b| b.total.cmp(&a.total).then(a.rule_id.cmp(&b.rule_id)));
    Ok(axum::Json(ShadowReport { days, rules }))
}

/// `policy.snapshot` vom Runner: Manifest und Inhalte (`{version, text}`) eines Sets.
pub async fn policy_snapshot(
    state: &AppState,
    session: SessionId,
    args: &Value,
) -> Result<Value, Problem> {
    let manifest: Manifest = serde_json::from_value(args["manifest"].clone())
        .map_err(|e| invalid(format!("Manifest: {e}")))?;
    let mut sources = Vec::new();
    for s in args["sources"].as_array().into_iter().flatten() {
        let (Some(version), Some(text)) = (s["version"].as_str(), s["text"].as_str()) else {
            return Err(invalid("Inhalt ohne `version` oder `text`"));
        };
        sources.push((version.to_owned(), text.as_bytes().to_vec()));
    }
    let stored = state
        .store
        .put_policy_snapshot(
            state.local.org,
            session,
            &manifest.policy_set_hash,
            &serde_json::to_value(&manifest).unwrap_or(Value::Null),
            &sources,
        )
        .await?;
    Ok(json!({ "stored": stored }))
}
