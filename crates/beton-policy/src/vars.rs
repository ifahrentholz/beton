//! Kontext-Variablen je Phase als Rust-Typen (POL-004 AC1).
//!
//! Diese Typen sind die einzige Quelle der CEL-Deklaration ([`crate::types::DECLS`]) und der
//! Referenztabelle in `docs/generated/policy-context.md` (`cargo xtask codegen`). Der Runner
//! füllt sie und übergibt sie als JSON an [`crate::context::Activation`]; nicht gesetzte
//! Felder fehlen und sind in CEL `null`.
//!
//! Variablen späterer Meilensteine (`browser.*` ab M3, `budget.*` und `team` ab M4) sind
//! deklariert, damit Policies sie schon prüfen können; der Runner füllt sie erst mit ihren
//! Features.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Phase;

/// Zeitpunkt (RFC 3339), in CEL `timestamp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CelTimestamp(pub String);

impl CelTimestamp {
    pub fn from_unix(secs: i64) -> Self {
        Self::from_datetime(
            OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        )
    }

    pub fn from_datetime(t: OffsetDateTime) -> Self {
        Self(t.format(&Rfc3339).unwrap_or_default())
    }

    pub fn now() -> Self {
        Self::from_datetime(OffsetDateTime::now_utc())
    }
}

impl JsonSchema for CelTimestamp {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "timestamp".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "format": "date-time" })
    }
}

/// Eine Map `string → dyn` (Zustand, Tool-Argumente).
pub type DynMap = BTreeMap<String, Value>;

/// Die Session (gemeinsam, alle Phasen).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "session")]
pub struct SessionVars {
    /// ID der Session (`ses_…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Titel der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Harness, z. B. `claude`, `codex`, `acp:gemini`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// `native` oder `tui`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Aktuelles Modell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Aktueller Reasoning-Effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Permission-Mode (`plan`, `default`, `accept_edits`, `yolo`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Kosten der Session in USD aus der Usage-Projektion (ohne Subscriptions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Listenpreis-Äquivalent der Subscription-Nutzung der Session in USD laut Preis-Katalog;
    /// nie Geld (ADR-0034). Budgets zählen `cost_usd + equivalent_usd` (POL-011).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equivalent_usd: Option<f64>,
    /// Modelle der Session, deren Nutzung weder Geld noch Listenpreis-Äquivalent hat (ohne
    /// Katalogpreis); Budgets fragen dann nach (`on_unpriced`, POL-011).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpriced_models: Option<Vec<String>>,
    /// Eingabe-Tokens bisher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    /// Ausgabe-Tokens bisher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    /// Abgeschlossene oder laufende Turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<i64>,
    /// Tool-Calls bisher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<i64>,
    /// Start der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<CelTimestamp>,
    /// Laufzeit in Sekunden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    /// Asynchroner Lauf (Timer, Schedule, Spawn mit `async`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_async: Option<bool>,
    /// Verbundene Clients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_clients: Option<i64>,
    /// Tiefe im Sub-Agent-Baum (0 = Wurzel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
    /// Eltern-Session eines Sub-Agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Projekt der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Arbeitsverzeichnis (absolut).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Projektverzeichnis (absolut): nächstes Verzeichnis mit `.beton/config.yaml`, sonst die
    /// Git-Wurzel des Arbeitsverzeichnisses (CLI-008); `${project_root}` in `path_guard`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Git-Branch des Arbeitsverzeichnisses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Labels der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
}

/// Der Run (interaktiv: `trigger = "interactive"`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "run")]
pub struct RunVars {
    /// ID des Runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `interactive`, `spawn`, `timer`, `schedule`, `webhook`, `fork` oder `import`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// Kosten des Runs in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Listenpreis-Äquivalent der Subscription-Nutzung des Runs in USD (ADR-0034).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equivalent_usd: Option<f64>,
    /// Modelle des Runs ohne Preis (POL-011).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpriced_models: Option<Vec<String>>,
    /// Kosten des Runs inklusive aller Child-Sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtree_cost_usd: Option<f64>,
    /// Listenpreis-Äquivalent des Runs inklusive aller Child-Sessions in USD (ADR-0034).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtree_equivalent_usd: Option<f64>,
    /// Modelle ohne Preis im Run inklusive aller Child-Sessions (POL-011).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtree_unpriced_models: Option<Vec<String>>,
    /// Laufzeit in Sekunden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    /// Turns des Runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<i64>,
    /// Budget des Runs in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
}

