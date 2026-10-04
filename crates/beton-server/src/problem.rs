//! Fehlerformat nach RFC 9457 (PROTO-011).
//!
//! Alle Fehler sind Problem-Objekte (`application/problem+json`) mit maschinenlesbarem
//! `code` aus [`ProblemCode`]. Problem-Details enthalten nie Secrets, Stacktraces oder
//! interne Pfade; interne Fehler werden geloggt und nach außen nur mit `trace_id` gemeldet.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

macro_rules! problem_codes {
    ($($(#[$m:meta])* $variant:ident = $code:literal, $status:literal, $title:literal;)*) => {
        /// Maschinenlesbare Fehlercodes; daraus wird `docs/generated/problem-codes.md` erzeugt.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum ProblemCode {
            $($(#[$m])* $variant,)*
        }

        impl ProblemCode {
            pub const ALL: &'static [ProblemCode] = &[$(Self::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code,)* }
            }

            pub fn status(self) -> StatusCode {
                match self {
                    $(Self::$variant => StatusCode::from_u16($status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),)*
                }
            }

            pub fn title(self) -> &'static str {
                match self { $(Self::$variant => $title,)* }
            }
        }
    };
}

problem_codes! {
    BadRequest = "bad_request", 400, "Ungültige Anfrage";
    ValidationFailed = "validation_failed", 400, "Validierung fehlgeschlagen";
    InvalidCursor = "invalid_cursor", 400, "Ungültiger Cursor";
    InvalidLimit = "invalid_limit", 400, "Ungültiges Limit";
    UnknownCommand = "unknown_command", 400, "Unbekanntes Kommando";
    Unauthorized = "unauthorized", 401, "Anmeldung erforderlich";
    InvalidCode = "invalid_code", 401, "Einmal-Code ungültig oder abgelaufen";
    Forbidden = "forbidden", 403, "Keine Berechtigung";
    HostNotAllowed = "host_not_allowed", 403, "Host nicht erlaubt";
    OriginNotAllowed = "origin_not_allowed", 403, "Origin nicht erlaubt";
    LoopbackOnly = "loopback_only", 403, "Nur über Loopback erreichbar";
    PathOutsideWorkspace = "path_outside_workspace", 403, "Pfad außerhalb des Workspace";
    NotFound = "not_found", 404, "Nicht gefunden";
    /// Agent-Ref im Suchpfad nicht gefunden (AGT-003).
    AgentNotFound = "agent_not_found", 404, "Agent nicht gefunden";
    /// Die Funktion liegt hinter einem nicht aktivierten Feature-Flag (UX-007).
    FeatureDisabled = "feature_disabled", 404, "Funktion nicht aktiviert";
    MethodNotAllowed = "method_not_allowed", 405, "Methode nicht erlaubt";
    Conflict = "conflict", 409, "Konflikt";
    /// Der Harness der Session kann das nicht (HAR-002 AC3, SES-004 AC4).
    CapabilityUnsupported = "capability_unsupported", 409, "Vom Harness nicht unterstützt";
    /// Steer ohne laufenden Turn (SES-004).
    NoActiveTurn = "no_active_turn", 409, "Kein laufender Turn";
    SeqConflict = "seq_conflict", 409, "Sequenzkonflikt";
    StaleEpoch = "stale_epoch", 409, "Veraltete Epoch";
    SeqAhead = "seq_ahead", 409, "from_seq liegt hinter head_seq";
    NotAGitRepo = "not_a_git_repo", 409, "Kein Git-Repository";
    WorktreeDirty = "worktree_dirty", 409, "Worktree hat uncommittete Änderungen";
    WorktreeUnpushed = "worktree_unpushed", 409, "Branch hat ungepushte Commits";
    /// Die Session stammt aus einer Exportdatei und läuft auf diesem Host nicht (SES-009).
    ImportedReadOnly = "imported_read_only", 409, "Importierte Session läuft hier nicht";
    Tombstoned = "tombstoned", 410, "Gelöscht";
    PreconditionFailed = "precondition_failed", 412, "Vorbedingung nicht erfüllt";
    PayloadTooLarge = "payload_too_large", 413, "Anfrage zu groß";
    UnsupportedMediaType = "unsupported_media_type", 415, "Medientyp nicht unterstützt";
    IdempotencyKeyReused = "idempotency_key_reused", 422, "Idempotency-Key mit anderem Inhalt wiederverwendet";
    BaseNotFound = "base_not_found", 422, "Base-Branch nicht auflösbar";
    /// Der Agent besteht die Validierung nicht (AGT-002); `errors` nennt die Befunde.
    AgentInvalid = "agent_invalid", 422, "Agent ungültig";
    /// Ein Parameterwert passt nicht zur Deklaration oder ist unbekannt (AGT-010 AC1).
    InvalidParam = "invalid_param", 422, "Parameter ungültig";
    /// Pflichtparameter ohne Default fehlen (AGT-010 AC2); `errors` nennt sie je
    /// `/params/<name>`, damit die UI nachfragen kann.
    ParamsRequired = "params_required", 422, "Pflichtparameter fehlen";
    /// Der Ziel-Harness eines Forks passt nicht, z. B. ohne `fork_history` oder ohne eine
    /// Capability, die der Agent braucht (SES-007).
    HarnessIncompatible = "harness_incompatible", 422, "Ziel-Harness passt nicht";
    /// Exportdatei mit höherer Formatversion, als diese Version liest (DATA-010 AC4).
    UnsupportedFormatVersion = "unsupported_format_version", 422, "Exportformat zu neu";
    /// Exportdatei mit Lücke in `seq` (DATA-010 AC2); nichts wurde angelegt.
    ImportSeqGap = "import_seq_gap", 422, "Import abgelehnt: Lücke im Verlauf";
    /// Ungültige Exportdatei, z. B. eine Zeile gegen das Schema (DATA-010 AC2).
    ImportInvalid = "import_invalid", 422, "Import abgelehnt: Datei ungültig";
    RateLimited = "rate_limited", 429, "Zu viele Anfragen";
    Internal = "internal", 500, "Interner Fehler";
    BlobCorrupt = "blob_corrupt", 500, "Blob beschädigt";
    Unavailable = "unavailable", 503, "Vorübergehend nicht verfügbar";
}

/// Ein Validierungsfehler mit JSON-Pointer auf das Feld.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldError {
    pub pointer: String,
    pub detail: String,
}

