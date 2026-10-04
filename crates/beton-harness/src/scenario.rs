//! Szenario-Format des Fake-Harness (Owner: HAR-026). Dieselben Dateien spielt die
//! Protokoll-Fake-CLI `beton-fake-cli` über eine echte Prozessgrenze ab (QA-002).
//!
//! ```yaml
//! capabilities: { approval: native_request, model_switch: live, fork_history: preamble }
//! turns:
//!   - expect_input: "Bitte pushen"
//!     emit:
//!       - { message_delta: "Ich pushe jetzt.", chunk: 4, delay_ms: 10 }
//!       - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
//!       - { on_gate: { allow: [{ tool_result: "ok" }], deny: [{ message: "Push abgelehnt." }] } }
//!       - { usage: { input_tokens: 1200, output_tokens: 80, cost_usd: 0.01 } }
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Ein abspielbares Szenario.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Überschreibt einzelne Capabilities des Fake-Harness (HAR-026 AC2).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub capabilities: Map<String, Value>,
    /// Verhalten beim Start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<StartBehavior>,
    /// Ausgabe von `--version` der Fake-CLI (Default: Version dieses Crates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Fehlerinjektion der Fake-CLI auf Protokollebene (QA-002 AC3).
    #[serde(default, skip_serializing_if = "Faults::is_empty")]
    pub faults: Faults,
    #[serde(default)]
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartBehavior {
    /// Start verweigern, z. B. weil eine Policy nicht durchsetzbar ist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuse: Option<String>,
}

/// Fehlerinjektion, gezählt in stdout-Zeilen der Fake-CLI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Faults {
    /// Nach so vielen Zeilen mit Exit-Code 1 beenden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_after: Option<u32>,
    /// Nach so vielen Zeilen nichts mehr ausgeben und nicht mehr reagieren.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hang_after: Option<u32>,
    /// Diese Zeile (1-basiert) als ungültiges JSON ausgeben.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub malformed_line: Option<u32>,
    /// JSON-RPC-Protokolle (`app-server`, `acp`): Den Handshake (`initialize`) mit einer
    /// Antwort beantworten, die nicht zum Schema passt (HAR-006 AC4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bad_handshake: bool,
}

impl Faults {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// Erwartete Eingabe; abweichende Eingaben werden abgelehnt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_input: Option<String>,
    #[serde(default)]
    pub emit: Vec<Step>,
}

/// Ein Schritt. Genau eine Aktion; `delay_ms` verzögert jeden Schritt, `chunk` und
/// `chunk_delay_ms` gelten für `message_delta`, `gate` für `tool_call`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    /// Text des Assistenten, in Stücken von `chunk` Zeichen gestreamt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk: Option<usize>,
    /// Pause zwischen zwei Stücken von `message_delta` (gleichmäßiges Streaming, z. B. für
    /// Performance-Tests); `delay_ms` wirkt nur einmal vor dem Schritt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_delay_ms: Option<u64>,
    /// Vollständige Nachricht des Assistenten ohne Streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallStep>,
    /// Ein Tool eines injizierten MCP-Servers aufrufen (HAR-009). Nur die Fake-CLI führt den
    /// Aufruf wirklich über MCP aus; der Fake-Harness meldet ihn als Fehler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_call: Option<McpCallStep>,
    /// Vor der Ausführung eine Entscheidung einholen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gate: bool,
    /// Verzweigung nach der letzten Gate-Entscheidung.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_gate: Option<OnGate>,
    /// Ergebnis des letzten Tool-Calls (Status `ok`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<Value>,
    /// Fehlerergebnis des letzten Tool-Calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Turn schlägt mit dieser Meldung fehl.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Harness stürzt mit diesem Exit-Code ab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash: Option<i32>,
    /// Login abgelaufen; Text ist der Hinweis an den Nutzer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_expired: Option<String>,
    /// Nichts mehr tun, bis unterbrochen wird.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hang: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallStep {
    pub name: String,
    /// Kanonische Klasse (POL-005), z. B. `shell`.
    #[serde(default = "other")]
    pub kind: String,
    #[serde(default)]
    pub args: Value,
}

/// Aufruf eines MCP-Tools: Server-Name (z. B. `beton`), Tool und Argumente.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCallStep {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