/// Der User, dem die Session gehört.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "user")]
pub struct UserVars {
    /// ID (`usr_…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// E-Mail-Adresse (nur mit Identität, ab M4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Rollen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// Teams (ab M4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teams: Option<Vec<String>>,
    /// Zeitzone (IANA), Default `UTC`; bestimmt die Tagesgrenze.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Kosten des Users heute (00:00 in `user.timezone`) in USD über alle Sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_cost_usd: Option<f64>,
    /// Listenpreis-Äquivalent der Subscription-Nutzung des Users heute in USD (ADR-0034).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_equivalent_usd: Option<f64>,
    /// Modelle ohne Preis, die der User heute genutzt hat (POL-012).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_unpriced_models: Option<Vec<String>>,
    /// Kosten des Users im laufenden Monat in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_cost_usd: Option<f64>,
}

/// Die Organisation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "org")]
pub struct OrgVars {
    /// ID (`org_…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Teams der Session (ab M4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "team")]
pub struct TeamVars {
    /// IDs der Teams.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
}

/// Der Agent der Session (leer ohne Agent).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "agent")]
pub struct AgentVars {
    /// Name des Agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Version des Agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Herkunft (`builtin`, `project`, `user`, Pfad).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Inhalts-Hash des Agent-Snapshots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Der Host, auf dem der Runner läuft.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "host")]
pub struct HostVars {
    /// ID (`hst_…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Betriebssystem (`macos`, `linux`, `windows`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Architektur (`aarch64`, `x86_64`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    /// Labels des Hosts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// Lokaler Host (derselbe Rechner wie der Server).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
    /// Home-Verzeichnis des Users auf dem Host (absolut); `~` und `${home}` in `path_guard`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
}

/// Wer die Aktion auslöst.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "actor")]
pub struct ActorVars {
    /// `user`, `agent` oder `system`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// ID des Akteurs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Budget-Lease im Offline-Betrieb (ab M4, POL-021).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "budget")]
pub struct BudgetVars {
    /// Verbindung zum Server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
    /// Restbetrag der Lease in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_remaining_usd: Option<f64>,
    /// Ablauf der Lease.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<CelTimestamp>,
}

/// Zustand und Zähler (POL-009).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "state")]
pub struct StateVars {
    /// Zustand der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<DynMap>,
    /// Zustand des Runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<DynMap>,
    /// Zustand des Users für den aktuellen Tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_day: Option<DynMap>,
    /// Zustand des Users.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<DynMap>,
}

/// Approvals der Session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "approvals")]
pub struct ApprovalsVars {
    /// Erteilte Freigaben in dieser Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_in_session: Option<i64>,
    /// Abgelehnte Freigaben in dieser Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_in_session: Option<i64>,
}

/// Sandbox beim Start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "start_sandbox")]
pub struct StartSandboxVars {
    /// Tool-Sandbox aktiv.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Backend (`seatbelt`, `landlock`, `none`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Profil der Sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// Netz beim Start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "start_network")]
pub struct StartNetworkVars {
    /// Egress-Proxy aktiv.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_enabled: Option<bool>,
}

/// Der Start einer Session (Phase `session_start`, auch Fork, Resume und Async-Run).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "start")]
pub struct StartVars {
    /// `interactive`, `spawn`, `timer`, `schedule`, `webhook`, `fork`, `resume` oder `import`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// Harness der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Angefragtes Modell (modifizierbar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Angefragter Effort (modifizierbar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Angefragter Permission-Mode (modifizierbar, nur strenger).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<StartSandboxVars>,
    /// Netz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<StartNetworkVars>,
    /// Arbeitsverzeichnis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Quelle eines Forks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_of: Option<String>,
    /// Agent-Referenz (`builtin:<name>`, Pfad).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
}

/// Ein Model-Request bzw. Turn (Phase `model_request`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "request")]
pub struct RequestVars {
    /// Modell des Requests (modifizierbar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Anbieter (`anthropic`, `openai`, …), soweit bekannt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Effort (modifizierbar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// `call` (jeder Model-Request, Direkt-API) oder `turn` (vor jedem Turn).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granularity: Option<String>,
    /// Letzte User-Nachricht (höchstens 64 KiB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Nachrichten des Requests (nur Direkt-API).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Value>>,
    /// Namen der angebotenen Tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Geschätzte Eingabe-Tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<i64>,
    /// Füllstand des Kontextfensters in Prozent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_used_pct: Option<f64>,
}

/// Ein normalisierter Tool-Call (Phasen `tool_call`, `tool_result`; POL-005).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "tool")]
pub struct ToolVars {
    /// Tool-Name ohne MCP-Präfix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Name beim Harness, z. B. `Bash`, `commandExecution`, `mcp__github__get`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_name: Option<String>,
    /// MCP-Server oder `builtin`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Kanonische Klasse: `shell`, `file_read`, `file_write`, `file_edit`, `search`,
    /// `web_fetch`, `mcp`, `system`, `browser`, `other`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Argumente (modifizierbar per JSON-Merge-Patch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<DynMap>,
    /// Shell-Kommando als String.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Geparste Teilkommandos (an `;`, `&&`, `||`, `|` getrennt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<Vec<String>>>,
    /// Arbeitsverzeichnis des Calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Betroffene Pfade, absolut und normalisiert.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    /// Pfade aus `paths`, in die der Call schreibt: Datei-Tools der Klassen `file_write` und
    /// `file_edit`, bei Shell-Kommandos Umleitungsziele und Ziele bekannter Datei-Kommandos
    /// (best effort, POL-016).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_paths: Option<Vec<String>>,
    /// URLs in den Argumenten.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urls: Option<Vec<String>>,
    /// Laufende Nummer des Calls in der Session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_index: Option<i64>,
    /// Wiederholungen eines identischen Calls im Fenster (POL-018).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_count: Option<i64>,
    /// Fehlgeschlagene Calls in Folge (POL-018).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consecutive_errors: Option<i64>,
    /// Startet einen Sub-Agent (Claude `Task`, `session_spawn`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_subagent_spawn: Option<bool>,
    /// Kommando nicht sicher parsebar (`eval`, `$(…)`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_uncertain: Option<bool>,
}

/// Ergebnis eines Tool-Calls (Phase `tool_result`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "result")]
pub struct ResultVars {
    /// Erfolgreich.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// Exit-Code (Shell).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// Fehlertext.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Text des Ergebnisses (höchstens 64 KiB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Größe in Bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    /// Dauer in Millisekunden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// `text` wurde gekürzt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// Navigation im beton-Browser (Phase `browser_navigate`, ab M3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "browser_navigate")]
pub struct BrowserNavigateVars {
    /// Ziel-URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Schema (`https`, `http`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Pfad.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `agent`, `user` oder `page`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    /// Tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    /// Browser-Profil.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// Aktion im beton-Browser (Phase `browser_action`, ab M3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "browser_action")]
pub struct BrowserActionVars {
    /// `click`, `type`, `submit`, `upload`, `download`, `eval` oder `select`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// URL der Seite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Host der Seite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Selektor des Elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// ARIA-Rolle des Elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_role: Option<String>,
    /// Die Aktion sendet ein Formular ab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_form_submit: Option<bool>,
    /// Ziel ist ein Passwortfeld.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_password_field: Option<bool>,
    /// Länge des getippten Texts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_len: Option<i64>,
    /// Dateiname bei Upload oder Download.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

/// Browser-Variable je nach Phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserVars {
    Navigate(BrowserNavigateVars),
    Action(BrowserActionVars),
}

/// Der vollständige Kontext einer Auswertung; nur die Variablen der jeweiligen Phase setzen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Vars {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<CelTimestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<RunVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<OrgVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<HostVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StateVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approvals: Option<ApprovalsVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<StartVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultVars>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserVars>,
}

impl Vars {
    /// Der Kontext als JSON-Objekt für [`crate::context::Activation::new`].
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    }
}

/// Eine Variable der obersten Ebene.
pub struct VarDef {
    pub name: &'static str,
    /// Phasen, in denen sie gilt; leer = alle.
    pub phases: &'static [Phase],
    pub doc: &'static str,
    /// JSON-Schema des Typs.
    pub schema: fn(&mut SchemaGenerator) -> Schema,
}

fn schema_of<T: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    g.subschema_for::<T>()
}

/// Alle Variablen der obersten Ebene (POL-004).
pub fn var_defs() -> Vec<VarDef> {
    use Phase::*;
    const ALL: &[Phase] = &[];
    vec![
        VarDef {
            name: "now",
            phases: ALL,
            doc: "Zeitpunkt der Auswertung.",
            schema: schema_of::<CelTimestamp>,
        },
        VarDef {
            name: "session",
            phases: ALL,
            doc: "Die Session.",
            schema: schema_of::<SessionVars>,
        },
        VarDef {
            name: "run",
            phases: ALL,
            doc: "Der Run.",
            schema: schema_of::<RunVars>,
        },
        VarDef {
            name: "user",
            phases: ALL,
            doc: "Der User.",
            schema: schema_of::<UserVars>,
        },
        VarDef {
            name: "org",
            phases: ALL,
            doc: "Die Organisation.",
            schema: schema_of::<OrgVars>,
        },
        VarDef {
            name: "team",
            phases: ALL,
            doc: "Teams (ab M4).",
            schema: schema_of::<TeamVars>,
        },
        VarDef {
            name: "agent",
            phases: ALL,
            doc: "Der Agent der Session.",
            schema: schema_of::<AgentVars>,
        },
        VarDef {
            name: "host",
            phases: ALL,
            doc: "Der Host.",
            schema: schema_of::<HostVars>,
        },
        VarDef {
            name: "actor",
            phases: ALL,
            doc: "Auslöser der Aktion.",
            schema: schema_of::<ActorVars>,
        },
        VarDef {
            name: "budget",
            phases: ALL,
            doc: "Budget-Lease (ab M4).",
            schema: schema_of::<BudgetVars>,
        },
        VarDef {
            name: "state",
            phases: ALL,
            doc: "Zustand und Zähler (POL-009).",
            schema: schema_of::<StateVars>,
        },
        VarDef {
            name: "approvals",
            phases: ALL,
            doc: "Approvals der Session.",
            schema: schema_of::<ApprovalsVars>,
        },
        VarDef {
            name: "start",
            phases: &[SessionStart],
            doc: "Der Start.",
            schema: schema_of::<StartVars>,
        },
        VarDef {
            name: "request",
            phases: &[ModelRequest],
            doc: "Der Model-Request bzw. Turn.",
            schema: schema_of::<RequestVars>,
        },
        VarDef {
            name: "tool",
            phases: &[ToolCall, ToolResult],
            doc: "Der normalisierte Tool-Call.",
            schema: schema_of::<ToolVars>,
        },
        VarDef {
            name: "result",
            phases: &[ToolResult],
            doc: "Das Tool-Ergebnis.",
            schema: schema_of::<ResultVars>,
        },
        VarDef {
            name: "browser",
            phases: &[BrowserNavigate],
            doc: "Navigation im beton-Browser (ab M3).",
            schema: schema_of::<BrowserNavigateVars>,
        },
        VarDef {
            name: "browser",
            phases: &[BrowserAction],
            doc: "Aktion im beton-Browser (ab M3).",
            schema: schema_of::<BrowserActionVars>,
        },
    ]
}

/// Referenztabelle aller Variablen (Markdown) für `docs/generated/policy-context.md`.
pub fn reference_markdown() -> String {
    use std::fmt::Write as _;
    let mut out = String::from(
        "<!-- Generiert von `cargo xtask codegen` aus beton_policy::vars. Nicht von Hand ändern. -->\n\n\
         # Policy-Kontext (CEL-Variablen)\n\n\
         Variablen, die CEL-Ausdrücke in Policies sehen, mit Typ und Phasen. Spezifikation: \
         POL-004 in [03-policies.md](../spec/03-policies.md#pol-004--kontext-variablen-je-phase). \
         Nicht gesetzte Felder sind `null`; eine Variable außerhalb ihrer Phasen ist ein \
         Ladefehler.\n\n\
         | Variable | Typ | Phasen | Beschreibung |\n| --- | --- | --- | --- |\n",
    );
    for row in crate::types::DECLS.rows() {
        let phases = if row.phases.is_empty() {
            "alle".to_owned()
        } else {
            row.phases
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} |",
            row.path,
            row.ty,
            phases,
            row.doc.replace('|', "\\|").replace('\n', " ")
        );
    }
    out
}
