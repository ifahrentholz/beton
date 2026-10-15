//! Harness-Adapter-Trait, Capabilities, Registry, Prozess-Supervision, Fake-Harness und
//! Golden-Transcript-Framework (HAR-001, HAR-002, HAR-003, HAR-025, HAR-026), gemeinsamer
//! JSON-RPC-Transport für Codex und ACP sowie die Harness-Contract-Suite (QA-016).
//!
//! Spec: `docs/spec/01-harnesses.md`. Vendor-Adapter (Claude Code, Codex, ACP) liegen in
//! eigenen Crates und implementieren [`HarnessAdapter`].

mod adapter;
mod capabilities;
pub mod contract;
pub mod fake;
pub mod golden;
pub mod handover;
pub mod hook;
mod id;
pub mod import;
pub mod jsonrpc;
pub mod permission;
pub mod process;
pub mod registry;
pub mod scenario;

pub use crate::adapter::{
    AdapterContext, AllowAll, AuthStatus, AutoCompaction, CompactionSettings,
    DEFAULT_COMPACTION_THRESHOLD, DenyAll, ExitInfo, Gate, GateDecision, GateOrigin, GateRequest,
    HarnessAdapter, HarnessError, HarnessSession, HostEnv, InputAttachment, InvalidPermissionMode,
    McpInjection, McpLaunch, Mode, ModelGateDecision, ModelGateRequest, NormalizedEvent,
    OneShotReply, OneShotRequest, PermissionMode, ProbeReport, RebuildRequest, ResultGateRequest,
    SessionSpec, Shutdown, SwitchOutcome, Transport, UserInput, VENDOR_DIR_VARS,
    prefix_instructions,
};
pub use crate::capabilities::{
    Action, ApprovalMechanism, Capabilities, CapabilityUnsupported, CompactionSupport,
    DEFAULT_CONTEXT_WINDOW, EFFORT_LEVELS, ForkHistory, InstructionsDelivery, ModelRequestGate,
    ResumeSupport, Subagents, SwitchSupport, ToolCallGate, UsageReporting, map_effort,
};
pub use crate::id::{HarnessId, InvalidHarnessId};
pub use crate::permission::{SandboxStatus, check_permission_mode};

/// Kanonische Tool-Klasse (POL-005) aus der Abbildungstabelle von `beton-policy` – die einzige
/// Quelle für `tool.call.requested.kind` und `GateRequest::kind` aller Adapter (#182).
/// `harness` ist die Familie (`claude`, `codex`, `acp`, `direct`); unbekannt → `other`.
pub fn tool_kind(
    harness: &str,
    native_name: &str,
    acp_kind: Option<&str>,
    mcp_server: Option<&str>,
) -> String {
    beton_policy::normalize::tool_map()
        .ok()
        .and_then(|m| {
            m.kind_of(
                beton_policy::normalize::harness_family(harness),
                native_name,
                acp_kind,
                mcp_server,
            )
        })
        .unwrap_or("other")
        .to_owned()
}
