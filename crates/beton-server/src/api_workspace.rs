//! Workspace-API einer Session (SES-017, SES-018): Dateibaum, Dateien lesen und schreiben,
//! Suche, Änderungen und zeilengenaue Diffs.
//!
//! Alle Pfade sind relativ zum Workspace (Worktree bzw. Arbeitsverzeichnis) und werden über
//! [`crate::workspace::Workspace`] begrenzt: `..`, absolute Pfade, `.git` und Symlinks aus dem
//! Workspace hinaus liefern `403 path_outside_workspace` (fail closed).

use std::collections::HashSet;
use std::path::PathBuf;

use axum::Extension;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::event::{Actor, EventPayload, FsChange, FsChangeKind, FsChanged};
use beton_core::id::{PrincipalId, SessionId, TurnId, UserId};
use beton_git::ShadowRepo;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::UtoipaMethodRouter;
use utoipa_axum::routes;

use crate::app::AppState;
use crate::extract::{ApiJson, ApiQuery, PageQuery, encode_cursor};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;
use crate::workspace::{EntryKind, PathError, SearchMode, Workspace};

/// Größte Datei, die `GET …/files/{path}` inline als Text liefert; darüber `?download=true`.
pub const INLINE_MAX: u64 = 5 * 1024 * 1024;
/// Größte Datei für den Download über die API.
pub const DOWNLOAD_MAX: u64 = 256 * 1024 * 1024;
/// Größter Request-Body beim Schreiben (JSON mit bis zu 5 MiB Text plus Escaping).
const WRITE_BODY_MAX: usize = 16 * 1024 * 1024;

/// Pfad im OpenAPI-Dokument; im Router ein Wildcard-Segment.
pub const FILE_PATH_DOC: &str = "/v1/sessions/{id}/workspace/files/{path}";
pub const FILE_PATH_ROUTE: &str = "/v1/sessions/{id}/workspace/files/{*path}";

fn session_id(raw: &str) -> Result<SessionId, Problem> {
    raw.parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {raw}")))
}

fn path_problem(e: PathError) -> Problem {
    match e {
        PathError::Forbidden(p) => Problem::new(ProblemCode::PathOutsideWorkspace).detail(format!(
            "Pfad `{p}` liegt außerhalb des Workspace oder ist nicht erlaubt (`..`, absolute \
             Pfade, `.git`, Symlinks nach draußen)."
        )),
        PathError::NotFound(p) => Problem::new(ProblemCode::NotFound).detail(format!("`{p}`")),
        PathError::Invalid(p, why) => {
            Problem::new(ProblemCode::ValidationFailed).detail(format!("`{p}`: {why}"))
        }
        PathError::Stale(p) => Problem::new(ProblemCode::PreconditionFailed).detail(format!(
            "`{p}` wurde inzwischen geändert (If-Match veraltet); neu laden und erneut speichern."
        )),
        PathError::Io(kind) => Problem::internal(&format!("Workspace-E/A: {kind}")),
    }
}

/// Session laden, Leserecht prüfen (fail closed) und ihren Workspace öffnen.
async fn open(
    state: &AppState,
    auth: Authenticated,
    raw_id: &str,
) -> Result<(beton_store::SessionRecord, Workspace), Problem> {
    let record = state
        .store
        .session(state.local.org, session_id(raw_id)?)
        .await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    let root = state.sessions().workspace_root(&record).await?;
    let ws = Workspace::open(&root).map_err(|_| {
        Problem::new(ProblemCode::NotFound).detail("Der Workspace der Session existiert nicht mehr")
    })?;
    Ok((record, ws))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Problem> + Send + 'static,
) -> Result<T, Problem> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Problem::internal(&e))?
}

/// Normalisierte Schreibweise eines Workspace-Pfads (`a/b`), nach der Pfadprüfung.
fn normalized(raw: &str) -> Result<String, Problem> {
    Ok(crate::workspace::components(raw)
        .map_err(path_problem)?
        .join("/"))
}

fn etag(sha: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("\"{sha}\"")).unwrap_or(HeaderValue::from_static("\"\""))
}

