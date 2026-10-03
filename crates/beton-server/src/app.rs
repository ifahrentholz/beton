//! Zusammenbau von Router, OpenAPI-Dokument und Schichten.

use std::any::Any;
use std::sync::Arc;

use axum::Router;
use axum::http::Method;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use beton_store::{LocalIdentity, Store};
use tower_http::catch_panic::CatchPanicLayer;
use utoipa::openapi::path::Operation;
use utoipa::openapi::{ContentBuilder, OpenApi, Ref, RefOr, ResponseBuilder};
use utoipa::{Modify, OpenApi as _};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api;
use crate::commands::CommandRegistry;
use crate::hub::{EventService, Hub};
use crate::idempotency::{self, Idempotency};
use crate::local_auth::{BrowserLogins, LocalToken};
use crate::problem::{FieldError, PROBLEM_CONTENT_TYPE, Problem, ProblemCode};
use crate::security::{self, Guard, OriginPolicy, PUBLIC_PATHS};
use crate::tunnel::{RunnerRegistry, TunnelConfig};
use crate::ws::{Authorizer, LocalAuthorizer, WsConfig};
use tokio::sync::watch;

/// Gemeinsamer Zustand der Handler.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub local: LocalIdentity,
    pub logins: Arc<BrowserLogins>,
    /// Host für Links, wenn der Request keinen brauchbaren `Host` hat (Socket).
    pub primary_host: String,
    pub openapi_json: Arc<Vec<u8>>,
    pub runtime: Runtime,
}

impl AppState {
    pub fn sessions(&self) -> crate::sessions::SessionManager<'_> {
        crate::sessions::SessionManager::new(self)
    }

    pub fn events(&self) -> EventService {
        EventService {
            store: self.store.clone(),
            hub: self.runtime.hub.clone(),
        }
    }
}

/// Laufzeit-Bausteine für Live-Verbindungen (WebSocket).
#[derive(Clone)]
pub struct Runtime {
    pub hub: Arc<Hub>,
    pub commands: Arc<CommandRegistry>,
    pub authorizer: Arc<dyn Authorizer>,
    pub ws: WsConfig,
    pub runners: Arc<RunnerRegistry>,
    pub tunnel: TunnelConfig,
    /// Der Host dieses Knotens mit seinen Providern.
    pub host: crate::api::HostInfo,
    pub sessions: crate::sessions::SessionsConfig,
    /// Harness-Katalog dieses Hosts (HAR-002).
    pub harnesses: beton_harness::registry::Registry,
    /// `true` beim Herunterfahren (Close 4503).
    pub shutdown: watch::Receiver<bool>,
}

impl Runtime {
    pub fn new(shutdown: watch::Receiver<bool>) -> Self {
        Self {
            hub: Arc::new(Hub::default()),
            commands: Arc::new(CommandRegistry::default()),
            authorizer: Arc::new(LocalAuthorizer),
            ws: WsConfig::default(),
            runners: Arc::new(RunnerRegistry::default()),
            tunnel: TunnelConfig::default(),
            host: crate::api::HostInfo::local(),
            sessions: crate::sessions::SessionsConfig {
                provider: Arc::new(beton_host::LocalProvider::new(
                    default_runner_command(),
                    std::env::temp_dir().join("beton-runners"),
                )),
                tunnel_socket: None,
                dev: cfg!(debug_assertions),
                launched: Arc::default(),
            },
            harnesses: default_registry(cfg!(debug_assertions)),
            shutdown,
        }
    }

    /// Laufzeit für eine Konfiguration: Runner-Zustand unter `<data_dir>/runners`, Tunnel-Socket.
    pub fn for_config(
        config: &crate::config::ServerConfig,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        let mut r = Self::new(shutdown);
        r.sessions.provider = Arc::new(beton_host::LocalProvider::new(
            default_runner_command(),
            config.data_dir.join("runners"),
        ));
        r.sessions.tunnel_socket.clone_from(&config.tunnel_socket);
        r
    }
}

