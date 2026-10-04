//! Anhänge im Composer (WEB-006): Upload in den Blob-Store der Session (DATA-006) und
//! Zustellung mit der Eingabe.
//!
//! Erlaubt sind Bilder (PNG, JPEG, GIF, WebP), PDF und Textdateien; Grenzen je Datei und je
//! Nachricht sind serverseitig konfigurierbar (`attachments.*`, Default 20 MiB und 10 Dateien).
//! Textdateien gehen als Text an jeden Harness, Bilder und PDF nur an Harnesses mit
//! Capability `images`.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::event::Attachment;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::UtoipaMethodRouter;
use utoipa_axum::routes;

use crate::app::AppState;
use crate::extract::ApiQuery;
use crate::problem::{ApiResult, Problem, ProblemCode};

/// Grenzen für Anhänge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentLimits {
    /// Höchstgröße je Datei in Bytes.
    pub max_file_bytes: u64,
    /// Höchstzahl der Dateien je Nachricht.
    pub max_files: u32,
}

impl Default for AttachmentLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 20 * 1024 * 1024,
            max_files: 10,
        }
    }
}

/// Art eines Anhangs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Pdf,
    Text,
}

impl Kind {
    /// Braucht die Capability `images` des Harness.
    pub fn binary(self) -> bool {
        matches!(self, Self::Image | Self::Pdf)
    }
}

/// Medientyp ohne Parameter, klein geschrieben.
fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Erlaubte Medientypen (WEB-006).
pub fn kind(mime: &str) -> Option<Kind> {
    let m = essence(mime);
    match m.as_str() {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => Some(Kind::Image),
        "application/pdf" => Some(Kind::Pdf),
        "application/json" | "application/xml" | "application/yaml" | "application/x-yaml"
        | "application/toml" | "application/x-sh" => Some(Kind::Text),
        _ if m.starts_with("text/") => Some(Kind::Text),
        _ => None,
    }
}

/// Dateiname zur Anzeige: nur der letzte Pfadteil, ohne Steuerzeichen, höchstens 200 Zeichen.
pub fn display_name(raw: Option<&str>) -> String {
    let base = raw
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    let clean = clean.trim();
    if clean.is_empty() || clean == "." || clean == ".." {
        "anhang".into()
    } else {
        clean.to_owned()
    }
}

/// Lesbare Größe für Fehlermeldungen, z. B. `48 MB`.
pub fn human_size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes >= MB {
        format!("{} MB", bytes.div_ceil(MB))
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

fn too_large(name: &str, size: Option<u64>, limit: u64) -> Problem {
    let size = size
        .map(|s| format!(" {},", human_size(s)))
        .unwrap_or_default();
    Problem::new(ProblemCode::PayloadTooLarge).detail(format!(
        "{name} nicht angehängt:{size} erlaubt sind {} pro Datei. Kürze die Datei oder hänge einen Ausschnitt an.",
        human_size(limit)
    ))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// Dateiname (nur Anzeige).
    pub name: Option<String>,
}

/// Anhang hochladen (WEB-006): Rohdaten im Body, Medientyp im `Content-Type`. Liefert die
/// Referenz für `attachments` in `POST /v1/sessions/{id}/input`.
#[utoipa::path(post, path = "/v1/sessions/{id}/attachments", tag = "sessions",
    params(("id" = String, Path), UploadQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Inhalt der Datei; `Content-Type` ist ihr Medientyp (Bild, PDF, Text)"),
    responses((status = 201, description = "Gespeichert", body = Object),
              (status = 413, description = "Datei zu groß", body = Problem, content_type = "application/problem+json"),
              (status = 415, description = "Medientyp nicht erlaubt", body = Problem, content_type = "application/problem+json")))]