/// Problem-Objekt nach RFC 9457.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    /// `urn:beton:problem:<code>`
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Maschinenlesbarer Code; offener Katalog (neue Codes sind kein Breaking Change),
    /// Referenz: `docs/generated/problem-codes.md`.
    #[schema(value_type = String, example = "not_found")]
    pub code: ProblemCode,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
}

impl Problem {
    pub fn new(code: ProblemCode) -> Self {
        Self {
            kind: format!("urn:beton:problem:{}", code.as_str()),
            title: code.title().to_owned(),
            status: code.status().as_u16(),
            detail: None,
            instance: None,
            code,
            trace_id: new_trace_id(),
            errors: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = errors;
        self
    }

    /// Interner Fehler: Ursache nur ins Log, nach außen nur die `trace_id`.
    pub fn internal(cause: &dyn std::fmt::Display) -> Self {
        let p = Self::new(ProblemCode::Internal);
        tracing::error!(trace_id = %p.trace_id, "interner Fehler: {cause}");
        p
    }
}

/// Zufällige ID zur Zuordnung von Antwort und Log.
pub fn new_trace_id() -> String {
    let mut bytes = [0u8; 8];
    // Ohne Zufall bleibt die ID eindeutig genug über die Zeit.
    if getrandom::fill(&mut bytes).is_err() {
        bytes = beton_core::id::EventId::new().ulid().0.to_le_bytes()[..8]
            .try_into()
            .unwrap_or_default();
    }
    hex::encode(bytes)
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&self).unwrap_or_default();
        let mut res = (status, body).into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
        );
        res
    }
}

impl From<beton_store::Error> for Problem {
    fn from(e: beton_store::Error) -> Self {
        use beton_store::Error as E;
        let code = match &e {
            E::NotFound(_) => ProblemCode::NotFound,
            E::SeqConflict { .. } => ProblemCode::SeqConflict,
            E::StaleEpoch { .. } => ProblemCode::StaleEpoch,
            E::Forbidden(_) => ProblemCode::Forbidden,
            E::Tombstoned(_) => ProblemCode::Tombstoned,
            E::Conflict(_) => ProblemCode::Conflict,
            E::InvalidEvent(_) => ProblemCode::ValidationFailed,
            E::BlobCorrupt(_) => {
                let p = Self::new(ProblemCode::BlobCorrupt);
                tracing::error!(trace_id = %p.trace_id, "{e}");
                return p;
            }
            E::BlobTooLarge { .. } => ProblemCode::PayloadTooLarge,
            E::SchemaTooNew { .. } | E::Db(_) | E::Io(_) | E::Corrupt(_) => {
                return Self::internal(&e);
            }
        };
        // Store-Meldungen enthalten nur IDs und Zahlen, keine Pfade oder Secrets.
        Self::new(code).detail(e.to_string())
    }
}

/// Ergebnis von Handlern.
pub type ApiResult<T> = Result<T, Problem>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_011_problem_has_rfc9457_shape() {
        let p = Problem::new(ProblemCode::HostNotAllowed).detail("Host evil.test");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], "urn:beton:problem:host_not_allowed");
        assert_eq!(v["status"], 403);
        assert_eq!(v["code"], "host_not_allowed");
        assert_eq!(v["trace_id"].as_str().unwrap().len(), 16);
        let res = p.into_response();
        assert_eq!(res.headers()[header::CONTENT_TYPE], PROBLEM_CONTENT_TYPE);
    }

    #[test]
    fn problem_codes_serialize_like_their_names() {
        for code in ProblemCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
            assert!(code.status().as_u16() >= 400);
        }
    }

    #[test]
    fn proto_011_internal_store_errors_do_not_leak_details() {
        let e = beton_store::Error::Io(std::io::Error::other("/home/geheim/.beton/beton.db"));
        let p = Problem::from(e);
        assert_eq!(p.code, ProblemCode::Internal);
        assert!(p.detail.is_none());
        assert!(!serde_json::to_string(&p).unwrap().contains("geheim"));
    }
}
