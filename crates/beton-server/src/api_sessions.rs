//! REST-Endpunkte und WS-Kommandos des Session-Lebenszyklus (SES-001, SES-002, SES-003,
//! SES-005, DATA-006, DATA-008). Jedes WS-Kommando hat hier seinen REST-Zwilling
//! (PROTO-006 AC3).

use std::sync::Arc;

use async_trait::async_trait;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::event::BlobRef;
use beton_core::id::{PrincipalId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::api::SessionSummary;
use crate::app::AppState;
use crate::commands::{Command, CommandCtx, CommandRegistry};
use crate::extract::{ApiJson, ApiQuery, PageQuery, encode_cursor};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;
use crate::sessions::CreateSession;

/// M0: Lokal handelt immer `usr_local` (Bearer und Cookie gehören demselben Menschen).
fn principal(_auth: Authenticated) -> (UserId, PrincipalId) {
    (UserId::LOCAL, PrincipalId::User(UserId::LOCAL))
}

fn session_id(raw: &str) -> Result<SessionId, Problem> {
    raw.parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {raw}")))
}

fn summary(s: beton_store::SessionRecord) -> SessionSummary {
    crate::api::summary(s)
}

#[derive(Debug, Deserialize, ToSchema, TS)]
pub struct CreateSessionRequest {
    /// Harness-ID, z. B. `claude` oder (nur mit `--dev`) `fake`.
    pub target: String,
    /// Arbeitsverzeichnis (Projekt oder Worktree).
    pub cwd: String,
    #[ts(optional)]
    pub title: Option<String>,
    #[ts(optional)]
    pub model: Option<String>,
    /// Harness-spezifische Optionen, z. B. `{"scenario": "…"}` beim Fake-Harness.
    #[serde(default)]
    #[schema(value_type = Object)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub harness_opts: Value,
}

/// Session anlegen und Runner starten (SES-001).
#[utoipa::path(post, path = "/v1/sessions", tag = "sessions",
    request_body = CreateSessionRequest,
    responses((status = 201, description = "Session angelegt", body = SessionSummary)))]
pub async fn create_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    ApiJson(req): ApiJson<CreateSessionRequest>,
) -> ApiResult<Response> {
    let (user, _) = principal(auth);
    let record = state
        .sessions()
        .create(
            user,
            CreateSession {
                target: req.target,
                cwd: req.cwd,
                title: req.title,
                model: req.model,
                harness_opts: req.harness_opts,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, axum::Json(summary(record))).into_response())
}

/// Eine Session.
#[utoipa::path(get, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Session", body = SessionSummary),
              (status = 404, description = "Unbekannt oder gelöscht", body = Problem, content_type = "application/problem+json")))]
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let record = state
        .store
        .session(state.local.org, session_id(&id)?)
        .await?;
    Ok(axum::Json(summary(record)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct SessionSettings {
    /// Neuer Titel; braucht keinen laufenden Runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub effort: Option<String>,
}

/// Einstellungen ändern (z. B. Modell); ohne Capability `capability_unsupported` (HAR-002 AC3).
#[utoipa::path(patch, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = String, Path)),
    request_body = SessionSettings,
    responses((status = 200, description = "Geändert", body = SessionSummary)))]
pub async fn patch_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiJson(mut req): ApiJson<SessionSettings>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let id = session_id(&id)?;
    if let Some(title) = req.title.take() {
        state
            .sessions()
            .rename(id, &title, principal(auth).1)
            .await?;
    }
    if req.model.is_some() || req.effort.is_some() {
        let args = serde_json::to_value(&req).map_err(|e| Problem::internal(&e))?;
        state.sessions().set(id, args).await?;
    }
    Ok(axum::Json(summary(
        state.store.session(state.local.org, id).await?,
    )))
}

/// Session löschen (nur Owner, SES-001 AC3).
#[utoipa::path(delete, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 204, description = "Gelöscht; Tombstone angelegt")))]
pub async fn delete_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .sessions()
        .delete(session_id(&id)?, principal(auth).1)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Archivieren: Runner beenden, aus der Standardliste ausblenden (SES-001 AC2).
