//! REST-Endpunkte und WS-Kommandos des Session-Lebenszyklus (SES-001, SES-002, SES-003,
//! SES-004, SES-005, SES-012, DATA-006, DATA-008). Jedes WS-Kommando hat hier seinen REST-Zwilling
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
use crate::sessions::{CreateSession, InputMode};

/// M0: Lokal handelt immer `usr_local` (Bearer und Cookie gehören demselben Menschen).
fn principal(_auth: Authenticated) -> (UserId, PrincipalId) {
    (UserId::LOCAL, PrincipalId::User(UserId::LOCAL))
}

fn session_id(raw: &str) -> Result<SessionId, Problem> {
    raw.parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {raw}")))
}

async fn summary_of(state: &AppState, id: SessionId) -> Result<SessionSummary, Problem> {
    crate::api::session_summary(state, id).await
}

#[derive(Debug, Deserialize, ToSchema, TS)]
pub struct CreateSessionRequest {
    /// Harness-ID, z. B. `claude` oder (nur mit `--dev`) `fake`. Mit `agent` optional: dann
    /// ein Override von `executor.harness`, vermerkt in `agent.resolved.overrides` (AGT-004).
    #[serde(default)]
    #[ts(optional)]
    pub target: Option<String>,
    /// Agent-Ref (AGT-003): Name (`pr-fixer`), Pfad (`./agents/x`) oder `builtin:<name>`. Der
    /// Agent wird beim Start aufgelöst und als Snapshot festgehalten (`agent.resolved`).
    #[serde(default)]
    #[ts(optional)]
    pub agent: Option<String>,
    /// Parameterwerte des Agents (AGT-010); Texte werden in den deklarierten Typ umgewandelt.
    /// Ungültige Werte: 422 `invalid_param`; fehlende Pflichtwerte: 422 `params_required` mit
    /// `errors[].pointer = /params/<name>`.
    #[serde(default)]
    #[schema(value_type = Object)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub params: Option<serde_json::Map<String, Value>>,
    /// Arbeitsverzeichnis (Projekt oder Worktree).
    pub cwd: String,
    #[ts(optional)]
    pub title: Option<String>,
    #[ts(optional)]
    pub model: Option<String>,
    /// Reasoning-Effort (`low`, `medium`, `high`, `xhigh`); eine Stufe, die der Harness nicht
    /// kennt, wird auf die nächstniedrigere gemappt (HAR-017).
    #[serde(default)]
    #[ts(optional)]
    pub effort: Option<String>,
    /// Permission-Mode (`plan`, `default`, `accept_edits`, `yolo`; HAR-027). `yolo` braucht
    /// Tool-Sandbox und Egress-Proxy, sonst `409 sandbox_required`.
    #[serde(default)]
    #[ts(optional)]
    pub permission_mode: Option<String>,
    /// Harness-spezifische Optionen, z. B. `{"scenario": "…"}` beim Fake-Harness.
    #[serde(default)]
    #[schema(value_type = Object)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub harness_opts: Value,
    /// Eigener `git worktree` mit eigenem Branch für die Session (SES-015). `cwd` muss dann in
    /// einem Git-Repository liegen; die Session arbeitet im Worktree.
    #[serde(default)]
    #[ts(optional)]
    pub worktree: Option<WorktreeRequest>,
}

/// Worktree-Wunsch beim Anlegen einer Session (SES-015).
#[derive(Debug, Default, Deserialize, Serialize, ToSchema, TS)]
pub struct WorktreeRequest {
    /// Branch-Name; Default `beton/<titel-slug>-<id4>`. Ein vorhandener Branch wird ausgecheckt.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    /// Base; Default `origin/HEAD`, sonst der aktuelle Branch.
    #[serde(default)]
    #[ts(optional)]
    pub base: Option<String>,
    /// `git fetch <remote> <branch>` vor dem Anlegen bei Remote-Tracking-Bases (Default `true`,
    /// Timeout 15 s). Ist das Remote nicht erreichbar, entsteht der Worktree aus dem lokalen
    /// Stand und die Session erhält einen Hinweis.
    #[serde(default)]
    #[ts(optional)]
    pub fetch: Option<bool>,
}

