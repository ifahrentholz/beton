use beton_core::event::BlobRef;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Fehler des Stores. [`Error::code`] liefert den maschinenlesbaren Code für RFC-9457-Probleme.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} nicht gefunden")]
    NotFound(String),
    #[error("seq_conflict: erwartet head_seq {expected}, aktuell {actual}")]
    SeqConflict { expected: u64, actual: u64 },
    #[error("stale_epoch: Schreibvorgang mit Epoch {given}, aktuell {current}")]
    StaleEpoch { given: u64, current: u64 },
    #[error(
        "schema_too_new: Datenbank hat Schema-Version {db}, dieses Binary kennt nur bis {binary}"
    )]
    SchemaTooNew { db: i64, binary: i64 },
    #[error("blob_corrupt: Inhalt von {0} passt nicht zu seinem Hash")]
    BlobCorrupt(BlobRef),
    #[error("blob_too_large: {size} Bytes, erlaubt sind {max}")]
    BlobTooLarge { size: u64, max: u64 },
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("tombstoned: {0} wurde gelöscht")]
    Tombstoned(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid_event: {0}")]
    InvalidEvent(String),
    #[error("Datenbankfehler: {0}")]
    Db(#[from] sqlx::Error),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
    #[error("Daten in der Datenbank sind ungültig: {0}")]
    Corrupt(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::SeqConflict { .. } => "seq_conflict",
            Self::StaleEpoch { .. } => "stale_epoch",
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::BlobCorrupt(_) => "blob_corrupt",
            Self::BlobTooLarge { .. } => "blob_too_large",
            Self::Forbidden(_) => "forbidden",
            Self::Tombstoned(_) => "tombstoned",
            Self::Conflict(_) => "conflict",
            Self::InvalidEvent(_) => "invalid_event",
            Self::Db(_) | Self::Io(_) | Self::Corrupt(_) => "internal",
        }
    }

    pub(crate) fn corrupt(what: impl std::fmt::Display) -> Self {
        Self::Corrupt(what.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::corrupt(e)
    }
}
