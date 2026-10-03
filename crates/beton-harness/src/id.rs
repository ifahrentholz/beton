use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stabiler Bezeichner eines Harness: `claude`, `codex`, `fake`, `acp:<slug>`,
/// `direct:<provider>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HarnessId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` ist keine Harness-ID (erlaubt: claude, codex, fake, acp:<slug>, direct:<provider>)")]
pub struct InvalidHarnessId(String);

impl HarnessId {
    pub const CLAUDE: &'static str = "claude";
    pub const CODEX: &'static str = "codex";
    pub const FAKE: &'static str = "fake";

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Name der Umgebungsvariable für den Binary-Pfad (HAR-003), z. B. `BETON_CLAUDE_PATH`
    /// oder `BETON_ACP_GEMINI_PATH`.
    pub fn path_env_var(&self) -> String {
        let name: String = self
            .0
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        format!("BETON_{name}_PATH")
    }
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl FromStr for HarnessId {
    type Err = InvalidHarnessId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = match s.split_once(':') {
            None => matches!(s, Self::CLAUDE | Self::CODEX | Self::FAKE),
            Some(("acp" | "direct", slug)) => valid_slug(slug),
            Some(_) => false,
        };
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidHarnessId(s.to_owned()))
        }
    }
}

impl fmt::Display for HarnessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for HarnessId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HarnessId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for HarnessId {
    fn schema_name() -> Cow<'static, str> {
        "HarnessId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(claude|codex|fake|(acp|direct):[a-z0-9_-]{1,64})$"
        })
    }
}

impl ts_rs::TS for HarnessId {
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
    use super::*;

    #[test]
    fn harness_ids_follow_the_spec() {
        for ok in ["claude", "codex", "fake", "acp:gemini", "direct:openrouter"] {
            assert!(ok.parse::<HarnessId>().is_ok(), "{ok}");
        }
        for bad in ["", "Claude", "acp:", "acp:Gemini", "ssh:x", "cursor"] {
            assert!(bad.parse::<HarnessId>().is_err(), "{bad}");
        }
        let id: HarnessId = "acp:gemini-cli".parse().unwrap();
        assert_eq!(id.path_env_var(), "BETON_ACP_GEMINI_CLI_PATH");
    }
}