// ---------------------------------------------------------------------------
// Überblick
// ---------------------------------------------------------------------------

/// Eigenschaften des Workspace, die Clients vor weiteren Abfragen brauchen.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct WorkspaceInfo {
    /// Der Workspace liegt in einem Git-Repository: Die Sichten `uncommitted` und `branch`
    /// stehen zur Verfügung (sonst `409 not_a_git_repo`, nur `turn`).
    pub git_repo: bool,
}

/// Überblick über den Workspace der Session (SES-018): Clients wählen damit die Sicht der
/// Änderungen, ohne eine erwartbar fehlschlagende Abfrage zu stellen.
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace", tag = "workspace",
    params(("id" = String, Path)),
    responses((status = 200, description = "Überblick", body = WorkspaceInfo),
              (status = 404, description = "Session oder Workspace unbekannt", body = Problem, content_type = "application/problem+json")))]
pub async fn workspace_info(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<WorkspaceInfo>> {
    let (_, ws) = open(&state, auth, &id).await?;
    let root = ws.root().to_path_buf();
    let git_repo = blocking(move || Ok(beton_git::changes::is_repo(&root))).await?;
    Ok(axum::Json(WorkspaceInfo { git_repo }))
}

// ---------------------------------------------------------------------------
// Dateibaum
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TreeQuery {
    /// Verzeichnis relativ zum Workspace (leer = Wurzel).
    pub path: Option<String>,
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct TreeEntry {
    pub name: String,
    /// Pfad relativ zum Workspace, mit `/` getrennt.
    pub path: String,
    pub kind: EntryKind,
    /// Größe in Bytes (nur Dateien).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct TreePage {
    pub path: String,
    pub items: Vec<TreeEntry>,
    pub next_cursor: Option<String>,
}

/// Eine Ebene des Dateibaums, nach Namen sortiert, ohne `.git` (SES-017).
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace/tree", tag = "workspace",
    params(("id" = String, Path), TreeQuery),
    responses((status = 200, description = "Einträge des Verzeichnisses", body = TreePage),
              (status = 403, description = "Pfad außerhalb des Workspace", body = Problem, content_type = "application/problem+json"),
              (status = 404, description = "Session oder Verzeichnis unbekannt", body = Problem, content_type = "application/problem+json")))]
pub async fn tree(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<TreeQuery>,
) -> ApiResult<axum::Json<TreePage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    let after: Option<String> = page.cursor()?;
    let (_, ws) = open(&state, auth, &id).await?;
    let dir = normalized(&q.path.unwrap_or_default())?;
    let dir_for_list = dir.clone();
    let entries = blocking(move || ws.list(&dir_for_list).map_err(path_problem)).await?;
    let mut items: Vec<TreeEntry> = entries
        .into_iter()
        .filter(|e| after.as_ref().is_none_or(|a| e.name > *a))
        .take(limit + 1)
        .map(|e| TreeEntry {
            name: e.name,
            path: e.path,
            kind: e.kind,
            size: e.size,
        })
        .collect();
    let next_cursor = (items.len() > limit)
        .then(|| {
            items.truncate(limit);
            items.last().map(|e| encode_cursor(&e.name))
        })
        .flatten();
    Ok(axum::Json(TreePage {
        path: dir,
        items,
        next_cursor,
    }))
}

// ---------------------------------------------------------------------------
// Dateien
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileQuery {
    /// `true`: Rohdaten als Download statt JSON (für Dateien über 5 MiB).
    pub download: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct WorkspaceFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 des Inhalts; zugleich das `ETag` für `If-Match`.
    pub sha256: String,
    /// Inhalt ist kein UTF-8-Text.
    pub binary: bool,
    /// Größer als 5 MiB: kein Inline-Inhalt, Download über `?download=true`.
    pub too_large: bool,
    /// Text der Datei (nur UTF-8 und höchstens 5 MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct WriteFileRequest {
    /// Neuer Inhalt (UTF-8).
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct WrittenFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// Die Datei wurde neu angelegt.
    pub created: bool,
}

