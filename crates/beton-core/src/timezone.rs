//! Zeitzone des Users (`user.timezone`, POL-004 AC4): Tages- und Monatsgrenzen um 00:00
//! Ortszeit für `user.daily_cost_usd`, `user.monthly_cost_usd`, den Zustand `user_day`
//! (POL-009) und die Tage der Usage-API (USE-005).
//!
//! Eine Zeitzone ist ein IANA-Name (`Europe/Berlin`), `UTC` oder ein fester Versatz
//! (`+02:00`). Der Versatz gilt immer zum jeweiligen Zeitpunkt, also mit Sommerzeit: Ein Tag
//! in `Europe/Berlin` kann 23 oder 25 Stunden haben. Die IANA-Datenbank ist ins Binary
//! eingebettet (`jiff`, Feature `tzdb-bundle-always`): keine Netzabrufe und kein System-tzdata
//! nötig (ADR-0033). Ein unbekannter Name ist ein Fehler, nie stillschweigend UTC.

use std::borrow::Cow;
use std::fmt;

use jiff::tz::{Offset, TimeZone};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{Date, OffsetDateTime, Time, UtcOffset};

/// Höchstzahl der Versatzwechsel, die [`UserTimeZone::offset_changes`] liefert (zwei je Jahr
/// mit Sommerzeit; mehr braucht kein sinnvoller Zeitraum).
pub const MAX_OFFSET_CHANGES: usize = 1000;

/// Zeitzone des Users; Default UTC.
#[derive(Clone)]
pub struct UserTimeZone {
    name: String,
    tz: TimeZone,
}

impl UserTimeZone {
    /// UTC (Default ohne Einstellung).
    pub fn utc() -> Self {
        Self {
            name: "UTC".into(),
            tz: TimeZone::UTC,
        }
    }

    /// Fester Versatz ohne Sommerzeit, Name z. B. `+02:00`.
    pub fn fixed(offset: UtcOffset) -> Self {
        if offset == UtcOffset::UTC {
            return Self::utc();
        }
        let tz = Offset::from_seconds(offset.whole_seconds())
            .map(TimeZone::fixed)
            .unwrap_or(TimeZone::UTC);
        Self {
            name: format_offset(offset),
            tz,
        }
    }

