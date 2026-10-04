//! Contract-Tests der REST-API (PROTO-010, PROTO-011, API-001, API-002).

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Request, header};
use axum::routing::get;
use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_server::security::PUBLIC_PATHS;
use beton_store::NewSession;
use common::{HOST, TestApp, app, app_with};
use serde_json::Value;

fn openapi() -> Value {
    serde_json::to_value(beton_server::openapi()).unwrap()
}

/// Alle dokumentierten (Methode, Pfad)-Paare.
fn operations() -> Vec<(String, String)> {
    let doc = openapi();
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                out.push((method.to_uppercase(), path.clone()));
            }
        }
    }
    out
}

#[tokio::test]
async fn proto_011_ac1_every_error_response_is_problem_json() {
    let t = app().await;
    let cases: Vec<(Request<Body>, u16, &str)> = vec![
        (
            t.get("/v1/info").body(Body::empty()).unwrap(),
            401,
            "unauthorized",
        ),
        (
            Request::get("/v1/info")
                .header(header::HOST, "evil.test")
                .body(Body::empty())
                .unwrap(),
            403,
            "host_not_allowed",
        ),
        (
            t.authed("GET", "/v1/gibt-es-nicht")
                .body(Body::empty())
                .unwrap(),
            404,
            "not_found",
        ),
        (
            t.authed("DELETE", "/v1/info").body(Body::empty()).unwrap(),
            405,
            "method_not_allowed",
        ),
        (
            t.authed("GET", "/v1/sessions?limit=0")
                .body(Body::empty())
                .unwrap(),
            400,
            "invalid_limit",
        ),
        (
            t.authed("GET", "/v1/sessions?cursor=kaputt")
                .body(Body::empty())
                .unwrap(),
            400,
            "invalid_cursor",
        ),
        (
            t.authed("GET", "/v1/sessions?include_archived=vielleicht")
                .body(Body::empty())
                .unwrap(),
            400,
            "bad_request",
        ),
        (
            t.get("/auth/local/redeem?code=falsch")
                .body(Body::empty())
                .unwrap(),
            401,
            "invalid_code",
        ),
        (
            t.get("/auth/local/redeem").body(Body::empty()).unwrap(),
            400,
            "bad_request",
        ),
    ];
    for (req, status, code) in cases {
        let res = t.send(req).await;
        assert_eq!(
            res.status,
            status,
            "{code}: {}",
            String::from_utf8_lossy(&res.body)
        );
        res.assert_problem(code);
    }
}

#[tokio::test]
async fn proto_011_ac3_panic_gives_500_with_trace_id_and_no_stacktrace() {
    async fn boom() -> &'static str {
        panic!("geheimer Pfad /home/kim/.beton/auth/local.token");
    }
    let t = app_with(|r| r.route("/v1/panic-test", get(boom)), "127.0.0.1:50000").await;
    let res = t
        .send(
            t.authed("GET", "/v1/panic-test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 500);
    res.assert_problem("internal");
    let body = String::from_utf8_lossy(&res.body);
    assert!(!body.contains("geheimer") && !body.contains("/home/") && !body.contains("panicked"));
    assert_eq!(res.json()["trace_id"].as_str().unwrap().len(), 16);
}

#[tokio::test]
async fn api_002_ac2_every_route_requires_authentication() {
    let t = app().await;
    let ops = operations();
    assert!(ops.len() >= 7);
    for (method, path) in ops {
        if PUBLIC_PATHS.contains(&path.as_str()) {
            continue;
        }
        let path = path
            .replace("{id}", "hst_local")
            .replace("{approval_id}", "x")
            .replace("{hash}", "x");
        let req = Request::builder()
            .method(method.as_str())
            .uri(&path)
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        let res = t.send(req).await;
        assert_eq!(res.status, 401, "{method} {path} ohne Anmeldung");
        res.assert_problem("unauthorized");
    }
}

#[tokio::test]
async fn api_001_ac2_every_route_is_documented() {
    // Router und OpenAPI entstehen aus denselben `routes!`-Deklarationen; direkte
    // `.route(`-Aufrufe am Produktions-Router würden das umgehen.
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains(".route(\""),
            "{}: Route ohne OpenAPI-Dokumentation",
            path.display()
        );
    }
    // Jede dokumentierte Operation ist erreichbar: kein 405 und kein 404 des Router-Fallbacks
    // (ein 404 „Ressource unbekannt“ des Handlers ist in Ordnung).
    let t = app().await;
    for (method, path) in operations() {
        let path = path
            .replace("{id}", "hst_local")
            .replace("{approval_id}", "x")
            .replace("{hash}", "x");
        let req = t.authed(&method, &path).body(Body::empty()).unwrap();
        let res = t.send(req).await;
        assert_ne!(res.status, 405, "{method} {path}");
        if res.status == 404 {
            assert_ne!(
                res.json()["detail"],
                beton_server::app::NO_ROUTE,
                "{method} {path}"
            );
        }
    }
}