/// Datei lesen: JSON mit Inhalt und `ETag` (SHA-256), über 5 MiB nur Metadaten; mit
/// `?download=true` die Rohdaten (SES-017).
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace/files/{*path}", tag = "workspace",
    params(("id" = String, Path), ("path" = String, Path, description = "Dateipfad relativ zum Workspace"), FileQuery),
    responses((status = 200, description = "Datei", body = WorkspaceFile),
              (status = 403, description = "Pfad außerhalb des Workspace", body = Problem, content_type = "application/problem+json"),
              (status = 404, description = "Session oder Datei unbekannt", body = Problem, content_type = "application/problem+json")))]
pub async fn get_file(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, path)): Path<(String, String)>,
    ApiQuery(q): ApiQuery<FileQuery>,
) -> ApiResult<Response> {
    let (_, ws) = open(&state, auth, &id).await?;
    let path = normalized(&path)?;
    let raw = path.clone();
    let resolved = blocking(move || {
        let p = ws.resolve_existing(&raw).map_err(path_problem)?;
        if !p.is_file() {
            return Err(path_problem(PathError::Invalid(
                raw,
                "ist keine Datei".into(),
            )));
        }
        let size = std::fs::metadata(&p)
            .map_err(|e| Problem::internal(&e))?
            .len();
        Ok((p, size))
    })
    .await?;
    let (file, size) = resolved;
    if q.download == Some(true) {
        if size > DOWNLOAD_MAX {
            return Err(Problem::new(ProblemCode::PayloadTooLarge)
                .detail("Datei ist für den Download über die API zu groß"));
        }
        let bytes = tokio::fs::read(&file)
            .await
            .map_err(|e| Problem::internal(&e))?;
        let sha = crate::workspace::sha256_hex(&bytes);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().replace(['"', '\\', '\r', '\n'], "_"))
            .unwrap_or_default();
        let mut res = ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response();
        res.headers_mut().insert(header::ETAG, etag(&sha));
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
        return Ok(res);
    }
    let body = blocking(move || {
        let sha256 = sha256_file(&file)?;
        if size > INLINE_MAX {
            return Ok(WorkspaceFile {
                path,
                size,
                sha256,
                binary: false,
                too_large: true,
                content: None,
            });
        }
        let bytes = std::fs::read(&file).map_err(|e| Problem::internal(&e))?;
        let text = String::from_utf8(bytes).ok();
        Ok(WorkspaceFile {
            path,
            size,
            sha256,
            binary: text.is_none(),
            too_large: false,
            content: text,
        })
    })
    .await?;
    let mut res = axum::Json(&body).into_response();
    res.headers_mut().insert(header::ETAG, etag(&body.sha256));
    Ok(res)
}

fn sha256_file(path: &std::path::Path) -> Result<String, Problem> {
    use sha2::Digest as _;
    let mut file = std::fs::File::open(path).map_err(|e| Problem::internal(&e))?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| Problem::internal(&e))?;
    Ok(hex::encode(hasher.finalize()))
}

/// Datei schreiben; mit `If-Match: <sha256>` nur, wenn sie seitdem unverändert ist, sonst
/// `412` ohne Änderung (SES-017 AC2). Erzeugt `fs.changed` mit dem User als Actor.
#[utoipa::path(put, path = "/v1/sessions/{id}/workspace/files/{*path}", tag = "workspace",
    params(("id" = String, Path), ("path" = String, Path, description = "Dateipfad relativ zum Workspace"),
           ("If-Match" = Option<String>, Header, description = "SHA-256 (ETag) des zuletzt gelesenen Inhalts")),
    request_body = WriteFileRequest,
    responses((status = 200, description = "Geschrieben", body = WrittenFile),
              (status = 201, description = "Angelegt", body = WrittenFile),
              (status = 403, description = "Pfad außerhalb des Workspace", body = Problem, content_type = "application/problem+json"),
              (status = 412, description = "If-Match veraltet; Datei unverändert", body = Problem, content_type = "application/problem+json")))]