fn other() -> String {
    "other".into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnGate {
    #[serde(default)]
    pub allow: Vec<Step>,
    #[serde(default)]
    pub deny: Vec<Step>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("Szenario {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("Szenario ungültig: {0}")]
    Invalid(String),
}

impl Step {
    /// Namen der gesetzten Aktionen (für die Validierung).
    fn actions(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut add = |set: bool, name| {
            if set {
                out.push(name);
            }
        };
        add(self.message_delta.is_some(), "message_delta");
        add(self.message.is_some(), "message");
        add(self.reasoning.is_some(), "reasoning");
        add(self.tool_call.is_some(), "tool_call");
        add(self.mcp_call.is_some(), "mcp_call");
        add(self.on_gate.is_some(), "on_gate");
        add(self.tool_result.is_some(), "tool_result");
        add(self.tool_error.is_some(), "tool_error");
        add(self.usage.is_some(), "usage");
        add(self.error.is_some(), "error");
        add(self.crash.is_some(), "crash");
        add(self.auth_expired.is_some(), "auth_expired");
        add(self.hang, "hang");
        out
    }

    fn validate(&self, at: &str) -> Result<(), ScenarioError> {
        let actions = self.actions();
        if actions.len() != 1 {
            return Err(ScenarioError::Invalid(format!(
                "{at}: genau eine Aktion erwartet, gefunden: {actions:?}"
            )));
        }
        if self.chunk.is_some() && self.message_delta.is_none() {
            return Err(ScenarioError::Invalid(format!(
                "{at}: `chunk` gilt nur für message_delta"
            )));
        }
        if self.chunk == Some(0) {
            return Err(ScenarioError::Invalid(format!(
                "{at}: `chunk` muss > 0 sein"
            )));
        }
        if self.gate && self.tool_call.is_none() {
            return Err(ScenarioError::Invalid(format!(
                "{at}: `gate` gilt nur für tool_call"
            )));
        }
        if let Some(on_gate) = &self.on_gate {
            for (i, s) in on_gate.allow.iter().enumerate() {
                s.validate(&format!("{at}.on_gate.allow[{i}]"))?;
            }
            for (i, s) in on_gate.deny.iter().enumerate() {
                s.validate(&format!("{at}.on_gate.deny[{i}]"))?;
            }
        }
        Ok(())
    }
}

impl Scenario {
    pub fn from_yaml(text: &str) -> Result<Self, ScenarioError> {
        let scenario: Self =
            serde_yaml_ng::from_str(text).map_err(|e| ScenarioError::Invalid(e.to_string()))?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_yaml(&text)
    }

    pub fn validate(&self) -> Result<(), ScenarioError> {
        for (t, turn) in self.turns.iter().enumerate() {
            for (i, step) in turn.emit.iter().enumerate() {
                step.validate(&format!("turns[{t}].emit[{i}]"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const PUSH_ASK: &str = r#"
capabilities: { approval: native_request, model_switch: live, fork_history: preamble }
turns:
  - expect_input: "Bitte pushen"
    emit:
      - { message_delta: "Ich pushe jetzt.", chunk: 4, delay_ms: 10 }
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "ok" }], deny: [{ message: "Push abgelehnt." }] } }
      - { usage: { input_tokens: 1200, output_tokens: 80, cost_usd: 0.01 } }
"#;

    #[test]
    fn har_026_spec_example_parses() {
        let s = Scenario::from_yaml(PUSH_ASK).unwrap();
        assert_eq!(s.turns.len(), 1);
        assert_eq!(s.turns[0].emit.len(), 4);
        assert!(s.turns[0].emit[1].gate);
        assert_eq!(s.capabilities["model_switch"], "live");
    }

    #[test]
    fn steps_need_exactly_one_action() {
        let err = Scenario::from_yaml("turns: [{ emit: [{ message: a, error: b }] }]").unwrap_err();
        assert!(err.to_string().contains("genau eine Aktion"), "{err}");
        assert!(Scenario::from_yaml("turns: [{ emit: [{ delay_ms: 5 }] }]").is_err());
        // `chunk_delay_ms` ist keine eigene Aktion.
        assert!(
            Scenario::from_yaml(
                "turns: [{ emit: [{ message_delta: ab, chunk: 1, chunk_delay_ms: 5 }] }]"
            )
            .is_ok()
        );
        assert!(Scenario::from_yaml("turns: [{ emit: [{ chunk_delay_ms: 5 }] }]").is_err());
        assert!(Scenario::from_yaml("turns: [{ emit: [{ message: a, gate: true }] }]").is_err());
        assert!(Scenario::from_yaml("turns: [{ emit: [{ unbekannt: 1 }] }]").is_err());
    }
}