#[utoipa::path(post, path = "/v1/sessions/{id}/archive", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Archiviert", body = SessionSummary)))]
pub async fn archive_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let id = session_id(&id)?;
    state.sessions().archive(id, principal(auth).1).await?;
    Ok(axum::Json(summary(
        state.store.session(state.local.org, id).await?,
    )))
}

/// Archivierte Session wiederherstellen.
#[utoipa::path(post, path = "/v1/sessions/{id}/unarchive", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Wiederhergestellt", body = SessionSummary)))]
pub async fn unarchive_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let id = session_id(&id)?;
    state.sessions().unarchive(id, principal(auth).1).await?;
    Ok(axum::Json(summary(
        state.store.session(state.local.org, id).await?,
    )))
}

/// Laufenden Turn abbrechen; idempotent (SES-005).
#[utoipa::path(post, path = "/v1/sessions/{id}/interrupt", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Unterbrochen bzw. nichts zu tun", body = Object)))]
pub async fn interrupt_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    state.sessions().interrupt(session_id(&id)?).await?;
    Ok(axum::Json(serde_json::json!({})))
}

/// Gestoppte Session fortsetzen (SES-003).
#[utoipa::path(post, path = "/v1/sessions/{id}/resume", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Runner läuft", body = SessionSummary)))]
pub async fn resume_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let record = state.sessions().resume(session_id(&id)?).await?;
    Ok(axum::Json(summary(record)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct InputRequest {
    pub text: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct InputAccepted {
    pub input_id: String,
    pub turn_id: String,
}

/// Eingabe senden; startet eine gestoppte Session automatisch (SES-003 AC3).
#[utoipa::path(post, path = "/v1/sessions/{id}/input", tag = "sessions",
    params(("id" = String, Path)),
    request_body = InputRequest,
    responses((status = 202, description = "Turn gestartet", body = InputAccepted)))]
pub async fn submit_input(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<InputRequest>,
) -> ApiResult<Response> {
    let result = state
        .sessions()
        .input(session_id(&id)?, req.text, principal(auth).1)
        .await?;
    Ok((StatusCode::ACCEPTED, axum::Json(result)).into_response())
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventsQuery {
    /// Nur Events mit `seq` größer als dieser Wert.
    pub after_seq: Option<u64>,
    /// 1 bis 200 (Default 50).
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct EventPage {
    /// Events im Envelope-Format (siehe `schemas/v1/events.schema.json`).
    #[schema(value_type = Vec<Object>)]
    pub items: Vec<beton_core::event::Event>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeqCursor {
    after: u64,
}

/// Dauerhafte Events ab `after_seq` (PROTO-010).
#[utoipa::path(get, path = "/v1/sessions/{id}/events", tag = "sessions",
    params(("id" = String, Path), EventsQuery),
    responses((status = 200, description = "Eine Seite Events", body = EventPage)))]
pub async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<EventsQuery>,
) -> ApiResult<axum::Json<EventPage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()?;
    let after = page
        .cursor::<SeqCursor>()?
        .map_or(q.after_seq.unwrap_or(0), |c| c.after);
    let items = state
        .store
        .events(state.local.org, session_id(&id)?, after, limit)
        .await?;
    let next_cursor = (items.len() as u32 == limit)
        .then(|| {
            items
                .last()
                .map(|e| encode_cursor(&SeqCursor { after: e.seq }))
        })
        .flatten();
    Ok(axum::Json(EventPage { items, next_cursor }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ApprovalView {
    pub id: String,
    pub kind: String,
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub subject: Value,
    pub options: Vec<String>,
    pub requested_seq: u64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ApprovalPage {
    pub items: Vec<ApprovalView>,
    pub next_cursor: Option<String>,
}

/// Offene Freigaben einer Session (WEB-018).
#[utoipa::path(get, path = "/v1/sessions/{id}/approvals", tag = "approvals",
    params(("id" = String, Path), PageQuery),
    responses((status = 200, description = "Offene Freigaben", body = ApprovalPage)))]
pub async fn list_approvals(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(page): ApiQuery<PageQuery>,
) -> ApiResult<axum::Json<ApprovalPage>> {
    let limit = page.limit()? as usize;
    let id = session_id(&id)?;
    state.store.session(state.local.org, id).await?;
    let items = state
        .store
        .open_approvals(state.local.org)
        .await?
        .into_iter()
        .filter(|a| a.session_id == id)
        .take(limit)
        .map(|a| ApprovalView {
            id: a.id.to_string(),
            kind: serde_json::to_value(a.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            subject: a.subject,
            options: a.options,
            requested_seq: a.requested_seq,
        })
        .collect();
    Ok(axum::Json(ApprovalPage {
        items,
        next_cursor: None,
    }))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct ResolveApprovalRequest {
    pub decision: Decision,
    /// Geänderte Argumente bei `allow` (HAR-005 AC2).
    #[schema(value_type = Option<Object>)]
    #[ts(optional, type = "unknown")]
    pub updated_args: Option<Value>,
    /// Begründung bei `deny`; das Modell sieht sie.
    #[ts(optional)]
    pub reason: Option<String>,
}

/// Freigabe entscheiden (WEB-018, HAR-005).
#[utoipa::path(post, path = "/v1/sessions/{id}/approvals/{approval_id}/resolve", tag = "approvals",
    params(("id" = String, Path), ("approval_id" = String, Path)),
    request_body = ResolveApprovalRequest,
    responses((status = 200, description = "Entscheidung zugestellt", body = Object)))]
pub async fn resolve_approval(
    State(state): State<AppState>,
    Path((id, approval_id)): Path<(String, String)>,
    ApiJson(req): ApiJson<ResolveApprovalRequest>,
) -> ApiResult<axum::Json<Value>> {
    let args = serde_json::to_value(&req).map_err(|e| Problem::internal(&e))?;
    state
        .sessions()
        .resolve_approval(session_id(&id)?, &approval_id, args)
        .await?;
    Ok(axum::Json(serde_json::json!({})))
}

/// Blob nur über die referenzierende Session (DATA-006 AC1, AC3).
#[utoipa::path(get, path = "/v1/sessions/{id}/blobs/{hash}", tag = "sessions",
    params(("id" = String, Path), ("hash" = String, Path, description = "`sha256:<hex>` oder `<hex>`")),
    responses((status = 200, description = "Inhalt", content_type = "application/octet-stream")))]
