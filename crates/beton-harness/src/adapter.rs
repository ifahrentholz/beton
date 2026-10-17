//! Adapter-Trait und Session-Lebenszyklus (HAR-001).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{EventPayload, RawJson};
use beton_core::id::TurnId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use ts_rs::TS;

use crate::capabilities::{Capabilities, CapabilityUnsupported};
use crate::id::HarnessId;
use crate::process::ProcessLauncher;
use crate::registry::HarnessesConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Strukturiertes Vendor-Protokoll (stream-json, JSON-RPC).
    Native,
    /// Agent Client Protocol über stdio.
    Acp,
    /// Pseudo-Terminal mit Vendor-Hooks.
    Pty,
    /// Im Runner-Prozess (Direkt-API, Fake).
    InProc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Native,
    Tui,
}

/// Login-Status laut Vendor-CLI (nie aus Token-Dateien gelesen, ADR-0005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    #[default]
    Unknown,
    LoggedIn,
    LoggedOut,
    NotApplicable,
}

/// Ergebnis von [`HarnessAdapter::probe`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProbeReport {
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    pub auth_status: AuthStatus,
    /// `--version` lief nicht durch (Timeout, Fehler) – mit Begründung (HAR-003 AC2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub probe_failed: Option<String>,
}

/// Umgebung eines Hosts, aus der Adapter Binaries und Konfiguration auflösen (HAR-003).
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    /// Relevante Umgebungsvariablen (z. B. `BETON_CLAUDE_PATH`).
    pub vars: BTreeMap<String, String>,
    /// Inhalt von `PATH`.
    pub path: Option<OsString>,
    /// `harnesses:` aus `.beton/config.yaml` im Projekt.
    pub project: HarnessesConfig,
    /// `harnesses:` aus `~/.beton/config.yaml`.
    pub user: HarnessesConfig,
}

impl HostEnv {
    /// Liest Umgebung und `PATH` des aktuellen Prozesses; Konfiguration bleibt leer.
    /// Übernommen werden `BETON_*` und die Variablen, mit denen Vendor-CLIs ihr
    /// Konfigurationsverzeichnis finden ([`VENDOR_DIR_VARS`]).
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars()
                .filter(|(k, _)| k.starts_with("BETON_") || VENDOR_DIR_VARS.contains(&k.as_str()))
                .collect(),
            path: std::env::var_os("PATH"),
            ..Self::default()
        }
    }
}

/// Variablen, über die Vendor-CLIs ihr Konfigurationsverzeichnis finden; Adapter brauchen sie,
/// um z. B. eine Session-Datei für den History-Rebuild abzulegen (HAR-019). Sie enthalten nur
/// Pfade, keine Zugangsdaten.
pub const VENDOR_DIR_VARS: [&str; 3] = ["HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME"];

/// Wer automatisch kompaktiert (`compaction.auto`, USE-009).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoCompaction {
    /// Der Harness entscheidet selbst (Vendor-CLIs; Direkt-API-Harness ab `threshold`).
    #[default]
    Harness,
    /// beton kompaktiert vor dem nächsten Turn, wenn der zuletzt gemeldete Kontext über
    /// `threshold` liegt – einmal je Überschreitung, bei Harnesses mit `compaction: native`.
    Beton,
    /// Nie automatisch, nur über den Button, `/compact` oder die API.
    Off,
}

/// `compaction:` in der User-Konfiguration (USE-009).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CompactionSettings {
    /// `harness` (Default), `beton` oder `off`.
    pub auto: AutoCompaction,
    /// Anteil des Kontextfensters (größer 0, höchstens 1), ab dem automatisch kompaktiert
    /// wird; Default 0,8.
    pub threshold: f64,
}

/// Default-Schwelle für automatische Compaction (SES-011, USE-009).
pub const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8;

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            auto: AutoCompaction::Harness,
            threshold: DEFAULT_COMPACTION_THRESHOLD,
        }
    }
}

impl CompactionSettings {
    /// Prüft die Schwelle; der Fehler nennt den Grund.
    pub fn validate(&self) -> Result<(), String> {
        if self.threshold.is_finite() && self.threshold > 0.0 && self.threshold <= 1.0 {
            Ok(())
        } else {
            Err(format!(
                "compaction.threshold {} liegt nicht in (0, 1]",
                self.threshold
            ))
        }
    }

