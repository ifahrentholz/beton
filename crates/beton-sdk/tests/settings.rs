//! Benachrichtigungs-Einstellungen über die API (UX-006): je User gespeichert, für alle
//! Clients gleich. Server im Prozess, nur Loopback.

#![allow(clippy::unwrap_used)]

use beton_core::notifications::NotificationSettings;
use beton_sdk::{Client, Error};
use beton_server::{ServerConfig, start};
use beton_store::{Store, StoreOptions};
use serde_json::json;

async fn server() -> (tempfile::TempDir, beton_server::Daemon, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    cfg.tunnel_socket = None;
    let daemon = start(cfg, store).await.unwrap();
    let base = format!("http://127.0.0.1:{}", daemon.addrs[0].port());
    let token = std::fs::read_to_string(dir.path().join("auth/local.token")).unwrap();
    (dir, daemon, base, token.trim().to_owned())
}

#[tokio::test]
async fn ux_006_ac3_second_client_sees_the_change_after_reload() {
    let (_dir, _daemon, base, token) = server().await;
    let desktop = Client::new(&base, &token).unwrap();
    let laptop = Client::new(&base, &token).unwrap();
    // Ohne gespeicherten Stand: die Defaults aus dem Entwurf.
    let defaults = desktop.notification_settings().await.unwrap();
    assert_eq!(defaults, NotificationSettings::default());
    assert!(!defaults.events.turn_completed.desktop);

    let mut changed = defaults.clone();
    changed.events.turn_completed.desktop = true;
    changed.quiet_hours.enabled = true;
    changed.preview = false;
    let saved = desktop.set_notification_settings(&changed).await.unwrap();
    assert_eq!(saved, changed);
    // Der zweite Client liest denselben Stand.
    assert_eq!(laptop.notification_settings().await.unwrap(), changed);
}

#[tokio::test]
async fn ux_006_invalid_documents_are_rejected_and_nothing_is_stored() {
    let (_dir, _daemon, base, token) = server().await;
    let http = reqwest::Client::new();
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
        let res = http
            .put(format!("{base}/v1/settings/notifications"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 400, "{body}");
        let problem: serde_json::Value = res.json().await.unwrap();
        assert_eq!(problem["code"], "validation_failed");
        assert!(
            problem.to_string().contains(pointer),
            "{pointer}: {problem}"
        );
    }
    let client = Client::new(&base, &token).unwrap();
    assert_eq!(
        client
            .notification_settings()
            .await
            .unwrap()
            .quiet_hours
            .start,
        "22:00"
    );
    let mut bad = NotificationSettings::default();
    bad.quiet_hours.end = "7 Uhr".into();
    assert!(matches!(
        client.set_notification_settings(&bad).await,
        Err(Error::Problem { status: 400, .. })
    ));
}
