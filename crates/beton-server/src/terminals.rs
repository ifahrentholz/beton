//! Workspace-Terminals (SES-019, WEB-010): REST-Endpunkte und WS-Kommandos. Die Terminals
//! laufen im Runner der Session (PTY in Stufe 0, Prozess in Stufe 2, SBX-017); der Server
//! prüft die Berechtigung und stellt das Kommando zu. Die Bytes laufen über Binärkanäle
//! (PROTO-007, [`crate::channels`]), nie durch das Session-Log (DATA-011).

use async_trait::async_trait;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::event::{Actor, TerminalKind};
use beton_core::id::{PrincipalId, SessionId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::commands::{Command, CommandCtx, CommandRegistry};
use crate::extract::{ApiJson, ApiQuery, PageQuery};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Neues Terminal (`POST /v1/sessions/{id}/terminals`, WS `terminal.open`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema, TS)]
pub struct TerminalOpenRequest {
    /// Anzeigename des Tabs, z. B. `Dev-Server`; Default `Shell <n>`.
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    /// Programm und Argumente; Default ist die Login-Shell des Benutzers (`SHELL`).
    #[serde(default)]
    #[ts(optional)]
    pub command: Option<Vec<String>>,
    pub cols: u16,
    pub rows: u16,
}

/// Ein laufendes Terminal der Session.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, TS)]
pub struct TerminalView {
    pub terminal_id: String,
    /// Kanal des Runners; Clients öffnen das Terminal per `open_channel {kind: terminal, ref:
    /// terminal_id}` und bekommen eine eigene Kanal-ID.
    pub channel_id: u32,
    pub name: String,
    #[schema(value_type = String)]
    pub kind: TerminalKind,
    pub cmd: String,
    pub cols: u16,
    pub rows: u16,
}

/// Laufende Terminals (höchstens [`MAX_TERMINALS`] je Session, daher immer vollständig;
/// `next_cursor` bleibt leer).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, TS)]
pub struct TerminalList {
    pub items: Vec<TerminalView>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Höchstzahl gleichzeitiger Terminals einer Session (Schutz des Runners).
pub const MAX_TERMINALS: usize = 16;

/// Umbenennen (`PATCH …/terminals/{terminal_id}`, WS `terminal.rename`).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, TS)]
pub struct TerminalRename {
    pub name: String,
}

/// Neue Fenstergröße (`POST …/terminals/{terminal_id}/resize`, WS `terminal.resize`).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, TS)]
pub struct TerminalResize {
    pub cols: u16,
    pub rows: u16,
}

/// Lokal handelt immer `usr_local` (Bearer und Cookie gehören demselben Menschen).
fn actor(_auth: Authenticated) -> Actor {
    Actor::User {
        id: PrincipalId::User(UserId::LOCAL),
        device_id: None,
    }
}

fn session_id(raw: &str) -> Result<SessionId, Problem> {
    raw.parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {raw}")))
}

fn valid_id(raw: &str) -> Result<(), Problem> {
    let ok = raw.starts_with("term_")
        && raw.len() <= 64
        && raw[5..].chars().all(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(Problem::new(ProblemCode::NotFound).detail(format!("Terminal {raw}")))
    }
}

/// Steuern verlangt Lese- und Steuerrecht auf die Session (SES-019 AC2; Rollen ab M4).
async fn authorize(
    state: &AppState,
    auth: Authenticated,
    session: SessionId,
) -> Result<(), Problem> {
    let record = state.store.session(state.local.org, session).await?;
    let authorizer = &state.runtime.authorizer;
    if !authorizer.can_read(auth, &record) || !authorizer.can_drive(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden)
            .detail("Terminals steuern braucht das Recht drive"));
    }
    if record.archived {
        return Err(Problem::new(ProblemCode::Conflict)
            .detail("Archivierte Session; erst wiederherstellen"));
    }
    Ok(())
}