pub async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    let session = id
        .parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {id}")))?;
    let name = display_name(q.name.as_deref());
    let mime = essence(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    );
    if kind(&mime).is_none() {
        return Err(Problem::new(ProblemCode::UnsupportedMediaType).detail(format!(
            "{name} nicht angehängt: Erlaubt sind Bilder (PNG, JPEG, GIF, WebP), PDF und Textdateien."
        )));
    }
    let limit = state.runtime.attachments.max_file_bytes;
    let announced = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if announced.is_some_and(|n| n > limit) {
        return Err(too_large(&name, announced, limit));
    }
    state.store.session(state.local.org, session).await?;
    let max = usize::try_from(limit).unwrap_or(usize::MAX);
    let bytes = axum::body::to_bytes(body, max)
        .await
        .map_err(|_| too_large(&name, None, limit))?;
    if bytes.is_empty() {
        return Err(Problem::new(ProblemCode::ValidationFailed)
            .detail(format!("{name} nicht angehängt: Die Datei ist leer.")));
    }
    let blob = state
        .store
        .put_session_blob(state.local.org, session, &bytes)
        .await?;
    let attachment = Attachment {
        blob,
        name,
        mime,
        size: bytes.len() as u64,
    };
    Ok((StatusCode::CREATED, axum::Json(attachment)).into_response())
}

/// Route mit eigener Größenprüfung statt des allgemeinen Body-Limits.
pub fn routes() -> UtoipaMethodRouter<AppState> {
    let (schemas, paths, router) = routes!(upload);
    (schemas, paths, router.layer(DefaultBodyLimit::disable()))
}

/// Prüft die Anhänge einer Eingabe (WEB-006): Anzahl, Typ, Zugehörigkeit zur Session und
/// Capability `images` für Bilder und PDF.
pub async fn validate(
    state: &AppState,
    session: beton_core::id::SessionId,
    attachments: &[Attachment],
    images: bool,
) -> Result<(), Problem> {
    let limits = state.runtime.attachments;
    if attachments.len() > limits.max_files as usize {
        return Err(Problem::new(ProblemCode::ValidationFailed).detail(format!(
            "Höchstens {} Dateien pro Nachricht.",
            limits.max_files
        )));
    }
    for a in attachments {
        let Some(k) = kind(&a.mime) else {
            return Err(Problem::new(ProblemCode::UnsupportedMediaType)
                .detail(format!("{}: Medientyp nicht erlaubt", a.name)));
        };
        if k.binary() && !images {
            return Err(
                Problem::new(ProblemCode::CapabilityUnsupported).detail(format!(
                    "{}: Dieser Harness nimmt keine Bilder oder PDFs an; Textdateien gehen.",
                    a.name
                )),
            );
        }
        // Nur Blobs dieser Session (DATA-006 AC1).
        let bytes = state
            .store
            .session_blob(state.local.org, session, &a.blob)
            .await?;
        if bytes.len() as u64 != a.size {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail(format!("{}: Größe passt nicht zum Blob", a.name)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_006_allowed_types_and_names() {
        assert_eq!(kind("image/png"), Some(Kind::Image));
        assert_eq!(kind("IMAGE/JPEG; q=1"), Some(Kind::Image));
        assert_eq!(kind("application/pdf"), Some(Kind::Pdf));
        assert_eq!(kind("text/markdown; charset=utf-8"), Some(Kind::Text));
        assert_eq!(kind("application/zip"), None);
        assert_eq!(kind("image/svg+xml"), None, "SVG kann Skripte enthalten");
        assert_eq!(kind(""), None);
        assert_eq!(display_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(display_name(Some("C:\\x\\bild.png")), "bild.png");
        assert_eq!(display_name(Some("a\u{0}b")), "ab");
        assert_eq!(display_name(None), "anhang");
        assert_eq!(human_size(48 * 1024 * 1024), "48 MB");
        assert_eq!(AttachmentLimits::default().max_file_bytes, 20 * 1024 * 1024);
        assert_eq!(AttachmentLimits::default().max_files, 10);
    }
}
