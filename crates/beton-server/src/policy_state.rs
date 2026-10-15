//! Zustand der Policies für den Runner (POL-009) und gemerkte Freigaben (POL-010).
//!
//! - `policy.state`: `session`/`run` aus den `policy.state_changed` des Session-Logs (ein Fork
//!   übernimmt sie bis zum Fork-Punkt), `user_day`/`user` aus dem Store (Kalendertag in
//!   `user.timezone`, Default UTC, POL-004 AC4) und die Grants der Session aus
//!   `approval.requested` (Gründe mit Fingerprint) + `approval.resolved { remember: session }`.
//! - `policy.state.apply`: eine Änderung in `user_day`/`user`, atomar im Store.
//! - `inbox.read`: Agent-Inbox der Session (AGT-007). Erzeuger kommen mit ASY-002 (M5); bis
//!   dahin ist sie leer.

use std::collections::HashMap;

use beton_core::event::{ApprovalDecision, EventPayload, RememberScope};
use beton_core::id::SessionId;
use beton_policy::state::{PolicyState, StateChange, StateOp, next_value};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::app::AppState;
use crate::problem::{Problem, ProblemCode};

/// Heutiger Tag von `user_day`: 00:00 bis 24:00 in `user.timezone` (POL-009 AC3).
fn user_day(state: &AppState) -> String {
    state.runtime.timezone.day_key(OffsetDateTime::now_utc())
}

/// `policy.state` für die gebundene Session.
pub async fn policy_state(state: &AppState, session: SessionId) -> Result<Value, Problem> {
    let org = state.local.org;
    let record = state.store.session(org, session).await?;
    let events = state.sessions().all_events(session).await?;
    let mut changes = Vec::new();
    let mut fingerprints: HashMap<String, Vec<Value>> = HashMap::new();
    let mut grants = Vec::new();
    for e in &events {
        match e.payload() {
            Some(EventPayload::PolicyStateChanged(c)) => {
                if let Some(change) = StateChange::from_event(c) {
                    changes.push(change);
                }
            }
            Some(EventPayload::ApprovalRequested(r)) => {
                let reasons: Vec<Value> = r.subject["reasons"]
                    .as_array()
                    .map(|rs| {
                        rs.iter()
                            .filter(|x| x["fingerprint"].is_string())
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                fingerprints.insert(r.approval_id.to_string(), reasons);
            }
            Some(EventPayload::ApprovalResolved(r))
                if r.decision == ApprovalDecision::Allow
                    && r.remember == Some(RememberScope::Session) =>
            {
                for (i, reason) in fingerprints
                    .get(&r.approval_id.to_string())
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    grants.push(json!({
                        "id": format!("grt_{}_{i}", r.approval_id),
                        "scope": reason["scope"],
                        "set": reason["set"],
                        "rule_id": reason["rule_id"],
                        "fingerprint": reason["fingerprint"],
                    }));
                }
            }
            _ => {}
        }
    }
    let replayed = PolicyState::replay(changes.iter());
    let day = user_day(state);
    let (user_day, user) = state
        .store
        .policy_user_state(org, record.owner, &day)
        .await?;
    Ok(json!({
        "session": replayed.snapshot(beton_policy::doc::StateScope::Session),
        "run": replayed.snapshot(beton_policy::doc::StateScope::Run),
        "user_day": user_day,
        "user": user,
        "grants": grants,
    }))
}

/// `policy.state.apply`: Änderung in `user_day`/`user` (POL-009).
pub async fn policy_state_apply(
    state: &AppState,
    session: SessionId,
    args: &Value,
) -> Result<Value, Problem> {
    let invalid = |m: String| Problem::new(ProblemCode::ValidationFailed).detail(m);
    let op: StateOp = serde_json::from_value(args.clone())
        .map_err(|e| invalid(format!("Zustandsänderung ungültig: {e}")))?;
    let scope = match op.scope {
        beton_policy::doc::StateScope::UserDay => "user_day",
        beton_policy::doc::StateScope::User => "user",
        _ => return Err(invalid("nur user_day und user liegen im Store".into())),
    };
    let org = state.local.org;
    let record = state.store.session(org, session).await?;
    let day = user_day(state);
    let value = op.value.clone();
    let next = state
        .store
        .update_policy_user_state(org, record.owner, scope, &day, &op.key, |current| {
            next_value(current.as_ref(), op.op, value).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| invalid(e.to_string()))?;
    Ok(json!({ "value": next }))
}

/// `inbox.read`: ungelesene Nachrichten an diese Session (Agent-Inbox).
pub async fn inbox_read(state: &AppState, session: SessionId) -> Result<Value, Problem> {
    state.store.session(state.local.org, session).await?;
    Ok(json!({ "messages": Vec::<Value>::new() }))
}
