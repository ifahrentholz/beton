//! Zeitstempel im Event-Log: RFC 3339, UTC, Millisekunden (PROTO-001).

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

/// Zeitpunkt mit Millisekunden-Genauigkeit in UTC, z. B. `2026-10-03T12:00:00.123Z`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    pub fn now() -> Self {
        Self::from(OffsetDateTime::now_utc())
    }

    pub fn as_offset_date_time(&self) -> OffsetDateTime {
        self.0
    }
}

impl From<OffsetDateTime> for Timestamp {
    /// Normalisiert auf UTC und kürzt auf Millisekunden.
    fn from(t: OffsetDateTime) -> Self {
        let t = t.to_offset(UtcOffset::UTC);
        let ms = t.millisecond();
        Self(t.replace_millisecond(ms).unwrap_or(t))
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self(OffsetDateTime::UNIX_EPOCH)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format = format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        );
        let s = self.0.format(&format).map_err(|_| fmt::Error)?;
        f.write_str(&s)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl std::str::FromStr for Timestamp {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        OffsetDateTime::parse(s, &Rfc3339)
            .map(Self::from)
            .map_err(|e| format!("ungültiger Zeitstempel `{s}`: {e}"))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Timestamp {
    fn schema_name() -> Cow<'static, str> {
        "Timestamp".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "format": "date-time" })
    }
}

impl ts_rs::TS for Timestamp {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &ts_rs::Config) -> String {
        "string".to_owned()
    }
    fn inline(cfg: &ts_rs::Config) -> String {
        <Self as ts_rs::TS>::name(cfg)
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn proto_001_ac1_timestamp_is_utc_with_milliseconds() {
        let t = Timestamp::from(datetime!(2026-10-03 14:00:00.123456 +02:00));
        assert_eq!(t.to_string(), "2026-10-03T12:00:00.123Z");
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), t);
    }
}