pub async fn put_file(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, path)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<WriteFileRequest>,
) -> ApiResult<Response> {
    let (record, ws) = open(&state, auth, &id).await?;
    let if_match = headers
        .get(header::IF_MATCH)
        .map(|v| v.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| Problem::new(ProblemCode::BadRequest).detail("If-Match ist kein Text"))?;
    let path = normalized(&path)?;
    let raw = path.clone();
    let written = blocking(move || {
        ws.write(&raw, req.content.as_bytes(), if_match.as_deref())
            .map_err(path_problem)
    })
    .await?;
    // Lokal handelt immer `usr_local` (wie in `api_sessions`).
    let by = PrincipalId::User(UserId::LOCAL);
    state
        .sessions()
        .append(
            record.id,
            Actor::User {
                id: by,
                device_id: None,
            },
            EventPayload::FsChanged(FsChanged {
                changes: vec![FsChange {
                    path: path.clone(),
                    change: if written.created {
                        FsChangeKind::Added
                    } else {
                        FsChangeKind::Modified
                    },
                    from: None,
                }],
                source: FS_SOURCE_API.into(),
            }),
        )
        .await?;
    let status = if written.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let mut res = (
        status,
        axum::Json(WrittenFile {
            path,
            size: written.size,
            sha256: written.sha256.clone(),
            created: written.created,
        }),
    )
        .into_response();
    res.headers_mut()
        .insert(header::ETAG, etag(&written.sha256));
    Ok(res)
}

/// `source` von `fs.changed` bei Schreibzugriffen über die Workspace-API.
pub const FS_SOURCE_API: &str = "api";

/// Routen für `…/files/{path}` (GET, PUT) mit größerem Body-Limit für PUT.
pub fn file_routes() -> UtoipaMethodRouter<AppState> {
    let (schemas, paths, router) = routes!(get_file, put_file);
    (
        schemas,
        paths,
        router.layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
    )
}

// ---------------------------------------------------------------------------
// Suche
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Suchbegriff (wörtlich; Smart-Case).
    pub q: String,
    /// `name` (Dateinamen, Default), `content` (Inhalte) oder `fuzzy` (Dateinamen unscharf,
    /// nach Treffergüte; für `@` im Composer, leere Anfrage erlaubt).
    #[param(inline)]
    pub mode: Option<SearchMode>,
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct SearchHit {
    pub path: String,
    /// Zeile (1-basiert), nur bei `mode=content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub line: Option<u32>,
    /// Spalte (1-basiert, Zeichen), nur bei `mode=content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub column: Option<u32>,
    /// Zeilentext (gekürzt), nur bei `mode=content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct SearchPage {
    pub items: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    /// Nur bei `mode=fuzzy`: Zahl der Dateien im Index (WEB-006).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub total: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SearchCursor {
    path: String,
    line: u32,
}

/// Dateinamen- und Inhaltssuche mit ripgrep-Semantik (`.gitignore` respektiert) (SES-017).
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace/search", tag = "workspace",
    params(("id" = String, Path), SearchQuery),
    responses((status = 200, description = "Treffer", body = SearchPage)))]
pub async fn search(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<SearchQuery>,
) -> ApiResult<axum::Json<SearchPage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    let after = page.cursor::<SearchCursor>()?.map(|c| (c.path, c.line));
    let (_, ws) = open(&state, auth, &id).await?;
    let mode = q.mode.unwrap_or_default();
    let query = q.q;
    if mode == SearchMode::Fuzzy {
        // `@`-Suche (WEB-006 AC2): aus dem Dateiindex, ohne Cursor.
        let cache = state.runtime.file_index.clone();
        let (items, total) = blocking(move || {
            let files = cache.get(ws.root());
            let items = crate::file_index::fuzzy(&files, &query, limit)
                .into_iter()
                .map(|path| SearchHit {
                    path: path.to_owned(),
                    line: None,
                    column: None,
                    text: None,
                })
                .collect();
            Ok((items, files.len() as u64))
        })
        .await?;
        return Ok(axum::Json(SearchPage {
            items,
            next_cursor: None,
            total: Some(total),
        }));
    }
    let (hits, more) =
        blocking(move || ws.search(&query, mode, after, limit).map_err(path_problem)).await?;
    let next_cursor = more
        .then(|| {
            hits.last().map(|h| {
                encode_cursor(&SearchCursor {
                    path: h.path.clone(),
                    line: h.line.unwrap_or(0),
                })
            })
        })
        .flatten();
    Ok(axum::Json(SearchPage {
        items: hits
            .into_iter()
            .map(|h| SearchHit {
                path: h.path,
                line: h.line,
                column: h.column,
                text: h.text,
            })
            .collect(),
        next_cursor,
        total: None,
    }))
}

