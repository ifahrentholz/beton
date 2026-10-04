//! Agent-Format v1 (AGT-001): die kanonische Struktur von `agent.yaml`.
//!
//! Diese Typen sind die einzige Quelle des JSON-Schemas `schemas/v1/agent.schema.json`
//! (AGT-002). Unbekannte Felder sind Fehler; nur auf oberster Ebene sind `x-`-Felder für
//! Erweiterungen erlaubt. Felder späterer Meilensteine (`timers`, `schedules`, `async` ab M5;
//! `policies`, `sandbox` ab M2) werden in v1 nur gelesen und geprüft, nicht ausgewertet.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use beton_harness::{HarnessId, Mode, PermissionMode};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Einzige unterstützte Version des Formats.
pub const SPEC_VERSION: u64 = 1;

/// Eine Agent-Definition (`agent.yaml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "beton Agent (agent.yaml, Format v1)",
    extend("patternProperties" = { "^x-": {} })
)]
pub struct AgentSpec {
    /// Version des Formats. Unterstützt: `1`.
    #[schemars(extend("const" = 1))]
    pub spec_version: u64,
    /// Name, `[a-z0-9-]`, eindeutig im Suchpfad; entspricht dem Verzeichnisnamen.
    pub name: AgentName,
    pub description: Option<String>,
    /// Frei wählbar, SemVer empfohlen.
    #[serde(default, deserialize_with = "string_or_number")]
    pub version: Option<String>,
    pub executor: Executor,
    pub instructions: Option<Instructions>,
    /// Typisierte Eingaben, nutzbar als `{{ params.x }}` (AGT-010).
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
    pub tools: Option<Tools>,
    pub skills: Option<SkillSelection>,
    /// Sub-Agents für `session_spawn`: Verweis (`ref`) oder inline definiert.
    #[serde(default)]
    pub agents: BTreeMap<String, SubAgent>,
    pub spawn: Option<Spawn>,
    /// Timer-Tools freischalten (ab M5, ASY-003).
    pub timers: Option<bool>,
    /// Wiederkehrende Auslöser (ab M5, ASY-004).
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    /// Verhalten als Async-Agent (ab M5, ASY-002, ASY-008).
    #[serde(rename = "async")]
    pub async_: Option<Async>,
    /// Agent-Ebene der Policy-Hierarchie (ab M2, AGT-014).
    #[serde(default)]
    pub policies: Vec<PolicyEntry>,
    /// Sandbox-Anforderungen (ab M2; Felder und Semantik: SBX).
    pub sandbox: Option<Sandbox>,
    /// `x-`-Felder der obersten Ebene (Erweiterungen); werden nicht ausgewertet.
    #[serde(skip)]
    #[schemars(skip)]
    pub extensions: BTreeMap<String, Value>,
}

impl AgentSpec {
    /// Ein Inline-Sub-Agent (`agents.<name>` mit `executor`) als eigenständige Definition;
    /// relative Dateien gelten weiter ab dem Verzeichnis des Parents (AGT-009).
    pub fn from_inline(name: &str, sub: &SubAgent) -> Result<Self, String> {
        let executor = sub
            .executor
            .clone()
            .ok_or_else(|| format!("Sub-Agent „{name}“ ohne executor"))?;
        Ok(Self {
            spec_version: SPEC_VERSION,
            name: name.parse().or_else(|_| {
                name.to_ascii_lowercase()
                    .replace(|c: char| !(c.is_ascii_alphanumeric() || c == '-'), "-")
                    .parse()
            })?,
            description: sub.description.clone(),
            version: None,
            executor,
            instructions: sub.instructions.clone(),
            params: sub.params.clone(),
            tools: sub.tools.clone(),
            skills: sub.skills.clone(),
            agents: sub.agents.clone(),
            spawn: sub.spawn.clone(),
            timers: None,
            schedules: Vec::new(),
            async_: None,
            policies: sub.policies.clone(),
            sandbox: sub.sandbox.clone(),
            extensions: BTreeMap::new(),
        })
    }
}

/// Name eines Agents: `[a-z0-9]` gefolgt von `[a-z0-9-]`, höchstens 64 Zeichen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentName(String);

