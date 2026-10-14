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
//! vereinigt, `env.set` und `limits` feldweise überschrieben.

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
        ProviderId::parse(s)
            .map(BackendChoice::Provider)
            .ok_or_else(|| {
                let known: Vec<&str> = std::iter::once("auto")
                    .chain(ProviderId::ALL.iter().map(|p| p.as_str()))
                    .collect();
                format!(
                    "unbekanntes Sandbox-Backend `{s}`; erlaubt: {}",
                    known.join(", ")
                )
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

/// Eingebautes Preset (SBX-012): Startpunkt, den eigene Felder gemäß den Merge-Regeln
/// überschreiben bzw. ergänzen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Workspace rw, in Stufe 2 kein Netz (keine Egress-Regel); Stufe 1 erreicht nur ihre
    /// Vendor-Hosts.
    #[default]
    Default,
    /// Wie `default`, zusätzlich lesend gängige Paket-Registries und Toolchain-Caches.
    Dev,
    /// Workspace read-only, kein Netz (z. B. Side-Chats, Reviewer-Agents).
    Readonly,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Default => "default",
            Preset::Dev => "dev",
            Preset::Readonly => "readonly",
        }
    }
}

/// `sandbox.harness` (Stufe 1, selten nötig).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessSandboxConfig {
    /// Zusätzliche Egress-Regeln der Stufe 1 (z. B. Remote-MCP-Server), mit MITM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_extra: Option<Vec<String>>,
    /// Zusätzliche Passthrough-Hosts der Stufe 1 (`host` oder `host:port`), ohne
    /// TLS-Terminierung (PRX-005).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_passthrough: Option<Vec<String>>,
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

/// `sandbox.limits` (SBX-013): Grenzen für den ganzen Prozessbaum einer Sandbox. Nicht gesetzte
/// Felder begrenzen nichts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Speicher des Baums in MiB (Linux: cgroup v2 `memory.max`, nur mit Delegation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    /// Höchstzahl gleichzeitiger Prozesse und Threads (cgroup `pids.max` bzw. `RLIMIT_NPROC`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids: Option<u32>,
    /// CPU-Zeit je Prozess in Sekunden (`RLIMIT_CPU`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_seconds: Option<u64>,
    /// Größe einzelner Dateien in MiB (`RLIMIT_FSIZE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size_mb: Option<u64>,
}

impl Limits {
    /// Kleinstes sinnvolles `pids`-Limit (Shell, Launcher und ein Werkzeug brauchen mehr).
    pub const MIN_PIDS: u32 = 16;
    /// Kleinstes `memory_mb`.
    pub const MIN_MEMORY_MB: u64 = 16;

    /// Ist irgendein Limit gesetzt?
    pub fn any(&self) -> bool {
        self.memory_mb.is_some()
            || self.pids.is_some()
            || self.cpu_seconds.is_some()
            || self.file_size_mb.is_some()
    }

    /// Prüft die Werte (lexikalisch, beim Laden).
    pub fn validate(&self) -> Result<(), SandboxError> {
        if let Some(p) = self.pids
            && p < Self::MIN_PIDS
        {
            return Err(SandboxError::invalid(
                "limits.pids",
                format!("{p} ist zu klein (mindestens {})", Self::MIN_PIDS),
            ));
        }
        if let Some(m) = self.memory_mb
            && m < Self::MIN_MEMORY_MB
        {
            return Err(SandboxError::invalid(
                "limits.memory_mb",
                format!("{m} ist zu klein (mindestens {})", Self::MIN_MEMORY_MB),
            ));
        }
        if self.cpu_seconds == Some(0) {
            return Err(SandboxError::invalid(
                "limits.cpu_seconds",
                "0 ist kein gültiges Limit",
            ));
        }
        if self.file_size_mb == Some(0) {
            return Err(SandboxError::invalid(
                "limits.file_size_mb",
                "0 ist kein gültiges Limit",
            ));
        }
        Ok(())
    }

