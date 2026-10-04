//! Session-Export und -Import als Datei (SES-009, Format und Prüfung: DATA-010 in
//! `beton_store::export`).
//!
//! - `GET /v1/sessions/{id}/export?with_raw=&with_blobs=` liefert JSONL bzw. `.tar.zst`.
//! - `POST /v1/sessions/import?force=` nimmt eine Exportdatei als Body und legt daraus eine
//!   neue Session an. Die Datei ist nicht vertrauenswürdig: Sie wird vollständig geprüft,
//!   bevor etwas geschrieben wird, Fehler nennen die Zeile, und nichts aus ihr wird als Pfad
//!   auf dem Host verwendet. Die importierte Session läuft hier nicht (siehe
//!   [`ensure_runnable`]).
//!
//! Beide Endpunkte lesen und schreiben nur den Request- bzw. Response-Body; Dateien auf dem
//! Host berührt erst der Client (ADR-0033). Inhalte erscheinen nicht im Log.

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_store::{ExportOptions, FileImportStatus, InvalidExport, InvalidKind, Store};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::ApiQuery;
use crate::imports::ImportStatus;
use crate::problem::{ApiResult, FieldError, Problem, ProblemCode};
use crate::security::Authenticated;

/// Header mit der Anzahl der exportierten Events.
pub const EVENTS_HEADER: &str = "beton-export-events";
/// Header mit der Anzahl der Blobs im Archiv.
pub const BLOBS_HEADER: &str = "beton-export-blobs";
/// Medientyp eines JSONL-Exports.
pub const JSONL_TYPE: &str = "application/x-ndjson";
/// Medientyp eines `.tar.zst`-Exports.
pub const ARCHIVE_TYPE: &str = "application/zstd";

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExportQuery {
    /// Redigiertes `raw` der Harnesses mitexportieren (Default `false`).
    pub with_raw: Option<bool>,
    /// `.tar.zst` mit `session.jsonl` und `blobs/<sha256>` statt JSONL (Default `false`).
    pub with_blobs: Option<bool>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImportFileQuery {
    /// Auch eine schon importierte Datei (gleicher Hash) erneut als neue Session anlegen.
    pub force: Option<bool>,
}

/// Inhalt einer Exportdatei: JSONL bzw. `.tar.zst` (DATA-010).
#[derive(Debug, ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct ExportFileBody(pub Vec<u8>);

/// Herkunft einer importierten Session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportedFromView {
    /// Session-ID in der exportierenden Instanz.
    pub session_id: String,
    /// Zeitpunkt des Exports (RFC 3339).
    pub exported_at: String,
    /// SHA-256 der `session.jsonl`; Schlüssel der Idempotenz.
    pub export_sha256: String,
}

/// Ergebnis von `POST /v1/sessions/import`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct SessionImportResult {
    /// `imported` oder `skipped` (dieselbe Datei wurde schon importiert).
    pub status: ImportStatus,
    /// Bei `skipped`: `already_imported`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// Neue bzw. (bei `already_imported`) bestehende Session.
    pub session_id: String,
    pub title: String,
    /// Anzahl der Events.
    pub events: u64,
    /// Anzahl der Blobs aus dem Archiv.
    pub blobs: u64,
    pub imported_from: ImportedFromView,
}

fn session_id(raw: &str) -> Result<SessionId, Problem> {
    raw.parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {raw}")))
}

/// Problem zu einer abgelehnten Importdatei (DATA-010 AC2, AC4).
pub fn invalid_export_problem(e: &InvalidExport) -> Problem {
    let code = match e.kind {
        InvalidKind::UnsupportedFormatVersion => ProblemCode::UnsupportedFormatVersion,
        InvalidKind::SeqGap => ProblemCode::ImportSeqGap,
        InvalidKind::TooLarge => ProblemCode::PayloadTooLarge,
        InvalidKind::Invalid => ProblemCode::ImportInvalid,
    };
    let errors = e
        .pointer
        .iter()
        .map(|p| FieldError {
            pointer: p.clone(),
            detail: e.detail.clone(),
        })
        .collect();
    Problem::new(code).detail(e.detail.clone()).errors(errors)
}

