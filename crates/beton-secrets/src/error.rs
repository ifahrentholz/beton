//! Fehler des Secret-Stores. Meldungen enthalten nie Secret-Werte (SEC-012 AC2).

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Fehler mit maschinenlesbarem Code ([`Error::code`]).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Der Store ist gesperrt oder die Passphrase stimmt nicht (SEC-003). Fail closed:
    /// Wer ein Secret braucht, bekommt keins.
    #[error("secrets_locked: Secret-Store ist gesperrt (beton secrets unlock)")]
    Locked,
    /// Kein Backend nutzbar, z. B. Keychain nicht erreichbar.
    #[error("secrets_unavailable: {0}")]
    Unavailable(String),
    /// Die verschlüsselte Datei ist beschädigt oder manipuliert (SEC-003 AC2).
    #[error("secrets_corrupt: {0}")]
    Corrupt(String),
    #[error("not_found: Secret {0} nicht gefunden")]
    NotFound(String),
    #[error("conflict: Secret {0} existiert bereits")]
    Exists(String),
    #[error("invalid_secret_ref: {0}")]
    InvalidRef(String),
    #[error("validation_failed: {0}")]
    Invalid(String),
    #[error("Datenbank: {0}")]
    Store(#[from] beton_store::Error),
    #[error("E/A-Fehler: {0}")]
    Io(String),
}

impl Error {
    /// Maschinenlesbarer Code für Problem-Antworten.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Locked => "secrets_locked",
            Self::Unavailable(_) => "secrets_unavailable",
            Self::Corrupt(_) => "secrets_corrupt",
            Self::NotFound(_) => "not_found",
            Self::Exists(_) => "conflict",
            Self::InvalidRef(_) => "invalid_secret_ref",
            Self::Invalid(_) => "validation_failed",
            Self::Store(e) => e.code(),
            Self::Io(_) => "internal",
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
