//! IDs mit Präfix und ULID (PROTO-001), z. B. `ses_01JB8Y2D0M3K4J5H6G7F8E9D0C`.
//!
//! Jede Art hat ihren eigenen Typ; eine ID mit falschem Präfix wird beim Parsen und
//! Deserialisieren abgelehnt. Im JSON-Schema sind IDs Strings mit Muster, in TypeScript
//! Template-Literal-Typen (`` `ses_${string}` ``).

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("ID `{value}` hat nicht das Präfix `{expected}_`")]
    WrongPrefix {
        value: String,
        expected: &'static str,
    },
    #[error("ID `{value}` enthält keine gültige ULID")]
    InvalidUlid { value: String },
}

/// Suffix der festen lokalen IDs (`org_local`, `usr_local`, DATA-001); intern die Null-ULID.
const LOCAL: &str = "local";

fn parse_prefixed(prefix: &'static str, local: bool, value: &str) -> Result<Ulid, IdError> {
    let rest = value
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .ok_or_else(|| IdError::WrongPrefix {
            value: value.to_owned(),
            expected: prefix,
        })?;
    if local && rest == LOCAL {
        return Ok(Ulid::nil());
    }
    // ULIDs sind genau 26 Zeichen Crockford-Base32 (Großbuchstaben).
    if rest.len() != 26 || rest.chars().any(|c| c.is_ascii_lowercase()) {
        return Err(IdError::InvalidUlid {
            value: value.to_owned(),
        });
    }
    Ulid::from_string(rest).map_err(|_| IdError::InvalidUlid {
        value: value.to_owned(),
    })
}

macro_rules! ids {
    ($($(#[$meta:meta])* $name:ident => $prefix:literal $(+ $local:ident)?,)*) => {$(
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Ulid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Neue, zeitlich sortierbare ID.
            pub fn new() -> Self {
                Self(Ulid::generate())
            }

            pub fn from_ulid(ulid: Ulid) -> Self {
                Self(ulid)
            }

            pub fn ulid(&self) -> Ulid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $name {
            /// Ob es für diese Art eine feste lokale ID (`<präfix>_local`) gibt.
            const HAS_LOCAL: bool = false $(|| stringify!($local).len() > 0)?;
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if Self::HAS_LOCAL && self.0.is_nil() {
                    write!(f, "{}_{}", $prefix, LOCAL)
                } else {
                    write!(f, "{}_{}", $prefix, self.0)
                }
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                parse_prefixed($prefix, Self::HAS_LOCAL, s).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = Cow::<str>::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                let pattern = if Self::HAS_LOCAL {
                    concat!("^", $prefix, "_([0-9A-HJKMNP-TV-Z]{26}|local)$")
                } else {
                    concat!("^", $prefix, "_[0-9A-HJKMNP-TV-Z]{26}$")
                };
                json_schema!({
                    "type": "string",
                    "pattern": pattern,
                    "examples": [concat!($prefix, "_01JB8Y2D0M3K4J5H6G7F8E9D0C")]
                })
            }
        }

        impl ts_rs::TS for $name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;
            fn name(_: &ts_rs::Config) -> String {
                concat!("`", $prefix, "_${string}`").to_owned()
            }
            fn inline(cfg: &ts_rs::Config) -> String {
                <Self as ts_rs::TS>::name(cfg)
            }
        }
    )*};
}

ids! {
    /// Session.
    SessionId => "ses",
    /// Event im Session-Log.
    EventId => "evt",
    /// Turn (eine Eingabe und die Arbeit des Agents daran).
    TurnId => "trn",
    /// Mensch; lokal genau `usr_local` (DATA-001).
    UserId => "usr" + local,
    ServiceAccountId => "sa",
    /// Organisation; lokal genau `org_local` (DATA-001).
    OrgId => "org" + local,
    TeamId => "team",
    ProjectId => "prj",
    AgentId => "agt",
    PolicyId => "pol",
    SecretId => "sec",
    HostId => "hst",
    RunnerId => "run",
    DeviceId => "dev",
    ScheduleId => "sch",
    TimerId => "tmr",
    CommentId => "cmt",
    ShareId => "shr",
    ApprovalId => "apr",
    BlobId => "bls",
    NodeId => "nod",
}

impl OrgId {
    /// Die einzige Org im lokalen Modus.
    pub const LOCAL: Self = Self(Ulid(0));
}

impl UserId {
    /// Der einzige User im lokalen Modus.
    pub const LOCAL: Self = Self(Ulid(0));
}

/// Ein Mensch oder ein Service-Account (`usr_…` bzw. `sa_…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalId {
    User(UserId),
    ServiceAccount(ServiceAccountId),
}

impl Default for PrincipalId {
    fn default() -> Self {
        Self::User(UserId::new())
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(id) => id.fmt(f),
            Self::ServiceAccount(id) => id.fmt(f),
        }
    }
}

impl FromStr for PrincipalId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("sa_") {
            s.parse().map(Self::ServiceAccount)
        } else {
            s.parse().map(Self::User)
        }
    }
}

impl Serialize for PrincipalId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PrincipalId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for PrincipalId {
    fn schema_name() -> Cow<'static, str> {
        "PrincipalId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(usr_([0-9A-HJKMNP-TV-Z]{26}|local)|sa_[0-9A-HJKMNP-TV-Z]{26})$"
        })
    }
}

impl ts_rs::TS for PrincipalId {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &ts_rs::Config) -> String {
        "`usr_${string}` | `sa_${string}`".to_owned()
    }
    fn inline(cfg: &ts_rs::Config) -> String {
        <Self as ts_rs::TS>::name(cfg)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn proto_001_ac2_id_with_wrong_prefix_is_rejected() {
        let user = UserId::new().to_string();
        let err = serde_json::from_value::<SessionId>(serde_json::Value::String(user)).unwrap_err();
        assert!(err.to_string().contains("ses_"), "{err}");
        assert!("ses_kurz".parse::<SessionId>().is_err());
        assert!(
            "ses_01jb8y2d0m3k4j5h6g7f8e9d0c"
                .parse::<SessionId>()
                .is_err()
        );
        assert!(
            "ses01JB8Y2D0M3K4J5H6G7F8E9D0C"
                .parse::<SessionId>()
                .is_err()
        );
    }

    #[test]
    fn local_ids_exist_only_for_org_and_user() {
        assert_eq!(OrgId::LOCAL.to_string(), "org_local");
        assert_eq!("usr_local".parse::<UserId>().unwrap(), UserId::LOCAL);
        assert_eq!(
            serde_json::to_value(PrincipalId::User(UserId::LOCAL)).unwrap(),
            "usr_local"
        );
        assert!("ses_local".parse::<SessionId>().is_err());
        assert_eq!(
            SessionId::from_ulid(Ulid::nil()).to_string(),
            "ses_00000000000000000000000000"
        );
    }

    #[test]
    fn principal_accepts_users_and_service_accounts_only() {
        let sa = ServiceAccountId::new().to_string();
        assert!(matches!(
            sa.parse::<PrincipalId>(),
            Ok(PrincipalId::ServiceAccount(_))
        ));
        assert!(SessionId::new().to_string().parse::<PrincipalId>().is_err());
    }

    proptest! {
        #[test]
        fn proto_001_ac1_ids_roundtrip(raw in any::<u128>()) {
            let id = SessionId::from_ulid(Ulid(raw));
            let json = serde_json::to_string(&id).unwrap();
            prop_assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);
            prop_assert!(json.starts_with("\"ses_"));
        }
    }
}