    /// Schwelle der harness-eigenen Automatik (Direkt-API-Harness): nur bei `auto: harness`;
    /// bei `beton` löst der Server aus, bei `off` niemand.
    pub fn harness_threshold(&self) -> Option<f64> {
        (self.auto == AutoCompaction::Harness).then_some(self.threshold)
    }

    /// Soll beton vor dem nächsten Turn kompaktieren (`auto: beton`)?
    pub fn beton_due(&self, used_tokens: u64, window_tokens: u64) -> bool {
        self.auto == AutoCompaction::Beton
            && window_tokens > 0
            && used_tokens as f64 > window_tokens as f64 * self.threshold
    }
}

/// Was eine neue Harness-Session braucht.
#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    pub workdir: PathBuf,
    pub model: Option<String>,
    pub mode: Option<Mode>,
    /// Native Session-Referenz zum Fortsetzen (z. B. Claude-Session-UUID).
    pub resume: Option<String>,
    /// Mit `resume`: die native Session nicht fortschreiben, sondern als neue Session
    /// abzweigen (Fork mit nativer History, HAR-019; Claude `--fork-session`).
    pub fork_session: bool,
    /// Nur Fake-Harness: Szenario-Datei (HAR-026).
    pub scenario: Option<PathBuf>,
    /// MCP-Server und Skills, die der Harness bekommt (HAR-009, AGT-008).
    pub mcp: McpInjection,
    /// Höchstzahl der Model-Requests je User-Turn (`executor.max_turns`, HAR-010); nur
    /// Harnesses mit eigenem Agent-Loop werten sie aus.
    pub max_turns: Option<u32>,
    /// Reasoning-Effort ab dem ersten Turn, bereits auf eine Stufe des Harness gemappt
    /// (HAR-017).
    pub effort: Option<String>,
    /// Permission-Mode ab dem Start (HAR-027); ohne Angabe der Vendor-Default. Der Runner hat
    /// ihn gegen die Capabilities und die Sandbox geprüft; [`crate::registry::Registry::start`]
    /// prüft `yolo` zusätzlich (fail closed).
    pub permission_mode: Option<PermissionMode>,
    /// Instructions des Agents (AGT-005), fertig zusammengesetzt. Der Adapter liefert sie je
    /// Capability `instructions_delivery` genau einmal aus.
    pub instructions: Option<String>,
    /// Der Agent legt `plan` fest (`executor.permission_mode: plan`, ohne Override): Die
    /// Session darf den Plan-Modus nicht selbst verlassen; Claude: `ExitPlanMode` wird ohne
    /// Rückfrage abgelehnt (AGT-011, HAR-027). Ein Moduswechsel durch den Nutzer bleibt möglich.
    pub plan_locked: bool,
    /// Vendor-Hooks über die Hook-Brücke (HAR-005 ab M2): Kommando und Laufzeitverzeichnis.
    /// Ohne Angabe installiert der Adapter keinen Hook (Durchsetzung nur an Rückfragen).
    pub hook: Option<crate::hook::HookSetup>,
    /// Automatische Compaction (USE-009); nur Harnesses mit eigenem Loop werten sie aus.
    pub compaction: CompactionSettings,
}

/// Erste Nachricht mit vorangestellten Instructions (`instructions_delivery:
/// first_message_prefix`, AGT-005).
pub fn prefix_instructions(instructions: &str, text: &str) -> String {
    format!("[Anweisungen für diese Session]\n{instructions}\n[Ende der Anweisungen]\n\n{text}")
}

