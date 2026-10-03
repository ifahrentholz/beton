//! REST-Endpunkte von WP-04 mit OpenAPI-Annotation (API-001, API-002, AUTH-004, PROTO-010).
//!
//! Alle Routen entstehen über `utoipa_axum::routes!`, damit Router und OpenAPI-Dokument aus
//! derselben Deklaration stammen (API-001 AC2).

use std::net::SocketAddr;
use std::time::Instant;

use axum::Extension;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::id::{HostId, OrgId, SessionId, UserId};
use beton_host::RunnerCapabilities;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::{ApiQuery, PageQuery, encode_cursor};
use crate::local_auth::{CODE_TTL, COOKIE_NAME};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::{Authenticated, ViaSocket};

/// Antwort von `/healthz`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Health {
    pub status: String,
}

/// Erreichbarkeit prüfen (ohne Anmeldung).
#[utoipa::path(get, path = "/healthz", tag = "system",
    responses((status = 200, description = "Server läuft", body = Health)))]
pub async fn healthz() -> axum::Json<Health> {
    axum::Json(Health {
        status: "ok".into(),
    })
}

/// Server-Informationen.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Info {
    pub version: String,
    pub schema_version: i64,
    pub protocol_version: u32,
    /// `local` (M0) oder `server`.
    pub mode: String,
    #[schema(value_type = String, example = "org_local")]
    pub org_id: OrgId,
}

/// Version, Schema- und Protokollversion des Servers.
#[utoipa::path(get, path = "/v1/info", tag = "system",
    responses((status = 200, description = "Server-Informationen", body = Info)))]
pub async fn info(State(state): State<AppState>) -> axum::Json<Info> {
    axum::Json(Info {
        version: env!("CARGO_PKG_VERSION").into(),
        schema_version: beton_store::SCHEMA_VERSION,
        protocol_version: 1,
        mode: "local".into(),
        org_id: state.local.org,
    })
}

/// Der angemeldete Benutzer.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Me {
    #[schema(value_type = String, example = "usr_local")]
    pub id: UserId,
    #[schema(value_type = String, example = "org_local")]
    pub org_id: OrgId,
    /// `bearer` oder `cookie`.
    pub auth: String,
}

/// Der angemeldete Benutzer und die Art der Anmeldung.
#[utoipa::path(get, path = "/v1/me", tag = "auth",
    responses((status = 200, description = "Angemeldeter Benutzer", body = Me)))]
pub async fn me(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
) -> axum::Json<Me> {
    axum::Json(Me {
        id: state.local.user,
        org_id: state.local.org,
        auth: match auth {
            Authenticated::Bearer => "bearer",
            Authenticated::Cookie => "cookie",
        }
        .into(),
    })
}

/// Eine Session in der Liste.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionSummary {
    #[schema(value_type = String, example = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C")]
    pub id: SessionId,
    pub title: String,
    /// Siehe `SessionStatus` im Event-Schema.
    pub status: String,
    pub kind: String,
    pub harness: String,
    pub archived: bool,
    pub head_seq: u64,
    /// Kosten in Mikro-Einheiten der Währung.
    pub cost_micro: i64,
    pub created_at: String,
    pub last_activity_at: String,
}

/// Eine Seite der Session-Liste (Cursor-Pagination, PROTO-010).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionPage {
    pub items: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionListQuery {
    /// Anzahl der Einträge, 1 bis 200 (Default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<String>,
    /// Opaker Cursor aus `next_cursor` der vorigen Seite.
    pub cursor: Option<String>,
    /// Archivierte Sessions einschließen.
    pub include_archived: Option<bool>,
}

impl SessionListQuery {
    fn page(&self) -> PageQuery {
        PageQuery {
            limit: self.limit.clone(),
            cursor: self.cursor.clone(),
        }
    }
}

/// Listen-Darstellung einer Session.
pub fn summary(s: beton_store::SessionRecord) -> SessionSummary {
    SessionSummary {
        id: s.id,
        title: s.title,
        status: enum_str(&s.status),
        kind: enum_str(&s.kind),
        harness: s.harness,
        archived: s.archived,
        head_seq: s.head_seq,
        cost_micro: s.cost_micro,
        created_at: s.created_at.to_string(),
        last_activity_at: s.last_activity_at.to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionCursor {
    before: SessionId,
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Session-Liste, neueste zuerst, mit Cursor-Pagination.
#[utoipa::path(get, path = "/v1/sessions", tag = "sessions",
    params(SessionListQuery),
    responses((status = 200, description = "Eine Seite der Session-Liste", body = SessionPage)))]
pub async fn list_sessions(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<SessionListQuery>,
) -> ApiResult<axum::Json<SessionPage>> {
    let page = q.page();
    let limit = page.limit()?;
    let before = page.cursor::<SessionCursor>()?.map(|c| c.before);
    let (sessions, next) = state
        .store
        .sessions_page(
            state.local.org,
            q.include_archived.unwrap_or(false),
            limit,
            before,
        )
        .await?;
    Ok(axum::Json(SessionPage {
        items: sessions.into_iter().map(summary).collect(),
        next_cursor: next.map(|before| encode_cursor(&SessionCursor { before })),
    }))
}

/// Ein Einmal-Code für die Browser-Anmeldung.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoginCode {
    pub code: String,
    /// Fertige URL zum Öffnen im Browser.
    pub redeem_url: String,
    pub expires_in_s: u64,
}

