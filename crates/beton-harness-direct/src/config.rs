//! Provider-Konfiguration (HAR-011): Abschnitt `providers:` der User-Konfiguration.
//!
//! ```yaml
//! providers:
//!   openrouter:
//!     kind: openai                       # anthropic | openai
//!     base_url: https://openrouter.ai/api/v1
//!     api_key_env: OPENROUTER_API_KEY    # M1: Variable des Daemons; ab M2 secret://…
//!     models:
//!       - id: qwen/qwen3-coder
//!         context_window: 262144
//!         pricing: { input_per_mtok: 0.4, output_per_mtok: 1.6 }
//!   ollama:
//!     kind: openai
//!     base_url: http://127.0.0.1:11434/v1
//!     api_key: none
//! ```
//!
//! Regeln (fail closed): Provider stehen nur in der User-Konfiguration – ein Repository kann
//! keine Endpunkte setzen, an die ein Key aus der Umgebung des Daemons ginge. Klartext-Keys in
//! der Datei werden abgelehnt (der Wert erscheint nie in der Meldung), `secret://` kommt mit
//! M2. Mit Key ist Klartext-HTTP nur zu Loopback erlaubt. Jeder Fehler nennt Pfad und Grund
//! (AC4) und betrifft nur den einen Provider.

use std::collections::BTreeMap;
use std::fmt;

use reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire-Familie eines Providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WireKind {
    /// Anthropic Messages-API (`POST {base_url}/v1/messages`).
    Anthropic,
    /// OpenAI Chat Completions (`POST {base_url}/chat/completions`), auch OpenRouter,
    /// LiteLLM, vLLM, Ollama, LM Studio.
    Openai,
}

/// Preise eines Modells in USD je Million Tokens (Preis-Override, Owner USE-003).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// Ein Modell unter `providers.<name>.models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub id: String,
    /// Kontextfenster in Tokens (für Kontextanzeige und Compaction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}

/// Ein Eintrag unter `providers.<name>` (Schema für `config.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub kind: WireKind,
    /// Basis-URL, z. B. `https://api.anthropic.com` oder `https://openrouter.ai/api/v1`.
    pub base_url: String,
    /// `none` für Endpunkte ohne Key (lokale Gateways); `secret://…` ab M2 (SEC-001).
    /// Klartext-Keys sind nicht erlaubt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// M1: Name der Umgebungsvariable des Daemons mit dem Key, z. B. `OPENROUTER_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Anthropic: `cache_control` für System-Prompt und Tools setzen (Default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelConfig>,
}

/// Woher der Key eines Providers kommt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRef {
    /// Kein Key (`api_key: none`), z. B. Ollama auf Loopback.
    None,
    /// Umgebungsvariable des Daemons.
    Env(String),
}

/// Ein geprüfter Provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub name: String,
    pub wire: WireKind,
    pub base_url: Url,
    pub key: KeyRef,
    pub prompt_caching: bool,
    pub models: Vec<ModelConfig>,
}

impl Provider {
    /// Harness-ID `direct:<name>`.
    pub fn harness_id(&self) -> String {
        format!("direct:{}", self.name)
    }

    pub fn model(&self, id: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.id == id)
    }

    /// URL eines Pfads unterhalb von `base_url`, z. B. `chat/completions`.
    pub fn endpoint(&self, path: &str) -> Url {
        let base = self.base_url.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}/{}", path.trim_start_matches('/')))
            .unwrap_or_else(|_| self.base_url.clone())
    }

    /// Pfad des Model-Requests.
    pub fn messages_path(&self) -> &'static str {
        match self.wire {
            WireKind::Anthropic => "v1/messages",
            WireKind::Openai => "chat/completions",
        }
    }

    /// Pfad der Modell-Liste.
    pub fn models_path(&self) -> &'static str {
        match self.wire {
            WireKind::Anthropic => "v1/models",
            WireKind::Openai => "models",
        }
    }
}

/// Ein ungültiger Eintrag mit Pfad und Grund (HAR-011 AC4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    /// z. B. `providers.openrouter.kind`.
    pub path: String,
    pub reason: String,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for ConfigProblem {}

fn problem(name: &str, field: &str, reason: impl Into<String>) -> ConfigProblem {
    let path = if field.is_empty() {
        format!("providers.{name}")
    } else {
        format!("providers.{name}.{field}")
    };
    ConfigProblem {
        path,
        reason: reason.into(),
    }
}

/// Provider-Name als Teil der Harness-ID: `[a-z0-9_-]`, 1–64 Zeichen.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn valid_env_name(var: &str) -> bool {
    let mut bytes = var.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Loopback-Ziel (127.0.0.0/8, `::1`, `localhost`)?
pub fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => bare.eq_ignore_ascii_case("localhost"),
    }
}

