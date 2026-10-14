//! Verbrauchswerte für Policies (POL-004 AC3/AC4) aus der Usage-Projektion (USE-001).
//!
//! Der Runner fragt sie per System-Call `policy.usage` ab, wenn eine Regel `session.cost_usd`,
//! `user.daily_cost_usd` o. Ä. benutzt. Geld zählt ohne Subscriptions (USE-001 AC3); die
//! Tagesgrenze ist 00:00 in der Zeitzone des Users (bis zu einer Einstellung dafür: UTC).

use beton_core::id::SessionId;
use beton_store::UsageScope;
use serde_json::{Value, json};
use time::{Date, OffsetDateTime, Time, UtcOffset};

use crate::app::AppState;
use crate::problem::Problem;

/// Tag `[00:00, 00:00 + 1 Tag)` in `offset`, der `now` enthält.
pub fn day_bounds(now: OffsetDateTime, offset: UtcOffset) -> (OffsetDateTime, OffsetDateTime) {
    let local = now.to_offset(offset);
    let start = local.replace_time(Time::MIDNIGHT);
    (start, start + time::Duration::DAY)
}

/// Monat `[1. 00:00, 1. des Folgemonats)` in `offset`, der `now` enthält.
pub fn month_bounds(now: OffsetDateTime, offset: UtcOffset) -> (OffsetDateTime, OffsetDateTime) {
    let local = now.to_offset(offset);
    let first = Date::from_calendar_date(local.year(), local.month(), 1).unwrap_or(local.date());
    let start = first.with_time(Time::MIDNIGHT).assume_offset(offset);
    let next_month = local.month().next();
    let year = if next_month == time::Month::January {
        local.year() + 1
    } else {
        local.year()
    };
    let end = Date::from_calendar_date(year, next_month, 1)
        .unwrap_or(first)
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset);
    (start, end)
}

fn usd(micro: i64) -> f64 {
    micro as f64 / 1_000_000.0
}

/// `policy.usage` für die gebundene Session.
pub async fn policy_usage(state: &AppState, session: SessionId) -> Result<Value, Problem> {
    let org = state.local.org;
    let record = state.store.session(org, session).await?;
    let user = record.owner;
    let offset = UtcOffset::UTC;
    let now = OffsetDateTime::now_utc();
    let s = state
        .store
        .usage_totals(org, UsageScope::Session(session))
        .await?;
    let (from, to) = day_bounds(now, offset);
    let day = state
        .store
        .usage_totals(org, UsageScope::UserBetween(user, from.into(), to.into()))
        .await?;
    let (from, to) = month_bounds(now, offset);
    let month = state
        .store
        .usage_totals(org, UsageScope::UserBetween(user, from.into(), to.into()))
        .await?;
    Ok(json!({
        "session_cost_usd": usd(s.cost_micro),
        "session_tokens_in": s.input_tokens,
        "session_tokens_out": s.output_tokens,
        "user_daily_cost_usd": usd(day.cost_micro),
        "user_monthly_cost_usd": usd(month.cost_micro),
        "timezone": "UTC",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn pol_004_ac4_day_starts_at_midnight_in_the_users_timezone() {
        // UTC (Default): 00:30Z gehört zum 2. Januar.
        let (from, to) = day_bounds(datetime!(2026-01-02 00:30 UTC), UtcOffset::UTC);
        assert_eq!(from, datetime!(2026-01-02 00:00 UTC));
        assert_eq!(to, datetime!(2026-01-03 00:00 UTC));
        // +02:00: 23:30Z am 1. ist schon 01:30 am 2. Januar Ortszeit.
        let plus2 = UtcOffset::from_hms(2, 0, 0).unwrap();
        let (from, to) = day_bounds(datetime!(2026-01-01 23:30 UTC), plus2);
        assert_eq!(from, datetime!(2026-01-01 22:00 UTC));
        assert_eq!(to, datetime!(2026-01-02 22:00 UTC));
        // -05:00: 03:00Z am 2. ist noch der 1. Januar Ortszeit.
        let minus5 = UtcOffset::from_hms(-5, 0, 0).unwrap();
        let (from, _) = day_bounds(datetime!(2026-01-02 03:00 UTC), minus5);
        assert_eq!(from, datetime!(2026-01-01 05:00 UTC));
    }

    #[test]
    fn pol_004_month_bounds_cover_the_calendar_month() {
        let (from, to) = month_bounds(datetime!(2026-12-15 12:00 UTC), UtcOffset::UTC);
        assert_eq!(from, datetime!(2026-12-01 00:00 UTC));
        assert_eq!(to, datetime!(2027-01-01 00:00 UTC));
    }
}