/// Einmal-Code erzeugen (AUTH-004). Nur mit lokalem Token und nur über Loopback.
#[utoipa::path(post, path = "/v1/auth/local/codes", tag = "auth",
    responses((status = 201, description = "Einmal-Code erzeugt", body = LoginCode)))]
pub async fn create_login_code(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    socket: Option<Extension<ViaSocket>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if auth != Authenticated::Bearer {
        return Err(
            Problem::new(ProblemCode::Forbidden).detail("Einmal-Codes nur mit dem lokalen Token")
        );
    }
    let loopback = socket.is_some() || peer.is_ok_and(|ConnectInfo(a)| a.ip().is_loopback());
    if !loopback {
        return Err(Problem::new(ProblemCode::LoopbackOnly));
    }
    let code = state
        .logins
        .issue_code(Instant::now())
        .map_err(|e| Problem::internal(&e))?;
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| !h.is_empty() && socket.is_none())
        .map(str::to_owned)
        .unwrap_or_else(|| state.primary_host.clone());
    let body = LoginCode {
        redeem_url: format!("http://{host}/auth/local/redeem?code={code}"),
        code,
        expires_in_s: CODE_TTL.as_secs(),
    };
    Ok((StatusCode::CREATED, axum::Json(body)).into_response())
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RedeemQuery {
    pub code: String,
    /// Relativer Pfad für die Weiterleitung (Default `/`).
    pub next: Option<String>,
}

/// Einmal-Code einlösen: setzt das Session-Cookie und leitet ohne Code weiter (AUTH-004).
#[utoipa::path(get, path = "/auth/local/redeem", tag = "auth",
    params(RedeemQuery),
    responses((status = 303, description = "Weiterleitung mit Set-Cookie")))]
pub async fn redeem_login_code(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<RedeemQuery>,
) -> ApiResult<Response> {
    let cookie = state
        .logins
        .redeem(&q.code, Instant::now())
        .ok_or_else(|| Problem::new(ProblemCode::InvalidCode))?;
    // Nur relative Pfade, keine Protokoll-relativen URLs (Open Redirect).
    let next = q
        .next
        .filter(|n| n.starts_with('/') && !n.starts_with("//") && !n.contains('\\'))
        .unwrap_or_else(|| "/".into());
    let mut res = StatusCode::SEE_OTHER.into_response();
    let headers = res.headers_mut();
    let set_cookie = format!("{COOKIE_NAME}={cookie}; HttpOnly; SameSite=Strict; Path=/");
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie).map_err(|e| Problem::internal(&e))?,
    );
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&next).map_err(|_| Problem::new(ProblemCode::BadRequest))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(res)
}

/// Das OpenAPI-Dokument (API-001).
#[utoipa::path(get, path = "/v1/openapi.json", tag = "system",
    responses((status = 200, description = "OpenAPI 3.1", content_type = "application/json")))]
pub async fn openapi_json(State(state): State<AppState>) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        state.openapi_json.as_ref().clone(),
    )
        .into_response()
}

/// Ein Provider eines Hosts mit seinen Capabilities (RUN-001).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderInfo {
    pub id: String,
    pub capabilities: RunnerCapabilities,
}

/// Ein Host (lokal genau `hst_local`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostInfo {
    #[schema(value_type = String, example = "hst_local")]
    pub id: HostId,
    pub name: String,
    pub providers: Vec<ProviderInfo>,
}

impl HostInfo {
    /// Der lokale Daemon mit dem Provider `local`.
    pub fn local() -> Self {
        Self {
            id: HostId::LOCAL,
            name: "lokal".into(),
            providers: vec![ProviderInfo {
                id: "local".into(),
                capabilities: beton_host::local::capabilities(),
            }],
        }
    }
}

/// Ein Host mit den unverändert gemeldeten Provider-Capabilities (RUN-001 AC3).
#[utoipa::path(get, path = "/v1/hosts/{id}", tag = "hosts",
    params(("id" = String, Path, description = "Host-ID, lokal `hst_local`")),
    responses((status = 200, description = "Host mit Providern", body = HostInfo),
              (status = 404, description = "Host unbekannt", body = Problem, content_type = "application/problem+json")))]
pub async fn get_host(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<axum::Json<HostInfo>> {
    let host = state.runtime.host.clone();
    if host.id.to_string() == id {
        Ok(axum::Json(host))
    } else {
        Err(Problem::new(ProblemCode::NotFound).detail(format!("Host {id}")))
    }
}
