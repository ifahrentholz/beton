//! Das Provider-Interface (SBX-001): `probe`, `resolve`, `spawn`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::caps::{Cap, CapSet};
use crate::child::SandboxedChild;
use crate::command::CommandSpec;
use crate::error::SandboxError;
use crate::resolve::{ResolveCtx, ResolvedPolicy};
use crate::spec::SandboxSpec;

/// Kennung eines Backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Seatbelt,
    Landlock,
    Bwrap,
    Windows,
    Docker,
    Podman,
    /// Keine Isolation; nur ausdrücklich wählbar (SBX-001, SBX-006).
    None,
}

impl ProviderId {
    pub const ALL: [ProviderId; 7] = [
        ProviderId::Seatbelt,
        ProviderId::Landlock,
        ProviderId::Bwrap,
        ProviderId::Windows,
        ProviderId::Docker,
        ProviderId::Podman,
        ProviderId::None,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Seatbelt => "seatbelt",
            ProviderId::Landlock => "landlock",
            ProviderId::Bwrap => "bwrap",
            ProviderId::Windows => "windows",
            ProviderId::Docker => "docker",
            ProviderId::Podman => "podman",
            ProviderId::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ergebnis der Capability-Probe eines Backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeReport {
    pub id: ProviderId,
    /// Ist das Backend auf diesem Rechner nutzbar?
    pub available: bool,
    /// Erzwingbare Capabilities (leer, wenn nicht verfügbar).
    pub caps: CapSet,
    /// Je nicht erzwingbarer Capability ein Grund.
    pub degraded_reasons: BTreeMap<Cap, String>,
    /// Warum das Backend nicht verfügbar ist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// Version des Backends bzw. des Profil-Formats.
    pub version: String,
    /// Nur Linux-Backends (`landlock`, `bwrap`): Landlock-ABI und Kernel (SBX-010 AC1), auch
    /// wenn das Backend nicht verfügbar ist.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<KernelInfo>,
}

/// Kernel-Angaben der Linux-Probe (SBX-010 AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelInfo {
    /// Landlock-ABI des Kernels (`null`: Landlock fehlt oder ist nicht aktiv). Eine per
    /// Konfiguration begrenzte ABI steht in `version`.
    pub landlock_abi: Option<u32>,
    /// Kernel-Release (`uname -r`).
    pub kernel: String,
}

impl ProbeReport {
    /// Ein nicht verfügbares Backend: keine Capability, für jede derselbe Grund.
    pub fn unavailable(id: ProviderId, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            id,
            available: false,
            caps: CapSet::new(),
            degraded_reasons: Cap::ALL.iter().map(|c| (*c, reason.clone())).collect(),
            unavailable_reason: Some(reason),
            version: String::new(),
            kernel: None,
        }
    }
}

/// Ein Isolations-Backend. Alle Prozessstarts von Harness- und Tool-Prozessen laufen über
/// [`SandboxProvider::spawn`]; ein direkter `Command::spawn` außerhalb dieses Crates ist per
/// Lint verboten (SBX-001 AC2).
pub trait SandboxProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Erkannte Capabilities und Gründe für fehlende.
    fn probe(&self) -> ProbeReport;

    /// Kanonisiert Pfade, expandiert Masken und prüft, ob das Backend alle geforderten
    /// Capabilities erzwingen kann (sonst [`SandboxError::Unavailable`]).
    fn resolve(
        &self,
        spec: &SandboxSpec,
        ctx: &ResolveCtx,
    ) -> Result<ResolvedPolicy, SandboxError> {
        let mut policy = crate::resolve::resolve(self.id(), spec, ctx)?;
        let probe = self.probe();
        crate::resolve::check_caps(&probe, &policy)?;
        crate::resolve::mark_unenforced_limits(&probe, &mut policy);
        Ok(policy)
    }

    /// Startet den Prozess(baum) isoliert gemäß `policy`.
    fn spawn(
        &self,
        policy: &ResolvedPolicy,
        cmd: CommandSpec,
    ) -> Result<SandboxedChild, SandboxError>;

    /// Wie [`SandboxProvider::spawn`]; erkannte Verstöße (z. B. ein erreichtes Limit,
    /// SBX-013 AC1) meldet das Backend zusätzlich an `violations`.
    fn spawn_observed(
        &self,
        policy: &ResolvedPolicy,
        cmd: CommandSpec,
        violations: Option<crate::registry::ViolationSink>,
    ) -> Result<SandboxedChild, SandboxError> {
        let _ = violations;
        self.spawn(policy, cmd)
    }
}