impl AgentName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_valid(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !s.starts_with('-')
    }
}

impl std::str::FromStr for AgentName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if Self::is_valid(s) {
            Ok(Self(s.to_owned()))
        } else {
            Err(format!(
                "„{s}“ ist kein gültiger Agent-Name (erlaubt: a-z, 0-9, -; höchstens 64 Zeichen)"
            ))
        }
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AgentName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for AgentName {
    fn schema_name() -> Cow<'static, str> {
        "AgentName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[a-z0-9][a-z0-9-]{0,63}$"
        })
    }
}

/// Dauer wie `90s`, `30m`, `2h`, `1d`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationText(String);

impl DurationText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Als Dauer; `None` bei Überlauf.
    pub fn to_duration(&self) -> Option<std::time::Duration> {
        let digits = self.0.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let n: u64 = digits.parse().ok()?;
        let ms = match &self.0[digits.len()..] {
            "ms" => Some(n),
            "s" => n.checked_mul(1_000),
            "m" => n.checked_mul(60_000),
            "h" => n.checked_mul(3_600_000),
            "d" => n.checked_mul(86_400_000),
            _ => None,
        }?;
        Some(std::time::Duration::from_millis(ms))
    }

    pub fn is_valid(s: &str) -> bool {
        let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let unit = &s[digits.len()..];
        !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && matches!(unit, "ms" | "s" | "m" | "h" | "d")
    }
}

impl Serialize for DurationText {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DurationText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        if Self::is_valid(&s) {
            Ok(Self(s.into_owned()))
        } else {
            Err(serde::de::Error::custom(format!(
                "„{s}“ ist keine Dauer (z. B. 90s, 30m, 2h, 1d)"
            )))
        }
    }
}

impl JsonSchema for DurationText {
    fn schema_name() -> Cow<'static, str> {
        "Duration".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9]+(ms|s|m|h|d)$"
        })
    }
}

/// `version: 1.0` (Zahl) wird wie `"1.0"` behandelt.
fn string_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "invalid type: {other}, expected a string"
        ))),
    }
}

/// Auf welchem Harness und wie der Agent läuft (AGT-004).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Executor {
    /// `claude`, `codex`, `fake`, `acp:<slug>` oder `direct:<provider>`.
    pub harness: HarnessId,
    /// `native` (Default) oder `tui`.
    pub mode: Option<Mode>,
    /// Modell; ohne Angabe der Default des Harness.
    pub model: Option<String>,
    /// Gegen Harness/Modell geprüft und ggf. gemappt (HAR-017).
    pub reasoning_effort: Option<Effort>,
    /// `plan`, `default`, `accept_edits` oder `yolo` (HAR-027).
    pub permission_mode: Option<PermissionMode>,
    #[schemars(range(min = 1))]
    pub max_turns: Option<u32>,
    /// Wanduhr pro Run, z. B. `2h`.
    pub timeout: Option<DurationText>,
    /// System-Tools über MCP bereitstellen (HAR-009).
    pub mcp_bridge: Option<bool>,
}

/// Reasoning-Effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
        }
    }
}

/// Instructions aus `text` oder `file`, optional mit `append` (AGT-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Instructions {
    /// Datei relativ zum Agent-Verzeichnis; alternativ `text`.
    pub file: Option<String>,
    pub text: Option<String>,
    /// Wird angehängt; darf Template-Ausdrücke enthalten.
    pub append: Option<String>,
    /// `auto`, `none` oder eine Liste von Dateien.
    pub project_files: Option<ProjectFiles>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ProjectFiles {
    Mode(ProjectFilesMode),
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectFilesMode {
    Auto,
    None,
}

/// Ein typisierter Parameter (AGT-010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Param {
    #[serde(rename = "type")]
    pub kind: ParamType,
    pub description: Option<String>,
    pub default: Option<Value>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub required: Option<bool>,
    /// Erlaubte Werte bei `type: enum`.
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Integer,
    Number,
    Boolean,
    Enum,
}

/// Tools sind ausschließlich MCP-Server (AGT-006) plus System-Tools (AGT-007).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    #[serde(default)]
    pub mcp: BTreeMap<String, McpServer>,
    #[serde(default)]
    pub system: Vec<SystemTool>,
    /// User- und Projekt-MCP-Server zusätzlich verwenden (Default `false`).
    pub inherit: Option<bool>,
}