/// Session anlegen und Runner starten (SES-001); optional mit eigenem Worktree (SES-015).
#[utoipa::path(post, path = "/v1/sessions", tag = "sessions",
    request_body = CreateSessionRequest,
    responses((status = 201, description = "Session angelegt", body = SessionSummary),
              (status = 404, description = "Agent nicht gefunden (`agent_not_found`)", body = Problem, content_type = "application/problem+json"),
              (status = 409, description = "Worktree gewünscht, aber kein Git-Repository; `sandbox_required` bei `yolo` ohne Sandbox; `capability_unsupported`, wenn der Harness Effort bzw. Permission-Mode nicht kennt", body = Problem, content_type = "application/problem+json"),
              (status = 422, description = "Base nicht auflösbar (`base_not_found`), Agent ungültig (`agent_invalid`), Parameter ungültig (`invalid_param`) oder Pflichtparameter fehlen (`params_required`), Harness passt nicht zum Agent (`harness_incompatible`)", body = Problem, content_type = "application/problem+json")))]
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
                target: match (req.target, &req.agent) {
                    (Some(t), _) => t,
                    (None, Some(_)) => String::new(),
                    (None, None) => {
                        return Err(Problem::new(ProblemCode::ValidationFailed)
                            .detail("`target` (Harness) oder `agent` fehlt"));
                    }
                },
                agent: req.agent,
                params: req.params.unwrap_or_default().into_iter().collect(),
                cwd: req.cwd,
                title: req.title,
                model: req.model,
                effort: req.effort,
                permission_mode: req.permission_mode,
                harness_opts: req.harness_opts,
                worktree: req.worktree.map(|w| crate::sessions::WorktreeSpec {
                    branch: w.branch,
                    base: w.base,
                    fetch: w.fetch.unwrap_or(true),
                }),
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(summary_of(&state, record.id).await?),
    )
        .into_response())
}

/// Fork-Anfrage (SES-006, SES-007).
#[derive(Debug, Default, Deserialize, Serialize, ToSchema, TS)]
pub struct ForkRequest {
    /// Fork-Punkt; ohne Angabe das Ende der Session. Liegt er mitten in einem Turn, beginnt der
    /// Fork am letzten vollständigen Turn-Ende davor (`effective_seq`).
    #[serde(default)]
    #[ts(optional)]
    pub at_seq: Option<u64>,
    /// Ziel-Harness, z. B. `codex`; ohne Angabe der Harness der Quelle. Ein anderer Harness
    /// bekommt den Verlauf als Übergabe-Präambel (HAR-018).
    #[serde(default)]
    #[ts(optional)]
    pub harness: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    /// Reasoning-Effort des Forks (HAR-017), gegen den Ziel-Harness geprüft.
    #[serde(default)]
    #[ts(optional)]
    pub effort: Option<String>,
    /// Permission-Mode des Forks (HAR-027); `yolo` ohne Sandbox: `409 sandbox_required`.
    #[serde(default)]
    #[ts(optional)]
    pub permission_mode: Option<String>,
    /// `new_worktree` (Default, falls die Quelle in einem Git-Repository arbeitet), `shared`
    /// oder `fresh`. Dateien werden nie auf den Stand von `at_seq` zurückgesetzt.
    #[serde(default)]
    #[ts(optional)]
    pub workspace: Option<crate::fork::ForkWorkspace>,
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    /// Harness-spezifische Startoptionen (wie bei `POST /v1/sessions`); ohne Angabe die der
    /// Quelle, falls der Harness gleich bleibt.
    #[serde(default)]
    #[schema(value_type = Object)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub harness_opts: Value,
}

/// Ergebnis eines Forks.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct ForkResponse {
    /// Die neue Session.
    pub session: SessionSummary,
    /// Tatsächlicher Fork-Punkt in der Quelle (letztes vollständiges Turn-Ende).
    pub effective_seq: u64,
    /// Gewählter Workspace-Modus.
    pub workspace: crate::fork::ForkWorkspace,
}

/// Neue Session ab einem Event abzweigen (SES-006), optional auf einem anderen Harness
/// (SES-007). Die Quelle bleibt unverändert und erhält nur `session.fork_created`.
#[utoipa::path(post, path = "/v1/sessions/{id}/fork", tag = "sessions",
    params(("id" = String, Path)),
    request_body = ForkRequest,
    responses((status = 201, description = "Fork angelegt; Runner startet", body = ForkResponse),
              (status = 409, description = "`new_worktree` ohne Git-Repository; `sandbox_required` bei `yolo` ohne Sandbox; `capability_unsupported` bei Effort bzw. Permission-Mode, die der Ziel-Harness nicht kennt", body = Problem, content_type = "application/problem+json"),
              (status = 422, description = "`harness_incompatible`: Ziel-Harness passt nicht", body = Problem, content_type = "application/problem+json")))]
