//! Capabilities-Modell (HAR-002). UI, Policy-Engine und Fork-Logik richten sich danach;
//! Aktionen ohne Capability werden mit `capability_unsupported` abgelehnt.

use beton_core::event::AuthSource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::adapter::{Mode, PermissionMode, Transport};

macro_rules! cap_enum {
    ($(#[$m:meta])* $name:ident { $first:ident $(, $rest:ident)* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            $first,
            $($rest,)*
        }
    };
}

cap_enum!(
    /// Wie der Harness Freigaben einholt.
    ApprovalMechanism { None, NativeRequest, Hook, AcpPermission, ScreenMirror }
);
cap_enum!(
    /// Wie weit beton Tool-Calls vor der Ausführung prüfen kann.
    ToolCallGate { ObserveOnly, ApprovalOnly, Full }
);
cap_enum!(
    /// Wo die Phase `model_request` geprüft wird (POL-003, POL-024): `turn` – der Runner vor
    /// jedem Turn bzw. jeder Eingabe; `call` – der Adapter vor jedem Model-Request
    /// (Direkt-API); `observe_only` – gar nicht durchsetzbar.
    ModelRequestGate { ObserveOnly, Turn, Call }
);
cap_enum!(SwitchSupport {
    None,
    Restart,
    Live
});
cap_enum!(ResumeSupport { None, Cold, Warm });
cap_enum!(ForkHistory {
    None,
    Preamble,
    Rebuild
});
cap_enum!(Subagents { None, Native });
cap_enum!(UsageReporting {
    None,
    Tokens,
    TokensAndCost
});
cap_enum!(CompactionSupport { None, Native });
cap_enum!(
    /// Wie Tool-Ausführungen in Stufe 2 landen (SBX-002): `inherited` – gar nicht, Tools laufen
    /// im Prozess des Harness (Stufe 1, *degraded*, nur mit `require_tool_isolation: false`);
    /// `shell_override` – über `bt-exec` als Shell des Harness; `beton` – beton führt die Tools
    /// selbst aus (Direkt-API, ACP mit `terminal/*`).
    ToolIsolation { Inherited, ShellOverride, Beton }
);
cap_enum!(InstructionsDelivery {
    FirstMessagePrefix,
    AppendSystemPrompt,
    SystemPrompt,
    DeveloperInstructions
});

/// Fähigkeiten eines Harness in einem Modus; ggf. nach `probe()` versionsabhängig verfeinert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub mode: Mode,
    pub transport: Transport,
    /// Getestete CLI-Versionen als SemVer-Bereich, z. B. `>=2.0.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version_range: Option<String>,
    pub auth_sources: Vec<AuthSource>,
    pub approval: ApprovalMechanism,
    pub tool_call_gate: ToolCallGate,
    /// Tool-Klassen (POL-005), für die der Vendor bei `tool_call_gate: approval_only` um
    /// Freigabe fragt; die übrigen sind nur beobachtbar (POL-024).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub approval_kinds: Vec<String>,
    /// Phase `model_request` (POL-024).
    #[serde(default)]
    pub model_request_gate: ModelRequestGate,
    /// Phase `tool_result`: `full` heißt, das Ergebnis lässt sich vor dem Modell ersetzen.
    #[serde(default)]
    pub tool_result_gate: ToolCallGate,
    pub model_switch: SwitchSupport,
    pub effort_switch: SwitchSupport,
    pub resume: ResumeSupport,
    pub fork_history: ForkHistory,
    pub interrupt: bool,
    pub steering: bool,
    pub subagents: Subagents,
    pub usage_reporting: UsageReporting,
    pub compaction: CompactionSupport,
    pub instructions_delivery: InstructionsDelivery,
    pub mcp_injection: bool,
    pub images: bool,
    pub transcript_import: bool,
    /// Wählbare Modelle (Aliase der CLI), z. B. für den Composer-Picker (WEB-004).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub models: Vec<String>,
    /// `models` ist die zuletzt erfolgreich abgerufene Liste, weil die Modell-Discovery des
    /// Anbieters gerade fehlschlägt (HAR-011 AC3); der Picker kennzeichnet sie als veraltet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub models_stale: bool,
    /// Wählbare Effort-Stufen; leer, wenn der Harness keine kennt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub efforts: Vec<String>,
    /// Permission-Modes, die der Harness abbilden kann (HAR-027); andere lehnt beton mit
    /// `capability_unsupported` ab. `yolo` braucht zusätzlich Sandbox und Egress-Proxy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<PermissionMode>>", optional)]
    pub permission_modes: Vec<PermissionMode>,
    /// Kontextfenster des Standardmodells in Tokens; bestimmt das Budget des
    /// Handover-Kontexts beim Fork (HAR-018). Ohne Angabe gilt [`DEFAULT_CONTEXT_WINDOW`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context_window: Option<u64>,
    /// Wie Tools in Stufe 2 landen (SBX-002).
    #[serde(default)]
    pub tool_isolation: ToolIsolation,
    /// Projektdateien, die der Harness selbst liest (z. B. Claude `CLAUDE.md`, Codex
    /// `AGENTS.md`); bei `project_files: auto` liefert beton nur die übrigen (AGT-005).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub native_project_files: Vec<String>,
}