/// MCP-Server: stdio (`command`, `args`, `env`) oder HTTP (`url`, `headers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Nur diese Tools des Servers sind sichtbar.
    pub allow: Option<Vec<String>>,
}

/// Eingebaute System-Tools des MCP-Servers `beton` (AGT-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemTool {
    SessionSpawn,
    SessionSend,
    SessionWait,
    SessionStatus,
    SessionList,
    SessionCancel,
    InboxRead,
    AskUser,
    PolicyQuery,
    TimerSet,
    TimerCancel,
    TimerList,
    ScheduleCreate,
    ScheduleList,
    ScheduleUpdate,
    ScheduleDelete,
    SkillLoad,
    SkillReadFile,
}

/// `all`, `none` oder eine Liste von Skill-Namen (AGT-008).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SkillSelection {
    Mode(SkillMode),
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillMode {
    All,
    None,
}

/// Sub-Agent: entweder nur `ref` (Agent-Ref, relativ zum Agent-Verzeichnis) oder inline mit
/// `executor` (AGT-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubAgent {
    /// `./agents/reviewer`, `name` oder `builtin:<name>`.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub description: Option<String>,
    pub executor: Option<Executor>,
    pub instructions: Option<Instructions>,
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
    pub tools: Option<Tools>,
    pub skills: Option<SkillSelection>,
    #[serde(default)]
    pub agents: BTreeMap<String, SubAgent>,
    pub spawn: Option<Spawn>,
    #[serde(default)]
    pub policies: Vec<PolicyEntry>,
    pub sandbox: Option<Sandbox>,
}

/// Default für `spawn.max_depth`: nur direkte Childs (AGT-009, *Annahme*).
pub const DEFAULT_MAX_DEPTH: u32 = 1;
/// Default für `spawn.max_concurrent` (AGT-009, *Annahme*).
pub const DEFAULT_MAX_CONCURRENT: u32 = 3;

/// Grenzen für Child-Sessions (AGT-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// Nur diese Sub-Agents (Schlüssel aus `agents`) dürfen gestartet werden.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Größte Tiefe des Session-Baums unter diesem Agent (Default 1: nur direkte Childs).
    #[schemars(range(min = 1))]
    pub max_depth: Option<u32>,
    /// Höchstzahl gleichzeitig laufender Childs (Default 3).
    #[schemars(range(min = 1))]
    pub max_concurrent: Option<u32>,
    /// Workspace der Kinder: `new` (eigener Worktree vom HEAD des Parents), `inherit`
    /// (Workspace des Parents, Default) oder `none` (Workspace des Parents, nur lesend ab M2).
    pub worktree: Option<WorktreeMode>,
    /// Maximaler Anteil am Run-Budget je Kind (0–1); ausgewertet ab M5 (ASY-010).
    #[schemars(range(min = 0.0, max = 1.0))]
    pub budget_share: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeMode {
    New,
    Inherit,
    None,
}

/// Schedule im Agent (ab M5, ASY-004); der Agent ist der Agent selbst.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub id: String,
    /// Standard-Cron mit 5 Feldern.
    pub cron: String,
    /// IANA-Zeitzone; ohne Angabe die des Users.
    pub timezone: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    pub catch_up: Option<CatchUp>,
    pub overlap: Option<Overlap>,
    pub keep_awake: Option<KeepAwake>,
    pub runner: Option<RunnerSelector>,
    pub budget: Option<Budget>,
    pub state: Option<ScheduleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    RunOnce,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Overlap {
    Skip,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleState {
    Active,
    Paused,
}

/// `false`, `during_run` oder `always` (ASY-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAwake {
    Off,
    DuringRun,
    Always,
}

impl Serialize for KeepAwake {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            KeepAwake::Off => s.serialize_bool(false),
            KeepAwake::DuringRun => s.serialize_str("during_run"),
            KeepAwake::Always => s.serialize_str("always"),
        }
    }
}