/// Eingebaute Harnesses; der Fake nur im Entwicklermodus (HAR-026 AC3).
pub fn default_registry(dev: bool) -> beton_harness::registry::Registry {
    let mut r =
        beton_harness::registry::Registry::new(beton_harness::registry::RegistryOptions { dev });
    r.register(Arc::new(beton_harness_claude::ClaudeAdapter::default()));
    r
}

/// Runner-Kommando: `beton-runner` neben dem eigenen Binary, sonst aus `PATH`.
/// (Mit WP-10 wird daraus `beton runner`.)
pub fn default_runner_command() -> Vec<String> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("beton-runner")))
        .filter(|p| p.is_file());
    vec![sibling.map_or_else(|| "beton-runner".to_owned(), |p| p.display().to_string())]
}

impl Runtime {
    /// Standard-Kommandos (PROTO-006) für diese Laufzeit.
    pub fn with_default_commands(mut self) -> Self {
        let mut reg = (*self.commands).clone();
        crate::api_sessions::register_commands(&mut reg);
        self.commands = Arc::new(reg);
        self
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "beton",
        description = "REST-API von beton. Fehler sind Problem-Objekte nach RFC 9457.",
        license(name = "Apache-2.0", identifier = "Apache-2.0")
    ),
    servers((url = "http://127.0.0.1:7420", description = "Lokaler Daemon (Standard-Port)")),
    components(schemas(Problem, FieldError, ProblemCode))
)]
struct ApiDoc;

struct Security;

impl Modify for Security {
    fn modify(&self, openapi: &mut OpenApi) {
        use utoipa::openapi::security::{
            ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme,
        };
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "local_token",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
        components.add_security_scheme(
            "browser_session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(
                crate::local_auth::COOKIE_NAME,
            ))),
        );
        openapi.security = Some(vec![
            utoipa::openapi::security::SecurityRequirement::new(
                "local_token",
                Vec::<String>::new(),
            ),
            utoipa::openapi::security::SecurityRequirement::new(
                "browser_session",
                Vec::<String>::new(),
            ),
        ]);
    }
}

/// Ergänzt jede Operation um die Problem-Antworten der Sicherheitsschicht und des Servers
/// (PROTO-010 AC4, PROTO-011 AC1).
struct ProblemResponses;

fn problem_response(description: &str) -> RefOr<utoipa::openapi::Response> {
    RefOr::T(
        ResponseBuilder::new()
            .description(description)
            .content(
                PROBLEM_CONTENT_TYPE,
                ContentBuilder::new()
                    .schema(Some(Ref::from_schema_name("Problem")))
                    .build(),
            )
            .build(),
    )
}

impl Modify for ProblemResponses {
    fn modify(&self, openapi: &mut OpenApi) {
        for (path, item) in openapi.paths.paths.iter_mut() {
            let public = PUBLIC_PATHS.contains(&path.as_str());
            let ops: [&mut Option<Operation>; 5] = [
                &mut item.get,
                &mut item.post,
                &mut item.put,
                &mut item.patch,
                &mut item.delete,
            ];
            for op in ops.into_iter().flatten() {
                let r = &mut op.responses.responses;
                r.entry("400".into())
                    .or_insert_with(|| problem_response("Ungültige Anfrage"));
                if !public {
                    r.entry("401".into())
                        .or_insert_with(|| problem_response("Nicht angemeldet"));
                    op.security = None;
                } else {
                    op.security = Some(vec![]);
                }
                r.entry("403".into()).or_insert_with(|| {
                    problem_response("Host, Origin oder Berechtigung abgelehnt")
                });
                r.entry("500".into())
                    .or_insert_with(|| problem_response("Interner Fehler (mit trace_id)"));
            }
        }
    }
}