// ---------------------------------------------------------------------------
// Änderungen und Diffs (SES-018)
// ---------------------------------------------------------------------------

/// Sicht der Änderungsübersicht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeScope {
    /// Arbeitsverzeichnis gegenüber `HEAD`, inklusive unversionierter Dateien.
    #[default]
    Uncommitted,
    /// Branch gegenüber seiner Base: `git diff <merge-base>...HEAD`.
    Branch,
    /// Änderungen eines Turns (aus `fs.changed` und den Snapshots vor/nach dem Turn).
    Turn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl From<beton_git::ChangeStatus> for ChangeStatus {
    fn from(s: beton_git::ChangeStatus) -> Self {
        match s {
            beton_git::ChangeStatus::Added => Self::Added,
            beton_git::ChangeStatus::Modified => Self::Modified,
            beton_git::ChangeStatus::Deleted => Self::Deleted,
            beton_git::ChangeStatus::Renamed => Self::Renamed,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ChangedFile {
    pub path: String,
    /// Alter Pfad bei Umbenennung.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// Hinzugefügte Zeilen; fehlt bei Binärdateien.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub additions: Option<u32>,
    /// Entfernte Zeilen; fehlt bei Binärdateien.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deletions: Option<u32>,
}

impl From<beton_git::ChangedFile> for ChangedFile {
    fn from(f: beton_git::ChangedFile) -> Self {
        Self {
            path: f.path,
            old_path: f.old_path,
            status: f.status.into(),
            additions: f.additions,
            deletions: f.deletions,
        }
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ChangesQuery {
    /// `uncommitted` (Default), `branch` oder `turn`.
    #[param(inline)]
    pub scope: Option<ChangeScope>,
    /// Turn-ID bei `scope=turn`; ohne Angabe der letzte Turn.
    pub turn: Option<String>,
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ChangesPage {
    pub scope: ChangeScope,
    /// Vergleichsbasis: `HEAD` (uncommitted), Merge-Base (branch) bzw. Snapshot vor dem Turn.
    pub base_sha: String,
    /// Vergleichsstand: `HEAD` (branch) bzw. Snapshot nach dem Turn; fehlt bei `uncommitted`
    /// (Arbeitsverzeichnis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head_sha: Option<String>,
    /// Turn bei `scope=turn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<String>,
    pub items: Vec<ChangedFile>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// Zeilennummer auf der alten Seite (fehlt bei hinzugefügten Zeilen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_line: Option<u32>,
    /// Zeilennummer auf der neuen Seite (fehlt bei entfernten Zeilen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub new_line: Option<u32>,
    pub text: String,
    /// Zeile endet ohne Zeilenumbruch.
    pub no_newline: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct FileDiff {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub binary: bool,
    pub scope: ChangeScope,
    pub base_sha: String,
    /// Fehlt bei `uncommitted` (Arbeitsverzeichnis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head_sha: Option<String>,
    /// Unified Diff.
    pub patch: String,
    /// Zeilengenau adressierbar über `old_line`/`new_line` zusammen mit `base_sha`/`head_sha`.
    pub hunks: Vec<DiffHunk>,
}

fn diff_view(
    d: beton_git::FileDiff,
    scope: ChangeScope,
    base_sha: String,
    head_sha: Option<String>,
) -> FileDiff {
    FileDiff {
        path: d.path,
        old_path: d.old_path,
        status: d.status.into(),
        binary: d.binary,
        scope,
        base_sha,
        head_sha,
        patch: d.patch,
        hunks: d
            .hunks
            .into_iter()
            .map(|h| DiffHunk {
                header: h.header,
                old_start: h.old_start,
                old_lines: h.old_lines,
                new_start: h.new_start,
                new_lines: h.new_lines,
                lines: h
                    .lines
                    .into_iter()
                    .map(|l| DiffLine {
                        kind: match l.kind {
                            beton_git::LineKind::Context => DiffLineKind::Context,
                            beton_git::LineKind::Add => DiffLineKind::Add,
                            beton_git::LineKind::Delete => DiffLineKind::Delete,
                        },
                        old_line: l.old_line,
                        new_line: l.new_line,
                        text: l.text,
                        no_newline: l.no_newline,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn git_problem(e: &beton_git::GitError) -> Problem {
    match e {
        beton_git::GitError::NotInstalled => {
            Problem::new(ProblemCode::Unavailable).detail("git ist nicht installiert")
        }
        other => Problem::internal(other),
    }
}

fn not_a_repo() -> Problem {
    Problem::new(ProblemCode::NotAGitRepo)
        .detail("Der Workspace ist kein Git-Repository; verfügbar ist nur die Sicht `scope=turn`.")
}

/// Base der Branch-Sicht: die des Worktrees, sonst `origin/HEAD` bzw. der aktuelle Branch.
fn branch_base(record: &beton_store::SessionRecord, root: &std::path::Path) -> String {
    match &record.worktree {
        Some(wt) => wt.base.clone(),
        None => beton_git::worktree::resolve_base(&beton_git::Git::new(root), None),
    }
}

/// Ergebnis einer Sicht: Basis, Stand, Dateien.
struct ScopeView {
    base_sha: String,
    head_sha: Option<String>,
    turn_id: Option<String>,
    files: Vec<beton_git::ChangedFile>,
    /// Für Diffs der Turn-Sicht.
    shadow: Option<ShadowRepo>,
}

/// Turn-Sicht: Pfade aus den `fs.changed`-Events des Turns, Zeilenzahlen aus den Snapshots
/// vor und nach dem Turn (bzw. dem aktuellen Stand, solange er läuft).
async fn turn_view(
    state: &AppState,
    record: &beton_store::SessionRecord,
    root: PathBuf,
    turn: Option<String>,
) -> Result<ScopeView, Problem> {
    let turn: Option<TurnId> = match turn {
        Some(t) => Some(t.parse().map_err(|_| {
            Problem::new(ProblemCode::ValidationFailed).detail(format!("`{t}` ist keine Turn-ID"))
        })?),
        None => None,
    };
    // Events des Turns einsammeln (bzw. den letzten Turn bestimmen).
    let mut last_turn = None;
    let mut paths: Vec<(TurnId, String)> = Vec::new();
    let mut after = 0;
    loop {
        let page = state
            .store
            .events(state.local.org, record.id, after, 500)
            .await?;
        let Some(last) = page.last() else { break };
        after = last.seq;
        for e in &page {
            match e.payload() {
                Some(EventPayload::TurnStarted(t)) => last_turn = Some(t.turn_id),
                Some(EventPayload::FsChanged(c)) if matches!(e.actor, Actor::Agent { .. }) => {
                    if let Some(t) = e.turn_id {
                        for ch in &c.changes {
                            paths.push((t, ch.path.clone()));
                            if let Some(from) = &ch.from {
                                paths.push((t, from.clone()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let turn = turn.or(last_turn).ok_or_else(|| {
        Problem::new(ProblemCode::NotFound).detail("Die Session hat noch keinen Turn")
    })?;
    let wanted: HashSet<String> = paths
        .into_iter()
        .filter(|(t, _)| *t == turn)
        .map(|(_, p)| p)
        .collect();
    let snapshots = state.runtime.sessions.snapshots_dir(record.id);
    let turn_text = turn.to_string();
    blocking(move || {
        let shadow = ShadowRepo::existing(&snapshots, &root).ok_or_else(|| {
            Problem::new(ProblemCode::NotFound)
                .detail("Für diese Session gibt es keine Turn-Snapshots")
        })?;
        let before = shadow
            .get_ref(&beton_git::snapshot::turn_ref(&turn_text, "before"))
            .ok_or_else(|| {
                Problem::new(ProblemCode::NotFound).detail(format!("Turn {turn_text} unbekannt"))
            })?;
        let after = match shadow.get_ref(&beton_git::snapshot::turn_ref(&turn_text, "after")) {
            Some(a) => a,
            // Turn läuft noch: aktueller Stand, ohne den Index des Runners zu berühren.
            None => shadow.snapshot_detached().map_err(|e| git_problem(&e))?,
        };
        let files = shadow
            .changes(&before, &after)
            .map_err(|e| git_problem(&e))?
            .into_iter()
            .filter(|f| {
                wanted.contains(&f.path) || f.old_path.as_ref().is_some_and(|o| wanted.contains(o))
            })
            .collect();
        Ok(ScopeView {
            base_sha: before,
            head_sha: Some(after),
            turn_id: Some(turn_text),
            files,
            shadow: Some(shadow),
        })
    })
    .await
}

async fn scope_view(
    state: &AppState,
    record: &beton_store::SessionRecord,
    root: PathBuf,
    scope: ChangeScope,
    turn: Option<String>,
) -> Result<ScopeView, Problem> {
    match scope {
        ChangeScope::Turn => turn_view(state, record, root, turn).await,
        ChangeScope::Uncommitted => {
            blocking(move || {
                if !beton_git::changes::is_repo(&root) {
                    return Err(not_a_repo());
                }
                let (base, files) =
                    beton_git::changes::uncommitted(&root).map_err(|e| git_problem(&e))?;
                Ok(ScopeView {
                    base_sha: base,
                    head_sha: None,
                    turn_id: None,
                    files,
                    shadow: None,
                })
            })
            .await
        }
        ChangeScope::Branch => {
            let base = branch_base(record, &root);
            blocking(move || {
                if !beton_git::changes::is_repo(&root) {
                    return Err(not_a_repo());
                }
                let (mb, head, files) =
                    beton_git::changes::branch(&root, &base).map_err(|e| match e {
                        beton_git::GitError::Failed { .. } => Problem::new(ProblemCode::Conflict)
                            .detail(format!("Base `{base}` ist nicht auflösbar")),
                        other => git_problem(&other),
                    })?;
                Ok(ScopeView {
                    base_sha: mb,
                    head_sha: Some(head),
                    turn_id: None,
                    files,
                    shadow: None,
                })
            })
            .await
        }
    }
}

/// Änderungsübersicht in drei Sichten (SES-018): `uncommitted`, `branch` (wie
/// `git diff <merge-base>...HEAD`), `turn` (auch ohne Git). In einem Verzeichnis ohne Git liefern
/// `uncommitted` und `branch` `409 not_a_git_repo`.
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace/changes", tag = "workspace",
    params(("id" = String, Path), ChangesQuery),
    responses((status = 200, description = "Geänderte Dateien", body = ChangesPage),
              (status = 409, description = "Kein Git-Repository", body = Problem, content_type = "application/problem+json")))]
pub async fn changes(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<ChangesQuery>,
) -> ApiResult<axum::Json<ChangesPage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    let after: Option<String> = page.cursor()?;
    let (record, ws) = open(&state, auth, &id).await?;
    let scope = q.scope.unwrap_or_default();
    let view = scope_view(&state, &record, ws.root().to_path_buf(), scope, q.turn).await?;
    let mut items: Vec<ChangedFile> = view
        .files
        .into_iter()
        .filter(|f| after.as_ref().is_none_or(|a| f.path > *a))
        .take(limit + 1)
        .map(ChangedFile::from)
        .collect();
    let next_cursor = (items.len() > limit)
        .then(|| {
            items.truncate(limit);
            items.last().map(|f| encode_cursor(&f.path))
        })
        .flatten();
    Ok(axum::Json(ChangesPage {
        scope,
        base_sha: view.base_sha,
        head_sha: view.head_sha,
        turn_id: view.turn_id,
        items,
        next_cursor,
    }))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DiffQuery {
    /// Dateipfad relativ zum Workspace.
    pub path: String,
    /// `uncommitted` (Default), `branch` oder `turn`.
    #[param(inline)]
    pub scope: Option<ChangeScope>,
    /// Turn-ID bei `scope=turn`; ohne Angabe der letzte Turn.
    pub turn: Option<String>,
}

/// Unified Diff einer Datei mit zeilengenau adressierbaren Hunks und `base_sha`/`head_sha`
/// (SES-018).
#[utoipa::path(get, path = "/v1/sessions/{id}/workspace/diff", tag = "workspace",
    params(("id" = String, Path), DiffQuery),
    responses((status = 200, description = "Diff der Datei", body = FileDiff),
              (status = 404, description = "Datei in dieser Sicht unverändert", body = Problem, content_type = "application/problem+json"),
              (status = 409, description = "Kein Git-Repository", body = Problem, content_type = "application/problem+json")))]
pub async fn diff(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<DiffQuery>,
) -> ApiResult<axum::Json<FileDiff>> {
    let (record, ws) = open(&state, auth, &id).await?;
    // Derselbe Pfadschutz wie beim Lesen (auch für gelöschte Dateien: nur die Form zählt).
    let parts = crate::workspace::components(&q.path).map_err(path_problem)?;
    if parts.is_empty() {
        return Err(Problem::new(ProblemCode::ValidationFailed).detail("path fehlt"));
    }
    let path = parts.join("/");
    let scope = q.scope.unwrap_or_default();
    let root = ws.root().to_path_buf();
    let unchanged = || {
        Problem::new(ProblemCode::NotFound)
            .detail(format!("`{path}` ist in dieser Sicht unverändert"))
    };
    match scope {
        ChangeScope::Turn => {
            let view = turn_view(&state, &record, root, q.turn).await?;
            if !view
                .files
                .iter()
                .any(|f| f.path == path || f.old_path.as_deref() == Some(path.as_str()))
            {
                return Err(unchanged());
            }
            let (base, head) = (
                view.base_sha.clone(),
                view.head_sha.clone().unwrap_or_default(),
            );
            let shadow = view.shadow.ok_or_else(unchanged)?;
            let p = path.clone();
            let d = blocking(move || shadow.diff(&base, &head, &p).map_err(|e| git_problem(&e)))
                .await?
                .ok_or_else(unchanged)?;
            Ok(axum::Json(diff_view(
                d,
                scope,
                view.base_sha,
                view.head_sha,
            )))
        }
        ChangeScope::Uncommitted => {
            let p = path.clone();
            let (base, d) = blocking(move || {
                if !beton_git::changes::is_repo(&root) {
                    return Err(not_a_repo());
                }
                let git = beton_git::Git::new(&root);
                let base = beton_git::changes::head_or_empty(&git).map_err(|e| git_problem(&e))?;
                let d =
                    beton_git::changes::diff_uncommitted(&root, &p).map_err(|e| git_problem(&e))?;
                Ok((base, d))
            })
            .await?;
            let d = d.ok_or_else(unchanged)?;
            Ok(axum::Json(diff_view(d, scope, base, None)))
        }
        ChangeScope::Branch => {
            let base = branch_base(&record, &root);
            let p = path.clone();
            let (mb, head, d) = blocking(move || {
                if !beton_git::changes::is_repo(&root) {
                    return Err(not_a_repo());
                }
                beton_git::changes::diff_branch(&root, &base, &p).map_err(|e| git_problem(&e))
            })
            .await?;
            let d = d.ok_or_else(unchanged)?;
            Ok(axum::Json(diff_view(d, scope, mb, Some(head))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_route_is_documented_without_wildcard() {
        let doc = crate::app::openapi();
        assert!(doc.paths.paths.contains_key(FILE_PATH_DOC));
        assert!(!doc.paths.paths.contains_key(FILE_PATH_ROUTE));
    }
}
