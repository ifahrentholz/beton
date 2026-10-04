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
mod id;
pub mod import;
pub mod jsonrpc;
pub mod process;
pub mod registry;
pub mod scenario;

pub use crate::adapter::{
    AdapterContext, AllowAll, AuthStatus, DenyAll, ExitInfo, Gate, GateDecision, GateRequest,
    HarnessAdapter, HarnessError, HarnessSession, HostEnv, McpInjection, McpLaunch, Mode,
    NormalizedEvent, PermissionMode, ProbeReport, RebuildRequest, SessionSpec, Shutdown,
    SwitchOutcome, Transport, UserInput, VENDOR_DIR_VARS, prefix_instructions,
};
pub use crate::capabilities::{
    Action, ApprovalMechanism, Capabilities, CapabilityUnsupported, CompactionSupport,
    DEFAULT_CONTEXT_WINDOW, EFFORT_LEVELS, ForkHistory, InstructionsDelivery, ResumeSupport,
    Subagents, SwitchSupport, ToolCallGate, UsageReporting, map_effort,
};
pub use crate::id::{HarnessId, InvalidHarnessId};