/// Kontextfenster, wenn ein Harness keines meldet (HAR-018).
pub const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

impl Capabilities {
    /// Nichts unterstützt; Basis für Adapter, die nur einzelne Fähigkeiten setzen.
    pub fn minimal(mode: Mode, transport: Transport) -> Self {
        Self {
            mode,
            transport,
            version_range: None,
            auth_sources: vec![AuthSource::None],
            approval: ApprovalMechanism::None,
            tool_call_gate: ToolCallGate::ObserveOnly,
            approval_kinds: Vec::new(),
            model_request_gate: ModelRequestGate::ObserveOnly,
            tool_result_gate: ToolCallGate::ObserveOnly,
            model_switch: SwitchSupport::None,
            effort_switch: SwitchSupport::None,
            resume: ResumeSupport::None,
            fork_history: ForkHistory::None,
            interrupt: false,
            steering: false,
            subagents: Subagents::None,
            usage_reporting: UsageReporting::None,
            compaction: CompactionSupport::None,
            instructions_delivery: InstructionsDelivery::FirstMessagePrefix,
            mcp_injection: false,
            images: false,
            transcript_import: false,
            models: Vec::new(),
            models_stale: false,
            efforts: Vec::new(),
            permission_modes: Vec::new(),
            context_window: None,
            tool_isolation: ToolIsolation::Inherited,
            native_project_files: Vec::new(),
        }
    }

    /// Kontextfenster in Tokens; ohne Angabe [`DEFAULT_CONTEXT_WINDOW`] (HAR-018).
    pub fn context_window(&self) -> u64 {
        self.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW)
    }

    /// Enforcement-Matrix (POL-024 AC2), abgeleitet aus diesen Capabilities:
    ///
    /// - `session_start`: immer `full` (der Runner entscheidet vor dem Start).
    /// - `model_request`: `turn`/`call` → `full`, sonst `observe_only`.
    /// - `tool_call`: `tool_call_gate: full` → alle Klassen `full`; `approval_only` → die
    ///   Klassen aus `approval_kinds` `approval_only`, der Rest `observe_only`. System- und
    ///   Browser-Tools laufen bei MCP-Injektion über den Relay-Hub des Runners und sind dort
    ///   immer voll durchsetzbar (HAR-009).
    /// - `tool_result`: `tool_result_gate`.
    /// - Modifikationen: `model`/`reasoning_effort` je Turn nur mit Live-Wechsel, beim Start
    ///   nur, wenn der Harness sie dort setzen kann ([`Capabilities::start_model`],
    ///   [`Capabilities::start_effort`], #283); `args` nur mit vollem Gate; `redact` gibt es
    ///   noch nicht (Redaction-Pipeline, WP-38).
    pub fn enforcement(&self) -> beton_policy::enforcement::Matrix {
        use beton_policy::Phase;
        use beton_policy::enforcement::{KINDS, Level, Matrix};
        use beton_policy::engine::Capabilities as Modify;
        let level = |g: ToolCallGate| match g {
            ToolCallGate::Full => Level::Full,
            ToolCallGate::ApprovalOnly => Level::ApprovalOnly,
            ToolCallGate::ObserveOnly => Level::ObserveOnly,
        };
        let mut tool_call = std::collections::BTreeMap::new();
        let mut tool_result = std::collections::BTreeMap::new();
        for kind in KINDS {
            let call = match self.tool_call_gate {
                ToolCallGate::ApprovalOnly if !self.approval_kinds.iter().any(|k| k == kind) => {
                    Level::ObserveOnly
                }
                other => level(other),
            };
            let via_hub = self.mcp_injection && matches!(kind, "system" | "browser");
            tool_call.insert(kind.to_owned(), if via_hub { Level::Full } else { call });
            tool_result.insert(kind.to_owned(), level(self.tool_result_gate));
        }
        let model_request = match self.model_request_gate {
            ModelRequestGate::Turn | ModelRequestGate::Call => Level::Full,
            ModelRequestGate::ObserveOnly => Level::ObserveOnly,
        };
        let mut modify = std::collections::BTreeMap::new();
        modify.insert(
            Phase::SessionStart,
            Modify {
                model: self.start_model(),
                reasoning_effort: self.start_effort(),
                redact: false,
                args: false,
                ..Modify::default()
            },
        );
        modify.insert(
            Phase::ModelRequest,
            Modify {
                model: model_request.enforces() && self.model_switch == SwitchSupport::Live,
                reasoning_effort: model_request.enforces()
                    && self.effort_switch == SwitchSupport::Live,
                ..Modify::none()
            },
        );
        modify.insert(
            Phase::ToolCall,
            Modify {
                args: self.tool_call_gate == ToolCallGate::Full,
                ..Modify::none()
            },
        );
        modify.insert(Phase::ToolResult, Modify::none());
        Matrix {
            session_start: Level::Full,
            model_request,
            tool_call,
            tool_result,
            // Browser-Phasen prüfen die beton-Browser-Tools selbst (ab M3).
            browser: Level::Full,
            modify,
        }
    }

    /// Lässt sich das Modell beim Start setzen (#283)? Native und In-Process-Harnesses haben
    /// einen Startparameter; ACP kennt keinen und setzt es nur per `session/set_model`
    /// (`model_switch: live`, HAR-017).
    pub fn start_model(&self) -> bool {
        self.transport != Transport::Acp || self.model_switch == SwitchSupport::Live
    }

    /// Lässt sich der Effort beim Start setzen (#283)? Nur mit abbildbaren Stufen, bei ACP
    /// zusätzlich nur mit Live-Wechsel.
    pub fn start_effort(&self) -> bool {
        !self.efforts.is_empty()
            && (self.transport != Transport::Acp || self.effort_switch == SwitchSupport::Live)
    }

    /// Prüft, ob eine Aktion möglich ist (HAR-002 AC3).
    pub fn check(&self, action: Action) -> Result<(), CapabilityUnsupported> {
        let supported = match action {
            Action::ModelSwitch => self.model_switch != SwitchSupport::None,
            Action::EffortSwitch => self.effort_switch != SwitchSupport::None,
            Action::Steer => self.steering,
            Action::Interrupt => self.interrupt,
            Action::Resume => self.resume != ResumeSupport::None,
            Action::Fork => self.fork_history != ForkHistory::None,
            Action::Compact => self.compaction != CompactionSupport::None,
            Action::Images => self.images,
            Action::TranscriptImport => self.transcript_import,
            Action::PermissionMode => !self.permission_modes.is_empty(),
        };
        if supported {
            Ok(())
        } else {
            Err(CapabilityUnsupported(action))
        }
    }
}

