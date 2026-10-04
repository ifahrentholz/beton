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
            attachments: Vec::new(),
        }
    }
}

/// Permission-Mode einer Session (HAR-027); auch Feld `executor.permission_mode` im Agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Plan,
    Default,
    AcceptEdits,
    Yolo,
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

/// Anfrage an das Gate vor einem Tool-Call (Policy und Approvals, POL bzw. WEB-018).
#[derive(Debug, Clone, PartialEq)]
pub struct GateRequest {
    pub turn_id: Option<TurnId>,
    pub call_id: String,
    pub tool: String,
    /// Kanonische Klasse, z. B. `shell` (POL-005).
    pub kind: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    Allow { updated_args: Option<Value> },
    Deny { reason: Option<String> },
}

/// Vom Runner bereitgestellte Entscheidungsstelle für Tool-Calls.
#[async_trait]
pub trait Gate: Send + Sync {
    async fn decide(&self, request: GateRequest) -> GateDecision;
}

/// Erlaubt alles (Tests).
#[derive(Debug, Default)]
pub struct AllowAll;

#[async_trait]
impl Gate for AllowAll {
    async fn decide(&self, _request: GateRequest) -> GateDecision {
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
}

/// Laufzeitumgebung eines Adapters.
#[derive(Clone)]
pub struct AdapterContext {
    pub gate: Arc<dyn Gate>,
    /// Startet Harness-Prozesse; in Golden-Tests ein simulierter Prozess.
    pub launcher: Arc<dyn ProcessLauncher>,
    pub env: HostEnv,
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
    async fn set_model(
        &mut self,
        model: String,
        effort: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError>;
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
