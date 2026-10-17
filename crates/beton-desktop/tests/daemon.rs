//! Laufender Daemon in anderer Version (DESK-002 AC3) und fehlendes Sidecar, gegen einen
//! Mock-Daemon auf Loopback.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_desktop::daemon::{DaemonError, Manager, Step};
use serde_json::json;

/// Mock für `GET /v1/info` mit Version `version`; legt `run/daemon.json` und das lokale Token
/// im Datenverzeichnis an.
async fn mock_daemon(home: &Path, version: &'static str) {
    let app = axum::Router::new().route(
        "/v1/info",
        axum::routing::get(move || async move {
            axum::Json(json!({"version": version, "protocol_version": 1, "schema_version": 14}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::fs::create_dir_all(home.join("run")).unwrap();
    std::fs::create_dir_all(home.join("auth")).unwrap();
    std::fs::write(
        home.join("run/daemon.json"),
        json!({"pid": 1, "http": addr.to_string(), "version": version}).to_string(),
    )
    .unwrap();
    std::fs::write(home.join("auth/local.token"), "t".repeat(64)).unwrap();
}

#[tokio::test]
async fn desk_002_ac3_incompatible_daemon_is_reported_with_both_versions() {
    let home = tempfile::tempdir().unwrap();
    mock_daemon(home.path(), "0.1.0").await;
    let manager = Manager::new(home.path().to_path_buf(), None, "0.3.2");
    let steps = std::sync::Mutex::new(Vec::new());
    let err = manager
        .connect(|s| steps.lock().unwrap().push(s))
        .await
        .unwrap_err();
    let DaemonError::Incompatible(v) = err else {
        panic!("statt eines generischen Fehlers: {err}");
    };
    assert_eq!((v.daemon.as_str(), v.app.as_str()), ("0.1.0", "0.3.2"));
    assert!(
        !steps.lock().unwrap().contains(&Step::Starting),
        "kein zweiter Daemon"
    );
}

#[tokio::test]
async fn desk_002_compatible_running_daemon_is_used_as_is() {
    let home = tempfile::tempdir().unwrap();
    mock_daemon(home.path(), "0.3.0").await;
    let manager = Manager::new(home.path().to_path_buf(), None, "0.3.2");
    let conn = manager.connect(|_| {}).await.unwrap();
    assert!(!conn.status.started_by_app);
    assert_eq!(conn.status.version, "0.3.0");
}

#[tokio::test]
async fn desk_002_without_daemon_and_sidecar_the_app_says_what_is_missing() {
    let home = tempfile::tempdir().unwrap();
    let manager = Manager::new(home.path().to_path_buf(), None, "0.3.2");
    assert!(matches!(
        manager.connect(|_| {}).await.unwrap_err(),
        DaemonError::NoSidecar
    ));
}
