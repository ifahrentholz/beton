//! Sicherheitstests des lokalen Modus (AUTH-001 … AUTH-004). Strikt TDD (AGENTS.md §4).

#![allow(clippy::unwrap_used)]

mod common;

use axum::body::Body;
use axum::extract::WebSocketUpgrade;
use axum::http::{Request, header};
use axum::response::IntoResponse;
use axum::routing::get;
use common::{HOST, ORIGIN, app, app_with};

#[tokio::test]
async fn auth_001_ac1_requests_need_the_local_token() {
    let t = app().await;
    let res = t.send(t.get("/v1/info").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status, 401);
    res.assert_problem("unauthorized");
    assert!(
        res.header("www-authenticate")
            .unwrap()
            .starts_with("Bearer")
    );

    let res = t
        .send(
            t.get("/v1/info")
                .header(header::AUTHORIZATION, "Bearer falsch")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 401);

    let res = t
        .send(t.authed("GET", "/v1/info").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["org_id"], "org_local");
}

#[tokio::test]
async fn auth_002_ac1_foreign_host_is_rejected_even_with_valid_credentials() {
    let t = app().await;
    let cookie = t.browser_cookie().await;
    for req in [
        Request::get("/v1/info")
            .header(header::HOST, "evil.test")
            .header(header::AUTHORIZATION, format!("Bearer {}", t.token)),
        Request::get("/v1/info")
            .header(header::HOST, "evil.test:7420")
            .header(header::COOKIE, cookie.clone()),
        Request::get("/healthz").header(header::HOST, "evil.test"),
        Request::get("/v1/info").header(header::AUTHORIZATION, format!("Bearer {}", t.token)),
    ] {
        let res = t.send(req.body(Body::empty()).unwrap()).await;
        assert_eq!(res.status, 403);
        res.assert_problem("host_not_allowed");
    }
    let ok = t
        .send(
            Request::get("/v1/info")
                .header(header::HOST, "LOCALHOST:7420")
                .header(header::AUTHORIZATION, format!("Bearer {}", t.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn auth_002_ac2_cookie_post_needs_allowed_origin_but_bearer_does_not() {
    let t = app().await;
    let cookie = t.browser_cookie().await;
    let post = |origin: Option<&str>, site: Option<&str>| {
        let mut b = Request::post("/v1/auth/local/codes")
            .header(header::HOST, HOST)
            .header(header::COOKIE, cookie.clone());
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        if let Some(s) = site {
            b = b.header("sec-fetch-site", s);
        }
        b.body(Body::empty()).unwrap()
    };
    for (origin, site) in [
        (Some("https://evil.test"), Some("cross-site")),
        (Some("https://evil.test"), Some("same-origin")),
        (None, Some("same-origin")),
        (Some(ORIGIN), Some("cross-site")),
        (Some(ORIGIN), None),
    ] {
        let res = t.send(post(origin, site)).await;
        assert_eq!(res.status, 403, "{origin:?} {site:?}");
        res.assert_problem("origin_not_allowed");
    }
    // Erlaubte Origin: die Sicherheitsschicht lässt durch; der Handler verlangt das Token.
    let res = t.send(post(Some(ORIGIN), Some("same-origin"))).await;
    assert_eq!(res.status, 403);
    res.assert_problem("forbidden");

    // Gleicher Request mit Bearer statt Cookie wird nach Token-Prüfung verarbeitet.
    let res = t
        .send(
            t.authed("POST", "/v1/auth/local/codes")
                .header(header::ORIGIN, "https://evil.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 201);

    // Lesende Cookie-Requests brauchen keine Origin.
    let res = t
        .send(
            t.get("/v1/me")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["auth"], "cookie");
}

#[tokio::test]
async fn auth_002_ac3_no_cors_or_private_network_headers() {
    let t = app().await;
    let res = t
        .send(
            Request::options("/v1/sessions")
                .header(header::HOST, HOST)
                .header(header::ORIGIN, "https://evil.test")
                .header("access-control-request-method", "POST")
                .header("access-control-request-private-network", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(res.status >= 400);
    for h in res.headers.keys() {
        assert!(
            !h.as_str().starts_with("access-control-"),
            "CORS-Header {h}"
        );
    }
    let res = t
        .send(
            t.authed("GET", "/v1/info")
                .header(header::ORIGIN, "https://evil.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(res.header("access-control-allow-origin").is_none());
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|_socket| async {})
}

fn ws_request(origin: Option<&str>) -> axum::http::request::Builder {
    let mut b = Request::get("/v1/ws-test")
        .header(header::HOST, HOST)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
    if let Some(o) = origin {
        b = b.header(header::ORIGIN, o);
    }
    b
}

#[tokio::test]
async fn auth_003_ac1_foreign_origin_websocket_is_rejected_before_upgrade() {
    let t = app_with(
        |r| r.route("/v1/ws-test", get(ws_handler)),
        "127.0.0.1:50000",
    )
    .await;
    let cookie = t.browser_cookie().await;
    let res = t
        .send(
            ws_request(Some("https://evil.test"))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 403);
    res.assert_problem("origin_not_allowed");
    assert!(res.header("sec-websocket-accept").is_none(), "kein Upgrade");

    for origin in [ORIGIN, "tauri://localhost"] {
        let res = t
            .send(
                ws_request(Some(origin))
                    .header(header::COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_ne!(res.status, 403, "{origin}");
    }
}

#[tokio::test]
async fn auth_003_ac2_websocket_without_origin_needs_authorization_header() {
    let t = app_with(
        |r| r.route("/v1/ws-test", get(ws_handler)),
        "127.0.0.1:50000",
    )
    .await;
    let cookie = t.browser_cookie().await;
    let res = t
        .send(
            ws_request(None)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 403);
    res.assert_problem("origin_not_allowed");

    let res = t
        .send(
            ws_request(None)
                .header(header::AUTHORIZATION, format!("Bearer {}", t.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_ne!(res.status, 403);
    assert_ne!(res.status, 401);
}

#[tokio::test]
async fn auth_004_ac2_redeem_sets_strict_cookie_and_redirects_without_code() {
    let t = app().await;
    let res = t
        .send(
            t.authed("POST", "/v1/auth/local/codes")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 201);
    let body = res.json();
    let code = body["code"].as_str().unwrap();
    assert_eq!(code.len(), 32);
    assert_eq!(body["expires_in_s"], 60);
    assert_eq!(
        body["redeem_url"],
        format!("http://{HOST}/auth/local/redeem?code={code}")
    );

    let res = t
        .send(
            t.get(&format!("/auth/local/redeem?code={code}&next=/sessions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 303);
    // AC2: Ziel der Weiterleitung enthält keinen Code; der Browser ersetzt den Eintrag.
    assert_eq!(res.header("location"), Some("/sessions"));
    let cookie = res.header("set-cookie").unwrap();
    assert!(cookie.starts_with("beton_session="));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    assert_eq!(res.header("cache-control"), Some("no-store"));
}

#[tokio::test]
async fn auth_004_ac1_code_is_single_use() {
    let t = app().await;
    let res = t
        .send(
            t.authed("POST", "/v1/auth/local/codes")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let code = res.json()["code"].as_str().unwrap().to_owned();
    let redeem = || {
        t.get(&format!("/auth/local/redeem?code={code}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(t.send(redeem()).await.status, 303);
    let res = t.send(redeem()).await;
    assert_eq!(res.status, 401);
    res.assert_problem("invalid_code");
    // Abgelaufene Codes deckt der Unit-Test in local_auth ab (60 s ohne Warten).
}

#[tokio::test]
async fn auth_004_open_redirects_are_not_possible() {
    let t = app().await;
    for next in ["//evil.test/x", "https://evil.test", "\\\\evil.test"] {
        let res = t
            .send(
                t.authed("POST", "/v1/auth/local/codes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let code = res.json()["code"].as_str().unwrap().to_owned();
        let res = t
            .send(
                t.get(&format!("/auth/local/redeem?code={code}&next={next}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.header("location"), Some("/"), "{next}");
    }
}

#[tokio::test]
async fn auth_004_ac3_codes_need_token_and_loopback_peer() {
    let t = app().await;
    let cookie = t.browser_cookie().await;
    let res = t
        .send(
            Request::post("/v1/auth/local/codes")
                .header(header::HOST, HOST)
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("sec-fetch-site", "same-origin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 403, "Cookie reicht nicht");

    let remote = app_with(|r| r, "192.168.1.20:40000").await;
    let res = remote
        .send(
            remote
                .authed("POST", "/v1/auth/local/codes")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 403);
    res.assert_problem("loopback_only");
}
