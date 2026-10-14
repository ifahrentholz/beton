//! Deklarative Sandbox-Konfiguration (SBX-003): `sandbox:` in Agent-YAML, Projekt- und
//! User-Konfiguration.
//!
//! Laden prüft Form und Inhalt (fail closed): unbekannte Felder und Backends, relative oder
//! nicht expandierbare Pfade (`$HOME/x`), `..`, verbotene Grants (`/`, `~` und dessen
//! Vorfahren als `write_paths`, `~/.beton`, `~/.ssh`, Docker-Socket, `/proc`, `/sys`) und
//! gesperrte Environment-Variablen sind Fehler mit Feldangabe, nie still ignoriert.
//!
//! Über die Ebenen (Agent > Projekt > User > Default) gilt: Skalare und Grants
//! (`read_paths`, `write_paths`, `allow_hidden`, `egress_rules`, `egress_allow_private`,
//! `env.passthrough`) kommen von der spezifischsten Ebene, die sie setzt; `masks` werden
//! vereinigt, `env.set` schlüsselweise überschrieben.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::SandboxError;
use crate::provider::ProviderId;

/// Wahl des Backends: `auto` oder eine Provider-ID.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackendChoice {
    /// macOS → `seatbelt`, Linux → `landlock` (bzw. `bwrap`, SBX-009), Windows → `windows`.
    #[default]
    Auto,
    Provider(ProviderId),
}

impl BackendChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendChoice::Auto => "auto",
            BackendChoice::Provider(p) => p.as_str(),
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        if s == "auto" {
            return Ok(BackendChoice::Auto);
        }
        ProviderId::parse(s).map(BackendChoice::Provider).ok_or_else(|| {
            let known: Vec<&str> = std::iter::once("auto")
                .chain(ProviderId::ALL.iter().map(|p| p.as_str()))
                .collect();
            format!("unbekanntes Sandbox-Backend `{s}`; erlaubt: {}", known.join(", "))
        })
    }
}

impl Serialize for BackendChoice {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BackendChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        BackendChoice::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for BackendChoice {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SandboxBackend".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let names: Vec<&str> = std::iter::once("auto")
            .chain(ProviderId::ALL.iter().map(|p| p.as_str()))
            .collect();
        schemars::json_schema!({
            "type": "string",
            "enum": names,
            "description": "Isolations-Backend; `auto` wählt je Betriebssystem (SBX-001)."
        })
    }
}

/// Zugriff auf den Workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    #[default]
    Rw,
    Ro,
}

/// `sandbox.env`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxEnvConfig {
    /// Namen oder Präfixe mit `*` (z. B. `LC_*`), die aus dem Daemon-Environment übernommen
    /// werden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough: Option<Vec<String>>,
    /// Feste Werte.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,
}

/// Eine Ebene der Sandbox-Konfiguration (`sandbox:`). Nicht gesetzte Felder erbt sie von der
/// weniger spezifischen Ebene.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Sandbox-Konfiguration (SBX-003)")]
pub struct SandboxSpec {
    /// `auto` (Default), `seatbelt`, `landlock`, `bwrap`, `windows`, `docker`, `podman` oder
    /// `none` (keine Isolation, nur ausdrücklich).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendChoice>,
    /// `rw` (Default) oder `ro`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceAccess>,
    /// Zusätzliche Lese-Wurzeln: absolut oder mit `~` (keine `$VAR`-Expansion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_paths: Option<Vec<String>>,
    /// Zusätzliche Schreib-Wurzeln (absolut oder mit `~`); `/`, `~` und dessen Vorfahren
    /// sind verboten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_paths: Option<Vec<String>>,
    /// Zusätzliche Masken: absolute Pfade (`~/…`, `/…`) oder Namensmuster mit `*`/`?` (ein
    /// Pfadsegment, z. B. `.env.*` oder `secrets/`), die in jeder Wurzel in jeder Tiefe
    /// gelten. Ohne Angabe gilt im Workspace `[.env, .env.*]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masks: Option<Vec<String>>,
    /// Dotfiles direkt in Grants, die nicht maskiert werden (z. B. `.venv`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_hidden: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<SandboxEnvConfig>,
    /// `false`: gar kein Netz; `true` (Default): nur über den Egress-Proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_network: Option<bool>,
    /// Egress-Regeln des Proxys, z. B. `"GET,HEAD registry.npmjs.org/**"` (PRX-003).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_rules: Option<Vec<String>>,
    /// Ausdrücklich erlaubte private Ziele, z. B. `127.0.0.1:5432` (PRX-007).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_allow_private: Option<Vec<String>>,
}

/// Environment-Variablen, die nie in eine Sandbox gelangen, auch nicht per `passthrough`
/// oder `set` (SBX-005). Einträge mit `*` am Ende sind Präfixe.
pub const DENIED_ENV: [&str; 8] = [
    "SSH_AUTH_SOCK",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "BETON_TOKEN",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_*",
    "BETON_RUNNER_TOKEN",
];

/// Ist `name` (bzw. das Muster `name`, z. B. `DYLD_*` oder `*`) gesperrt?
pub fn env_denied(name: &str) -> bool {
    let (base, is_prefix) = match name.strip_suffix('*') {
        Some(b) => (b, true),
        None => (name, false),
    };
    DENIED_ENV.iter().any(|d| match d.strip_suffix('*') {
        // Gesperrtes Präfix: trifft jeden Namen und jedes Muster, das damit beginnt.
        Some(dp) => base.starts_with(dp) || (is_prefix && dp.starts_with(base)),
        // Gesperrter Name: trifft den Namen selbst und jedes Muster, das ihn abdeckt.
        None => base == *d || (is_prefix && d.starts_with(base)),
    })
}

impl SandboxSpec {
    /// Liest eine Ebene aus YAML (der Inhalt unter `sandbox:`) und prüft sie.
    pub fn from_yaml(text: &str) -> Result<Self, SandboxError> {
        let value: serde_json::Value = serde_norway::from_str(text)
            .map_err(|e| SandboxError::invalid("sandbox", format!("kein gültiges YAML: {e}")))?;
        Self::from_value(value)
    }

    /// Liest eine Ebene aus einem JSON-Wert und prüft sie.
    pub fn from_value(value: serde_json::Value) -> Result<Self, SandboxError> {
        let spec: Self = serde_json::from_value(value)
            .map_err(|e| SandboxError::invalid("sandbox", e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Prüft eine Ebene ohne Dateisystemzugriff (lexikalisch).
    pub fn validate(&self) -> Result<(), SandboxError> {
        crate::paths::validate_spec(self)
    }

    /// Effektive Konfiguration aus den Ebenen `user` < `project` < `agent`.
    pub fn merged(layers: &[&SandboxSpec]) -> SandboxSpec {
        todo!("SBX-003")
    }
}
