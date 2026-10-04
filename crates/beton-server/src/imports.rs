//! Import fremder Chats (SES-008): vorhandene Claude-Code- und Codex-Sessions werden zu
//! beton-Sessions. Parsing ist Sache der Harness-Adapter (HAR-023, HAR-024).
//!
//! - `GET /v1/imports/candidates?harness=claude|codex` listet die lokalen Sessions des Hosts.
//! - `POST /v1/imports {harness, refs[] | last_n, force}` importiert sie. Dedup-Schlüssel ist
//!   `(host_id, harness, vendor_session_id)`; ohne `force` liefert ein zweiter Import
//!   `skipped: already_imported` mit der bestehenden Session.
//!
//! Datenschutz: Die Dateien werden nur auf diese ausdrückliche Anfrage gelesen, nie im
//! Hintergrund. Clients nennen Vendor-Session-IDs, nie Pfade; welche Datei dazu gehört,
//! bestimmt allein die Discovery der Adapter über ihre Pfad-Allowlist. Inhalte erscheinen
//! nicht im Log.

use std::path::Path;
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use beton_core::event::{
    Actor, Event, EventPayload, Notice, NoticeLevel, SessionImported, SessionKind, SessionStatus,
    SessionStatusChanged, SessionTitleChanged, SessionTrigger, SystemComponent, TitleSource,
};
use beton_core::id::{PrincipalId, SessionId, UserId};
use beton_core::time::Timestamp;
use beton_harness::HarnessId;
use beton_harness::import::{
    ExternalSessionRef, ImportError, ImportedTranscript, TranscriptImporter,
};
use beton_store::{DeleteAuthority, ImportKey, NewSession};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::{ApiJson, ApiQuery, PageQuery, encode_cursor};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Cursor der Kandidatenliste: Position in der nach Änderung sortierten Liste.
#[derive(Debug, Serialize, Deserialize)]
struct CandidateCursor {
    offset: usize,
}

/// Höchstzahl der Sessions je Import-Anfrage.
pub const MAX_IMPORTS: usize = 500;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CandidatesQuery {
    /// `claude` oder `codex`.
    pub harness: String,
    /// Anzahl der Einträge, 1 bis 200 (Default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<String>,
    /// Opaker Cursor aus `next_cursor` der vorigen Seite.
    pub cursor: Option<String>,
}

/// Eine lokale Session einer Vendor-CLI.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportCandidate {
    pub harness: String,
    /// Session-ID der Vendor-CLI; mit ihr wird importiert.
    pub vendor_session_id: String,
    /// Verlaufsdatei auf dem Host (nur zur Anzeige).
    pub path: String,
    /// Arbeitsverzeichnis, in dem der Chat lief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    /// Letzte Änderung der Datei (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub updated_at: Option<String>,
    pub size_bytes: u64,
    /// Bereits importiert: die beton-Session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub imported_session_id: Option<String>,
}

/// Kandidaten, zuletzt geänderte zuerst.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportCandidatePage {
    pub items: Vec<ImportCandidate>,
    pub next_cursor: Option<String>,
}

/// Import-Anfrage: entweder `refs` oder `last_n`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportRequest {
    /// `claude` oder `codex`.
    pub harness: String,
    /// Vendor-Session-IDs aus den Kandidaten.
    #[serde(default)]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub refs: Vec<String>,
    /// Die `n` zuletzt geänderten Sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_n: Option<u32>,
    /// Auch bereits importierte Sessions erneut importieren (neue Session).
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Imported,
    Skipped,
    Failed,
}

/// Hinweis des Parsers (ohne Inhalte, nur Zeilennummern und Zahlen).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportWarningView {
    /// z. B. `corrupt_line`, `discarded_branches`, `unmapped`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub count: Option<u64>,
    pub detail: String,
}

/// Ergebnis je Vendor-Session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportResult {
    pub vendor_session_id: String,
    pub status: ImportStatus,
    /// Bei `skipped` bzw. `failed`, z. B. `already_imported`, `not_found`, `unknown_format`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// Neue bzw. (bei `already_imported`) bestehende Session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<String>,
    /// Übernommene Inhalts-Events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub events: Option<u64>,
    pub warnings: Vec<ImportWarningView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
pub struct ImportResponse {
    pub results: Vec<ImportResult>,
}

