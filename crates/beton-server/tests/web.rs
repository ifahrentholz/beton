//! Auslieferung der Web-UI (WEB-001): Deep-Links, Assets, CSP, Anmeldung im Browser.

#![allow(clippy::unwrap_used)]

mod common;

use axum::body::Body;
use axum::http::header;
use common::app;

#[tokio::test]
async fn web_001_ac2_deep_link_serves_the_app_shell() {
    let app = app().await;
    let cookie = app.browser_cookie().await;
    for path in [
        "/",
        "/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C",
        "/s/ses_x?tab=chat",
    ] {
        let res = app
            .send(
                app.get(path)
                    .header(header::COOKIE, &cookie)
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 200, "{path}");
        assert!(
            String::from_utf8_lossy(&res.body).contains("<div id=app>"),
            "{path}"
        );
        assert_eq!(res.header("cache-control"), Some("no-cache"));
        let csp = res.header("content-security-policy").unwrap();
        assert!(csp.contains("default-src 'self'"), "{csp}");
        assert!(!csp.contains("http"), "keine fremden Origins: {csp}");
    }
}

#[tokio::test]
async fn web_001_assets_are_served_and_cached() {
    let app = app().await;
    let res = app
        .send(
            app.authed("GET", "/assets/app-1234.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(res.header("cache-control").unwrap().contains("immutable"));
    // Fehlende Assets sind 404, nicht die App.
    let res = app
        .send(
            app.authed("GET", "/assets/fehlt.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    res.assert_problem("not_found");
    // Pfade außerhalb des Builds bleiben unerreichbar.
    let res = app
        .send(
            app.authed("GET", "/../beton.db")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_ne!(res.status, 200);
    // API-Pfade fallen nie auf die App zurück.
    let res = app
        .send(
            app.authed("GET", "/v1/gibt-es-nicht")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    res.assert_problem("not_found");
}

#[tokio::test]
async fn auth_001_browser_without_login_gets_a_hint_page_not_the_app() {
    let app = app().await;
    let res = app
        .send(
            app.get("/s/ses_x")
                .header(header::ACCEPT, "text/html,application/xhtml+xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 401);
    let body = String::from_utf8_lossy(&res.body);
    assert!(body.contains("beton open"), "{body}");
    assert!(!body.contains("<div id=app>"));
    // Ohne HTML-Wunsch bleibt es beim Problem-JSON; Assets brauchen ebenfalls Anmeldung.
    let res = app
        .send(app.get("/assets/app-1234.js").body(Body::empty()).unwrap())
        .await;
    res.assert_problem("unauthorized");
    let res = app
        .send(
            app.get("/v1/sessions")
                .header(header::ACCEPT, "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    res.assert_problem("unauthorized");
}