/// Prüft einen Eintrag `providers.<name>`.
pub fn parse_provider(name: &str, value: &Value) -> Result<Provider, ConfigProblem> {
    if !valid_name(name) {
        return Err(problem(
            name,
            "",
            "Name muss aus [a-z0-9_-] bestehen (höchstens 64 Zeichen); er wird Teil der Harness-ID direct:<name>",
        ));
    }
    let Some(map) = value.as_object() else {
        return Err(problem(name, "", "Eintrag muss ein Objekt sein"));
    };
    // Felder, deren Fehlen bzw. Form wir selbst mit Pfad melden.
    match map.get("base_url") {
        None | Some(Value::Null) => return Err(problem(name, "base_url", "fehlt")),
        Some(Value::String(s)) if s.trim().is_empty() => {
            return Err(problem(name, "base_url", "ist leer"));
        }
        _ => {}
    }
    if map.get("kind").is_none_or(Value::is_null) {
        return Err(problem(name, "kind", "fehlt (anthropic oder openai)"));
    }
    // Ein Klartext-Key darf in keiner Meldung erscheinen: `api_key` vor serde prüfen.
    let key = match (map.get("api_key"), map.get("api_key_env")) {
        (Some(_), Some(_)) => {
            return Err(problem(
                name,
                "api_key",
                "api_key und api_key_env schließen sich aus",
            ));
        }
        (Some(Value::String(v)), None) if v == "none" => KeyRef::None,
        (Some(Value::String(v)), None) if v.starts_with("secret://") => {
            return Err(problem(
                name,
                "api_key",
                "secret://-Referenzen gibt es ab M2 (SEC-001); in M1 api_key_env: <VARIABLE> verwenden",
            ));
        }
        (Some(_), None) => {
            return Err(problem(
                name,
                "api_key",
                "Klartext-Keys gehören nicht in Konfigurationsdateien; api_key_env: <VARIABLE> oder api_key: none verwenden",
            ));
        }
        (None, Some(Value::String(var))) if valid_env_name(var) && !var.starts_with("BETON_") => {
            KeyRef::Env(var.clone())
        }
        (None, Some(_)) => {
            return Err(problem(
                name,
                "api_key_env",
                "muss der Name einer Umgebungsvariable sein ([A-Z_][A-Z0-9_]*, nicht BETON_*)",
            ));
        }
        (None, None) => {
            return Err(problem(
                name,
                "api_key_env",
                "fehlt (oder api_key: none für Endpunkte ohne Key)",
            ));
        }
    };
    let mut sanitized = map.clone();
    sanitized.remove("api_key");
    let cfg: ProviderConfig = {
        let mut probe = sanitized;
        probe.insert("api_key".into(), Value::Null);
        let de = Value::Object(probe);
        serde_path_to_error::deserialize(de).map_err(|e| {
            let field = e.path().to_string();
            let field = if field == "." { String::new() } else { field };
            problem(name, &field, e.inner().to_string())
        })?
    };
    let base_url = Url::parse(cfg.base_url.trim())
        .map_err(|e| problem(name, "base_url", format!("keine gültige URL: {e}")))?;
    match base_url.scheme() {
        "https" => {}
        "http" if key == KeyRef::None || is_loopback(&base_url) => {}
        "http" => {
            return Err(problem(
                name,
                "base_url",
                "mit API-Key nur https (Klartext-HTTP nur zu Loopback)",
            ));
        }
        other => {
            return Err(problem(
                name,
                "base_url",
                format!("Schema {other} nicht erlaubt (http oder https)"),
            ));
        }
    }
    if !base_url.username().is_empty() || base_url.password().is_some() {
        return Err(problem(
            name,
            "base_url",
            "Zugangsdaten in der URL sind nicht erlaubt",
        ));
    }
    for (i, m) in cfg.models.iter().enumerate() {
        if m.id.trim().is_empty() {
            return Err(problem(name, &format!("models[{i}].id"), "ist leer"));
        }
        if m.context_window == Some(0) {
            return Err(problem(
                name,
                &format!("models[{i}].context_window"),
                "muss größer als 0 sein",
            ));
        }
    }
    Ok(Provider {
        name: name.to_owned(),
        wire: cfg.kind,
        base_url,
        key,
        prompt_caching: cfg.prompt_caching.unwrap_or(true),
        models: cfg.models,
    })
}