pub async fn get_blob(
    State(state): State<AppState>,
    Path((id, hash)): Path<(String, String)>,
) -> ApiResult<Response> {
    let blob: BlobRef = hash
        .parse()
        .or_else(|_| BlobRef::from_hex(&hash))
        .map_err(|_| Problem::new(ProblemCode::NotFound))?;
    let bytes = state
        .store
        .session_blob(state.local.org, session_id(&id)?, &blob)
        .await?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TombstoneQuery {
    /// Nur Tombstones nach diesem Zeitpunkt (RFC 3339).
    pub since: Option<String>,
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TombstoneView {
    pub kind: String,
    pub id: String,
    pub owner: String,
    pub deleted_at: String,
    pub deleted_by: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TombstonePage {
    pub items: Vec<TombstoneView>,
    pub next_cursor: Option<String>,
}

/// Gelöschte Entitäten für Admin und Sync (DATA-008 AC1).
#[utoipa::path(get, path = "/v1/tombstones", tag = "sync",
    params(TombstoneQuery),
    responses((status = 200, description = "Tombstones", body = TombstonePage)))]
pub async fn list_tombstones(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<TombstoneQuery>,
) -> ApiResult<axum::Json<TombstonePage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    let since: Timestamp = match page.cursor::<String>()?.or(q.since) {
        Some(s) => s
            .parse()
            .map_err(|e: String| Problem::new(ProblemCode::BadRequest).detail(e))?,
        None => Timestamp::default(),
    };
    let all = state.store.tombstones_since(state.local.org, since).await?;
    let more = all.len() > limit;
    let items: Vec<TombstoneView> = all
        .into_iter()
        .take(limit)
        .map(|t| TombstoneView {
            kind: t.kind,
            id: t.id,
            owner: t.owner.to_string(),
            deleted_at: t.deleted_at.to_string(),
            deleted_by: t.deleted_by.to_string(),
        })
        .collect();
    let next_cursor = more
        .then(|| items.last().map(|t| encode_cursor(&t.deleted_at)))
        .flatten();
    Ok(axum::Json(TombstonePage { items, next_cursor }))
}

// ---------------------------------------------------------------------------
// WS-Kommandos (PROTO-006) mit REST-Zwilling
// ---------------------------------------------------------------------------