/// Kommando an den Runner; `terminal.open` startet einen gestoppten Runner (wie eine Eingabe).
async fn deliver(
    state: &AppState,
    auth: Authenticated,
    session: SessionId,
    name: &str,
    args: Value,
) -> Result<Value, Problem> {
    authorize(state, auth, session).await?;
    let runners = &state.runtime.runners;
    if name == "terminal.open" {
        crate::exports::ensure_runnable(&state.store, state.local.org, session).await?;
        state.sessions().ensure_runner(session).await?;
    } else if !runners.connected(session) {
        if name == "terminal.list" {
            return Ok(json!({"items": []}));
        }
        return Err(Problem::new(ProblemCode::NotFound)
            .detail("Für diese Session läuft kein Runner, also auch kein Terminal"));
    }
    runners
        .deliver_as(
            session,
            name,
            args,
            Some(actor(auth)),
            state.runtime.tunnel.cmd_timeout,
        )
        .await
}

fn view(v: Value) -> Result<TerminalView, Problem> {
    serde_json::from_value(v)
        .map_err(|e| Problem::internal(&format!("Antwort des Runners auf terminal.*: {e}")))
}

/// Terminal öffnen (SES-019): PTY im Runner, Prozess in der Tool-Sandbox der Session.
#[utoipa::path(post, path = "/v1/sessions/{id}/terminals", tag = "terminals",
    params(("id" = String, Path)),
    request_body = TerminalOpenRequest,
    responses((status = 201, description = "Terminal läuft", body = TerminalView),
              (status = 403, description = "Kein Recht drive", body = Problem, content_type = "application/problem+json"),
              (status = 503, description = "Sandbox nicht verfügbar (fail closed)", body = Problem, content_type = "application/problem+json")))]
pub async fn open_terminal(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<TerminalOpenRequest>,
) -> ApiResult<Response> {
    let session = session_id(&id)?;
    let args = serde_json::to_value(&req).unwrap_or(Value::Null);
    let t = view(deliver(&state, auth, session, "terminal.open", args).await?)?;
    let location = format!("/v1/sessions/{session}/terminals/{}", t.terminal_id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        axum::Json(t),
    )
        .into_response())
}

/// Laufende Terminals der Session (leer, wenn kein Runner läuft).
#[utoipa::path(get, path = "/v1/sessions/{id}/terminals", tag = "terminals",
    params(("id" = String, Path), PageQuery),
    responses((status = 200, description = "Terminals", body = TerminalList)))]
pub async fn list_terminals(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    ApiQuery(_page): ApiQuery<PageQuery>,
) -> ApiResult<axum::Json<TerminalList>> {
    let session = session_id(&id)?;
    let record = state.store.session(state.local.org, session).await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    if !state.runtime.runners.connected(session) {
        return Ok(axum::Json(TerminalList {
            items: Vec::new(),
            next_cursor: None,
        }));
    }
    let v = state
        .runtime
        .runners
        .deliver_as(
            session,
            "terminal.list",
            Value::Null,
            Some(actor(auth)),
            state.runtime.tunnel.cmd_timeout,
        )
        .await?;
    serde_json::from_value(v)
        .map(axum::Json)
        .map_err(|e| Problem::internal(&format!("Antwort des Runners auf terminal.list: {e}")))
}

/// Terminal umbenennen (`terminal.renamed`).
#[utoipa::path(patch, path = "/v1/sessions/{id}/terminals/{terminal_id}", tag = "terminals",
    params(("id" = String, Path), ("terminal_id" = String, Path)),
    request_body = TerminalRename,
    responses((status = 200, description = "Umbenannt", body = TerminalView)))]
pub async fn rename_terminal(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, terminal)): Path<(String, String)>,
    ApiJson(req): ApiJson<TerminalRename>,
) -> ApiResult<axum::Json<TerminalView>> {
    valid_id(&terminal)?;
    let args = json!({"terminal_id": terminal, "name": req.name});
    let v = deliver(&state, auth, session_id(&id)?, "terminal.rename", args).await?;
    Ok(axum::Json(view(v)?))
}

/// Fenstergröße setzen; das PTY schickt `SIGWINCH` an den Prozess (WEB-010 AC2).
#[utoipa::path(post, path = "/v1/sessions/{id}/terminals/{terminal_id}/resize", tag = "terminals",
    params(("id" = String, Path), ("terminal_id" = String, Path)),
    request_body = TerminalResize,
    responses((status = 200, description = "Neue Größe gilt", body = TerminalView)))]