impl ImportResult {
    fn new(vendor_session_id: &str, status: ImportStatus) -> Self {
        Self {
            vendor_session_id: vendor_session_id.to_owned(),
            status,
            reason: None,
            detail: None,
            session_id: None,
            events: None,
            warnings: Vec::new(),
        }
    }

    fn failed(vendor_session_id: &str, reason: &str, detail: String) -> Self {
        Self {
            reason: Some(reason.to_owned()),
            detail: Some(detail),
            ..Self::new(vendor_session_id, ImportStatus::Failed)
        }
    }
}

fn import_problem(e: &ImportError) -> Problem {
    match e {
        ImportError::Unconfigured(d) => Problem::new(ProblemCode::Unavailable).detail(d.clone()),
        ImportError::NotAllowed(d) => Problem::new(ProblemCode::Forbidden).detail(d.clone()),
        other => Problem::internal(other),
    }
}

/// Importer des Harness (Capability `transcript_import`).
fn importer(
    state: &AppState,
    harness: &str,
) -> Result<(HarnessId, Arc<dyn TranscriptImporter>), Problem> {
    let id: HarnessId = harness
        .parse()
        .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(format!("{e}")))?;
    let adapter = state.runtime.harnesses.get(&id).ok_or_else(|| {
        Problem::new(ProblemCode::ValidationFailed)
            .detail(format!("Harness `{id}` ist auf diesem Host nicht bekannt"))
    })?;
    let importer = adapter.transcript_importer().ok_or_else(|| {
        Problem::new(ProblemCode::CapabilityUnsupported).detail(format!(
            "Harness `{id}` unterstützt keinen Import (transcript_import)"
        ))
    })?;
    Ok((id, importer))
}

async fn discover(
    state: &AppState,
    importer: Arc<dyn TranscriptImporter>,
) -> Result<Vec<ExternalSessionRef>, Problem> {
    let env = state.runtime.sessions.vendor_env.clone();
    tokio::task::spawn_blocking(move || importer.discover(&env))
        .await
        .map_err(|e| Problem::internal(&e))?
        .map_err(|e| import_problem(&e))
}

/// Lokale Sessions einer Vendor-CLI (SES-008). Liest nur Verlaufsdateien.
#[utoipa::path(get, path = "/v1/imports/candidates", tag = "imports",
    params(CandidatesQuery),
    responses((status = 200, description = "Kandidaten, zuletzt geänderte zuerst", body = ImportCandidatePage),
              (status = 409, description = "`capability_unsupported`: Harness ohne Import", body = Problem, content_type = "application/problem+json"),
              (status = 503, description = "Verzeichnis der CLI unbekannt (kein `HOME`)", body = Problem, content_type = "application/problem+json")))]
pub async fn list_candidates(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<CandidatesQuery>,
) -> ApiResult<axum::Json<ImportCandidatePage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    let offset = page.cursor::<CandidateCursor>()?.map_or(0, |c| c.offset);
    let (id, importer) = importer(&state, &q.harness)?;
    let found = discover(&state, importer).await?;
    let imported = state
        .store
        .imported_sessions(state.local.org, state.local.node, id.as_str())
        .await?;
    let next_cursor = (found.len() > offset + limit).then(|| {
        encode_cursor(&CandidateCursor {
            offset: offset + limit,
        })
    });
    Ok(axum::Json(ImportCandidatePage {
        next_cursor,
        items: found
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|r| ImportCandidate {
                imported_session_id: imported.get(&r.id).map(ToString::to_string),
                harness: r.harness,
                vendor_session_id: r.id,
                path: r.path.display().to_string(),
                cwd: r.cwd,
                title: r.title,
                model: r.model,
                updated_at: r.updated_at.map(|t| t.to_string()),
                size_bytes: r.size_bytes,
            })
            .collect(),
    }))
}

/// Importiert Sessions einer Vendor-CLI als beton-Sessions (SES-008). Ergebnis je Session:
/// `imported`, `skipped` (`already_imported`, mit der bestehenden Session) oder `failed`.
#[utoipa::path(post, path = "/v1/imports", tag = "imports",
    request_body = ImportRequest,
    responses((status = 200, description = "Ergebnis je Session", body = ImportResponse),
              (status = 409, description = "`capability_unsupported`: Harness ohne Import", body = Problem, content_type = "application/problem+json")))]