    /// IANA-Name (`Europe/Berlin`, Groß-/Kleinschreibung egal), `UTC`/`Z` oder fester Versatz
    /// (`+02:00`, `-0530`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("utc") || s.eq_ignore_ascii_case("z") {
            return Ok(Self::utc());
        }
        if s.starts_with('+') || s.starts_with('-') {
            return parse_offset(s).map(Self::fixed);
        }
        let tz = TimeZone::get(s).map_err(|_| {
            format!(
                "Zeitzone `{raw}` ist unbekannt (erwartet ein IANA-Name wie Europe/Berlin, UTC \
                 oder ein Versatz wie +02:00)"
            )
        })?;
        Ok(Self {
            name: tz.iana_name().unwrap_or(s).to_owned(),
            tz,
        })
    }

    /// Name wie eingestellt (IANA-Namen in kanonischer Schreibweise).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Versatz zu UTC zum Zeitpunkt `at` (mit Sommerzeit).
    pub fn offset_at(&self, at: OffsetDateTime) -> UtcOffset {
        let seconds = self.tz.to_offset(to_jiff(at)).seconds();
        UtcOffset::from_whole_seconds(seconds).unwrap_or(UtcOffset::UTC)
    }

    /// Kalendertag von `at` in dieser Zeitzone.
    pub fn local_date(&self, at: OffsetDateTime) -> Date {
        at.to_offset(self.offset_at(at)).date()
    }

    /// Beginn des Kalendertags `date` (00:00 Ortszeit; fällt 00:00 in eine Sommerzeit-Lücke,
    /// der erste Zeitpunkt danach).
    pub fn start_of(&self, date: Date) -> OffsetDateTime {
        let civil = jiff::civil::Date::new(
            i16::try_from(date.year()).unwrap_or(i16::MAX),
            i8::try_from(u8::from(date.month())).unwrap_or(1),
            i8::try_from(date.day()).unwrap_or(1),
        );
        civil
            .and_then(|d| d.to_zoned(self.tz.clone()))
            .ok()
            .and_then(|z| from_jiff(z.timestamp()))
            .unwrap_or_else(|| {
                // Außerhalb des darstellbaren Bereichs: Versatz um Mitternacht UTC.
                let utc = date.with_time(Time::MIDNIGHT).assume_utc();
                date.with_time(Time::MIDNIGHT)
                    .assume_offset(self.offset_at(utc))
            })
    }

    /// Tag `[00:00, 00:00 des Folgetags)` Ortszeit, der `now` enthält.
    pub fn day_bounds(&self, now: OffsetDateTime) -> (OffsetDateTime, OffsetDateTime) {
        let date = self.local_date(now);
        let next = date.next_day().unwrap_or(date);
        (self.start_of(date), self.start_of(next))
    }

    /// Monat `[1. 00:00, 1. des Folgemonats 00:00)` Ortszeit, der `now` enthält.
    pub fn month_bounds(&self, now: OffsetDateTime) -> (OffsetDateTime, OffsetDateTime) {
        let local = self.local_date(now);
        let first = Date::from_calendar_date(local.year(), local.month(), 1).unwrap_or(local);
        let next_month = local.month().next();
        let year = if next_month == time::Month::January {
            local.year() + 1
        } else {
            local.year()
        };
        let next = Date::from_calendar_date(year, next_month, 1).unwrap_or(first);
        (self.start_of(first), self.start_of(next))
    }

    /// Kalendertag von `now` als `JJJJ-MM-TT` (Schlüssel von `user_day`, POL-009).
    pub fn day_key(&self, now: OffsetDateTime) -> String {
        let d = self.local_date(now);
        format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
    }

    /// Versatzwechsel in `(from, to)`: ab dem Zeitpunkt gilt der Versatz (höchstens
    /// [`MAX_OFFSET_CHANGES`]). Vor dem ersten Wechsel gilt `offset_at(from)`.
    pub fn offset_changes(
        &self,
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> Vec<(OffsetDateTime, UtcOffset)> {
        let mut out = Vec::new();
        for t in self.tz.following(to_jiff(from)) {
            let Some(at) = from_jiff(t.timestamp()) else {
                break;
            };
            if at >= to || out.len() >= MAX_OFFSET_CHANGES {
                break;
            }
            if let Ok(o) = UtcOffset::from_whole_seconds(t.offset().seconds()) {
                out.push((at, o));
            }
        }
        out
    }
}

fn to_jiff(t: OffsetDateTime) -> jiff::Timestamp {
    jiff::Timestamp::from_nanosecond(t.unix_timestamp_nanos()).unwrap_or(if t.year() < 1970 {
        jiff::Timestamp::MIN
    } else {
        jiff::Timestamp::MAX
    })
}

fn from_jiff(t: jiff::Timestamp) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(t.as_nanosecond()).ok()
}

/// Versatz `+02:00`, `-0530`, `Z` bzw. `UTC`.
pub fn parse_offset(raw: &str) -> Result<UtcOffset, String> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("z") || s.eq_ignore_ascii_case("utc") {
        return Ok(UtcOffset::UTC);
    }
    let invalid = || format!("tz `{raw}` ist ungültig (erwartet z. B. +02:00 oder Z)");
    let (sign, rest) = match s.as_bytes().first() {
        Some(b'+') => (1, &s[1..]),
        Some(b'-') => (-1, &s[1..]),
        _ => return Err(invalid()),
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    let h: i8 = digits[..2].parse().map_err(|_| invalid())?;
    let m: i8 = digits[2..].parse().map_err(|_| invalid())?;
    // Reale Versätze liegen zwischen -12:00 und +14:00; mehr als 18 Stunden ist ein Tippfehler.
    if h > 18 || m > 59 {
        return Err(invalid());
    }
    UtcOffset::from_hms(sign * h, sign * m, 0).map_err(|_| invalid())
}

