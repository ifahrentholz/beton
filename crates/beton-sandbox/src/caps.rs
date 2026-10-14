//! Capabilities eines Backends (SBX-001): was ein Provider erzwingen kann.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Eine vom Backend erzwingbare Eigenschaft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cap {
    /// Lesen nur aus freigegebenen Wurzeln.
    FsRead,
    /// Schreiben nur in freigegebene Wurzeln.
    FsWrite,
    /// Masken innerhalb freigegebener Wurzeln (SBX-004).
    Masks,
    /// Netz nur zum Proxy (PRX-008).
    Net,
    /// Explizites Environment ohne Erben (SBX-005).
    Env,
    /// Prozess- und Signalrechte nur auf den eigenen Baum.
    Proc,
    /// Syscall-Filter (Linux: seccomp).
    Syscall,
    /// Ressourcenlimits (SBX-013).
    Resources,
}

impl Cap {
    pub const ALL: [Cap; 8] = [
        Cap::FsRead,
        Cap::FsWrite,
        Cap::Masks,
        Cap::Net,
        Cap::Env,
        Cap::Proc,
        Cap::Syscall,
        Cap::Resources,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Cap::FsRead => "fs_read",
            Cap::FsWrite => "fs_write",
            Cap::Masks => "masks",
            Cap::Net => "net",
            Cap::Env => "env",
            Cap::Proc => "proc",
            Cap::Syscall => "syscall",
            Cap::Resources => "resources",
        }
    }
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Menge von Capabilities (sortiert, als JSON-Liste).
pub type CapSet = BTreeSet<Cap>;

/// Die Capabilities, die eine Sandbox standardmäßig erzwingen muss. `syscall` und `resources`
/// sind (noch) nicht Pflicht: Seatbelt filtert keine Syscalls, Limits folgen mit SBX-013.
pub fn required_default() -> CapSet {
    [
        Cap::FsRead,
        Cap::FsWrite,
        Cap::Masks,
        Cap::Net,
        Cap::Env,
        Cap::Proc,
    ]
    .into_iter()
    .collect()
}

/// Alle Capabilities.
pub fn all() -> CapSet {
    Cap::ALL.into_iter().collect()
}
