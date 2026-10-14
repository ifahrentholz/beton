//! Linux-Backends (SBX-008, SBX-009): Landlock + seccomp + Namespaces nativ in Rust, bubblewrap
//! als Fallback, Ressourcenlimits über cgroup v2 bzw. rlimits (SBX-013).
//!
//! Ablauf eines Starts (beide Backends):
//!
//! ```text
//! Runner ──spawn──► L: `beton __sandbox-exec`   (Host-Namespaces, Plan per Pipe auf FD 3,
//!                   │                            Status-Socket auf FD 4, kein Env, kein argv)
//!                   ├─fork─► C: Connector        (Host-Netz, verbindet nur zum Proxy-Port,
//!                   │                            reicht Sockets per SCM_RIGHTS weiter)
//!   landlock:       ├─unshare(USER|NS|PID|NET|IPC|UTS)─fork─► P1 (PID 1, Reaper)
//!   bwrap:          └─fork─exec─► bwrap … -- beton __sandbox-exec ─► P1 (innen, ohne Mounts)
//!                                   P1: Mounts (Masken, privates /tmp, /proc), lo, Relay R
//!                                   └─fork─► T: rlimits, Caps weg, NO_NEW_PRIVS, Landlock,
//!                                             seccomp, execve(Ziel)
//! ```
//!
//! Fail closed: Jeder Fehler vor dem `execve` des Ziels bricht den Start ab und erreicht den
//! Runner als [`SandboxError::SpawnFailed`](crate::SandboxError); das Ziel läuft dann nie.
//! Stirbt der Runner, sieht L das Ende des Status-Sockets und beendet P1; mit P1 endet der
//! ganze PID-Namespace (SBX-008 AC6).

pub mod bwrap;
pub mod plan;

#[cfg(target_os = "linux")]
pub(crate) mod sys;

use std::path::PathBuf;
use std::sync::Arc;

use crate::provider::{ProviderId, SandboxProvider};

/// Argument, mit dem `beton` (bzw. der Test-Launcher) zum Sandbox-Launcher wird.
pub const LAUNCHER_ARG: &str = "__sandbox-exec";

/// Version des Plan-Formats zwischen Runner und Launcher.
pub const PLAN_VERSION: u32 = 1;

/// Exit-Code des Launchers, wenn er das Ziel nicht starten konnte (wie `env`/`chroot`).
pub const LAUNCH_FAILED: u8 = 125;

/// Mindestversion von bubblewrap (SBX-009 AC2).
pub const BWRAP_MIN: (u32, u32, u32) = (0, 8, 0);

/// Feste Orte von bubblewrap (nie über `PATH`).
pub const BWRAP_PATHS: [&str; 2] = ["/usr/bin/bwrap", "/usr/local/bin/bwrap"];

/// Einstellungen der Linux-Backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxConfig {
    /// Programm, das `__sandbox-exec` versteht (`beton` selbst; in Tests der Test-Launcher).
    pub launcher: PathBuf,
    /// Pfad zu bwrap; ohne Angabe die festen Orte aus [`BWRAP_PATHS`].
    pub bwrap: Option<PathBuf>,
    /// Höchste genutzte Landlock-ABI (nur Tests: emuliert ältere Kernel, z. B. ABI 2).
    pub abi_cap: Option<u8>,
    /// cgroup-v2-Delegation suchen und nutzen (SBX-013); sonst nur rlimits.
    pub cgroup: bool,
}

impl LinuxConfig {
    pub fn new(launcher: impl Into<PathBuf>) -> Self {
        Self {
            launcher: launcher.into(),
            bwrap: None,
            abi_cap: None,
            cgroup: true,
        }
    }
}

/// Das Landlock-Backend (SBX-008); außerhalb von Linux `unavailable`.
pub fn landlock_provider(cfg: LinuxConfig) -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(sys::provider::LinuxProvider::new(ProviderId::Landlock, cfg))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cfg;
        unavailable(ProviderId::Landlock)
    }
}

/// Das bubblewrap-Backend (SBX-009); außerhalb von Linux `unavailable`.
pub fn bwrap_provider(cfg: LinuxConfig) -> Arc<dyn SandboxProvider> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(sys::provider::LinuxProvider::new(ProviderId::Bwrap, cfg))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cfg;
        unavailable(ProviderId::Bwrap)
    }
}

#[cfg(not(target_os = "linux"))]
fn unavailable(id: ProviderId) -> Arc<dyn SandboxProvider> {
    Arc::new(crate::registry::UnavailableProvider {
        id,
        reason: "nur unter Linux".into(),
    })
}

/// Landlock-ABI des laufenden Kernels (`None`: kein Landlock oder nicht Linux).
pub fn kernel_landlock_abi() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        sys::probe::landlock_abi()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Einstieg des Launchers: Ist `args[1]` [`LAUNCHER_ARG`], läuft der Launcher und liefert den
/// Exit-Code; sonst `None`. Muss vor jedem weiteren Thread (Tokio-Laufzeit) aufgerufen werden:
/// `unshare(CLONE_NEWUSER)` verlangt einen Prozess mit nur einem Thread.
pub fn launcher_dispatch() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os();
    let _ = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new(LAUNCHER_ARG)) {
        return None;
    }
    let rest: Vec<std::ffi::OsString> = args.collect();
    #[cfg(target_os = "linux")]
    {
        Some(sys::launcher::main(&rest))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = rest;
        eprintln!("beton sandbox: {LAUNCHER_ARG} gibt es nur unter Linux");
        Some(std::process::ExitCode::from(LAUNCH_FAILED))
    }
}