impl<'de> Deserialize<'de> for KeepAwake {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Bool(false) => Ok(KeepAwake::Off),
            Value::String(s) if s == "during_run" => Ok(KeepAwake::DuringRun),
            Value::String(s) if s == "always" => Ok(KeepAwake::Always),
            other => Err(serde::de::Error::custom(format!(
                "unknown variant `{}`, expected one of `false`, `during_run`, `always`",
                other
                    .as_str()
                    .map_or_else(|| other.to_string(), str::to_owned)
            ))),
        }
    }
}

impl JsonSchema for KeepAwake {
    fn schema_name() -> Cow<'static, str> {
        "KeepAwake".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "enum": [false, "during_run", "always"] })
    }
}

/// Ziel-Runner per Label-Selektor (nur zentral, RUN-016).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunnerSelector {
    pub selector: String,
}

/// Budget eines Runs (ASY-010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_cost_usd: Option<f64>,
    pub max_duration: Option<DurationText>,
    pub max_turns: Option<u32>,
}

/// Verhalten als Async-Agent (ab M5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Async {
    /// Zustellung an den Parent: `wake` (Default) oder `inbox_only`.
    pub on_result: Option<OnResult>,
    pub approval: Option<AsyncApproval>,
    #[serde(default)]
    pub notify: Vec<NotifyChannel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnResult {
    Wake,
    InboxOnly,
}

/// Approvals ohne Zuschauer (ASY-008).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncApproval {
    pub timeout: Option<DurationText>,
    pub on_timeout: Option<OnTimeout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnTimeout {
    Deny,
    Allow,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotifyChannel {
    Push,
    Inbox,
}

/// Policy-Eintrag: Verweis auf eine Policy-Datei (`ref`) oder Inline-Regel mit `id`
/// (Format POL-001). Die Regelfelder prüft ab M2 das Policy-Schema; v1 prüft nur `ref`/`id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyEntry {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub id: Option<String>,
    #[serde(flatten)]
    pub rule: BTreeMap<String, Value>,
}

/// Sandbox-Anforderungen des Agents; Felder und Semantik wie die Sandbox-Konfiguration in
/// SBX (04-sandbox.md). Ausgewertet ab M2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    pub backend: Option<SandboxBackend>,
    /// `default`, `dev` oder `readonly` (SBX-012).
    pub preset: Option<SandboxPreset>,
    pub require_tool_isolation: Option<bool>,
    pub workspace: Option<WorkspaceAccess>,
    #[serde(default)]
    pub read_paths: Vec<String>,
    #[serde(default)]
    pub write_paths: Vec<String>,
    #[serde(default)]
    pub masks: Vec<String>,
    #[serde(default)]
    pub allow_hidden: Vec<String>,
    pub env: Option<SandboxEnv>,
    pub allow_network: Option<bool>,
    /// Egress-Regeln, z. B. `"GET,HEAD registry.npmjs.org/**"`.
    #[serde(default)]
    pub egress_rules: Vec<String>,
    #[serde(default)]
    pub egress_allow_private: Vec<String>,
    /// Credential-Bindungen (Format PRX-006, ab M2).
    #[serde(default)]
    pub credentials: Vec<Value>,
    pub limits: Option<SandboxLimits>,
    pub harness: Option<SandboxHarness>,
    pub windows: Option<SandboxWindows>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackend {
    Auto,
    Seatbelt,
    Landlock,
    Bwrap,
    Windows,
    Docker,
    Podman,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPreset {
    Default,
    Dev,
    Readonly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    Rw,
    Ro,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxEnv {
    #[serde(default)]
    pub passthrough: Vec<String>,
    #[serde(default)]
    pub set: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxLimits {
    pub memory_mb: Option<u64>,
    pub pids: Option<u64>,
    pub cpu_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxHarness {
    #[serde(default)]
    pub egress_extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxWindows {
    pub allow_unenforced_network: Option<bool>,
}

/// JSON-Schema des Formats (AGT-002), wie es `cargo xtask codegen` nach
/// `schemas/v1/agent.schema.json` schreibt und `beton agent schema` ausgibt.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(AgentSpec);
    let mut json = serde_json::to_string_pretty(&schema).unwrap_or_default();
    json.push('\n');
    json
}
