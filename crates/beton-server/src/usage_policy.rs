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

use beton_core::id::{OrgId, SessionId};
use beton_core::timezone::UserTimeZone;
use beton_store::{Store, UsageScope};
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
    Ok(json!({
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