pub async fn fork_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<ForkRequest>,
) -> ApiResult<Response> {
    let (user, _) = principal(auth);
    let forked = state
        .sessions()
        .fork(
            session_id(&id)?,
            user,
            crate::fork::ForkSession {
                at_seq: req.at_seq,
                harness: req.harness,
                model: req.model,
                effort: req.effort,
                permission_mode: req.permission_mode,
                workspace: req.workspace,
                title: req.title,
                harness_opts: req.harness_opts,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(ForkResponse {
            session: summary_of(&state, forked.session.id).await?,
            effective_seq: forked.effective_seq,
            workspace: forked.workspace,
        }),
    )
        .into_response())
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
    Ok(axum::Json(summary_of(&state, session_id(&id)?).await?))
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
    /// `plan`, `default`, `accept_edits` oder `yolo` (HAR-027).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub permission_mode: Option<String>,
}

/// Einstellungen ändern: Titel, Modell, Effort, Permission-Mode (HAR-017, HAR-027). Ohne
/// Capability `capability_unsupported` (HAR-002 AC3), `yolo` ohne Sandbox
/// `sandbox_required`. Während eines Turns gilt ein Wechsel ab dem nächsten Turn; er erscheint
/// als `session.settings_changed` mit `effective_from_turn`.
#[utoipa::path(patch, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = String, Path)),
    request_body = SessionSettings,
    responses((status = 200, description = "Geändert bzw. für das Turn-Ende vorgemerkt", body = SessionSummary),
              (status = 409, description = "`capability_unsupported`, `sandbox_required` oder Session läuft nicht", body = Problem, content_type = "application/problem+json")))]
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
    if req.model.is_some() || req.effort.is_some() || req.permission_mode.is_some() {
        let args = serde_json::to_value(&req).map_err(|e| Problem::internal(&e))?;
        state.sessions().set(id, args).await?;
    }
    Ok(axum::Json(summary_of(&state, id).await?))
}

/// Was mit uncommitteten Änderungen im Worktree geschehen soll (SES-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UncommittedAction {
    /// WIP-Commit auf dem Branch.
    Commit,
    /// Verwerfen.
    Discard,
}

/// Was mit einem Branch mit ungepushten, nicht gemergten Commits geschehen soll (SES-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BranchAction {
    Keep,
    Delete,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteSessionQuery {
    /// Antwort auf `409 worktree_dirty`: `commit` (WIP-Commit) oder `discard`.
    #[param(inline)]
    pub uncommitted: Option<UncommittedAction>,
    /// Antwort auf `409 worktree_unpushed`: `keep` oder `delete`.
    #[param(inline)]
    pub branch: Option<BranchAction>,
}

/// Session löschen (nur Owner, SES-001 AC3). Ein Worktree wird kontrolliert entfernt
/// (SES-016): uncommittete Änderungen bzw. ungepushte Commits ohne Entscheidung liefern `409`
/// und verändern nichts; ein gemergter Branch wird mitgelöscht.
#[utoipa::path(delete, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = String, Path), DeleteSessionQuery),
    responses((status = 204, description = "Gelöscht; Tombstone angelegt"),
              (status = 409, description = "`worktree_dirty` oder `worktree_unpushed`: Rückfrage nötig, nichts verändert", body = Problem, content_type = "application/problem+json")))]
pub async fn delete_session(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<DeleteSessionQuery>,
) -> ApiResult<StatusCode> {
    use beton_git::worktree as wt;
    state
        .sessions()
        .delete(
            session_id(&id)?,
            principal(auth).1,
            crate::sessions::DeleteOptions {
                worktree: wt::RemoveOptions {
                    uncommitted: q.uncommitted.map(|u| match u {
                        UncommittedAction::Commit => wt::Uncommitted::Commit,
                        UncommittedAction::Discard => wt::Uncommitted::Discard,
                    }),
                    branch: q.branch.map(|b| match b {
                        BranchAction::Keep => wt::BranchAction::Keep,
                        BranchAction::Delete => wt::BranchAction::Delete,
                    }),
                },
            },
        )
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
    Ok(axum::Json(summary_of(&state, id).await?))
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
    Ok(axum::Json(summary_of(&state, id).await?))
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

/// Kontext kompaktieren (SES-011, HAR-022): Der Harness kompaktiert (Claude: `/compact`,
/// Direkt-API: eigene Compaction); das Ergebnis folgt als `compaction.started`,
/// `compaction.completed` und `context.usage`.
#[utoipa::path(post, path = "/v1/sessions/{id}/compact", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 202, description = "Compaction angestoßen", body = Object),
              (status = 409, description = "Harness ohne Capability `compaction` (`capability_unsupported`), Session gestoppt oder Turn läuft (`conflict`)", body = Problem, content_type = "application/problem+json")))]