/// Router und OpenAPI-Dokument aus denselben Routen-Deklarationen.
pub fn routes() -> (Router<AppState>, OpenApi) {
    let (router, mut doc) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(api::healthz))
        .routes(routes!(api::info))
        .routes(routes!(api::me))
        .routes(routes!(api::list_sessions))
        .routes(routes!(api::create_login_code))
        .routes(routes!(api::redeem_login_code))
        .routes(routes!(api::openapi_json))
        .routes(routes!(crate::ws::ws_upgrade))
        .routes(routes!(api::get_host))
        .routes(routes!(crate::api_sessions::create_session))
        .routes(routes!(
            crate::api_sessions::get_session,
            crate::api_sessions::delete_session,
            crate::api_sessions::patch_session
        ))
        .routes(routes!(crate::api_sessions::list_harnesses))
        .routes(routes!(crate::api_sessions::archive_session))
        .routes(routes!(crate::api_sessions::unarchive_session))
        .routes(routes!(crate::api_sessions::interrupt_session))
        .routes(routes!(crate::api_sessions::resume_session))
        .routes(routes!(crate::api_sessions::submit_input))
        .routes(routes!(crate::api_sessions::list_events))
        .routes(routes!(crate::api_sessions::list_approvals))
        .routes(routes!(crate::api_sessions::resolve_approval))
        .routes(routes!(crate::api_sessions::get_blob))
        .routes(routes!(crate::api_sessions::list_tombstones))
        .split_for_parts();
    // Erst nach dem Einsammeln aller Pfade, sonst sehen die Modifier keine Operationen.
    Security.modify(&mut doc);
    ProblemResponses.modify(&mut doc);
    (router, doc)
}

/// Das OpenAPI-Dokument (für `cargo xtask codegen`, API-001 AC1).
pub fn openapi() -> OpenApi {
    routes().1
}

/// Zutaten für die App.
pub struct AppParts {
    pub store: Store,
    pub local: LocalIdentity,
    pub token: Arc<LocalToken>,
    pub logins: Arc<BrowserLogins>,
    pub hosts: Vec<String>,
    pub origins: Vec<String>,
    pub primary_host: String,
    pub runtime: Runtime,
}

/// Die vollständige App mit allen Schichten.
pub fn build(parts: AppParts) -> Router {
    let (router, doc) = routes();
    layered(router, &doc, parts)
}

/// Legt Zustand, Fallbacks und alle Schichten um einen Router (auch für Tests mit
/// zusätzlichen Routen).
pub fn layered(router: Router<AppState>, doc: &OpenApi, parts: AppParts) -> Router {
    let openapi_json = Arc::new(doc.to_pretty_json().unwrap_or_default().into_bytes());
    let state = AppState {
        store: parts.store.clone(),
        local: parts.local,
        logins: parts.logins.clone(),
        primary_host: parts.primary_host,
        openapi_json,
        runtime: parts.runtime,
    };
    let guard = Guard {
        hosts: Arc::new(parts.hosts),
        origins: Arc::new(OriginPolicy::new(parts.origins)),
        token: parts.token,
        logins: parts.logins,
    };
    let idem = Idempotency {
        store: parts.store,
        org: parts.local.org,
    };
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
        .layer(middleware::from_fn_with_state(idem, idempotency::layer))
        .layer(middleware::from_fn_with_state(guard, security::guard))
        .layer(CatchPanicLayer::custom(panic_response))
}

/// Kennzeichnet ein 404 ohne passende Route (im Unterschied zu „Ressource unbekannt“).
pub const NO_ROUTE: &str = "Keine Route für diesen Pfad";

async fn not_found() -> Problem {
    Problem::new(ProblemCode::NotFound).detail(NO_ROUTE)
}

async fn method_not_allowed(method: Method) -> Problem {
    Problem::new(ProblemCode::MethodNotAllowed).detail(format!("{method} ist hier nicht erlaubt"))
}

/// Panic in einem Handler: 500 mit `trace_id`, Ursache nur im Log (PROTO-011 AC3).
fn panic_response(err: Box<dyn Any + Send + 'static>) -> Response {
    let cause = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "unbekannt".into());
    Problem::internal(&format!("Panic im Handler: {cause}")).into_response()
}
