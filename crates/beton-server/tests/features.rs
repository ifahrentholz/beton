//! Interne Feature-Flags über die API (UX-007): aktive Flags in `GET /v1/info`, Funktionen
//! hinter einem nicht aktivierten Flag sind nicht erreichbar.

#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use beton_core::feature::FeatureSet;
use common::{TestApp, app_runtime};
use serde_json::json;

/// Ohne `--dev`, mit Fake im Katalog: Nur das Flag entscheidet.
async fn app(features: FeatureSet) -> TestApp {
    app_runtime(move |mut r| {
        r.sessions.dev = false;
        r.harnesses = beton_server::app::default_registry(true);
        r.features = Arc::new(features);
        r
    })
    .await
}

async fn harness_ids(app: &TestApp) -> Vec<String> {
    let res = app
        .send(
            app.authed("GET", "/v1/harnesses")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn ux_007_ac1_experimental_flag_is_unreachable_without_activation() {
    let app = app(FeatureSet::from_sources(&[], None)).await;
    let info = app
        .send(app.authed("GET", "/v1/info").body(Body::empty()).unwrap())
        .await;
    assert_eq!(info.json()["features"], json!([]));
    // UI: Der Harness-Katalog (Grundlage der Picker) enthält den Fake nicht.
    assert!(!harness_ids(&app).await.contains(&"fake".to_owned()));
    // API: 404 `feature_disabled`.
    let res = app
        .send(
            app.authed("POST", "/v1/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"target": "fake", "cwd": app.dir.path()}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 404);
    res.assert_problem("feature_disabled");
}

#[tokio::test]
async fn ux_007_ac1_activated_flag_is_listed_in_info_and_reachable() {
    let app = app(FeatureSet::from_sources(&[], Some("fake_harness"))).await;
    let info = app
        .send(app.authed("GET", "/v1/info").body(Body::empty()).unwrap())
        .await;
    assert_eq!(info.json()["features"], json!(["fake_harness"]));
    assert!(harness_ids(&app).await.contains(&"fake".to_owned()));
}
