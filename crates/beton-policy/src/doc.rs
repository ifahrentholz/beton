//! Policy-Dokumentformat v1 (POL-001). Diese Typen sind die einzige Quelle des JSON-Schemas
//! `schemas/v1/policy.schema.json` (`cargo xtask codegen`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Phase;

/// Einzige unterstützte Version des Formats.
pub const SPEC_VERSION: u64 = 1;

/// Pfad des Schemas relativ zum beton-Verzeichnis (`.beton/` bzw. `~/.beton/`).
pub const SCHEMA_PATH: &str = "schemas/v1/policy.schema.json";

/// Kopfzeile für Policy-Dateien unter `<beton-dir>/policies/`: zeigt auf die lokale
/// Schema-Datei, nie auf eine URL (ADR-0033).
pub const SCHEMA_COMMENT: &str =
    "# yaml-language-server: $schema=../schemas/v1/policy.schema.json";

/// Muster für Regel-IDs und Set-Namen: `[a-z0-9-]`, höchstens 64 Zeichen.
pub const ID_PATTERN: &str = "^[a-z0-9][a-z0-9-]{0,63}$";

/// Ein Policy-Set (eine YAML-Datei bzw. ein Inline-Block).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "beton Policy-Set (Format v1)")]
pub struct PolicySetDoc {
    /// Version des Formats. Unterstützt: `1`.
    #[schemars(extend("const" = 1))]
    pub spec_version: u64,
    /// Name des Sets, `[a-z0-9-]`, höchstens 64 Zeichen.
    #[schemars(extend("pattern" = ID_PATTERN))]
    pub name: String,
    pub description: Option<String>,
    /// `enforce` (Default) oder `dry_run` (Shadow-Betrieb, POL-027).
    #[serde(default)]
    pub mode: Mode,
    /// Ergebnis je Phase, wenn keine Regel dieser Ebene ausdrücklich erlaubt (Default `allow`).
    #[serde(default)]
    pub defaults: BTreeMap<Phase, DefaultAction>,
    #[serde(default)]
    pub rules: Vec<RuleDoc>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Enforce,
    DryRun,
}

/// Default einer Ebene für eine Phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefaultAction {
    Allow,
    Ask,
    Deny,
}

/// Aktion einer Regel (POL-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allow,
    Deny,
    Ask,
    Modify,
    Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

/// Eingebaute Regeltypen (Zucker, der zu CEL-Regeln entzuckert wird).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    SpendCap,
    DailyBudget,
    ModelRoute,
    RequireApproval,
    ToolAllowlist,
    PathGuard,
    GitGuard,
    LoopDetection,
    PiiRedaction,
    BrowserGuard,
}

impl RuleType {
    /// Phasen, in denen der Regeltyp entscheidet (03-policies.md, POL-011 ff.).
    pub fn phases(self) -> &'static [Phase] {
        use Phase::*;
        match self {
            RuleType::SpendCap | RuleType::DailyBudget | RuleType::ModelRoute => {
                &[SessionStart, ModelRequest]
            }
            RuleType::RequireApproval
            | RuleType::ToolAllowlist
            | RuleType::PathGuard
            | RuleType::GitGuard
            | RuleType::LoopDetection => &[ToolCall],
            RuleType::PiiRedaction => &[ToolResult, ModelRequest],
            RuleType::BrowserGuard => &[BrowserNavigate, BrowserAction],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RuleType::SpendCap => "spend_cap",
            RuleType::DailyBudget => "daily_budget",
            RuleType::ModelRoute => "model_route",
            RuleType::RequireApproval => "require_approval",
            RuleType::ToolAllowlist => "tool_allowlist",
            RuleType::PathGuard => "path_guard",
            RuleType::GitGuard => "git_guard",
            RuleType::LoopDetection => "loop_detection",
            RuleType::PiiRedaction => "pii_redaction",
            RuleType::BrowserGuard => "browser_guard",
        }
    }
}

