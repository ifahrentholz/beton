//! API-Keys als Secrets (HAR-011 AC5).
//!
//! Ein [`ApiKey`] verrät seinen Wert weder über `Debug` noch über `Display`; nur
//! [`ApiKey::expose`] liefert ihn – ausschließlich für den HTTP-Header des Model-Requests.
//! Texte, die von außen kommen (Fehlermeldungen des Anbieters), laufen vor der Übernahme in
//! Events durch [`ApiKey::scrub`]. In M1 stammt ein Key aus einer Umgebungsvariable des
//! Daemons (`api_key_env`); der Daemon reicht ihn über stdin an den Runner, nie über Env
//! oder argv. `secret://` und Proxy-Platzhalter folgen mit M2 (SEC-001, PRX-006).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Ersatztext für einen Key in Texten.
pub const REDACTED: &str = "[REDACTED]";

/// Ein API-Key. `Debug` und `Display` zeigen nie den Wert.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(Arc<str>);

impl ApiKey {
    /// `None` für leere Werte.
    pub fn new(value: &str) -> Option<Self> {
        let v = value.trim();
        (!v.is_empty()).then(|| Self(Arc::from(v)))
    }

    /// Der Klartext, nur für den Header des Requests.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Ersetzt jedes Vorkommen des Keys in `text`.
    pub fn scrub(&self, text: &str) -> String {
        if self.0.len() < 4 {
            return text.to_owned();
        }
        text.replace(&*self.0, REDACTED)
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([REDACTED])")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Woher ein Adapter Keys für `api_key_env` bekommt.
pub trait KeySource: Send + Sync {
    fn get(&self, var: &str) -> Option<ApiKey>;
}

/// Die Umgebung des eigenen Prozesses (Daemon: Modell-Discovery im Katalog).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnv;

impl KeySource for ProcessEnv {
    fn get(&self, var: &str) -> Option<ApiKey> {
        std::env::var(var).ok().and_then(|v| ApiKey::new(&v))
    }
}

/// Keys, die der Daemon dem Runner übergeben hat (Name der Variable → Wert).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct KeyMap(BTreeMap<String, ApiKey>);

impl KeyMap {
    pub fn new(map: BTreeMap<String, ApiKey>) -> Self {
        Self(map)
    }

    /// Aus der JSON-Zeile, die der Host auf stdin schreibt (`{"VAR": "wert"}`); ungültige
    /// oder leere Zeilen ergeben eine leere Map.
    pub fn from_json_line(line: &str) -> Self {
        let parsed: BTreeMap<String, String> =
            serde_json::from_str(line.trim()).unwrap_or_default();
        Self(
            parsed
                .into_iter()
                .filter_map(|(k, v)| ApiKey::new(&v).map(|key| (k, key)))
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

impl fmt::Debug for KeyMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl KeySource for KeyMap {
    fn get(&self, var: &str) -> Option<ApiKey> {
        self.0.get(var).cloned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const MARKER: &str = "bt-fake-key-MARKER-0123456789";

    #[test]
    fn har_011_ac5_key_never_shows_in_debug_or_display() {
        let key = ApiKey::new(MARKER).unwrap();
        assert!(!format!("{key:?}").contains(MARKER));
        assert!(!format!("{key}").contains(MARKER));
        let map = KeyMap::from_json_line(&format!("{{\"OPENROUTER_API_KEY\": \"{MARKER}\"}}"));
        assert!(!format!("{map:?}").contains(MARKER));
        assert_eq!(map.get("OPENROUTER_API_KEY").unwrap().expose(), MARKER);
        assert_eq!(
            key.scrub(&format!("invalid key {MARKER}!")),
            "invalid key [REDACTED]!"
        );
        assert!(ApiKey::new("  ").is_none());
        assert!(KeyMap::from_json_line("kein json").is_empty());
    }
}