pub async fn compact_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    state.sessions().compact(session_id(&id)?).await?;
    Ok((StatusCode::ACCEPTED, axum::Json(serde_json::json!({}))).into_response())
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
    Ok(axum::Json(summary_of(&state, record.id).await?))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct InputRequest {
    pub text: String,
    /// `queue` (Default): sofort bzw. nach dem laufenden Turn; `steer`: in den laufenden
    /// Turn, falls der Harness `steering` kann (sonst `409 capability_unsupported`).
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<InputMode>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct InputAccepted {
    pub input_id: String,
    /// Nur bei `status: started`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<String>,
    /// `started`, `queued` oder `steered`.
    pub status: String,
}

/// Eingabe senden (SES-004): ohne laufenden Turn sofort, sonst in die Queue bzw. als Steer;
/// startet eine gestoppte Session automatisch (SES-003 AC3).
#[utoipa::path(post, path = "/v1/sessions/{id}/input", tag = "sessions",
    params(("id" = String, Path)),
    request_body = InputRequest,
    responses((status = 202, description = "Gestartet, eingereiht oder eingespeist", body = InputAccepted),
              (status = 409, description = "Steer ohne Capability `steering`", body = Problem, content_type = "application/problem+json")))]
pub async fn submit_input(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<InputRequest>,
) -> ApiResult<Response> {
    let result = state
        .sessions()
        .input(
            session_id(&id)?,
            req.text,
            principal(auth).1,
            req.mode.unwrap_or_default(),
        )
        .await?;
    Ok((StatusCode::ACCEPTED, axum::Json(result)).into_response())
}

/// Stand der Queue einer Session (SES-004); immer vollständig, `next_cursor` bleibt leer.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct QueueView {
    /// Einträge in Ausführungsreihenfolge (`{id, author, text, attachments, created_at}`).
    #[schema(value_type = Vec<Object>)]
    pub items: Vec<beton_core::event::QueueItem>,
    /// Nach einem Interrupt pausiert, bis jemand fortsetzt oder neuen Input sendet.
    pub paused: bool,
    pub next_cursor: Option<String>,
}

async fn queue_view(state: &AppState, id: SessionId) -> Result<QueueView, Problem> {
    state.store.session(state.local.org, id).await?;
    let q = state.queue().lock(id).await?.snapshot();
    Ok(QueueView {
        items: q.items,
        paused: q.paused,
        next_cursor: None,
    })
}

/// Die Queue einer Session (SES-004).
#[utoipa::path(get, path = "/v1/sessions/{id}/queue", tag = "queue",
    params(("id" = String, Path), PageQuery),
    responses((status = 200, description = "Queue", body = QueueView)))]
pub async fn get_queue(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(_page): ApiQuery<PageQuery>,
) -> ApiResult<axum::Json<QueueView>> {
    Ok(axum::Json(queue_view(&state, session_id(&id)?).await?))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct QueueEditRequest {
    pub text: String,
}

/// Queue-Eintrag bearbeiten (`queue.edit`).
#[utoipa::path(patch, path = "/v1/sessions/{id}/queue/{item_id}", tag = "queue",
    params(("id" = String, Path), ("item_id" = String, Path)),
    request_body = QueueEditRequest,
    responses((status = 204, description = "Geändert; neuer Stand als `queue.updated`")))]
pub async fn edit_queue_item(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, item)): Path<(String, String)>,
    ApiJson(req): ApiJson<QueueEditRequest>,
) -> ApiResult<StatusCode> {
    let id = session_id(&id)?;
    queue_op(&state, id, QueueOp::Edit(item, req.text), principal(auth).1).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Queue-Eintrag löschen (`queue.delete`).
#[utoipa::path(delete, path = "/v1/sessions/{id}/queue/{item_id}", tag = "queue",
    params(("id" = String, Path), ("item_id" = String, Path)),
    responses((status = 204, description = "Geändert; neuer Stand als `queue.updated`")))]