fn need_session(session: Option<SessionId>) -> Result<SessionId, Problem> {
    session.ok_or_else(|| Problem::new(ProblemCode::BadRequest).detail("session_id fehlt"))
}

struct InputSubmit;

#[async_trait]
impl Command for InputSubmit {
    fn name(&self) -> &'static str {
        "input.submit"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("post", "/v1/sessions/{id}/input")
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        args: Value,
    ) -> Result<Value, Problem> {
        let text = args["text"]
            .as_str()
            .ok_or_else(|| Problem::new(ProblemCode::ValidationFailed).detail("text fehlt"))?;
        ctx.state
            .sessions()
            .input(
                need_session(session)?,
                text.to_owned(),
                PrincipalId::User(UserId::LOCAL),
            )
            .await
    }
}

struct TurnInterrupt;

#[async_trait]
impl Command for TurnInterrupt {
    fn name(&self) -> &'static str {
        "turn.interrupt"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("post", "/v1/sessions/{id}/interrupt")
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        _: Value,
    ) -> Result<Value, Problem> {
        ctx.state
            .sessions()
            .interrupt(need_session(session)?)
            .await?;
        Ok(serde_json::json!({}))
    }
}

struct SessionSet;

#[async_trait]
impl Command for SessionSet {
    fn name(&self) -> &'static str {
        "session.set"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("patch", "/v1/sessions/{id}")
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        args: Value,
    ) -> Result<Value, Problem> {
        ctx.state
            .sessions()
            .set(need_session(session)?, args)
            .await?;
        Ok(serde_json::json!({}))
    }
}

struct ApprovalResolve;

#[async_trait]
impl Command for ApprovalResolve {
    fn name(&self) -> &'static str {
        "approval.resolve"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("post", "/v1/sessions/{id}/approvals/{approval_id}/resolve")
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        args: Value,
    ) -> Result<Value, Problem> {
        let approval = args["approval_id"]
            .as_str()
            .ok_or_else(|| Problem::new(ProblemCode::ValidationFailed).detail("approval_id fehlt"))?
            .to_owned();
        ctx.state
            .sessions()
            .resolve_approval(need_session(session)?, &approval, args)
            .await?;
        Ok(serde_json::json!({}))
    }
}

/// Die Kommandos dieses Moduls.
pub fn register_commands(reg: &mut CommandRegistry) {
    reg.register(Arc::new(InputSubmit));
    reg.register(Arc::new(TurnInterrupt));
    reg.register(Arc::new(ApprovalResolve));
    reg.register(Arc::new(SessionSet));
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HarnessQuery {
    /// Host-ID (lokal `hst_local`).
    pub host: Option<String>,
    pub limit: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HarnessPage {
    /// Einträge nach `schemas/v1/harness-catalog.schema.json`.
    #[schema(value_type = Vec<Object>)]
    pub items: Vec<beton_harness::registry::HarnessInfo>,
    pub next_cursor: Option<String>,
}

/// Harness-Katalog eines Hosts mit Capabilities und Probe (HAR-002 AC1).
#[utoipa::path(get, path = "/v1/harnesses", tag = "harnesses",
    params(HarnessQuery),
    responses((status = 200, description = "Katalog", body = HarnessPage)))]
pub async fn list_harnesses(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<HarnessQuery>,
) -> ApiResult<axum::Json<HarnessPage>> {
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    };
    let limit = page.limit()? as usize;
    if let Some(host) = q.host.filter(|h| *h != state.runtime.host.id.to_string()) {
        return Err(Problem::new(ProblemCode::NotFound).detail(format!("Host {host}")));
    }
    // UX-007 AC1: Der Fake-Harness erscheint nur, wenn er freigeschaltet ist.
    let fake = state.sessions().fake_allowed();
    let items = state
        .runtime
        .harnesses
        .catalog(&beton_harness::HostEnv {
            user: state.runtime.sessions.harnesses_user.clone(),
            ..beton_harness::HostEnv::from_process()
        })
        .await
        .into_iter()
        .filter(|h| fake || h.id.as_str() != beton_harness::HarnessId::FAKE)
        .take(limit)
        .collect();
    Ok(axum::Json(HarnessPage {
        items,
        next_cursor: None,
    }))
}