/// Aktionen, die eine Capability voraussetzen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ModelSwitch,
    EffortSwitch,
    Steer,
    Interrupt,
    Resume,
    Fork,
    Compact,
    Images,
    TranscriptImport,
    PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("capability_unsupported: {0:?} wird von diesem Harness nicht unterstützt")]
pub struct CapabilityUnsupported(pub Action);

impl CapabilityUnsupported {
    pub fn code(&self) -> &'static str {
        "capability_unsupported"
    }
}

/// Reihenfolge der Effort-Stufen (HAR-017).
pub const EFFORT_LEVELS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// Effort-Mapping (HAR-017): Eine nicht unterstützte Stufe wird auf die nächstniedrigere
/// unterstützte gemappt; gibt es keine niedrigere, auf die niedrigste unterstützte. Kennt der
/// Harness keine Stufen, ist das Ergebnis `None` (der Wert wird nicht angewendet).
pub fn map_effort(requested: &str, supported: &[String]) -> Option<String> {
    if supported.iter().any(|s| s == requested) {
        return Some(requested.to_owned());
    }
    let rank = |e: &str| EFFORT_LEVELS.iter().position(|l| *l == e);
    let wanted = rank(requested)?;
    let mut known: Vec<(usize, &String)> = supported
        .iter()
        .filter_map(|s| rank(s).map(|r| (r, s)))
        .collect();
    known.sort();
    known
        .iter()
        .rev()
        .find(|(r, _)| *r <= wanted)
        .or_else(|| known.first())
        .map(|(_, s)| (*s).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_mapping_picks_the_next_lower_supported_level() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        let lmh = s(&["low", "medium", "high"]);
        assert_eq!(map_effort("xhigh", &lmh).as_deref(), Some("high"));
        assert_eq!(map_effort("medium", &lmh).as_deref(), Some("medium"));
        assert_eq!(map_effort("low", &s(&["high"])).as_deref(), Some("high"));
        assert_eq!(map_effort("high", &[]), None);
    }

    #[test]
    fn har_002_ac3_unsupported_action_is_rejected() {
        let mut caps = Capabilities::minimal(Mode::Native, Transport::Native);
        let err = caps.check(Action::ModelSwitch).unwrap_err();
        assert_eq!(err.code(), "capability_unsupported");
        caps.model_switch = SwitchSupport::Live;
        assert!(caps.check(Action::ModelSwitch).is_ok());
    }

    #[test]
    fn har_002_spec_example_parses() {
        let yaml = r#"
mode: native
transport: native
version_range: ">=2.0.0"
auth_sources: [vendor_cli, api_key]
approval: native_request
tool_call_gate: full
model_switch: live
effort_switch: live
resume: warm
fork_history: rebuild
interrupt: true
steering: false
subagents: native
usage_reporting: tokens_and_cost
compaction: native
instructions_delivery: append_system_prompt
mcp_injection: true
images: true
transcript_import: true
"#;
        let caps: Capabilities = serde_norway::from_str(yaml).unwrap();
        assert_eq!(caps.approval, ApprovalMechanism::NativeRequest);
        assert_eq!(caps.usage_reporting, UsageReporting::TokensAndCost);
    }
}