pub async fn delete_queue_item(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, item)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let id = session_id(&id)?;
    queue_op(&state, id, QueueOp::Delete(item), principal(auth).1).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct QueueMoveRequest {
    /// Neue Position, 0 = als Nächstes; größere Werte setzen ans Ende.
    pub position: u32,
}

/// Queue-Eintrag verschieben (`queue.reorder`, Drag & Drop in WEB-005).
#[utoipa::path(post, path = "/v1/sessions/{id}/queue/{item_id}/move", tag = "queue",
    params(("id" = String, Path), ("item_id" = String, Path)),
    request_body = QueueMoveRequest,
    responses((status = 204, description = "Geändert; neuer Stand als `queue.updated`")))]
pub async fn move_queue_item(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, item)): Path<(String, String)>,
    ApiJson(req): ApiJson<QueueMoveRequest>,
) -> ApiResult<StatusCode> {
    let id = session_id(&id)?;
    queue_op(
        &state,
        id,
        QueueOp::Move(item, req.position as usize),
        principal(auth).1,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Queue-Eintrag in den laufenden Turn einspeisen (`queue.steer`, „Als Steer senden“).
#[utoipa::path(post, path = "/v1/sessions/{id}/queue/{item_id}/steer", tag = "queue",
    params(("id" = String, Path), ("item_id" = String, Path)),
    responses((status = 202, description = "Eingespeist (oder als Nächstes gestartet)", body = InputAccepted),
              (status = 409, description = "Harness ohne `steering`", body = Problem, content_type = "application/problem+json")))]
pub async fn steer_queue_item(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, item)): Path<(String, String)>,
) -> ApiResult<Response> {
    let result = state
        .sessions()
        .steer_item(session_id(&id)?, &item, principal(auth).1)
        .await?;
    Ok((StatusCode::ACCEPTED, axum::Json(result)).into_response())
}

/// Pausierte Queue fortsetzen (`queue.resume`, SES-005 AC2).
#[utoipa::path(post, path = "/v1/sessions/{id}/queue/resume", tag = "queue",
    params(("id" = String, Path)),
    responses((status = 204, description = "Geändert; neuer Stand als `queue.updated`")))]
pub async fn resume_queue(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let id = session_id(&id)?;
    state.sessions().resume_queue(id, principal(auth).1).await?;
    Ok(StatusCode::NO_CONTENT)
}

enum QueueOp {
    Edit(String, String),
    Delete(String),
    Move(String, usize),
}

async fn queue_op(
    state: &AppState,
    id: SessionId,
    op: QueueOp,
    by: PrincipalId,
) -> Result<(), Problem> {
    state.store.session(state.local.org, id).await?;
    let mut q = state.queue().lock(id).await?;
    match op {
        QueueOp::Edit(item, text) => q.edit(&item, text, by).await,
        QueueOp::Delete(item) => q.delete(&item, by).await,
        QueueOp::Move(item, to) => q.reorder(&item, to, by).await,
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct ReadStateRequest {
    /// Bis hierhin hat der Client die Session angezeigt.
    pub seq: u64,
}

/// Gelesen-Stand setzen; gilt auf allen Geräten des Users und sinkt nie (SES-012).
#[utoipa::path(put, path = "/v1/sessions/{id}/read-state", tag = "sessions",
    params(("id" = String, Path)),
    request_body = ReadStateRequest,
    responses((status = 200, description = "Session mit neuem Gelesen-Stand", body = SessionSummary)))]
pub async fn put_read_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<ReadStateRequest>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let view = state
        .store
        .mark_read(state.local.org, state.local.user, session_id(&id)?, req.seq)
        .await?;
    Ok(axum::Json(crate::api::summary(view)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema, TS)]
pub struct PinRequest {
    pub pinned: bool,
}

/// Anpinnen bzw. lösen (je User); angepinnte Sessions stehen in der Liste oben (SES-012).
#[utoipa::path(put, path = "/v1/sessions/{id}/pin", tag = "sessions",
    params(("id" = String, Path)),
    request_body = PinRequest,
    responses((status = 200, description = "Session mit neuem Pin-Stand", body = SessionSummary)))]
