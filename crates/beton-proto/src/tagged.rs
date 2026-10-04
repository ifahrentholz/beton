//! Lesen intern getaggter Nachrichten, die Events mit `raw` tragen.
//!
//! serde puffert den Inhalt intern getaggter Enums (`#[serde(tag = "t")]`); aus diesem Puffer
//! lässt sich `serde_json::value::RawValue` nicht lesen. Varianten mit Events werden deshalb
//! direkt als Struct gelesen, alle anderen wie gewohnt.

use serde::Deserialize;

/// Nur das Tag einer Nachricht.
#[derive(Deserialize)]
pub(crate) struct Tag<'a> {
    #[serde(borrow)]
    pub t: std::borrow::Cow<'a, str>,
}

pub(crate) fn tag(text: &str) -> Result<String, serde_json::Error> {
    Ok(serde_json::from_str::<Tag<'_>>(text)?.t.into_owned())
}
