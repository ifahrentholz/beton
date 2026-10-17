//! Verbrauchswerte für Policies (POL-004 AC3/AC4) aus der Usage-Projektion (USE-001).
//!
//! Der Runner fragt sie per System-Call `policy.usage` ab, wenn eine Regel `session.cost_usd`,
//! `user.daily_cost_usd`, `user.timezone` o. Ä. benutzt. Geld zählt ohne Subscriptions
//! (USE-001 AC3); Tag und Monat beginnen um 00:00 in `user.timezone` (Default UTC), mit dem
//! Versatz zum Auswertungszeitpunkt (Sommerzeit, [`UserTimeZone`]).
//!
//! Für Budgets (POL-011, POL-012) kommen das Listenpreis-Äquivalent der Subscription-Nutzung
//! (`*_equivalent_usd`, ADR-0034), die Kosten des Teilbaums (Session samt Nachfahren) und die
//! Zahl der Records ohne Preis (`*_without_price`, `policy.rule_inapplicable`) hinzu.

use beton_core::event::{EventPayload, SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId};
use beton_core::timezone::UserTimeZone;
use beton_store::{SessionRecord, Store, UsageScope};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::app::AppState;
use crate::problem::Problem;

fn usd(micro: i64) -> f64 {
    micro as f64 / 1_000_000.0
}

/// `policy.usage` für die gebundene Session.
pub async fn policy_usage(state: &AppState, session: SessionId) -> Result<Value, Problem> {
    usage_values(
        &state.store,
        state.local.org,
        session,
        &state.runtime.timezone,
        OffsetDateTime::now_utc(),
    )
    .await
}

/// Verbrauchswerte der Session und ihres Owners zum Zeitpunkt `now` in der Zeitzone `tz`.
pub async fn usage_values(
    store: &Store,
    org: OrgId,
    session: SessionId,
    tz: &UserTimeZone,
    now: OffsetDateTime,
) -> Result<Value, Problem> {
    let record = store.session(org, session).await?;
    let user = record.owner;
    let s = store
        .usage_totals(org, UsageScope::Session(session))
        .await?;
    let (from, to) = tz.day_bounds(now);
    let day = store
        .usage_totals(org, UsageScope::UserBetween(user, from.into(), to.into()))
        .await?;
    let (from, to) = tz.month_bounds(now);
    let month = store
        .usage_totals(org, UsageScope::UserBetween(user, from.into(), to.into()))
        .await?;
    let tree = store
        .usage_totals(org, UsageScope::Subtree(session))
        .await?;
    // Modelle ohne Preis je Bereich (`on_unpriced`, POL-011, Security-Review #284).
    let (from, to) = tz.day_bounds(now);
    let unpriced_day = store
        .unpriced_models(org, UsageScope::UserBetween(user, from.into(), to.into()))
        .await?;
    let unpriced_session = store
        .unpriced_models(org, UsageScope::Session(session))
        .await?;
    let unpriced_tree = store
        .unpriced_models(org, UsageScope::Subtree(session))
        .await?;
    let meta = session_meta(store, org, &record).await?;
    Ok(json!({
        "session_title": meta.title,
        "session_parent_id": meta.parent_id,
        "session_depth": meta.depth,
        "session_is_async": meta.is_async,
        "session_project": meta.project,
        "run_trigger": meta.run_trigger,
        "session_unpriced_models": unpriced_session,
        "subtree_unpriced_models": unpriced_tree,
        "user_daily_unpriced_models": unpriced_day,
        "session_cost_usd": usd(s.cost_micro),
        "session_equivalent_usd": usd(s.equivalent_micro),
        "session_without_price": s.without_price,
        "session_tokens_in": s.input_tokens,
        "session_tokens_out": s.output_tokens,
        "subtree_cost_usd": usd(tree.cost_micro),
        "subtree_equivalent_usd": usd(tree.equivalent_micro),
        "subtree_without_price": tree.without_price,
        "user_daily_cost_usd": usd(day.cost_micro),
        "user_daily_equivalent_usd": usd(day.equivalent_micro),
        "user_daily_without_price": day.without_price,
        "user_monthly_cost_usd": usd(month.cost_micro),
        "timezone": tz.name(),
    }))
}

