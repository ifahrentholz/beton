//! Strukturierte Fehler der Sandbox (SBX-001). Jeder Fehler führt zur Ablehnung des Starts;
//! es gibt keinen Fallback auf einen unsandboxed oder schwächeren Start (SBX-006).

use crate::caps::CapSet;
use crate::provider::ProviderId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    /// Das Backend fehlt oder kann geforderte Capabilities nicht erzwingen.
    #[error("sandbox_unavailable: {provider}: {hint}{}", missing_list(.missing))]
    Unavailable {
        provider: ProviderId,
        missing: CapSet,
        hint: String,
    },
    /// Ungültige Konfiguration; `path` nennt das Feld, z. B. `write_paths[0]`.
    #[error("invalid_spec: {path}: {message}")]
    InvalidSpec { path: String, message: String },
    /// Ein Grant zeigt (über Symlinks) in eine verbotene Wurzel.
    #[error("path_escape: {path}: {message}")]
    PathEscape { path: String, message: String },
    /// Der Prozess konnte nicht gestartet werden.
    #[error("spawn_failed: {message}")]
    SpawnFailed {
        message: String,
        kind: std::io::ErrorKind,
    },
}

fn missing_list(missing: &CapSet) -> String {
    if missing.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = missing.iter().map(|c| c.as_str()).collect();
    format!(" (fehlt: {})", names.join(", "))
}

impl SandboxError {
    /// Fehlercode (RFC-9457-Problem bzw. CLI-Ausgabe).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "sandbox_unavailable",
            Self::InvalidSpec { .. } => "invalid_spec",
            Self::PathEscape { .. } => "path_escape",
            Self::SpawnFailed { .. } => "spawn_failed",
        }
    }

    pub(crate) fn invalid(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidSpec {
            path: path.into(),
            message: message.into(),
        }
    }

    pub(crate) fn spawn(message: impl Into<String>) -> Self {
        Self::SpawnFailed {
            message: message.into(),
            kind: std::io::ErrorKind::Other,
        }
    }

    pub(crate) fn escape(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::PathEscape {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl From<SandboxError> for std::io::Error {
    fn from(e: SandboxError) -> Self {
        let kind = match &e {
            SandboxError::SpawnFailed { kind, .. } => *kind,
            SandboxError::Unavailable { .. } | SandboxError::PathEscape { .. } => {
                std::io::ErrorKind::PermissionDenied
            }
            SandboxError::InvalidSpec { .. } => std::io::ErrorKind::InvalidInput,
        };
        std::io::Error::new(kind, e.to_string())
    }
}
