//! Sandbox-Provider-Interface von beton (`beton-sandbox`, SBX-001 … SBX-007, ADR-0007).
//!
//! - [`SandboxProvider`] mit `probe`/`resolve`/`spawn` und eine [`Registry`] der Backends pro
//!   Plattform; `backend: none` ist ein eigener, ausdrücklich zu wählender Provider (SBX-001).
//! - [`SandboxSpec`] (YAML `sandbox:`) wird beim Laden geprüft und über die Ebenen gemergt,
//!   [`resolve()`] löst sie fail closed zu einer [`ResolvedPolicy`] auf: nur `~` wird expandiert,
//!   Pfade werden kanonisiert, verbotene Grants und Symlinks in verbotene Wurzeln sind Fehler
//!   (SBX-003); Credential-Pfade und Dotfiles sind maskiert (SBX-004).
//! - Seatbelt (macOS, SBX-007): deny-default-Profil aus einem versionierten Template mit
//!   Parametern.
//! - Linux (SBX-008, SBX-009): Landlock + seccomp + Namespaces über den Launcher
//!   `beton __sandbox-exec`, bubblewrap als Fallback.
//! - Ressourcenlimits und Aufräumen des Prozessbaums (SBX-013).
//!
//! Alle Prozessstarts von Harness- und Tool-Prozessen laufen über dieses Crate; außerhalb
//! verbietet ein Clippy-Lint (`disallowed-methods`) den direkten `Command`-Start (SBX-001 AC2).
//! Fail closed: Fehlt eine geforderte Capability, startet nichts.

pub mod caps;
pub mod child;
pub mod command;
pub mod error;
pub mod explain;
pub mod git;
pub mod linux;
pub mod masks;
pub mod none;
pub mod paths;
pub mod preset;
#[cfg(target_os = "macos")]
mod proctree;
pub mod provider;
pub mod registry;
pub mod resolve;
pub mod seatbelt;
pub mod spec;

pub use caps::{Cap, CapSet};
pub use child::{Output, SandboxedChild};
pub use command::{CommandSpec, EnvSpec, Stdio};
pub use error::SandboxError;
pub use git::GitLayout;
pub use masks::HarnessGrants;
pub use none::NoneProvider;
pub use provider::{ProbeReport, ProviderId, SandboxProvider};
pub use registry::{EventSink, Os, Registry, Sandbox, ViolationSink, auto_backend};
pub use resolve::{
    ExecAccess, Mask, NetMode, ProxyEndpoint, ResolveCtx, ResolvedPolicy, Secret, Stage,
    check_caps, mark_unenforced_limits, resolve,
};
pub use spec::{BackendChoice, HarnessSandboxConfig, Limits, Preset, SandboxSpec, WorkspaceAccess};