/// Höchstens so viele Ebenen über `parent_id` (wie `policy_query`, #276); tiefer gilt die
/// Tiefe als unbekannt.
const MAX_TREE_DEPTH: i64 = 64;

/// Höchstens so viele `agent.spawned` eines Parents werden nach dem Child durchsucht.
const MAX_SPAWNS: u32 = 10_000;

/// Session-Daten für Policies, die nur der Server kennt (#196, POL-004). Unbekanntes bleibt
/// `None` (in CEL `null`; Regeln darauf scheitern, fail closed), nie ein Default.
struct SessionMeta {
    title: Option<String>,
    parent_id: Option<String>,
    depth: Option<i64>,
    is_async: Option<bool>,
    project: Option<String>,
    run_trigger: Option<&'static str>,
}

async fn session_meta(
    store: &Store,
    org: OrgId,
    record: &SessionRecord,
) -> Result<SessionMeta, Problem> {
    // Tiefe im Sub-Agent-Baum: Schritte bis zur Wurzel.
    let mut depth = Some(0);
    let mut current = record.parent_id;
    let mut steps = 0;
    while let Some(parent) = current {
        steps += 1;
        if steps > MAX_TREE_DEPTH {
            depth = None;
            break;
        }
        current = store.session(org, parent).await?.parent_id;
    }
    if depth.is_some() {
        depth = Some(steps);
    }
    let created = match store
        .events(org, record.id, 0, 1)
        .await?
        .first()
        .and_then(|e| e.payload().cloned())
    {
        Some(EventPayload::SessionCreated(c)) => Some(c),
        _ => None,
    };
    let is_async = match &created {
        None => None,
        Some(c) if c.kind == SessionKind::Async => Some(true),
        Some(c) => match c.trigger {
            SessionTrigger::Schedule | SessionTrigger::Timer | SessionTrigger::Webhook => {
                Some(true)
            }
            SessionTrigger::User | SessionTrigger::Api => Some(false),
            // `session_spawn(async: true)` steht als `agent.spawned` im Parent.
            SessionTrigger::Spawn => match record.parent_id {
                Some(parent) => spawned_async(store, org, parent, record.id).await?,
                None => None,
            },
        },
    };
    // Bis es Runs gibt (ASY, M5), ist der Run einer Session die Session selbst.
    let run_trigger = if store.count_events(org, record.id, "session.forked").await? > 0 {
        Some("fork")
    } else if store
        .count_events(org, record.id, "session.imported")
        .await?
        > 0
    {
        Some("import")
    } else {
        created.as_ref().map(|c| match c.trigger {
            SessionTrigger::User | SessionTrigger::Api => "interactive",
            SessionTrigger::Spawn => "spawn",
            SessionTrigger::Schedule => "schedule",
            SessionTrigger::Timer => "timer",
            SessionTrigger::Webhook => "webhook",
        })
    };
    Ok(SessionMeta {
        title: Some(record.title.clone()).filter(|t| !t.is_empty()),
        parent_id: record.parent_id.map(|p| p.to_string()),
        depth,
        is_async,
        project: record.project_id.map(|p| p.to_string()),
        run_trigger,
    })
}

/// Lief das Child laut `agent.spawned` seines Parents mit `async: true`? `None`, wenn der
/// Parent den Start nicht verzeichnet hat.
async fn spawned_async(
    store: &Store,
    org: OrgId,
    parent: SessionId,
    child: SessionId,
) -> Result<Option<bool>, Problem> {
    for event in store
        .events_of_type(org, parent, "agent.spawned", MAX_SPAWNS)
        .await?
    {
        let payload = match event.payload() {
            Some(p) => p.clone(),
            None => store.resolve_payload(org, &event).await?,
        };
        if let EventPayload::AgentSpawned(s) = payload
            && s.child_session_id == child
        {
            return Ok(Some(s.r#async));
        }
    }
    Ok(None)
}