pub async fn import_sessions(
    State(state): State<AppState>,
    Extension(_auth): Extension<Authenticated>,
    ApiJson(req): ApiJson<ImportRequest>,
) -> ApiResult<axum::Json<ImportResponse>> {
    // M1: lokal handelt immer `usr_local`.
    let by = UserId::LOCAL;
    let invalid = |d: &str| Problem::new(ProblemCode::ValidationFailed).detail(d.to_owned());
    match (req.refs.is_empty(), req.last_n) {
        (true, None) => return Err(invalid("`refs` oder `last_n` angeben")),
        (false, Some(_)) => return Err(invalid("entweder `refs` oder `last_n`, nicht beides")),
        (_, Some(0)) => return Err(invalid("`last_n` muss mindestens 1 sein")),
        _ => {}
    }
    if req.refs.len() > MAX_IMPORTS || req.last_n.is_some_and(|n| n as usize > MAX_IMPORTS) {
        return Err(invalid(&format!(
            "höchstens {MAX_IMPORTS} Sessions je Anfrage"
        )));
    }
    let (id, importer) = importer(&state, &req.harness)?;
    let _guard = state.runtime.sessions.import_lock.lock().await;
    let found = discover(&state, importer.clone()).await?;
    let selected: Vec<(String, Option<ExternalSessionRef>)> = match req.last_n {
        Some(n) => found
            .into_iter()
            .take(n as usize)
            .map(|r| (r.id.clone(), Some(r)))
            .collect(),
        None => req
            .refs
            .iter()
            .map(|wanted| {
                (
                    wanted.clone(),
                    found.iter().find(|r| r.id == *wanted).cloned(),
                )
            })
            .collect(),
    };
    let mut results = Vec::with_capacity(selected.len());
    for (vendor_id, r) in selected {
        let Some(r) = r else {
            results.push(ImportResult::failed(
                &vendor_id,
                "not_found",
                "Keine solche Session in den Verzeichnissen der CLI".into(),
            ));
            continue;
        };
        results.push(import_one(&state, &id, importer.clone(), r, req.force, by).await?);
    }
    Ok(axum::Json(ImportResponse { results }))
}

async fn import_one(
    state: &AppState,
    harness: &HarnessId,
    importer: Arc<dyn TranscriptImporter>,
    r: ExternalSessionRef,
    force: bool,
    by: UserId,
) -> Result<ImportResult, Problem> {
    let org = state.local.org;
    let vendor_id = r.id.clone();
    let key = ImportKey {
        host: state.local.node,
        harness: harness.as_str(),
        vendor_session_id: &vendor_id,
    };
    if !force && let Some(existing) = state.store.imported_session(org, &key).await? {
        return Ok(ImportResult {
            reason: Some("already_imported".into()),
            session_id: Some(existing.to_string()),
            ..ImportResult::new(&vendor_id, ImportStatus::Skipped)
        });
    }
    let fallback_cwd = r.cwd.clone();
    let parsed = tokio::task::spawn_blocking(move || importer.parse(&r))
        .await
        .map_err(|e| Problem::internal(&e))?;
    let t = match parsed {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!(harness = %harness, code = e.code(), "Import fehlgeschlagen");
            return Ok(ImportResult::failed(&vendor_id, e.code(), e.to_string()));
        }
    };
    let Some(cwd) = t.cwd.clone().or(fallback_cwd) else {
        return Ok(ImportResult::failed(
            &vendor_id,
            "unknown_cwd",
            "Die Session nennt kein Arbeitsverzeichnis".into(),
        ));
    };
    let session = state
        .store
        .create_session(
            org,
            NewSession {
                id: SessionId::new(),
                owner: by,
                kind: SessionKind::Main,
                harness: harness.to_string(),
                cwd,
                model: t.model.clone(),
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: state.local.node,
                harness_opts: serde_json::Value::Null,
            },
        )
        .await?;
    let warnings: Vec<ImportWarningView> = t
        .warnings
        .iter()
        .map(|w| ImportWarningView {
            kind: w.kind.as_str().to_owned(),
            line: w.line,
            count: w.count,
            detail: w.detail.clone(),
        })
        .collect();
    let content = t.events.len() as u64;
    let written = async {
        append_import(state, session.id, harness, &vendor_id, by, t).await?;
        state
            .store
            .record_import(org, &key, session.id, Timestamp::now(), force)
            .await?;
        Ok::<(), Problem>(())
    }
    .await;
    if let Err(e) = written {
        // Keine halbe Session zurücklassen.
        let _ = state
            .store
            .delete_session(
                org,
                session.id,
                PrincipalId::User(by),
                DeleteAuthority::Owner,
            )
            .await;
        return Err(e);
    }
    tracing::debug!(session_id = %session.id, harness = %harness, events = content, "Session importiert");
    Ok(ImportResult {
        session_id: Some(session.id.to_string()),
        events: Some(content),
        warnings,
        ..ImportResult::new(&vendor_id, ImportStatus::Imported)
    })
}