/// Session als Datei exportieren (SES-009, DATA-010).
#[utoipa::path(get, path = "/v1/sessions/{id}/export", tag = "sessions",
    params(("id" = String, Path), ExportQuery),
    responses(
        (status = 200, description = "Exportdatei: JSONL (`application/x-ndjson`) bzw. mit `with_blobs` ein `.tar.zst` (`application/zstd`)",
            content((ExportFileBody = "application/x-ndjson"), (ExportFileBody = "application/zstd")),
            headers(
                ("beton-export-events" = u64, description = "Anzahl der Events"),
                ("beton-export-blobs" = u64, description = "Anzahl der Blobs im Archiv"))),
        (status = 403, description = "Keine Leseberechtigung", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Session unbekannt", body = Problem, content_type = "application/problem+json")))]
pub async fn export_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<ExportQuery>,
) -> ApiResult<Response> {
    let id = session_id(&id)?;
    let record = state.store.session(state.local.org, id).await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    let opts = ExportOptions {
        with_raw: q.with_raw.unwrap_or(false),
        with_blobs: q.with_blobs.unwrap_or(false),
    };
    let file = state
        .store
        .export_session(state.local.org, id, opts)
        .await?;
    tracing::debug!(session_id = %id, events = file.events, blobs = file.blobs, "Session exportiert");
    let (content_type, ext) = if file.archive {
        (ARCHIVE_TYPE, "tar.zst")
    } else {
        (JSONL_TYPE, "jsonl")
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{id}.{ext}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    headers.insert(EVENTS_HEADER, HeaderValue::from(file.events));
    headers.insert(BLOBS_HEADER, HeaderValue::from(file.blobs));
    Ok((headers, file.bytes).into_response())
}

/// Exportdatei als neue Session importieren (SES-009, DATA-010). Prüft die ganze Datei,
/// bevor etwas angelegt wird; Fehler nennen die Zeile (`errors[].pointer`
/// `/lines/<zeile>/<feld>`).
#[utoipa::path(post, path = "/v1/sessions/import", tag = "sessions",
    params(ImportFileQuery),
    request_body(description = "Exportdatei: JSONL oder `.tar.zst` (bis 256 MiB)",
        content((ExportFileBody = "application/x-ndjson"), (ExportFileBody = "application/zstd"))),
    responses(
        (status = 200, description = "Importiert bzw. schon vorhanden (`skipped`)", body = SessionImportResult),
        (status = 413, description = "Datei zu groß", body = Problem, content_type = "application/problem+json"),
        (status = 415, description = "Medientyp nicht unterstützt", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "`unsupported_format_version`, `import_seq_gap` oder `import_invalid`; nichts angelegt", body = Problem, content_type = "application/problem+json")))]
pub async fn import_session(
    State(state): State<AppState>,
    Extension(_auth): Extension<Authenticated>,
    ApiQuery(q): ApiQuery<ImportFileQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<axum::Json<SessionImportResult>> {
    let media = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
    if let Some(media) = media
        && !matches!(
            media.as_str(),
            JSONL_TYPE | ARCHIVE_TYPE | "application/jsonl" | "application/octet-stream"
        )
    {
        return Err(Problem::new(ProblemCode::UnsupportedMediaType)
            .detail(format!("Erwartet {JSONL_TYPE} oder {ARCHIVE_TYPE}")));
    }
    let bytes = axum::body::to_bytes(body, beton_store::export::MAX_IMPORT_BYTES)
        .await
        .map_err(|_| {
            Problem::new(ProblemCode::PayloadTooLarge).detail(format!(
                "Die Datei ist größer als {} MiB",
                beton_store::export::MAX_IMPORT_BYTES / 1024 / 1024
            ))
        })?;
    let parsed = tokio::task::spawn_blocking(move || beton_store::parse_export(&bytes))
        .await
        .map_err(|e| Problem::internal(&e))?
        .map_err(|e| {
            tracing::debug!(code = e.kind.code(), line = e.line, "Import abgelehnt");
            invalid_export_problem(&e)
        })?;
    // M1: lokal handelt immer `usr_local`.
    let outcome = state
        .store
        .import_export(
            state.local.org,
            UserId::LOCAL,
            state.local.node,
            parsed,
            q.force.unwrap_or(false),
        )
        .await?;
    tracing::debug!(session_id = %outcome.session_id, events = outcome.events, "Exportdatei importiert");
    let skipped = outcome.status == FileImportStatus::Skipped;
    Ok(axum::Json(SessionImportResult {
        status: if skipped {
            ImportStatus::Skipped
        } else {
            ImportStatus::Imported
        },
        reason: skipped.then(|| "already_imported".to_owned()),
        session_id: outcome.session_id.to_string(),
        title: outcome.title,
        events: outcome.events,
        blobs: outcome.blobs,
        imported_from: ImportedFromView {
            session_id: outcome.imported_from.session_id.to_string(),
            exported_at: outcome.imported_from.exported_at.to_string(),
            export_sha256: outcome.imported_from.export_sha256,
        },
    }))
}

/// Eine aus einer Exportdatei importierte Session läuft auf diesem Host nicht: Ihr
/// Arbeitsverzeichnis, Worktree, Agent-Ref und Startoptionen stammen aus der Datei
/// (SES-009). Runner, Fork, Eingaben und Workspace-Zugriffe lehnt der Server deshalb ab.
pub async fn ensure_runnable(store: &Store, org: OrgId, session: SessionId) -> ApiResult<()> {
    if store.file_import(org, session).await?.is_some() {
        return Err(Problem::new(ProblemCode::ImportedReadOnly).detail(
            "Diese Session stammt aus einer Exportdatei. Arbeitsverzeichnis und Startoptionen \
             kommen aus der Datei, deshalb startet beton dafür keinen Agent und öffnet keine \
             Dateien. Verlauf, Suche und Export bleiben verfügbar.",
        ));
    }
    Ok(())
}