#[tokio::test]
async fn api_002_ac1_contract_for_wp04_resources() {
    let t = app().await;
    for path in ["/v1/info", "/v1/me", "/v1/sessions", "/v1/openapi.json"] {
        let ok = t
            .send(t.authed("GET", path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(ok.status, 200, "{path}");
        let forbidden = t
            .send(
                Request::get(path)
                    .header(header::HOST, "evil.test")
                    .header(header::AUTHORIZATION, format!("Bearer {}", t.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(forbidden.status, 403, "{path}");
        let missing = t
            .send(
                t.authed("GET", &format!("{path}/unbekannt"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(missing.status, 404, "{path}");
    }
    let me = t
        .send(t.authed("GET", "/v1/me").body(Body::empty()).unwrap())
        .await;
    assert_eq!(me.json()["id"], "usr_local");
    assert_eq!(me.json()["auth"], "bearer");
}

async fn create_sessions(t: &TestApp, n: usize) {
    let local = t.store.ensure_local().await.unwrap();
    for _ in 0..n {
        t.store
            .create_session(
                OrgId::LOCAL,
                NewSession {
                    id: SessionId::new(),
                    owner: UserId::LOCAL,
                    kind: SessionKind::Main,
                    harness: "fake".into(),
                    cwd: "/".into(),
                    model: None,
                    agent_ref: None,
                    project_id: None,
                    parent_id: None,
                    trigger: SessionTrigger::User,
                    home_node: local.node,
                    harness_opts: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn proto_010_ac1_pagination_returns_every_session_exactly_once() {
    let t = app().await;
    create_sessions(&t, 1_000).await;
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let path = match &cursor {
            Some(c) => format!("/v1/sessions?limit=50&cursor={c}"),
            None => "/v1/sessions?limit=50".into(),
        };
        let res = t
            .send(t.authed("GET", &path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status, 200);
        let page = res.json();
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= 50);
        seen.extend(items.iter().map(|s| s["id"].as_str().unwrap().to_owned()));
        pages += 1;
        // Während des Paginierens entstehen neue Sessions.
        create_sessions(&t, 3).await;
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!(pages, 20);
    let unique: BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 1_000);
    assert_eq!(seen.len(), 1_000);
}

#[tokio::test]
async fn proto_010_ac2_idempotency_key_replays_or_rejects() {
    let t = app().await;
    let post = |body: &'static str| {
        t.authed("POST", "/v1/auth/local/codes")
            .header("idempotency-key", "schluessel-1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let first = t.send(post("")).await;
    assert_eq!(first.status, 201);
    let again = t.send(post("")).await;
    assert_eq!(again.status, 201);
    assert_eq!(again.body, first.body, "gespeicherte Antwort");
    assert_eq!(again.header("idempotent-replayed"), Some("true"));

    let other = t.send(post("{\"anders\":true}")).await;
    assert_eq!(other.status, 422);
    other.assert_problem("idempotency_key_reused");
}

fn is_snake_case(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !s.starts_with('_')
}

fn property_names(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if let Some(Value::Object(props)) = map.get("properties") {
                out.extend(props.keys().cloned());
            }
            for child in map.values() {
                property_names(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| property_names(i, out)),
        _ => {}
    }
}

/// PROTO-010 AC4: Lint über das OpenAPI-Dokument.
#[test]
fn proto_010_ac4_openapi_lint() {
    let doc = openapi();
    assert!(doc["openapi"].as_str().unwrap().starts_with("3.1"));

    // snake_case für alle Felder und Parameter.
    let mut names = Vec::new();
    property_names(&doc["components"], &mut names);
    property_names(&doc["paths"], &mut names);
    for (_, item) in doc["paths"].as_object().unwrap() {
        for op in item.as_object().unwrap().values() {
            for p in op["parameters"].as_array().into_iter().flatten() {
                names.push(p["name"].as_str().unwrap().to_owned());
            }
        }
    }
    for n in &names {
        assert!(is_snake_case(n), "kein snake_case: {n}");
    }

    let schemas = &doc["components"]["schemas"];
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            // Fehlerantworten: immer Problem.
            let responses = op["responses"].as_object().unwrap();
            assert!(
                responses.keys().any(|s| s.starts_with('4')),
                "{method} {path}: keine 4xx"
            );
            for (status, r) in responses {
                if status.starts_with('4') || status.starts_with('5') {
                    let schema = &r["content"]["application/problem+json"]["schema"]["$ref"];
                    assert_eq!(
                        schema, "#/components/schemas/Problem",
                        "{method} {path} {status}"
                    );
                }
            }
            // Listen: Cursor-Pagination statt nackter Arrays.
            let ok = &responses
                .get("200")
                .map(|r| &r["content"]["application/json"]["schema"]);
            let Some(schema) = ok else { continue };
            assert_ne!(
                schema["type"], "array",
                "{method} {path}: Liste ohne Pagination"
            );
            let resolved = schema["$ref"]
                .as_str()
                .and_then(|r| r.strip_prefix("#/components/schemas/"))
                .map(|name| &schemas[name]);
            if let Some(s) = resolved
                && s["properties"].get("items").is_some()
            {
                assert!(
                    s["properties"].get("next_cursor").is_some(),
                    "{method} {path}"
                );
                let params: Vec<&str> = op["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["name"].as_str())
                    .collect();
                assert!(
                    params.contains(&"limit") && params.contains(&"cursor"),
                    "{method} {path}: {params:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn run_001_ac3_host_reports_provider_capabilities_unchanged() {
    let t = app().await;
    let res = t
        .send(
            t.authed("GET", "/v1/hosts/hst_local")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    let host = res.json();
    assert_eq!(host["id"], "hst_local");
    assert_eq!(
        host["providers"][0]["capabilities"],
        serde_json::to_value(beton_host::local::capabilities()).unwrap()
    );
    let missing = t
        .send(
            t.authed("GET", "/v1/hosts/hst_01JB8Y2D0M3K4J5H6G7F8E9D0C")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(missing.status, 404);
    // Schema-Snapshot: RunnerCapabilities steht in der generierten OpenAPI.
    assert!(openapi()["components"]["schemas"]["RunnerCapabilities"].is_object());
}
