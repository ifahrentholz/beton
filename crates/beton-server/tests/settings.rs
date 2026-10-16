//! Benachrichtigungs-Einstellungen über die API (UX-006): je User gespeichert, für alle
//! Clients gleich.

#![allow(clippy::unwrap_used)]

mod common;

use axum::body::Body;
use axum::http::header;
use common::{ORIGIN, app};
use serde_json::json;

#[tokio::test]
async fn ux_006_ac3_second_client_sees_the_change_after_reload() {
    let app = app().await;
    // Ohne gespeicherten Stand: die Defaults aus dem Entwurf.
    let res = app
        .send(
            app.authed("GET", "/v1/settings/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    let defaults = res.json();
    assert_eq!(defaults["preview"], true);
    assert_eq!(defaults["events"]["turn_completed"]["desktop"], false);
    assert_eq!(defaults["quiet_hours"]["start"], "22:00");

    // Client 1 (Desktop-App mit lokalem Token) ändert Kanal, Ruhezeiten und Vorschau.
    let mut changed = defaults.clone();
    changed["events"]["turn_completed"]["desktop"] = json!(true);
    changed["quiet_hours"]["enabled"] = json!(true);
    changed["preview"] = json!(false);
    let res = app
        .send(
            app.authed("PUT", "/v1/settings/notifications")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(changed.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    assert_eq!(res.json(), changed);

    // Client 2 (Browser mit Cookie) sieht denselben Stand.
    let cookie = app.browser_cookie().await;
    let res = app
        .send(
            app.get("/v1/settings/notifications")
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("sec-fetch-site", "same-origin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json(), changed);
}

#[tokio::test]
async fn ux_006_invalid_documents_are_rejected_and_nothing_is_stored() {
    let app = app().await;
    for (body, pointer) in [
        (
            json!({"quiet_hours": {"start": "25:00"}}),
            "quiet_hours.start",
        ),
        (json!({"web_push": true}), "web_push"),
        (
            json!({"events": {"turn_completed": {"desktop": "ja"}}}),
            "desktop",
        ),
    ] {
        let res = app
            .send(
                app.authed("PUT", "/v1/settings/notifications")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 400, "{body}");
        res.assert_problem("validation_failed");
        let text = String::from_utf8_lossy(&res.body).to_string();
        assert!(text.contains(pointer), "{pointer}: {text}");
    }
    let res = app
        .send(
            app.authed("GET", "/v1/settings/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.json()["quiet_hours"]["start"], "22:00");
}