/// Ein MCP-Server, wie ihn der Harness starten soll (HAR-009). Immer ein stdio-Relay
/// (`beton mcp serve|proxy`): Definitionen, Env-Werte, Header und das Relay-Token bleiben
/// beim Runner und erscheinen nie in der Konfiguration des Harness.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpLaunch {
    /// Name des Servers, z. B. `beton` oder `github` (`[a-z0-9_-]`).
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// MCP-Konfiguration einer Session (HAR-009) und das Session-Skill-Verzeichnis (AGT-008).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpInjection {
    pub servers: Vec<McpLaunch>,
    /// Plugin-Verzeichnis mit `.claude-plugin/plugin.json` und `skills/<name>/SKILL.md` für
    /// Harnesses mit nativen Skills (Claude: `--plugin-dir`, Codex: `skills/extraRoots/set`
    /// mit `<dir>/skills`).
    pub skills_dir: Option<PathBuf>,
}

impl McpInjection {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty() && self.skills_dir.is_none()
    }

    /// Namen der Server in Übergabe-Reihenfolge.
    pub fn names(&self) -> Vec<String> {
        self.servers.iter().map(|s| s.name.clone()).collect()
    }
}

/// Eingabe für [`HarnessAdapter::rebuild_history`]: der Verlauf bis zum Fork-Punkt (HAR-019).
#[derive(Debug, Clone, Default)]
pub struct RebuildRequest {
    /// Arbeitsverzeichnis der neuen Session (bestimmt z. B. das Claude-Projektverzeichnis).
    pub workdir: PathBuf,
    /// Inhalts-Events bis einschließlich `at_seq`, in Log-Reihenfolge.
    pub history: Vec<crate::handover::HistoryEvent>,
    /// Modell der neuen Session, falls bekannt.
    pub model: Option<String>,
}

/// Einmal-Aufruf ohne Session (SES-010), z. B. für einen Session-Titel: der
/// nicht-interaktive Modus der Vendor-CLI (`claude -p`, `codex exec`) mit ihrer eigenen
/// Anmeldung (HAR-015), nie über einen API-Key von beton (ADR-0034).
#[derive(Debug, Clone, Default)]
pub struct OneShotRequest {
    /// Anweisung an das Modell (System-Prompt bzw. vorangestellt).
    pub instructions: String,
    /// Inhalt, z. B. die erste Nachricht der Session. Geht über stdin, nie über argv.
    pub prompt: String,
    pub workdir: PathBuf,
    /// Modell; ohne Angabe wählt der Adapter sein kleinstes bzw. den Default der CLI.
    pub model: Option<String>,
    /// Höchstdauer; danach wird der Prozess beendet.
    pub timeout: Duration,
    /// Nur Fake-Harness: Szenario-Datei (Abschnitt `one_shot`).
    pub scenario: Option<PathBuf>,
}

/// Antwort eines Einmal-Aufrufs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OneShotReply {
    /// Antworttext des Modells.
    pub text: String,
    /// Tatsächlich genutztes Modell, soweit die CLI es meldet.
    pub model: String,
    /// Verbrauch des Aufrufs (`purpose` setzt der Aufrufer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<beton_core::event::CostDelta>,
}

/// Eingabe eines Nutzers für einen Turn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UserInput {
    pub text: String,
    /// Vom Runner vorab vergebene Turn-ID, z. B. weil ein `session.settings_changed` schon auf
    /// diesen Turn verweist (`effective_from_turn`, HAR-017). Ohne Angabe vergibt der Adapter
    /// die ID selbst.
    pub turn_id: Option<TurnId>,
    /// Bilder und PDF zur Eingabe (WEB-006); nur bei Capability `images`. Textdateien stehen
    /// schon im Text.
    pub attachments: Vec<InputAttachment>,
}

/// Ein binärer Anhang einer Eingabe (WEB-006).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputAttachment {
    /// Dateiname (nur Anzeige).
    pub name: String,
    /// Medientyp, z. B. `image/png` oder `application/pdf`.
    pub mime: String,
    /// Inhalt als Base64 (Standard-Alphabet mit Padding).
    pub data_base64: String,
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        Self {
            text: text.into(),
            turn_id: None,
            attachments: Vec::new(),
        }
    }
}

/// Permission-Mode einer Session (HAR-027); auch Feld `executor.permission_mode` im Agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Nur lesen und planen.
    Plan,
    /// Vendor-Default; Freigaben laut Policy.
    Default,
    /// Datei-Edits im Worktree ohne Rückfrage.
    AcceptEdits,
    /// Keine Vendor-Rückfragen; nur mit Tool-Sandbox Stufe 2 und Egress-Proxy startbar.
    Yolo,
}

