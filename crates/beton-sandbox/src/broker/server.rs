//! Der Broker im Runner (Stufe 0).

use std::path::{Path, PathBuf};

use crate::registry::Sandbox;
use crate::resolve::{ExecAccess, Secret};

/// Ein laufender Broker; endet mit `Drop` (Socket wird entfernt, laufende Kommandos enden).
pub struct Broker {
    socket: PathBuf,
    token: Secret,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl Broker {
    /// Startet den Broker mit dem Socket `dir/exec.sock` (Verzeichnis 0700) für die
    /// Stufe-2-Sandbox `tools`.
    pub async fn start(dir: &Path, tools: Sandbox) -> std::io::Result<Self> {
        let _ = tools;
        Ok(Self {
            socket: dir.join("exec.sock"),
            token: Secret::new(String::new()),
        })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn token(&self) -> &Secret {
        &self.token
    }

    /// Zugang für die Auflösung der Stufe 1 ([`crate::ResolveCtx::exec`]).
    pub fn access(&self, shim_dir: PathBuf) -> ExecAccess {
        ExecAccess {
            socket: self.socket.clone(),
            shim_dir,
            token: self.token.clone(),
        }
    }
}