pub async fn put_pin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<PinRequest>,
) -> ApiResult<axum::Json<SessionSummary>> {
    let view = state
        .store
        .set_pinned(
            state.local.org,
            state.local.user,
            session_id(&id)?,
            req.pinned,
        )
        .await?;
    Ok(axum::Json(crate::api::summary(view)))
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
        let mode = match args.get("mode") {
            None | Some(Value::Null) => InputMode::Queue,
            Some(m) => serde_json::from_value(m.clone()).map_err(|_| {
                Problem::new(ProblemCode::ValidationFailed).detail("mode: queue oder steer")
            })?,
        };
        ctx.state
            .sessions()
            .input(
                need_session(session)?,
                text.to_owned(),
                PrincipalId::User(UserId::LOCAL),
                mode,
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

struct SessionCompact;

#[async_trait]
impl Command for SessionCompact {
    fn name(&self) -> &'static str {
        "session.compact"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("post", "/v1/sessions/{id}/compact")
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        _: Value,
    ) -> Result<Value, Problem> {
        ctx.state.sessions().compact(need_session(session)?).await?;
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
        // Antwort des Runners: `{mechanism, effective_from_turn}` bzw. `{deferred: true}`.
        let result = ctx
            .state
            .sessions()
            .set(need_session(session)?, args)
            .await?;
        Ok(if result.is_object() {
            result
        } else {
            serde_json::json!({})
        })
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

fn item_arg(args: &Value) -> Result<String, Problem> {
    args["item_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Problem::new(ProblemCode::ValidationFailed).detail("item_id fehlt"))
}

/// Queue-Kommandos (SES-004); REST-Zwillinge unter `/v1/sessions/{id}/queue`.
struct QueueCommand {
    name: &'static str,
    twin: (&'static str, &'static str),
}

#[async_trait]
impl Command for QueueCommand {
    fn name(&self) -> &'static str {
        self.name
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        self.twin
    }
    async fn run(
        &self,
        ctx: &CommandCtx,
        session: Option<SessionId>,
        args: Value,
    ) -> Result<Value, Problem> {
        let id = need_session(session)?;
        let by = PrincipalId::User(UserId::LOCAL);
        let state = &ctx.state;
        match self.name {
            "queue.edit" => {
                let text = args["text"].as_str().unwrap_or_default().to_owned();
                queue_op(state, id, QueueOp::Edit(item_arg(&args)?, text), by).await?;
            }
            "queue.delete" => queue_op(state, id, QueueOp::Delete(item_arg(&args)?), by).await?,
            "queue.reorder" => {
                let to = args["position"].as_u64().ok_or_else(|| {
                    Problem::new(ProblemCode::ValidationFailed).detail("position fehlt")
                })?;
                queue_op(
                    state,
                    id,
                    QueueOp::Move(item_arg(&args)?, usize::try_from(to).unwrap_or(usize::MAX)),
                    by,
                )
                .await?;
            }
            "queue.steer" => return state.sessions().steer_item(id, &item_arg(&args)?, by).await,
            "queue.resume" => state.sessions().resume_queue(id, by).await?,
            other => {
                return Err(Problem::new(ProblemCode::UnknownCommand).detail(other.to_owned()));
            }
        }
        serde_json::to_value(queue_view(state, id).await?).map_err(|e| Problem::internal(&e))
    }
}

/// Die Kommandos dieses Moduls.
pub fn register_commands(reg: &mut CommandRegistry) {
    for (name, twin) in [
        ("queue.edit", ("patch", "/v1/sessions/{id}/queue/{item_id}")),
        (
            "queue.delete",
            ("delete", "/v1/sessions/{id}/queue/{item_id}"),
        ),
        (
            "queue.reorder",
            ("post", "/v1/sessions/{id}/queue/{item_id}/move"),
        ),
        (
            "queue.steer",
            ("post", "/v1/sessions/{id}/queue/{item_id}/steer"),
        ),
        ("queue.resume", ("post", "/v1/sessions/{id}/queue/resume")),
    ] {
        reg.register(Arc::new(QueueCommand { name, twin }));
    }
    reg.register(Arc::new(InputSubmit));
    reg.register(Arc::new(TurnInterrupt));
    reg.register(Arc::new(ApprovalResolve));
    reg.register(Arc::new(SessionSet));
    reg.register(Arc::new(SessionCompact));
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