impl PermissionMode {
    pub const ALL: [Self; 4] = [Self::Plan, Self::Default, Self::AcceptEdits, Self::Yolo];

    /// Name im Wire-Format, z. B. `accept_edits`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Default => "default",
            Self::AcceptEdits => "accept_edits",
            Self::Yolo => "yolo",
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unbekannter Permission-Mode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unbekannter Permission-Mode `{0}` (erlaubt: plan, default, accept_edits, yolo)")]
pub struct InvalidPermissionMode(pub String);

impl std::str::FromStr for PermissionMode {
    type Err = InvalidPermissionMode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| InvalidPermissionMode(s.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchOutcome {
    /// Wirksam ab dem nächsten Turn, ohne Neustart.
    Live,
    /// Harness wurde neu gestartet.
    Restarted,
}

/// Wie eine Session beendet wird (HAR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// Protokolleigenes Ende, nach `timeout` SIGTERM, nach weiteren 5 s SIGKILL.
    Graceful { timeout: Duration },
    /// Sofort SIGKILL auf den gesamten Prozessbaum.
    Kill,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::Graceful {
            timeout: Duration::from_secs(10),
        }
    }
}

/// Wie ein Harness-Prozess endete.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExitInfo {
    pub code: Option<i32>,
    /// Signalname, z. B. `SIGKILL`.
    pub signal: Option<String>,
    /// Letzte höchstens 8 KiB von stderr.
    pub stderr_tail: String,
}

/// Ein normalisiertes Event aus dem Adapter; `session_id`, `seq`, `ts` und `actor` setzt
/// der Runner. `raw` ist die Original-Zeile des Harness, byte-genau (HAR-001 AC3).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedEvent {
    pub payload: EventPayload,
    pub raw: Option<RawJson>,
    pub turn_id: Option<TurnId>,
}

impl NormalizedEvent {
    pub fn new(payload: EventPayload, turn_id: Option<TurnId>) -> Self {
        Self {
            payload,
            raw: None,
            turn_id,
        }
    }
}

/// Woher eine Tool-Call-Anfrage an das Gate kommt (POL-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateOrigin {
    /// Vor jedem Tool-Call, unabhängig vom Vendor (Claude `PreToolUse`-Hook, Direkt-API,
    /// Fake-Harness): volle Durchsetzung.
    PreTool,
    /// Der Vendor fragt um Freigabe (Claude `can_use_tool`, Codex-Approval, ACP
    /// `session/request_permission`): Durchsetzung nur an dieser Stelle.
    #[default]
    VendorAsk,
    /// Rückfrage des Vendors, die der Permission-Mode ohne Menschen beantwortet (Codex
    /// `accept_edits` für Datei-Änderungen im Workspace, `yolo` für alle, HAR-027): Die Policy
    /// entscheidet trotzdem, nur die menschliche Rückfrage entfällt.
    AutoApproval,
}

/// Anfrage an das Gate vor einem Tool-Call (Phase `tool_call`, POL-003).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GateRequest {
    pub turn_id: Option<TurnId>,
    pub call_id: String,
    /// Nativer Tool-Name (z. B. `Bash`, `commandExecution`, `mcp__github__get`).
    pub tool: String,
    /// Kanonische Klasse aus `beton_policy::normalize` (POL-005).
    pub kind: String,
    pub args: Value,
    /// MCP-Server, wenn der Harness ihn getrennt meldet (Codex, Direkt-API).
    pub mcp_server: Option<String>,
    /// ACP `tool_call.kind`.
    pub acp_kind: Option<String>,
    /// Arbeitsverzeichnis des Calls, wenn der Harness es meldet (Claude `PreToolUse.cwd`:
    /// Bash behält ein `cd` zwischen Calls, #293); sonst gilt das der Session.
    pub cwd: Option<String>,
    pub origin: GateOrigin,
    /// Original-Zeile der Rückfrage (z. B. `can_use_tool`), damit kein `raw` verloren geht:
    /// Der Runner hängt sie an sein `policy.decision` (HAR-001 AC3).
    pub raw: Option<RawJson>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    Allow { updated_args: Option<Value> },
    Deny { reason: Option<String> },
}