/// Prüft alle Provider; ungültige werden mit Pfad und Grund gemeldet und übersprungen.
pub fn parse_providers(section: &BTreeMap<String, Value>) -> (Vec<Provider>, Vec<ConfigProblem>) {
    let mut ok = Vec::new();
    let mut problems = Vec::new();
    for (name, value) in section {
        match parse_provider(name, value) {
            Ok(p) => ok.push(p),
            Err(e) => problems.push(e),
        }
    }
    (ok, problems)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn har_011_spec_example_parses() {
        let yaml = r"
anthropic:
  kind: anthropic
  base_url: https://api.anthropic.com
  api_key_env: ANTHROPIC_API_KEY
  prompt_caching: true
openrouter:
  kind: openai
  base_url: https://openrouter.ai/api/v1
  api_key_env: OPENROUTER_API_KEY
  models:
    - id: qwen/qwen3-coder
      context_window: 262144
      pricing: { input_per_mtok: 0.4, output_per_mtok: 1.6 }
ollama:
  kind: openai
  base_url: http://127.0.0.1:11434/v1
  api_key: none
";
        let section: BTreeMap<String, Value> = serde_yaml_ng::from_str(yaml).unwrap();
        let (ok, problems) = parse_providers(&section);
        assert!(problems.is_empty(), "{problems:?}");
        let or = ok.iter().find(|p| p.name == "openrouter").unwrap();
        assert_eq!(or.wire, WireKind::Openai);
        assert_eq!(or.key, KeyRef::Env("OPENROUTER_API_KEY".into()));
        assert_eq!(or.harness_id(), "direct:openrouter");
        assert_eq!(
            or.endpoint(or.messages_path()).as_str(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            or.model("qwen/qwen3-coder").unwrap().context_window,
            Some(262_144)
        );
        let an = ok.iter().find(|p| p.name == "anthropic").unwrap();
        assert_eq!(
            an.endpoint(an.messages_path()).as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        let ol = ok.iter().find(|p| p.name == "ollama").unwrap();
        assert_eq!(ol.key, KeyRef::None);
    }

    fn err(name: &str, v: Value) -> ConfigProblem {
        parse_provider(name, &v).unwrap_err()
    }

    #[test]
    fn har_011_ac4_invalid_provider_config_names_path_and_reason() {
        let e = err(
            "x",
            json!({"kind": "gemini", "base_url": "https://a.example", "api_key": "none"}),
        );
        assert_eq!(e.path, "providers.x.kind");
        assert!(e.reason.contains("gemini"), "{e}");
        let e = err("x", json!({"kind": "openai", "api_key": "none"}));
        assert_eq!(e.path, "providers.x.base_url");
        assert_eq!(e.reason, "fehlt");
        let e = err(
            "x",
            json!({"base_url": "https://a.example", "api_key": "none"}),
        );
        assert_eq!(e.path, "providers.x.kind");
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example", "api_key": "none", "foo": 1}),
        );
        assert!(e.reason.contains("foo"), "{e}");
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example", "api_key": "none", "models": [{"id": "m", "context_window": 0}]}),
        );
        assert_eq!(e.path, "providers.x.models[0].context_window");
        let e = err("Groß", json!({}));
        assert_eq!(e.path, "providers.Groß");
        // Ein ungültiger Provider beeinträchtigt die anderen nicht.
        let section: BTreeMap<String, Value> = [
            (
                "gut".to_owned(),
                json!({"kind": "openai", "base_url": "http://127.0.0.1:1/v1", "api_key": "none"}),
            ),
            ("kaputt".to_owned(), json!({"kind": "foo"})),
        ]
        .into();
        let (ok, problems) = parse_providers(&section);
        assert_eq!(ok.len(), 1);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().starts_with("providers.kaputt."));
    }

    #[test]
    fn har_011_plaintext_keys_are_rejected_without_echoing_the_value() {
        let secret = "sk-or-v1-0123456789abcdef0123456789abcdef";
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example", "api_key": secret}),
        );
        assert_eq!(e.path, "providers.x.api_key");
        assert!(!e.to_string().contains(secret), "{e}");
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example", "api_key": "secret://openrouter/default"}),
        );
        assert!(e.reason.contains("M2"), "{e}");
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example", "api_key_env": "BETON_TOKEN"}),
        );
        assert_eq!(e.path, "providers.x.api_key_env");
        let e = err(
            "x",
            json!({"kind": "openai", "base_url": "https://a.example"}),
        );
        assert_eq!(e.path, "providers.x.api_key_env");
    }

    #[test]
    fn har_011_keys_only_over_https_or_loopback() {
        let ok = |url: &str| {
            parse_provider(
                "x",
                &json!({"kind": "openai", "base_url": url, "api_key_env": "OPENAI_API_KEY"}),
            )
        };
        assert!(ok("https://api.openai.com/v1").is_ok());
        assert!(ok("http://127.0.0.1:8080/v1").is_ok());
        assert!(ok("http://localhost:8080/v1").is_ok());
        assert!(ok("http://[::1]:8080/v1").is_ok());
        assert_eq!(
            ok("http://10.0.0.5:8000/v1").unwrap_err().path,
            "providers.x.base_url"
        );
        assert!(ok("ftp://a.example").is_err());
        assert!(ok("https://user:pw@a.example").is_err());
        // Ohne Key darf ein LAN-Gateway per HTTP laufen.
        assert!(
            parse_provider(
                "x",
                &json!({"kind": "openai", "base_url": "http://10.0.0.5:8000/v1", "api_key": "none"})
            )
            .is_ok()
        );
    }
}
