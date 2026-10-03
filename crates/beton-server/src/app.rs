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
use crate::idempotency::{self, Idempotency};
use crate::local_auth::{BrowserLogins, LocalToken};
use crate::problem::{FieldError, PROBLEM_CONTENT_TYPE, Problem, ProblemCode};
use crate::security::{self, Guard, OriginPolicy, PUBLIC_PATHS};

/// Gemeinsamer Zustand der Handler.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub local: LocalIdentity,
    pub logins: Arc<BrowserLogins>,
    /// Host für Links, wenn der Request keinen brauchbaren `Host` hat (Socket).
    pub primary_host: String,
    pub openapi_json: Arc<Vec<u8>>,
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

async fn not_found() -> Problem {
    Problem::new(ProblemCode::NotFound)
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