/// Anfrage vor einem Model-Request (Phase `model_request`, Granularität `call`; HAR-010 AC5).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelGateRequest {
    pub turn_id: Option<TurnId>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Letzte User-Nachricht (höchstens 64 KiB).
    pub prompt: Option<String>,
    /// Nachrichten des Requests (nur Direkt-API).
    pub messages: Option<Vec<Value>>,
    /// Namen der angebotenen Tools.
    pub tools: Vec<String>,
    pub estimated_input_tokens: Option<u64>,
}

/// Entscheidung für einen Model-Request; `Allow` mit geänderten Werten (Pass 1).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelGateDecision {
    Allow {
        model: Option<String>,
        reasoning_effort: Option<String>,
    },
    Deny {
        reason: Option<String>,
    },
}

/// Anfrage nach einem Tool-Call, bevor das Ergebnis zum Modell geht (Phase `tool_result`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResultGateRequest {
    pub call: GateRequest,
    pub ok: bool,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    /// Text des Ergebnisses (höchstens 64 KiB).
    pub text: String,
    pub bytes: u64,
    pub duration_ms: u64,
    pub truncated: bool,
}

/// Vom Runner bereitgestellte Entscheidungsstelle (POL-003). Fail closed: Wer eine Phase
/// nicht entscheiden kann, lehnt ab.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Phase `tool_call`.
    async fn decide(&self, request: GateRequest) -> GateDecision;
    /// Phase `model_request` je Model-Request (Granularität `call`).
    async fn model_request(&self, request: ModelGateRequest) -> ModelGateDecision;
    /// Phase `tool_result`; `Deny` ersetzt das Ergebnis durch die Begründung.
    async fn tool_result(&self, request: ResultGateRequest) -> GateDecision;
}

/// Erlaubt alles (Tests).
#[derive(Debug, Default)]
pub struct AllowAll;

#[async_trait]
impl Gate for AllowAll {
    async fn decide(&self, _request: GateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }

    async fn model_request(&self, _request: ModelGateRequest) -> ModelGateDecision {
        ModelGateDecision::Allow {
            model: None,
            reasoning_effort: None,
        }
    }

    async fn tool_result(&self, _request: ResultGateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }
}

/// Verbietet alles (Tests, Fail-closed-Default).
#[derive(Debug, Default)]
pub struct DenyAll;

#[async_trait]
impl Gate for DenyAll {
    async fn decide(&self, _request: GateRequest) -> GateDecision {
        GateDecision::Deny { reason: None }
    }

    async fn model_request(&self, _request: ModelGateRequest) -> ModelGateDecision {
        ModelGateDecision::Deny { reason: None }
    }

    async fn tool_result(&self, _request: ResultGateRequest) -> GateDecision {
        GateDecision::Deny { reason: None }
    }
}

/// Laufzeitumgebung eines Adapters.
#[derive(Clone)]
pub struct AdapterContext {
    pub gate: Arc<dyn Gate>,
    /// Startet Harness-Prozesse; in Golden-Tests ein simulierter Prozess.
    pub launcher: Arc<dyn ProcessLauncher>,
    pub env: HostEnv,
    /// Zweistufige Sandbox der Session (SBX-002), falls aktiv: Der Harness läuft in Stufe 1,
    /// seine Tools sollen über `bt-exec` in Stufe 2 laufen.
    pub stage: Option<StageEnv>,
}

