//! Event-Envelope und Event-Katalog v1 (PROTO-001, PROTO-002).
//!
//! Der Katalog in `docs/spec/06-data-sync-protocol.md` ist verbindlich; das Makro
//! `catalog!` unten ist seine einzige Abbildung in Code. Ein Test vergleicht beide.
//! Jeder Typ existiert ab M0 – auch wenn sein Producer erst in einem späteren Meilenstein kommt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::id::{
    AgentId, ApprovalId, CommentId, DeviceId, EventId, HostId, NodeId, PrincipalId, ProjectId,
    RunnerId, ScheduleId, SessionId, TimerId, TurnId,
};
use crate::time::Timestamp;

/// Aktuelle Version der Envelope (`v`).
pub const ENVELOPE_VERSION: u8 = 1;

/// Ein Event im Session-Log bzw. auf dem Draht.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct Event {
    /// Envelope-Version, derzeit immer 1.
    pub v: u8,
    pub id: EventId,
    pub session_id: SessionId,
    /// Lückenlose Position im Session-Log. Transiente Events tragen die letzte dauerhafte `seq`.
    pub seq: u64,
    pub ts: Timestamp,
    pub actor: Actor,
    /// `type` und `payload`, bei ausgelagerten Payloads `type` und `payload_ref`.
    #[serde(flatten)]
    pub body: EventBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub causation_id: Option<EventId>,
    /// Original-Payload des Harness (optional, vor Persistenz redigiert), byte-genau erhalten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    #[ts(as = "Option<Value>", optional)]
    pub raw: Option<RawJson>,
    /// Nur bei transienten Events: `true`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub transient: bool,
    /// Nur bei transienten Events: pro Session und Epoch monotone Folgenummer (PROTO-003).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tseq: Option<u64>,
}

impl Event {
    /// Neues dauerhaftes Event; `seq` vergibt der Home-Knoten beim Anhängen.
    pub fn new(session_id: SessionId, seq: u64, actor: Actor, body: EventPayload) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id: EventId::new(),
            session_id,
            seq,
            ts: Timestamp::now(),
            actor,
            body: EventBody::Inline(body),
            turn_id: None,
            causation_id: None,
            raw: None,
            transient: false,
            tseq: None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        self.body.event_type().as_str()
    }

    /// Die Nutzlast, sofern sie nicht in den Blob-Store ausgelagert ist.
    pub fn payload(&self) -> Option<&EventPayload> {
        match &self.body {
            EventBody::Inline(p) => Some(p),
            EventBody::Offloaded(_) => None,
        }
    }
}

/// JSON-Text, der unverändert (byte-genau) erhalten bleibt, z. B. die Original-Zeile eines
/// Harness (HAR-001 AC3). Vergleiche erfolgen auf dem Text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawJson(Box<serde_json::value::RawValue>);

impl RawJson {
    /// Übernimmt gültigen JSON-Text unverändert.
    pub fn from_string(json: String) -> Result<Self, serde_json::Error> {
        serde_json::value::RawValue::from_string(json).map(Self)
    }

    /// Kompakte Serialisierung eines Werts.
    pub fn from_value(value: &Value) -> Self {
        // Ein `Value` ist immer gültiges JSON.
        Self::from_string(value.to_string()).unwrap_or_else(|_| Self::null())
    }

    fn null() -> Self {
        Self(serde_json::value::RawValue::NULL.to_owned())
    }

    pub fn get(&self) -> &str {
        self.0.get()
    }

    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_str(self.get())
    }
}

impl PartialEq for RawJson {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

/// Grenze, ab der Payloads als Blob ausgelagert werden (PROTO-001 AC4): 64 KiB serialisiert.
pub const PAYLOAD_INLINE_LIMIT: usize = 64 * 1024;

/// Nutzlast eines Events: direkt enthalten oder als Blob ausgelagert (PROTO-001 AC4).
///
/// Auf dem Draht unterscheiden sich beide Formen nur durch `payload` bzw. `payload_ref`;
/// den Inhalt einer ausgelagerten Nutzlast holen Clients über die Blob-API der Session.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema, TS)]
#[serde(untagged)]
// Inline ist der Normalfall; ein Box dort kostete jede Event-Allokation.
#[allow(clippy::large_enum_variant)]
pub enum EventBody {
    Inline(EventPayload),
    Offloaded(OffloadedPayload),
}

impl EventBody {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::Inline(p) => p.event_type(),
            Self::Offloaded(o) => o.event_type,
        }
    }
}

impl<'de> Deserialize<'de> for EventBody {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Selbst geschrieben statt `untagged`, damit Fehler in der Nutzlast lesbar bleiben.
        let map = serde_json::Map::<String, Value>::deserialize(d)?;
        let value = Value::Object(map);
        if value.get("payload_ref").is_some() {
            serde_json::from_value(value)
                .map(Self::Offloaded)
                .map_err(serde::de::Error::custom)
        } else {
            serde_json::from_value(value)
                .map(Self::Inline)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// Platzhalter für eine in den Blob-Store ausgelagerte Nutzlast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct OffloadedPayload {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub payload_ref: BlobRef,
}

/// Inhaltsadresse eines Blobs: `sha256:<64 Hex-Zeichen>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobRef(String);

impl BlobRef {
    const PREFIX: &'static str = "sha256:";

