//! `backend: none` (SBX-001): ein eigener Provider ohne Isolation, der ausdrücklich gewählt
//! werden muss. Bis SBX-006 (WP-49) starten Harness- und Tool-Prozesse über ihn; das Verhalten
//! entspricht dem bisherigen direkten Start.

use crate::caps::{Cap, CapSet};
use crate::child::{self, SandboxedChild};
use crate::command::CommandSpec;
use crate::error::SandboxError;
use crate::provider::{ProbeReport, ProviderId, SandboxProvider};
use crate::resolve::ResolvedPolicy;

/// Keine Isolation.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoneProvider;

/// Grund für jede fehlende Capability.
pub const REASON: &str = "backend: none isoliert nicht";

impl SandboxProvider for NoneProvider {
    fn id(&self) -> ProviderId {
        ProviderId::None
    }

    fn probe(&self) -> ProbeReport {
        ProbeReport {
            id: ProviderId::None,
            available: true,
            caps: CapSet::new(),
            degraded_reasons: Cap::ALL.iter().map(|c| (*c, REASON.to_owned())).collect(),
            unavailable_reason: None,
            version: String::new(),
        }
    }

    fn spawn(&self, policy: &ResolvedPolicy, cmd: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        todo!("SBX-001")
    }
}