pub async fn resize_terminal(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, terminal)): Path<(String, String)>,
    ApiJson(req): ApiJson<TerminalResize>,
) -> ApiResult<axum::Json<TerminalView>> {
    valid_id(&terminal)?;
    let args = json!({"terminal_id": terminal, "cols": req.cols, "rows": req.rows});
    let v = deliver(&state, auth, session_id(&id)?, "terminal.resize", args).await?;
    Ok(axum::Json(view(v)?))
}

/// Terminal schließen: Prozessbaum beenden, letzter Snapshot, `terminal.closed`.
#[utoipa::path(delete, path = "/v1/sessions/{id}/terminals/{terminal_id}", tag = "terminals",
    params(("id" = String, Path), ("terminal_id" = String, Path)),
    responses((status = 204, description = "Geschlossen")))]
pub async fn close_terminal(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path((id, terminal)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    valid_id(&terminal)?;
    let args = json!({"terminal_id": terminal});
    deliver(&state, auth, session_id(&id)?, "terminal.close", args).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// WS-Kommando `terminal.*` mit REST-Zwilling.
struct TerminalCommand {
    name: &'static str,
    twin: (&'static str, &'static str),
}

#[async_trait]
impl Command for TerminalCommand {
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
        let session = session.ok_or_else(|| {
            Problem::new(ProblemCode::ValidationFailed).detail("session_id fehlt")
        })?;
        // Gleiche Argumente wie die REST-Zwillinge (PROTO-006 AC3).
        let args = match self.name {
            "terminal.open" => serde_json::from_value::<TerminalOpenRequest>(args)
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null)),
            "terminal.rename" => {
                serde_json::from_value::<TerminalRename>(args.clone()).map(|_| args)
            }
            "terminal.resize" => {
                serde_json::from_value::<TerminalResize>(args.clone()).map(|_| args)
            }
            _ => Ok(args),
        }
        .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(e.to_string()))?;
        if self.name != "terminal.open" {
            let id = args["terminal_id"].as_str().unwrap_or_default();
            // Steuerrecht vor allem anderen (SES-019 AC2): auch ein unbekanntes Terminal
            // verrät einem Leser nichts.
            authorize(&ctx.state, ctx.auth, session).await?;
            valid_id(id)?;
        }
        deliver(&ctx.state, ctx.auth, session, self.name, args).await
    }
}

/// Registriert `terminal.open|rename|resize|close` (PROTO-006).
pub fn register_commands(reg: &mut CommandRegistry) {
    for (name, twin) in [
        ("terminal.open", ("post", "/v1/sessions/{id}/terminals")),
        (
            "terminal.rename",
            ("patch", "/v1/sessions/{id}/terminals/{terminal_id}"),
        ),
        (
            "terminal.resize",
            ("post", "/v1/sessions/{id}/terminals/{terminal_id}/resize"),
        ),
        (
            "terminal.close",
            ("delete", "/v1/sessions/{id}/terminals/{terminal_id}"),
        ),
    ] {
        reg.register(std::sync::Arc::new(TerminalCommand { name, twin }));
    }
}

/// Größter Snapshot eines Terminals (Scrollback samt Formatierung).
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

/// System-Call `terminal.snapshot {data_b64}` des Runners: legt den Bildschirmzustand als Blob
/// der Session ab und liefert `{blob_ref}` (DATA-011). Das Event `terminal.snapshot` schreibt
/// danach der Runner.
pub async fn store_snapshot(
    state: &AppState,
    session: SessionId,
    args: &Value,
) -> Result<Value, Problem> {
    use base64::Engine as _;
    let data = args["data_b64"]
        .as_str()
        .ok_or_else(|| Problem::new(ProblemCode::ValidationFailed).detail("data_b64 fehlt"))?;
    if data.len() > MAX_SNAPSHOT_BYTES / 3 * 4 + 4 {
        return Err(Problem::new(ProblemCode::PayloadTooLarge)
            .detail(format!("Snapshot größer als {MAX_SNAPSHOT_BYTES} Bytes")));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(e.to_string()))?;
    let blob = state
        .store
        .put_session_blob(state.local.org, session, &bytes)
        .await?;
    Ok(json!({ "blob_ref": blob }))
}