    /// Je Feld gewinnt `other`, wenn gesetzt.
    pub fn merged_with(self, other: &Limits) -> Limits {
        Limits {
            memory_mb: other.memory_mb.or(self.memory_mb),
            pids: other.pids.or(self.pids),
            cpu_seconds: other.cpu_seconds.or(self.cpu_seconds),
            file_size_mb: other.file_size_mb.or(self.file_size_mb),
        }
    }
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
    /// Eingebautes Preset `default`, `dev` oder `readonly` (SBX-012).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    /// Startet ein Harness nur, wenn er Tools in Stufe 2 erzwingt (Default `true`, SBX-002
    /// AC4); `false` erlaubt `tool_isolation: inherited` als *degraded*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_tool_isolation: Option<bool>,
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
    /// Passthrough in Stufe 2 (`host` oder `host:port`): nur ausdrücklich, ohne
    /// TLS-Terminierung und ohne Credential-Injection (PRX-005).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_passthrough: Option<Vec<String>>,
    /// Loopback-Ports für Dev-Server in der Sandbox (PRX-008 AC3, #210): `bind` und `connect`
    /// nur auf diesen Ports. Linux: im eigenen Net-Namespace; macOS: Loopback des Hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_ports: Option<Vec<u16>>,
    /// Ressourcenlimits des Prozessbaums (SBX-013); je Feld gilt die spezifischste Ebene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// Stufe-1-Overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessSandboxConfig>,
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

    /// Effektive Konfiguration aus den Ebenen `user` < `project` < `agent`. Setzt eine Ebene
    /// ein Preset (SBX-012), liegt das Preset der spezifischsten Ebene als unterste Ebene
    /// darunter.
    pub fn merged(layers: &[&SandboxSpec]) -> SandboxSpec {
        Self::merged_with_origins(&layers.iter().map(|l| ("config", *l)).collect::<Vec<_>>()).0
    }

    /// Wie [`SandboxSpec::merged`] mit Herkunft je Feld (`explain`, SBX-012 AC3): Schlüssel
    /// ist der Feldname (bei vereinigten Listen `masks[<eintrag>]`, bei `env.set` und
    /// `limits` der Pfad des Eintrags), Wert der Name der Ebene bzw. `preset:<name>`.
    pub fn merged_with_origins(
        layers: &[(&str, &SandboxSpec)],
    ) -> (SandboxSpec, BTreeMap<String, String>) {
        let mut all: Vec<(String, SandboxSpec)> = Vec::new();
        if let Some(p) = layers.iter().rev().find_map(|(_, l)| l.preset) {
            all.push((format!("preset:{}", p.as_str()), crate::preset::layer(p)));
        }
        for (name, layer) in layers {
            all.push(((*name).to_owned(), (*layer).clone()));
        }
        let mut out = SandboxSpec::default();
        let mut origins = BTreeMap::new();
        for (name, layer) in &all {
            let mut set = |field: &str| {
                origins.insert(field.to_owned(), name.clone());
            };
            macro_rules! scalar {
                ($($f:ident),*) => {$(
                    if layer.$f.is_some() {
                        out.$f = layer.$f.clone();
                        set(stringify!($f));
                    }
                )*};
            }
            scalar!(
                backend,
                preset,
                require_tool_isolation,
                workspace,
                allow_network,
                read_paths,
                write_paths,
                allow_hidden,
                egress_rules,
                egress_allow_private,
                egress_passthrough,
                dev_ports
            );
            if let Some(l) = &layer.limits {
                for (field, present) in [
                    ("limits.memory_mb", l.memory_mb.is_some()),
                    ("limits.pids", l.pids.is_some()),
                    ("limits.cpu_seconds", l.cpu_seconds.is_some()),
                    ("limits.file_size_mb", l.file_size_mb.is_some()),
                ] {
                    if present {
                        set(field);
                    }
                }
                out.limits = Some(out.limits.unwrap_or_default().merged_with(l));
            }
            if let Some(masks) = &layer.masks {
                let list = out.masks.get_or_insert_with(Vec::new);
                for m in masks {
                    if !list.contains(m) {
                        list.push(m.clone());
                        set(&format!("masks[{m}]"));
                    }
                }
            }
            if let Some(env) = &layer.env {
                let target = out.env.get_or_insert_with(SandboxEnvConfig::default);
                if env.passthrough.is_some() {
                    target.passthrough.clone_from(&env.passthrough);
                    set("env.passthrough");
                }
                for (k, v) in &env.set {
                    target.set.insert(k.clone(), v.clone());
                    set(&format!("env.set.{k}"));
                }
            }
            if let Some(h) = &layer.harness {
                let target = out
                    .harness
                    .get_or_insert_with(HarnessSandboxConfig::default);
                if h.egress_extra.is_some() {
                    target.egress_extra.clone_from(&h.egress_extra);
                    set("harness.egress_extra");
                }
                if h.egress_passthrough.is_some() {
                    target.egress_passthrough.clone_from(&h.egress_passthrough);
                    set("harness.egress_passthrough");
                }
            }
        }
        (out, origins)
    }
}