    /// Aus dem SHA-256-Digest als Kleinbuchstaben-Hex.
    pub fn from_hex(hex: &str) -> Result<Self, InvalidBlobRef> {
        format!("{}{hex}", Self::PREFIX).parse()
    }

    /// Der Digest als Hex (ohne Präfix).
    pub fn hex(&self) -> &str {
        &self.0[Self::PREFIX.len()..]
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` ist keine Blob-Referenz der Form sha256:<64 Hex-Zeichen>")]
pub struct InvalidBlobRef(String);

impl std::str::FromStr for BlobRef {
    type Err = InvalidBlobRef;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix(Self::PREFIX)
            .ok_or_else(|| InvalidBlobRef(s.to_owned()))?;
        if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidBlobRef(s.to_owned()))
        }
    }
}

impl std::fmt::Display for BlobRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for BlobRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BlobRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = std::borrow::Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for BlobRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BlobRef".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" })
    }
}

impl TS for BlobRef {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &ts_rs::Config) -> String {
        "`sha256:${string}`".to_owned()
    }
    fn inline(cfg: &ts_rs::Config) -> String {
        <Self as TS>::name(cfg)
    }
}

/// Wer ein Event ausgelöst hat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    User {
        id: PrincipalId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        device_id: Option<DeviceId>,
    },
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        id: Option<AgentId>,
        /// Harness-ID, z. B. `claude`, `codex`, `acp:gemini-cli`.
        harness: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        agent_ref: Option<String>,
    },
    System {
        component: SystemComponent,
    },
}

impl Default for Actor {
    fn default() -> Self {
        Self::System {
            component: SystemComponent::default(),
        }
    }
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $(#[$fm:meta])* $first:ident $(, $(#[$vm:meta])* $rest:ident)* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema, TS)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(#[$fm])*
            #[default]
            $first,
            $($(#[$vm])* $rest,)*
        }
    };
}

string_enum!(SystemComponent {
    Server,
    Policy,
    Sandbox,
    Proxy,
    Scheduler,
    Sync,
    Runner,
    /// Akteur importierter Events, deren User auf dieser Instanz unbekannt ist (DATA-010).
    Import
});

/// Ob ein Event ins Log geschrieben wird (PROTO-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    /// Wird ins Session-Log geschrieben und repliziert.
    Durable,
    /// Nur im Ringpuffer des Home-Knotens und auf dem Draht.
    Transient,
}

/// Leere Nutzlast für Events ohne Felder.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, TS)]
pub struct Empty {}

// ---------------------------------------------------------------------------
// Aufzählungen
// ---------------------------------------------------------------------------

string_enum!(SessionKind {
    Main,
    SideChat,
    Subagent,
    Async
});
string_enum!(SessionTrigger {
    User,
    Api,
    Schedule,
    Timer,
    Spawn,
    Webhook
});
string_enum!(SessionStatus {
    Starting,
    Idle,
    Running,
    WaitingApproval,
    Paused,
    Stopped,
    Failed
});
string_enum!(ResumeMode { Native, Handover });
string_enum!(SettingsMechanism { Live, Restart });
string_enum!(TitleSource {
    Generated,
    User,
    Harness
});
string_enum!(ForkReason {
    User,
    SideChat,
    Divergence
});
string_enum!(HistoryMode {
    Native,
    Rebuild,
    Preamble
});
string_enum!(OwnershipMode { Planned, Forced });
string_enum!(MessageRole { User, Assistant });
string_enum!(ToolSource {
    Harness,
    BetonMcp,
    Acp
});
string_enum!(OutputStream { Stdout, Stderr });
string_enum!(ToolStatus {
    Ok,
    Error,
    Denied,
    Cancelled
});
string_enum!(ApprovalKind {
    Tool,
    Policy,
    Browser,
    Budget,
    Question
});
string_enum!(TimeoutAction { Deny, Allow, Abort });
string_enum!(ApprovalDecision {
    Allow,
    Deny,
    Abort,
    Answer
});
string_enum!(ResolvedVia {
    User,
    Timeout,
    Policy,
    System
});
string_enum!(RememberScope { None, Session });
string_enum!(PolicyOutcome { Allow, Deny, Ask });
string_enum!(BudgetAction { Ask, Deny });
string_enum!(CostSource {
    Reported,
    Estimated,
    Subscription
});
string_enum!(AuthSource {
    VendorCli,
    ApiKey,
    Gateway,
    None
});
string_enum!(ContextSource { Harness, Estimated });
string_enum!(FsChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed
});
string_enum!(CrOrigin {
    Created,
    Attached,
    Inferred
});
string_enum!(ConflictKind {
    Content,
    DeleteModify,
    Rename,
    Binary
});
string_enum!(ConflictSource { Local, Provider });
string_enum!(ConflictPhase {
    Started,
    HunkResolved,
    Completed,
    Aborted
});
string_enum!(MergeStrategy { Merge, Rebase });
string_enum!(ConflictResolutionKind {
    Ours,
    Theirs,
    Both,
    Edited,
    Agent
});
string_enum!(TerminalKind {
    HarnessPty,
    User,
    Tool
});
string_enum!(BrowserMode { Embedded, Window });
string_enum!(BrowserCloseReason {
    Stopped,
    Idle,
    Crashed
});
string_enum!(SandboxStage { Harness, Tools });
string_enum!(ViolationKind {
    Fs,
    Net,
    Syscall,
    Limit,
    ProxyDown
});
string_enum!(EgressSource {
    Tools,
    Harness,
    Browser
});
string_enum!(NoticeLevel { Info, Warn });

// ---------------------------------------------------------------------------
// Nutzlasten
// ---------------------------------------------------------------------------

macro_rules! payload {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, TS)]
        pub struct $name {
            $($(#[$fm])* pub $field: $ty,)*
        }
    };
}

payload!(SessionCreated {
    owner: PrincipalId,
    kind: SessionKind,
    harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] model: Option<String>,
    cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] parent_session_id: Option<SessionId>,
    trigger: SessionTrigger,
    /// Harness-spezifische Startoptionen aus `POST /v1/sessions` (z. B. Fake-Szenario).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[ts(optional, as = "Option<Value>")]
    harness_opts: Value,
});
payload!(SessionStarted {
    runner_id: RunnerId,
    host_id: HostId,
    harness: String,
    harness_version: String,
    capabilities: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] harness_session_ref: Option<String>,
});
payload!(SessionStatusChanged {
    status: SessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<String>,
});
payload!(SessionResumed { mode: ResumeMode });
payload!(SessionSettingsChanged {
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] requested_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] mechanism: Option<SettingsMechanism>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] effective_from_turn: Option<TurnId>,
});
payload!(SessionTitleChanged {
    title: String,
    source: TitleSource
});
payload!(SessionForked {
    from_session: SessionId,
    at_seq: u64,
    reason: ForkReason,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] from_harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] history_mode: Option<HistoryMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] fallback_reason: Option<String>,
});
payload!(SessionForkCreated {
    child: SessionId,
    at_seq: u64,
    reason: ForkReason
});
payload!(SessionImported {
    source: String,
    vendor_session_id: String,
    imported_at: Timestamp
});
payload!(SessionOwnershipChanged {
    home_node: NodeId,
    epoch: u64,
    mode: OwnershipMode
});
payload!(SessionStartDenied {
    reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] rules: Option<Vec<String>>,
});

payload!(HarnessReady {
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] harness_session_ref: Option<String>,
    tools: Vec<String>,
    mcp_servers: Vec<String>,
});
payload!(HarnessExited {
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] signal: Option<String>,
    stderr_tail: String,
});
payload!(HarnessAuthRequired {
    harness: String,
    hint: String
});
payload!(HarnessIncompatible {
    detected_version: String,
    expected_range: String
});
payload!(McpServerFailed {
    name: String,
    error: String
});
payload!(RunnerStatus {
    runner_id: RunnerId,
    from: String,
    to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<String>,
});

// Ein eingereihter Input (SES-004). `text` ist vollständig, damit jeder Client mit `drive`
// ihn bearbeiten kann.
payload!(QueueItem {
    id: String,
    author: PrincipalId,
    text: String,
    attachments: Vec<String>,
    created_at: Timestamp,
});
// Vollständiger Queue-Stand nach jeder Änderung (SES-004); `paused` nach einem Interrupt
// (SES-005 AC2).
payload!(QueueUpdated { items: Vec<QueueItem>, paused: bool });
payload!(TurnStarted {
    turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] input_id: Option<String>,
    author: PrincipalId,
});
payload!(TurnCompleted {
    turn_id: TurnId,
    stop_reason: String,
    usage_summary: Value
});
payload!(TurnFailed {
    turn_id: TurnId,
    problem: Value
});
payload!(TurnInterrupted {
    turn_id: TurnId,
    by: PrincipalId,
    /// Warum, wenn nicht auf Wunsch eines Menschen: `timed_out` (`executor.timeout`, AGT-004).
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<String>,
});

payload!(
    /// Text-Delta einer Nachricht oder Überlegung. Mit `snapshot: true` enthält `text` den
    /// gesamten bisherigen Text (Snapshot nach Reconnect, PROTO-003).
    TextDelta {
        message_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] #[ts(as = "Option<bool>", optional)] snapshot: bool,
    }
);
payload!(MessageCompleted {
    message_id: String,
    role: MessageRole,
    content: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] author: Option<PrincipalId>,
});
payload!(ReasoningCompleted {
    message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] summary: Option<String>,
    redacted: bool,
});

payload!(ToolCallRequested {
    call_id: String,
    tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] mcp_server: Option<String>,
    args: Value,
    source: ToolSource,
    /// Tool-Call eines Vendor-Sub-Agents: der auslösende Tool-Call (z. B. Claude `Task`), unter
    /// dem dieser verschachtelt ist (HAR-023).
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] parent_call_id: Option<String>,
});
payload!(ToolCallStarted {
    call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] sandbox_stage: Option<SandboxStage>,
    /// Ausgeführte Argumente, wenn das Gate sie geändert hat (HAR-005 AC2).
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] args: Option<Value>,
});
payload!(ToolCallOutputDelta {
    call_id: String,
    stream: OutputStream,
    text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    snapshot: bool,
});
payload!(ToolCallCompleted {
    call_id: String,
    status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] result_ref: Option<String>,
    duration_ms: u64,
});

payload!(ApprovalRequested {
    approval_id: ApprovalId,
    kind: ApprovalKind,
    subject: Value,
    options: Vec<String>,
    expires_at: Timestamp,
    on_timeout: TimeoutAction,
});
payload!(ApprovalResolved {
    approval_id: ApprovalId,
    decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] answer: Option<String>,
    actor: Actor,
    via: ResolvedVia,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] remember: Option<RememberScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] on_timeout_applied: Option<TimeoutAction>,
});
payload!(PolicyDecision {
    decision_id: String,
    phase: String,
    outcome: PolicyOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] modified: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] approval_id: Option<ApprovalId>,
    matched: Vec<Value>,
    defaults_applied: Vec<Value>,
    conflicts: Vec<Value>,
    errors: Vec<Value>,
    grants_used: Vec<Value>,
    enforcement: String,
    policy_set_hash: String,
    eval_us: u64,
});
payload!(PolicySetChanged {
    scope: String,
    set: String,
    version: String
});
payload!(PolicySetInvalid {
    file: String,
    line: u32,
    error: String
});
payload!(PolicyStateChanged {
    scope: String,
    key: String,
    op: String,
    value: Value
});
payload!(PolicyRuleIssue { rule_ids: Vec<String>, reason: String });
payload!(BudgetExhausted {
    scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] lease_id: Option<String>,
    action: BudgetAction,
});
payload!(BudgetLeaseOverdrawn {
    lease_id: String,
    overrun_micro: i64
});

payload!(CostDelta {
    harness: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] cost_micro: Option<i64>,
    currency: String,
    source: CostSource,
    auth_source: AuthSource,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] purpose: Option<String>,
});
payload!(UsageSubscription {
    vendor: String,
    window: String,
    used_pct: f64,
    resets_at: Timestamp
});
payload!(ContextUsage {
    used_tokens: u64,
    window_tokens: u64,
    source: ContextSource
});
payload!(Compaction {
    before_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] after_tokens: Option<u64>,
});

payload!(FsChange {
    path: String,
    change: FsChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] from: Option<String>,
});
payload!(FsChanged { changes: Vec<FsChange>, source: String });
payload!(GitWorktreeCreated {
    path: String,
    branch: String,
    base: String,
    base_sha: String
});
payload!(GitCommitCreated {
    sha: String,
    branch: String,
    subject: String
});
payload!(CrLink {
    url: String,
    provider: String,
    number: u64,
    origin: CrOrigin
});
payload!(ConflictFile {
    path: String,
    kind: ConflictKind
});
payload!(GitConflictsDetected {
    base: String,
    base_sha: String,
    head_sha: String,
    files: Vec<ConflictFile>,
    source: ConflictSource,
});
payload!(GitConflictResolution {
    phase: ConflictPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] strategy: Option<MergeStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] hunk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] resolution: Option<ConflictResolutionKind>,
    actor: Actor,
});

payload!(TerminalOpened {
    terminal_id: String,
    channel_id: u32,
    kind: TerminalKind,
    cmd: String,
    cols: u16,
    rows: u16,
});
payload!(
    /// Metadaten zum Binärkanal; die Bytes laufen über den Kanal (PROTO-007).
    TerminalOutput { terminal_id: String, channel_id: u32 }
);
payload!(TerminalSnapshot {
    terminal_id: String,
    blob_ref: String,
    cols: u16,
    rows: u16,
    cursor: Value
});
payload!(TerminalClosed {
    terminal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] exit_code: Option<i32>,
});

payload!(BrowserOpened {
    browser_id: String,
    channel_id: u32,
    mode: BrowserMode
});
payload!(
    /// Metadaten zum Binärkanal; Bild und Metadaten laufen über den Kanal (PROTO-007).
    BrowserFrame { browser_id: String, channel_id: u32 }
);
payload!(BrowserNavigated {
    browser_id: String,
    url: String,
    title: String
});
payload!(BrowserAction {
    browser_id: String,
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] target_ref: Option<String>,
    decision: PolicyOutcome,
});
payload!(BrowserSnapshot {
    browser_id: String,
    screenshot_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] a11y_ref: Option<String>,
});
payload!(BrowserPicked {
    browser_id: String,
    selector: String,
    outer_html_ref: String,
    styles: Value,
    bbox: Value,
    screenshot_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] source_location: Option<Value>,
});
payload!(BrowserClosed {
    browser_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<BrowserCloseReason>,
});
payload!(BrowserDevserverDetected {
    url: String,
    pid: u32,
    source: String
});

payload!(SandboxStarted {
    stage: SandboxStage,
    backend: String,
    caps: Value,
    degraded: bool
});
payload!(SandboxViolation {
    stage: SandboxStage,
    kind: ViolationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] target: Option<String>,
    suppressed_count: u32,
});
payload!(EgressBlocked {
    method: String,
    host: String,
    path: String,
    reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] rule_hint: Option<String>,
    source: EgressSource,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] tool_call_id: Option<String>,
});
payload!(EgressHost {
    host: String,
    requests: u64,
    bytes: u64
});
payload!(EgressSummary { window_s: u32, hosts: Vec<EgressHost> });

payload!(AgentSpawned {
    child_session_id: SessionId,
    agent_ref: String,
    harness: String,
    r#async: bool
});
payload!(AgentCompleted {
    child_session_id: SessionId,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] cost_micro: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] cancel_reason: Option<String>,
});
payload!(AgentResolved {
    name: String,
    version: String,
    source: String,
    hash: String,
    blob_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] overrides: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] params: Option<Value>,
});
payload!(AgentMessage {
    from_session: SessionId,
    to_session: SessionId,
    text: String
});
payload!(TimerEvent {
    timer_id: TimerId,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] fire_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] late_by_s: Option<u64>,
});
payload!(AsyncRunEvent {
    run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] trigger: Option<SessionTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] cost_micro: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] duration_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] summary: Option<String>,
});
payload!(ScheduleEvent {
    schedule_id: ScheduleId,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] scheduled_for: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] count: Option<u32>,
});

payload!(CommentChange {
    comment_id: CommentId,
    thread_id: String,
    anchor: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] body: Option<String>,
    author: PrincipalId,
});
payload!(CommentAddressed { comment_ids: Vec<CommentId>, turn_id: TurnId });
payload!(ShareChange {
    principal: PrincipalId,
    role: String,
    by: PrincipalId
});
payload!(PresenceUser { user_id: PrincipalId, devices: Vec<DeviceId>, typing: bool });
payload!(PresenceUpdated { users: Vec<PresenceUser> });

payload!(EventRedacted {
    target_seq: u64,
    by: PrincipalId,
    reason: String
});
payload!(HarnessUnmapped { raw: Value });
payload!(Notice {
    level: NoticeLevel,
    text: String
});
payload!(ErrorEvent { problem: Value });

// ---------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------

macro_rules! catalog {
    ($($variant:ident = $name:literal, $persistence:ident, $payload:ty;)*) => {
        /// Typ und Nutzlast eines Events: `{"type": "…", "payload": {…}}`.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
        #[serde(tag = "type", content = "payload")]
        pub enum EventPayload {
            $(#[serde(rename = $name)] $variant($payload),)*
        }

        /// Typname eines Events ohne Nutzlast, z. B. für ausgelagerte Payloads.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
        pub enum EventType {
            $(#[serde(rename = $name)] $variant,)*
        }

        impl EventType {
            /// Typname laut Katalog, z. B. `tool.call.requested`.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name,)* }
            }

            pub fn persistence(self) -> Persistence {
                match self { $(Self::$variant => Persistence::$persistence,)* }
            }

            pub fn parse(name: &str) -> Option<Self> {
                match name { $($name => Some(Self::$variant),)* _ => None }
            }
        }

        impl EventPayload {
            pub fn event_type(&self) -> EventType {
                match self { $(Self::$variant(_) => EventType::$variant,)* }
            }

            /// Typname laut Katalog, z. B. `tool.call.requested`.
            pub fn type_name(&self) -> &'static str {
                self.event_type().as_str()
            }

            pub fn persistence(&self) -> Persistence {
                self.event_type().persistence()
            }

            pub fn is_transient(&self) -> bool {
                self.persistence() == Persistence::Transient
            }

            /// Je Katalog-Typ eine Beispiel-Nutzlast mit Standardwerten (für Tests und Doku).
            pub fn examples() -> Vec<Self> {
                vec![$(Self::$variant(<$payload>::default()),)*]
            }
        }

        /// Alle Event-Typen v1 mit ihrer Persistenz, in Katalog-Reihenfolge.
        pub const CATALOG: &[(&str, Persistence)] = &[$(($name, Persistence::$persistence),)*];
    };
}

catalog! {
    SessionCreated = "session.created", Durable, SessionCreated;
    SessionStarted = "session.started", Durable, SessionStarted;
    SessionStatus = "session.status", Durable, SessionStatusChanged;
    SessionResumed = "session.resumed", Durable, SessionResumed;
    SessionSettingsChanged = "session.settings_changed", Durable, SessionSettingsChanged;
    SessionTitleChanged = "session.title_changed", Durable, SessionTitleChanged;
    SessionArchived = "session.archived", Durable, Empty;
    SessionUnarchived = "session.unarchived", Durable, Empty;
    SessionForked = "session.forked", Durable, SessionForked;
    SessionForkCreated = "session.fork_created", Durable, SessionForkCreated;
    SessionImported = "session.imported", Durable, SessionImported;
    SessionOwnershipChanged = "session.ownership_changed", Durable, SessionOwnershipChanged;
    SessionStartDenied = "session.start_denied", Durable, SessionStartDenied;
    HarnessReady = "harness.ready", Durable, HarnessReady;
    HarnessExited = "harness.exited", Durable, HarnessExited;
    HarnessAuthRequired = "harness.auth_required", Durable, HarnessAuthRequired;
    HarnessIncompatible = "harness.incompatible", Durable, HarnessIncompatible;
    McpServerFailed = "mcp.server_failed", Durable, McpServerFailed;
    RunnerStatus = "runner.status", Durable, RunnerStatus;
    QueueUpdated = "queue.updated", Durable, QueueUpdated;
    TurnStarted = "turn.started", Durable, TurnStarted;
    TurnCompleted = "turn.completed", Durable, TurnCompleted;
    TurnFailed = "turn.failed", Durable, TurnFailed;
    TurnInterrupted = "turn.interrupted", Durable, TurnInterrupted;
    MessageDelta = "message.delta", Transient, TextDelta;
    MessageCompleted = "message.completed", Durable, MessageCompleted;
    ReasoningDelta = "reasoning.delta", Transient, TextDelta;
    ReasoningCompleted = "reasoning.completed", Durable, ReasoningCompleted;
    ToolCallRequested = "tool.call.requested", Durable, ToolCallRequested;
    ToolCallStarted = "tool.call.started", Durable, ToolCallStarted;
    ToolCallOutputDelta = "tool.call.output.delta", Transient, ToolCallOutputDelta;
    ToolCallCompleted = "tool.call.completed", Durable, ToolCallCompleted;
    ApprovalRequested = "approval.requested", Durable, ApprovalRequested;
    ApprovalResolved = "approval.resolved", Durable, ApprovalResolved;
    PolicyDecision = "policy.decision", Durable, PolicyDecision;
    PolicySetChanged = "policy.set_changed", Durable, PolicySetChanged;
    PolicySetInvalid = "policy.set_invalid", Durable, PolicySetInvalid;
    PolicyStateChanged = "policy.state_changed", Durable, PolicyStateChanged;
    PolicyEnforcementDegraded = "policy.enforcement_degraded", Durable, PolicyRuleIssue;
    PolicyRuleInapplicable = "policy.rule_inapplicable", Durable, PolicyRuleIssue;
    BudgetExhausted = "budget.exhausted", Durable, BudgetExhausted;
    BudgetLeaseOverdrawn = "budget.lease_overdrawn", Durable, BudgetLeaseOverdrawn;
    CostDelta = "cost.delta", Durable, CostDelta;
    UsageSubscription = "usage.subscription", Durable, UsageSubscription;
    ContextUsage = "context.usage", Durable, ContextUsage;
    CompactionStarted = "compaction.started", Durable, Compaction;
    CompactionCompleted = "compaction.completed", Durable, Compaction;
    FsChanged = "fs.changed", Durable, FsChanged;
    GitWorktreeCreated = "git.worktree_created", Durable, GitWorktreeCreated;
    GitCommitCreated = "git.commit_created", Durable, GitCommitCreated;
    CrLinked = "cr.linked", Durable, CrLink;
    CrUnlinked = "cr.unlinked", Durable, CrLink;
    GitConflictsDetected = "git.conflicts_detected", Durable, GitConflictsDetected;
    GitConflictResolution = "git.conflict_resolution", Durable, GitConflictResolution;
    TerminalOpened = "terminal.opened", Durable, TerminalOpened;
    TerminalOutput = "terminal.output", Transient, TerminalOutput;
    TerminalSnapshot = "terminal.snapshot", Durable, TerminalSnapshot;
    TerminalClosed = "terminal.closed", Durable, TerminalClosed;
    BrowserOpened = "browser.opened", Durable, BrowserOpened;
    BrowserFrame = "browser.frame", Transient, BrowserFrame;
    BrowserNavigated = "browser.navigated", Durable, BrowserNavigated;
    BrowserAction = "browser.action", Durable, BrowserAction;
    BrowserSnapshot = "browser.snapshot", Durable, BrowserSnapshot;
    BrowserPicked = "browser.picked", Durable, BrowserPicked;
    BrowserClosed = "browser.closed", Durable, BrowserClosed;
    BrowserDevserverDetected = "browser.devserver.detected", Durable, BrowserDevserverDetected;
    SandboxStarted = "sandbox.started", Durable, SandboxStarted;
    SandboxViolation = "sandbox.violation", Durable, SandboxViolation;
    EgressBlocked = "egress.blocked", Durable, EgressBlocked;
    EgressSummary = "egress.summary", Durable, EgressSummary;
    AgentSpawned = "agent.spawned", Durable, AgentSpawned;
    AgentCompleted = "agent.completed", Durable, AgentCompleted;
    AgentResolved = "agent.resolved", Durable, AgentResolved;
    AgentMessage = "agent.message", Durable, AgentMessage;
    TimerSet = "timer.set", Durable, TimerEvent;
    TimerFired = "timer.fired", Durable, TimerEvent;
    TimerCancelled = "timer.cancelled", Durable, TimerEvent;
    AsyncRunQueued = "async.run.queued", Durable, AsyncRunEvent;
    AsyncRunStarted = "async.run.started", Durable, AsyncRunEvent;
    AsyncRunPaused = "async.run.paused", Durable, AsyncRunEvent;
    AsyncRunResumed = "async.run.resumed", Durable, AsyncRunEvent;
    AsyncRunFinished = "async.run.finished", Durable, AsyncRunEvent;
    ScheduleCreated = "schedule.created", Durable, ScheduleEvent;
    ScheduleUpdated = "schedule.updated", Durable, ScheduleEvent;
    ScheduleDeleted = "schedule.deleted", Durable, ScheduleEvent;
    ScheduleFired = "schedule.fired", Durable, ScheduleEvent;
    ScheduleSkipped = "schedule.skipped", Durable, ScheduleEvent;
    CommentAdded = "comment.added", Durable, CommentChange;
    CommentUpdated = "comment.updated", Durable, CommentChange;
    CommentResolved = "comment.resolved", Durable, CommentChange;
    CommentDeleted = "comment.deleted", Durable, CommentChange;
    CommentAddressed = "comment.addressed", Durable, CommentAddressed;
    ShareGranted = "share.granted", Durable, ShareChange;
    ShareChanged = "share.changed", Durable, ShareChange;
    ShareRevoked = "share.revoked", Durable, ShareChange;
    PresenceUpdated = "presence.updated", Transient, PresenceUpdated;
    EventRedacted = "event.redacted", Durable, EventRedacted;
    HarnessUnmapped = "harness.unmapped", Durable, HarnessUnmapped;
    Notice = "notice", Durable, Notice;
    Error = "error", Durable, ErrorEvent;
}

impl Default for EventPayload {
    fn default() -> Self {
        Self::Notice(Notice::default())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use proptest::prelude::*;

    use super::*;

    fn envelope(body: EventPayload) -> Event {
        let mut e = Event::new(
            SessionId::new(),
            42,
            Actor::Agent {
                id: None,
                harness: "claude".into(),
                agent_ref: None,
            },
            body,
        );
        if e.payload().is_some_and(EventPayload::is_transient) {
            e.transient = true;
            e.tseq = Some(7);
        }
        e
    }

    fn roundtrip(e: &Event) -> Event {
        let json = serde_json::to_string(e).unwrap();
        serde_json::from_str(&json).unwrap_or_else(|err| panic!("{}: {err}\n{json}", e.type_name()))
    }

    #[test]
    fn proto_001_ac1_every_payload_roundtrips() {
        let examples = EventPayload::examples();
        assert_eq!(examples.len(), CATALOG.len());
        for body in examples {
            let e = envelope(body);
            assert_eq!(roundtrip(&e), e);
            let v = serde_json::to_value(&e).unwrap();
            assert_eq!(v["type"], e.type_name());
            assert!(v.get("payload").is_some(), "{} ohne payload", e.type_name());
        }
    }

    #[test]
    fn proto_001_envelope_matches_spec_example_shape() {
        let json = serde_json::json!({
            "v": 1,
            "id": "evt_01JB8Y3K6V9Q7W2N4R5T6Y7Z8A",
            "session_id": "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C",
            "seq": 42,
            "ts": "2026-10-03T12:00:00.123Z",
            "actor": { "kind": "agent", "id": "agt_01JB8Y2D0M3K4J5H6G7F8E9D0C", "harness": "claude" },
            "type": "tool.call.requested",
            "payload": { "call_id": "call_7", "tool": "bash", "args": { "command": "cargo test" }, "source": "harness" },
            "turn_id": "trn_01JB8Y2D0M3K4J5H6G7F8E9D0C",
        });
        let e: Event = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(e.type_name(), "tool.call.requested");
        assert_eq!(serde_json::to_value(&e).unwrap(), json);
    }

    #[test]
    fn proto_001_ac4_offloaded_payload_carries_ref_instead_of_payload() {
        let hash = "ab".repeat(32);
        let mut e = envelope(EventPayload::default());
        e.body = EventBody::Offloaded(OffloadedPayload {
            event_type: EventType::ToolCallCompleted,
            payload_ref: BlobRef::from_hex(&hash).unwrap(),
        });
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "tool.call.completed");
        assert_eq!(v["payload_ref"], format!("sha256:{hash}"));
        assert!(v.get("payload").is_none());
        assert_eq!(roundtrip(&e), e);
        assert_eq!(e.type_name(), "tool.call.completed");
        assert!(e.payload().is_none());

        assert!(BlobRef::from_hex("AB").is_err());
        assert!("md5:00".parse::<BlobRef>().is_err());
    }

    #[test]
    fn har_001_ac3_raw_keeps_original_bytes() {
        let original = r#"{"z":1,  "a":"\u00e4","n":1.50}"#;
        let mut e = envelope(EventPayload::default());
        e.raw = Some(RawJson::from_string(original.to_owned()).unwrap());
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(original), "{json}");
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back.raw.unwrap().get(), original);
    }

    #[test]
    fn invalid_payload_error_names_the_problem() {
        let mut v = serde_json::to_value(envelope(EventPayload::default())).unwrap();
        v["payload"]["level"] = "laut".into();
        let err = serde_json::from_value::<Event>(v).unwrap_err().to_string();
        assert!(err.contains("laut"), "{err}");
    }

    #[test]
    fn event_type_names_match_catalog() {
        for (name, persistence) in CATALOG {
            let t = EventType::parse(name).unwrap();
            assert_eq!(t.as_str(), *name);
            assert_eq!(t.persistence(), *persistence);
            assert_eq!(serde_json::to_value(t).unwrap(), *name);
        }
    }

    #[test]
    fn proto_002_ac3_system_actor_requires_component() {
        let ok: Actor =
            serde_json::from_value(serde_json::json!({"kind": "system", "component": "policy"}))
                .unwrap();
        assert_eq!(
            ok,
            Actor::System {
                component: SystemComponent::Policy
            }
        );
        assert!(serde_json::from_value::<Actor>(serde_json::json!({"kind": "system"})).is_err());
        assert!(
            serde_json::from_value::<Actor>(
                serde_json::json!({"kind": "system", "component": "kantine"})
            )
            .is_err()
        );
    }

    #[test]
    fn proto_002_ac3_event_without_actor_is_rejected() {
        let mut v = serde_json::to_value(envelope(EventPayload::default())).unwrap();
        v.as_object_mut().unwrap().remove("actor");
        assert!(serde_json::from_value::<Event>(v).is_err());
    }

    /// Liest die Katalogtabelle aus der Spec: Typname → D/T.
    fn spec_catalog() -> BTreeMap<String, char> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/spec/06-data-sync-protocol.md"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let start = text
            .find("### Event-Katalog v1")
            .expect("Katalog-Abschnitt fehlt");
        let section = &text[start..];
        let end = section[4..].find("\n### ").map_or(section.len(), |i| i + 4);
        let mut out = BTreeMap::new();
        for line in section[..end].lines().filter(|l| l.starts_with('|')) {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() < 5 || !cells[2].starts_with('`') {
                continue;
            }
            let dt = cells[3].chars().next().unwrap_or('?');
            let names: Vec<&str> = cells[2]
                .split('/')
                .map(|p| p.trim().trim_matches('`'))
                .collect();
            let first = names[0];
            for name in &names {
                let full = if let Some(suffix) = name.strip_prefix('.') {
                    let base = &first[..first.rfind('.').unwrap_or(first.len())];
                    format!("{base}.{suffix}")
                } else {
                    (*name).to_owned()
                };
                out.insert(full, dt);
            }
        }
        out
    }

    #[test]
    fn proto_002_ac1_catalog_matches_spec_table() {
        let spec = spec_catalog();
        let code: BTreeMap<String, char> = CATALOG
            .iter()
            .map(|(name, p)| {
                (
                    (*name).to_owned(),
                    if *p == Persistence::Durable { 'D' } else { 'T' },
                )
            })
            .collect();
        let spec_names: BTreeSet<_> = spec.keys().collect();
        let code_names: BTreeSet<_> = code.keys().collect();
        assert_eq!(
            spec_names.difference(&code_names).collect::<Vec<_>>(),
            Vec::<&&String>::new(),
            "In der Spec, aber nicht im Code"
        );
        assert_eq!(
            code_names.difference(&spec_names).collect::<Vec<_>>(),
            Vec::<&&String>::new(),
            "Im Code, aber nicht in der Spec"
        );
        assert_eq!(spec, code, "D/T-Einstufung weicht ab");
    }

    #[test]
    fn proto_002_ac1_generated_schema_lists_all_catalog_types() {
        let schema = serde_json::to_string(&schemars::schema_for!(Event)).unwrap();
        for (name, _) in CATALOG {
            assert!(
                schema.contains(&format!("\"{name}\"")),
                "{name} fehlt im Schema"
            );
        }
    }

    fn json_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::from),
            ".*".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::btree_map("[a-z_]{1,8}", inner, 0..4)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn proto_001_ac1_roundtrip_with_arbitrary_content(
            seq in any::<u64>(),
            text in ".*",
            args in json_value(),
            // `raw: null` und fehlendes `raw` sind gleichbedeutend.
            raw in proptest::option::of(json_value().prop_filter("nicht null", |v| !v.is_null())),
            ms in 0i64..4_102_444_800_000,
        ) {
            let ts = Timestamp::from(time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).unwrap());
            for body in [
                EventPayload::MessageDelta(TextDelta { message_id: "msg_1".into(), text: text.clone(), snapshot: false }),
                EventPayload::ToolCallRequested(ToolCallRequested { call_id: "c".into(), tool: "bash".into(), mcp_server: None, args: args.clone(), source: ToolSource::Harness, parent_call_id: None }),
                EventPayload::Notice(Notice { level: NoticeLevel::Warn, text: text.clone() }),
            ] {
                let mut e = envelope(body);
                e.seq = seq;
                e.ts = ts;
                e.raw = raw.as_ref().map(RawJson::from_value);
                prop_assert_eq!(roundtrip(&e), e);
            }
        }
    }
}