/// Was ein Adapter für die zweistufige Sandbox braucht (SBX-002).
#[derive(Clone)]
pub struct StageEnv {
    /// Verzeichnis mit `bt-exec`, `bt-exec-zsh`, `bt-exec-bash` und `bt-exec-sh`.
    pub shim_dir: std::path::PathBuf,
    /// Austausch zwischen Stufe 1 und Stufe 2 (Stufe-2-Scratch), z. B. für
    /// `CLAUDE_CODE_TMPDIR`.
    pub exchange: std::path::PathBuf,
    /// Startet Tools in Stufe 2 für Harnesses, deren Tools beton selbst ausführt
    /// (`tool_isolation: beton`, z. B. `shell_exec` des Direkt-API-Harness).
    pub tools: Option<Arc<dyn ProcessLauncher>>,
    /// Was das Stufenmodell dieser Session tatsächlich durchsetzt (für `yolo`, HAR-027).
    pub sandbox: crate::SandboxStatus,
    /// Policy der Stufe 1: Schreibt beton im Auftrag des Harness (ACP `fs/write_text_file`),
    /// gilt sie auch dafür (Security-Review #327).
    pub harness_policy: Option<Arc<beton_sandbox::ResolvedPolicy>>,
    /// Terminals in Stufe 2 für Harnesses mit `tool_isolation: beton` (ACP `terminal/*`, #326).
    pub terminals: Option<Arc<dyn crate::terminal::TerminalHost>>,
}

impl std::fmt::Debug for StageEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageEnv")
            .field("shim_dir", &self.shim_dir)
            .field("exchange", &self.exchange)
            .field("tools", &self.tools.is_some())
            .field("sandbox", &self.sandbox)
            .field("harness_policy", &self.harness_policy.is_some())
            .field("terminals", &self.terminals.is_some())
            .finish()
    }
}

impl std::fmt::Debug for AdapterContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterContext")
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("harness.incompatible: Version {detected} liegt nicht in {expected}")]
    Incompatible { detected: String, expected: String },
    #[error("harness_not_found: {0}")]
    NotFound(String),
    #[error("start_refused: {0}")]
    StartRefused(String),
    #[error(transparent)]
    Unsupported(#[from] CapabilityUnsupported),
    #[error("unexpected_input: erwartet `{expected}`, erhalten `{got}`")]
    UnexpectedInput { expected: String, got: String },
    #[error("session_closed: der Harness läuft nicht mehr")]
    Closed,
    /// Die Aktion geht nicht, solange ein Turn läuft (z. B. Compaction, SES-011).
    #[error("turn_active: {0}")]
    Busy(String),
    /// `yolo` ohne Tool-Sandbox Stufe 2 und Egress-Proxy (HAR-027 AC1).
    #[error("sandbox_required: {0}")]
    SandboxRequired(String),
    #[error("protocol_error: {0}")]
    Protocol(String),
    #[error("capability_unsupported: dieser Harness hat keinen Einmal-Modus")]
    OneShotUnsupported,
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

impl HarnessError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Incompatible { .. } => "harness_incompatible",
            Self::NotFound(_) => "harness_not_found",
            Self::StartRefused(_) => "start_refused",
            Self::Unsupported(e) => e.code(),
            Self::UnexpectedInput { .. } => "unexpected_input",
            Self::Closed => "session_closed",
            Self::Busy(_) => "turn_active",
            Self::SandboxRequired(_) => "sandbox_required",
            Self::Protocol(_) => "protocol_error",
            Self::OneShotUnsupported => "capability_unsupported",
            Self::Timeout(_) => "timeout",
            Self::Io(_) => "internal",
        }
    }
}