/// Versatz als Text, z. B. `+02:00`.
pub fn format_offset(o: UtcOffset) -> String {
    let (h, m, _) = o.as_hms();
    let sign = if o.is_negative() { '-' } else { '+' };
    format!("{sign}{:02}:{:02}", h.unsigned_abs(), m.unsigned_abs())
}

impl Default for UserTimeZone {
    fn default() -> Self {
        Self::utc()
    }
}

impl PartialEq for UserTimeZone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for UserTimeZone {}

impl fmt::Debug for UserTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UserTimeZone").field(&self.name).finish()
    }
}

impl fmt::Display for UserTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl std::str::FromStr for UserTimeZone {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for UserTimeZone {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.name)
    }
}

impl<'de> Deserialize<'de> for UserTimeZone {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for UserTimeZone {
    fn schema_name() -> Cow<'static, str> {
        "UserTimeZone".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "IANA-Zeitzone (z. B. `Europe/Berlin`), `UTC` oder fester Versatz \
                            (`+02:00`).",
            "examples": ["Europe/Berlin", "UTC"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime, offset};

    fn berlin() -> UserTimeZone {
        UserTimeZone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn pol_004_ac4_offset_is_taken_at_the_evaluation_time() {
        let tz = berlin();
        assert_eq!(tz.offset_at(datetime!(2026-07-01 12:00 UTC)), offset!(+2));
        assert_eq!(tz.offset_at(datetime!(2026-12-01 12:00 UTC)), offset!(+1));
        // Umstellung am 29. März 2026 um 01:00 UTC.
        assert_eq!(tz.offset_at(datetime!(2026-03-29 00:59 UTC)), offset!(+1));
        assert_eq!(tz.offset_at(datetime!(2026-03-29 01:00 UTC)), offset!(+2));
    }

    #[test]
    fn pol_004_ac4_day_starts_at_local_midnight_across_daylight_saving_changes() {
        let tz = berlin();
        // Winter: 23:30Z am 1. Januar ist schon 00:30 am 2. Januar Ortszeit.
        let (from, to) = tz.day_bounds(datetime!(2026-01-01 23:30 UTC));
        assert_eq!(from, datetime!(2026-01-01 23:00 UTC));
        assert_eq!(to, datetime!(2026-01-02 23:00 UTC));
        // Sommer: 22:30Z am 1. Juli ist schon 00:30 am 2. Juli Ortszeit.
        let (from, to) = tz.day_bounds(datetime!(2026-07-01 22:30 UTC));
        assert_eq!(from, datetime!(2026-07-01 22:00 UTC));
        assert_eq!(to, datetime!(2026-07-02 22:00 UTC));
        // Beginn der Sommerzeit: der Tag hat 23 Stunden, begann aber noch mit +01:00.
        let (from, to) = tz.day_bounds(datetime!(2026-03-29 12:00 UTC));
        assert_eq!(from, datetime!(2026-03-28 23:00 UTC));
        assert_eq!(to, datetime!(2026-03-29 22:00 UTC));
        assert_eq!(to - from, time::Duration::hours(23));
        // Ende der Sommerzeit: 25 Stunden.
        let (from, to) = tz.day_bounds(datetime!(2026-10-25 12:00 UTC));
        assert_eq!(from, datetime!(2026-10-24 22:00 UTC));
        assert_eq!(to, datetime!(2026-10-25 23:00 UTC));
        assert_eq!(to - from, time::Duration::hours(25));
    }

    #[test]
    fn pol_004_ac4_utc_and_fixed_offsets() {
        let (from, to) = UserTimeZone::utc().day_bounds(datetime!(2026-01-02 00:30 UTC));
        assert_eq!(from, datetime!(2026-01-02 00:00 UTC));
        assert_eq!(to, datetime!(2026-01-03 00:00 UTC));
        let minus5 = UserTimeZone::parse("-05:00").unwrap();
        assert_eq!(minus5.name(), "-05:00");
        let (from, _) = minus5.day_bounds(datetime!(2026-01-02 03:00 UTC));
        assert_eq!(from, datetime!(2026-01-01 05:00 UTC));
        assert_eq!(UserTimeZone::default(), UserTimeZone::utc());
    }

    #[test]
    fn pol_004_ac4_month_bounds_in_the_users_timezone() {
        let tz = berlin();
        // März 2026: beginnt mit +01:00, endet mit +02:00.
        let (from, to) = tz.month_bounds(datetime!(2026-03-15 12:00 UTC));
        assert_eq!(from, datetime!(2026-02-28 23:00 UTC));
        assert_eq!(to, datetime!(2026-03-31 22:00 UTC));
        // 23:30Z am 31. Dezember ist in Berlin schon Januar.
        let (from, to) = tz.month_bounds(datetime!(2026-12-31 23:30 UTC));
        assert_eq!(from, datetime!(2026-12-31 23:00 UTC));
        assert_eq!(to, datetime!(2027-01-31 23:00 UTC));
        let (from, to) = UserTimeZone::utc().month_bounds(datetime!(2026-12-15 12:00 UTC));
        assert_eq!(from, datetime!(2026-12-01 00:00 UTC));
        assert_eq!(to, datetime!(2027-01-01 00:00 UTC));
    }

    #[test]
    fn pol_009_day_key_follows_the_users_timezone() {
        let tz = berlin();
        assert_eq!(tz.day_key(datetime!(2026-07-01 21:59 UTC)), "2026-07-01");
        assert_eq!(tz.day_key(datetime!(2026-07-01 22:00 UTC)), "2026-07-02");
        assert_eq!(tz.day_key(datetime!(2026-12-01 22:59 UTC)), "2026-12-01");
        assert_eq!(tz.day_key(datetime!(2026-12-01 23:00 UTC)), "2026-12-02");
    }

    #[test]
    fn pol_004_ac4_unknown_timezone_is_an_error_not_utc() {
        for bad in ["Europe/Berln", "Mars/Olympus", "", "+25:00", "Berlin"] {
            let err = UserTimeZone::parse(bad).unwrap_err();
            assert!(err.contains(bad) || bad.is_empty(), "{err}");
        }
        let err = serde_json::from_value::<UserTimeZone>(serde_json::json!("Europe/Berln"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unbekannt"), "{err}");
    }

    #[test]
    fn names_are_canonical_and_round_trip() {
        let tz = UserTimeZone::parse("europe/berlin").unwrap();
        assert_eq!(tz.name(), "Europe/Berlin");
        assert_eq!(UserTimeZone::parse("Z").unwrap().name(), "UTC");
        let json = serde_json::to_value(&tz).unwrap();
        assert_eq!(json, serde_json::json!("Europe/Berlin"));
        assert_eq!(serde_json::from_value::<UserTimeZone>(json).unwrap(), tz);
    }

    #[test]
    fn offset_changes_list_the_transitions_in_the_range() {
        let tz = berlin();
        let changes = tz.offset_changes(
            datetime!(2026-03-01 00:00 UTC),
            datetime!(2026-11-01 00:00 UTC),
        );
        assert_eq!(
            changes,
            vec![
                (datetime!(2026-03-29 01:00 UTC), offset!(+2)),
                (datetime!(2026-10-25 01:00 UTC), offset!(+1)),
            ]
        );
        assert!(
            UserTimeZone::utc()
                .offset_changes(
                    datetime!(2026-01-01 0:00 UTC),
                    datetime!(2027-01-01 0:00 UTC)
                )
                .is_empty()
        );
        assert_eq!(
            tz.start_of(date!(2026 - 03 - 29)),
            datetime!(2026-03-28 23:00 UTC)
        );
    }

    #[test]
    fn offsets_round_trip() {
        for s in ["+02:00", "-05:30", "+00:00"] {
            assert_eq!(format_offset(parse_offset(s).unwrap()), s);
        }
        assert_eq!(parse_offset("Z").unwrap(), UtcOffset::UTC);
        assert_eq!(format_offset(parse_offset("+0530").unwrap()), "+05:30");
    }
}