/// Eine Regel: `on` + `action` (+ `when`) oder ein eingebauter Regeltyp (`type` + `params`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleDoc {
    /// Eindeutig im Set, `[a-z0-9-]`, höchstens 64 Zeichen.
    #[schemars(extend("pattern" = ID_PATTERN))]
    pub id: String,
    /// Phase, in der die Regel entscheidet (nicht bei `type`).
    pub on: Option<Phase>,
    /// Bedingung (CEL, Typ `bool`); fehlt sie, greift die Regel immer.
    pub when: Option<String>,
    pub action: Option<Action>,
    /// Eingebauter Regeltyp; schließt `on`, `when`, `action` und deren Felder aus.
    #[serde(rename = "type")]
    pub rule_type: Option<RuleType>,
    /// Parameter des Regeltyps.
    pub params: Option<BTreeMap<String, Value>>,
    /// Nur Darstellung.
    pub severity: Option<Severity>,
    /// Begründung; bei `deny` erhält das Modell diesen Text.
    pub reason: Option<String>,
    /// Einstellungen der Approval-Card (nur `ask`).
    pub approval: Option<ApprovalDoc>,
    /// Modifikation (nur `modify`); Werte sind Literale.
    pub modify: Option<ModifyDoc>,
    /// Wenn der Harness die Modifikation nicht anwenden kann (Default `deny`).
    pub on_unsupported: Option<OnUnsupported>,
    /// Benachrichtigung (nur `notify`).
    pub notify: Option<NotifyDoc>,
    /// Zustandsänderungen, wenn die Regel greift (POL-009).
    #[serde(default)]
    pub effects: Vec<EffectDoc>,
    /// Endentscheidungen, bei denen die `effects` angewendet werden (Default `[allow, deny]`).
    pub effects_on: Option<Vec<DefaultAction>>,
    /// Regel nur im Shadow-Betrieb auswerten (POL-027).
    #[serde(default)]
    pub dry_run: bool,
    /// `required` (Default für deny/ask/modify) oder `best_effort` (POL-024).
    pub enforcement: Option<Enforcement>,
    /// Betroffene Tool-Klassen, falls aus `when` nicht ableitbar (POL-024).
    pub kinds: Option<Vec<String>>,
    /// Fingerprint für gemerkte Freigaben (CEL, Typ `string`, POL-010).
    pub remember_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDoc {
    /// Dauer wie `30m`, `2h`.
    pub timeout: Option<String>,
    pub on_timeout: Option<TimeoutAction>,
    pub remember: Option<Remember>,
    /// `false` verhindert `allow` bei Ablauf auf niedrigeren Ebenen.
    pub allow_on_timeout: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    Allow,
    Deny,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Remember {
    None,
    Session,
}

/// Modifizierbare Felder (je Phase erlaubt laut POL-003).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModifyDoc {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub permission_mode: Option<String>,
    /// JSON-Merge-Patch auf die Tool-Argumente.
    pub args: Option<Value>,
    /// Regex-Muster, deren Treffer geschwärzt werden.
    pub redact: Option<Vec<String>>,
}

impl ModifyDoc {
    /// Gesetzte Felder in fester Reihenfolge.
    pub fn fields(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.model.is_some() {
            out.push("model");
        }
        if self.reasoning_effort.is_some() {
            out.push("reasoning_effort");
        }
        if self.permission_mode.is_some() {
            out.push("permission_mode");
        }
        if self.args.is_some() {
            out.push("args");
        }
        if self.redact.is_some() {
            out.push("redact");
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnUnsupported {
    Deny,
    Ask,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotifyDoc {
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Text; `{{ … }}` enthält CEL-Ausdrücke.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Inbox,
    Push,
    Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectDoc {
    pub op: EffectOp,
    pub scope: StateScope,
    pub key: String,
    /// Wert (CEL), für `set` und `append`.
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectOp {
    Set,
    Increment,
    Append,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StateScope {
    Session,
    Run,
    UserDay,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    Required,
    BestEffort,
}

/// Das JSON-Schema, wie `cargo xtask codegen` es nach `schemas/v1/policy.schema.json` schreibt.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(PolicySetDoc);
    let mut json = serde_json::to_string_pretty(&schema).unwrap_or_default();
    json.push('\n');
    json
}

/// Modifizierbare Felder je Phase (POL-003).
pub fn modifiable(phase: Phase) -> &'static [&'static str] {
    match phase {
        Phase::SessionStart => &["model", "reasoning_effort", "permission_mode"],
        Phase::ModelRequest => &["model", "reasoning_effort", "redact"],
        Phase::ToolCall => &["args", "redact"],
        Phase::ToolResult => &["redact"],
        Phase::BrowserNavigate | Phase::BrowserAction => &[],
    }
}