/// Schreibt die Events einer importierten Session: `session.imported`, Titel, ggf. ein
/// Hinweis mit den Warnungen, der Verlauf mit Ursprungs-Zeitstempeln und `stopped`.
async fn append_import(
    state: &AppState,
    session: SessionId,
    harness: &HarnessId,
    vendor_id: &str,
    by: UserId,
    t: ImportedTranscript,
) -> Result<(), Problem> {
    let now = Timestamp::now();
    let user = Actor::User {
        id: PrincipalId::User(by),
        device_id: None,
    };
    let agent = Actor::Agent {
        id: None,
        harness: harness.to_string(),
        agent_ref: None,
    };
    let server = Actor::System {
        component: SystemComponent::Server,
    };
    let mut batch = vec![Event::new(
        session,
        0,
        user.clone(),
        EventPayload::SessionImported(SessionImported {
            source: harness.to_string(),
            vendor_session_id: vendor_id.to_owned(),
            imported_at: now,
        }),
    )];
    let title = t.title.clone().unwrap_or_else(|| {
        format!(
            "Importiert aus {}",
            beton_harness::handover::harness_label(harness.as_str())
        )
    });
    batch.push(Event::new(
        session,
        0,
        server.clone(),
        EventPayload::SessionTitleChanged(SessionTitleChanged {
            title,
            source: TitleSource::Harness,
        }),
    ));
    if !t.warnings.is_empty() {
        let text = t
            .warnings
            .iter()
            .map(|w| w.detail.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        batch.push(Event::new(
            session,
            0,
            server.clone(),
            EventPayload::Notice(Notice {
                level: NoticeLevel::Warn,
                text: format!("Import: {text}."),
            }),
        ));
    }
    for e in t.events {
        let mut ev = Event::new(
            session,
            0,
            if e.by_user {
                user.clone()
            } else {
                agent.clone()
            },
            e.payload,
        );
        ev.ts = e.ts.unwrap_or(now);
        ev.turn_id = e.turn_id;
        ev.raw = e.raw;
        batch.push(ev);
    }
    let mut stopped = Event::new(
        session,
        0,
        server,
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Stopped,
            reason: Some("imported".into()),
        }),
    );
    stopped.ts = Timestamp::now();
    batch.push(stopped);
    let events = state.events();
    let mut iter = batch.into_iter().peekable();
    while iter.peek().is_some() {
        let chunk: Vec<Event> = iter.by_ref().take(200).collect();
        let record = state.store.session(state.local.org, session).await?;
        events
            .append(
                state.local.org,
                session,
                record.head_seq,
                record.epoch,
                chunk,
            )
            .await?;
    }
    Ok(())
}

impl crate::sessions::SessionManager<'_> {
    /// Native Referenz einer importierten Session, solange die Vendor-CLI sie aus dem
    /// Arbeitsverzeichnis fortsetzen kann (Datei vorhanden, SES-008 AC2). Prüft nur die
    /// Existenz der Datei.
    pub(crate) async fn imported_native_ref(
        &self,
        harness: &str,
        cwd: &str,
        vendor_session_id: &str,
    ) -> Option<String> {
        let (_, importer) = importer(self.state(), harness).ok()?;
        let env = self.cfg().vendor_env.clone();
        let (cwd, id) = (cwd.to_owned(), vendor_session_id.to_owned());
        tokio::task::spawn_blocking(move || {
            importer
                .native_resumable(&env, Path::new(&cwd), &id)
                .then_some(id)
        })
        .await
        .ok()
        .flatten()
    }
}