/// Ein Harness-Typ, z. B. Claude Code. Objekt-sicher (HAR-001 AC4).
#[async_trait]
pub trait HarnessAdapter: Send + Sync + 'static {
    fn id(&self) -> HarnessId;
    fn modes(&self) -> &[Mode];
    /// Statisch deklariert, ggf. nach `probe()` versionsabhängig verfeinert.
    fn capabilities(&self, mode: Mode, probe: &ProbeReport) -> Capabilities;
    /// Binary finden, Version prüfen, Login-Status über die Vendor-CLI erfragen.
    async fn probe(&self, env: &HostEnv) -> ProbeReport;
    /// Login-Status über das Statuskommando der Vendor-CLI (HAR-016), nie aus Token-Dateien.
    /// Ohne solches Kommando `unknown`.
    async fn auth_status(&self, env: &HostEnv) -> AuthStatus {
        let _ = env;
        AuthStatus::Unknown
    }
    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError>;
    /// Rekonstruiert eine native Vendor-Session aus dem Event-Log (HAR-019, Capability
    /// `fork_history: rebuild`) und liefert ihre Referenz für [`SessionSpec::resume`].
    /// Ohne Unterstützung `capability_unsupported`; der Aufrufer fällt dann auf die Präambel
    /// zurück (HAR-018).
    async fn rebuild_history(
        &self,
        request: &RebuildRequest,
        env: &HostEnv,
    ) -> Result<String, HarnessError> {
        let _ = (request, env);
        Err(crate::capabilities::CapabilityUnsupported(crate::capabilities::Action::Fork).into())
    }
    /// Einmal-Aufruf über den nicht-interaktiven Modus der Vendor-CLI mit deren eigener
    /// Anmeldung (SES-010, ADR-0034). Ohne Einmal-Modus `capability_unsupported`.
    async fn one_shot(
        &self,
        request: &OneShotRequest,
        ctx: &AdapterContext,
    ) -> Result<OneShotReply, HarnessError> {
        let _ = (request, ctx);
        Err(HarnessError::OneShotUnsupported)
    }
    /// Import vorhandener Chats der Vendor-CLI (HAR-023, HAR-024, Capability
    /// `transcript_import`); ohne Unterstützung `None`.
    fn transcript_importer(&self) -> Option<Arc<dyn crate::import::TranscriptImporter>> {
        None
    }
}

/// Eine laufende Harness-Session. Genau eine pro beton-Session.
#[async_trait]
pub trait HarnessSession: Send {
    /// Startet einen neuen Turn.
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError>;
    /// Eingabe in einen laufenden Turn (Capability `steering`).
    async fn steer(&mut self, input: UserInput) -> Result<(), HarnessError>;
    async fn interrupt(&mut self) -> Result<(), HarnessError>;
    /// Modell und/oder Effort live wechseln (HAR-017), wirksam ab dem nächsten Turn. Der Runner
    /// ruft das nur ohne laufenden Turn und nur bei Capability `live` auf; `effort` ist bereits
    /// gemappt. Das Event `session.settings_changed` schreibt der Runner, nicht der Adapter.
    async fn set_model(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError>;
    /// Permission-Mode live wechseln (HAR-027). Ein Modus außerhalb von
    /// `capabilities.permission_modes` wird mit `capability_unsupported` abgelehnt.
    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), HarnessError>;
    async fn compact(&mut self) -> Result<(), HarnessError>;
    /// Der Event-Strom; kann genau einmal entnommen werden. Er endet, wenn der Harness endet.
    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>>;
    /// Z. B. Claude-Session-UUID oder Codex-Thread-ID.
    fn native_session_ref(&self) -> Option<String>;
    /// Capabilities dieser Session, wenn sie von denen des Adapters abweichen (z. B. die
    /// Überschreibungen eines Fake-Szenarios, HAR-026 AC2).
    fn capabilities(&self) -> Option<Capabilities> {
        None
    }
    async fn shutdown(self: Box<Self>, how: Shutdown) -> Result<ExitInfo, HarnessError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// USE-009: Wer bei welcher Einstellung automatisch kompaktiert.
    #[test]
    fn compaction_settings_decide_who_compacts() {
        let harness = CompactionSettings::default();
        assert_eq!(harness.harness_threshold(), Some(0.8));
        assert!(!harness.beton_due(190_000, 200_000));
        let beton = CompactionSettings {
            auto: AutoCompaction::Beton,
            threshold: 0.8,
        };
        assert_eq!(beton.harness_threshold(), None);
        assert!(beton.beton_due(170_000, 200_000));
        assert!(
            !beton.beton_due(160_000, 200_000),
            "genau 80 % ist nicht darüber"
        );
        assert!(!beton.beton_due(170_000, 0), "ohne Fenster keine Automatik");
        let off = CompactionSettings {
            auto: AutoCompaction::Off,
            ..CompactionSettings::default()
        };
        assert_eq!(off.harness_threshold(), None);
        assert!(!off.beton_due(199_000, 200_000));
        for bad in [0.0, -0.1, 1.5, f64::NAN] {
            let s = CompactionSettings {
                threshold: bad,
                ..CompactionSettings::default()
            };
            assert!(s.validate().is_err(), "{bad}");
        }
        assert!(beton.validate().is_ok());
    }
}
