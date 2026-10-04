//! Integrationstests gegen das Binary `beton` (CLI-001, CLI-004, CLI-008, AUTH-001, AUTH-004,
//! DATA-005). Der Daemon läuft als eigener Prozess mit eigenem `BETON_HOME`; Sessions nutzen
//! den Fake-Harness (`--dev`), Runner starten als `beton __runner`.

#![allow(clippy::unwrap_used)]

mod common;

use std::process::{Command, Stdio};

use beton_core::id::OrgId;
use beton_sdk::DaemonInfo;
use common::{Serve, beton, open_store, run, stderr, stdout};
use serde_json::Value;

// --------------------------------------------------------------------------- CLI-001

#[tokio::test]
async fn cli_001_ac1_session_list_json_is_pure_json_on_stdout() {
    let serve = Serve::start();
    let id = serve.create_idle_session().await;
    let out = run(beton(serve.home()).args(["session", "list", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let parsed: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout ist kein JSON ({e}): {}", stdout(&out)));
    let items = parsed.as_array().expect("JSON-Array");
    assert!(items.iter().any(|s| s["id"] == id.as_str()), "{parsed}");

    // Ohne `--json`: Tabelle mit Kopfzeile.
    let out = run(beton(serve.home()).args(["session", "list"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("ID"), "{text}");
    assert!(text.contains(&id), "{text}");
}

#[test]
fn cli_001_ac2_unknown_flag_exits_2_with_help_on_stderr() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        &["--gibt-es-nicht"][..],
        &["session", "list", "--bogus"],
        &["nix"],
    ] {
        let out = run(beton(home.path()).args(args));
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = stderr(&out);
        assert!(err.contains("Usage:"), "{args:?}: {err}");
        assert!(err.contains("--help"), "{args:?}: {err}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn cli_001_unreachable_server_exits_7() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("run")).unwrap();
    std::fs::create_dir_all(home.path().join("auth")).unwrap();
    std::fs::write(
        DaemonInfo::path_in(home.path()),
        serde_json::to_vec(&DaemonInfo {
            pid: 1,
            http: "127.0.0.1:9".into(),
            version: "0".into(),
        })
        .unwrap(),
    )
    .unwrap();
    std::fs::write(home.path().join("auth/local.token"), "0".repeat(64)).unwrap();
    let out = run(beton(home.path()).args(["session", "list", "--json"]));
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert!(out.stdout.is_empty());
}

#[test]
fn cli_001_version_and_completion_write_to_stdout() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args(["version", "--json"]));
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], beton_cli::VERSION);
    let out = run(beton(home.path()).args(["completion", "zsh"]));
    assert!(out.status.success());
    assert!(stdout(&out).contains("beton"));
}

// --------------------------------------------------------------------------- CLI-004

#[test]
fn cli_004_ac1_bind_all_interfaces_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args(["serve", "--bind", "0.0.0.0", "--port", "0"]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("AUTH-009"), "{}", stderr(&out));
    assert!(DaemonInfo::read(home.path()).is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn cli_004_ac3_sigterm_shuts_down_cleanly() {
    let mut serve = Serve::start();
    let id = serve.create_idle_session().await;
    // Runner sind direkte Kindprozesse des Daemons (`beton __runner`).
    let serve_pid = serve.child.as_ref().unwrap().id().to_string();
    let children = run(Command::new("pgrep").args(["-P", &serve_pid]));
    let runners: Vec<String> = stdout(&children).lines().map(str::to_owned).collect();
    assert!(!runners.is_empty(), "kein Runner-Prozess gefunden");
    let status = serve.terminate();
    assert!(status.success(), "{status}: {}", serve.log());
    for pid in &runners {
        let alive = Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "Runner {pid} läuft nach dem Herunterfahren weiter");
    }
    assert!(
        DaemonInfo::read(serve.home()).is_none(),
        "daemon.json bleibt liegen"
    );
    // Event-Log ist konsistent: lückenlos 1..=head_seq.
    let store = open_store(serve.home()).await;
    let session_id = id.parse().unwrap();
    let record = store.session(OrgId::LOCAL, session_id).await.unwrap();
    let events = store
        .events(OrgId::LOCAL, session_id, 0, 10_000)
        .await
        .unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=record.head_seq).collect::<Vec<_>>());
    store.close().await;
    // Der Lock ist frei: ein neuer Daemon startet.
    let again = Serve::start_with(&[]);
    drop(again);
}

#[tokio::test]
async fn cli_004_second_serve_for_the_same_home_is_refused() {
    let serve = Serve::start();
    let out = run(beton(serve.home()).args(["serve", "--foreground", "--port", "0", "--dev"]));
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("läuft bereits"), "{}", stderr(&out));
    // Der laufende Daemon bleibt erreichbar.
    serve.client().info().await.unwrap();
    let _ = serve.info.pid;
}

// --------------------------------------------------------------------------- CLI-008

#[test]
fn cli_008_ac1_config_list_shows_sources() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).current_dir(work.path()).args([
        "config",
        "set",
        "server.allowed_hosts",
        "[beton.local]",
    ]));
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(beton(home.path())
        .current_dir(work.path())
        .env("BETON_CFG_EVENTS__STORE_RAW", "false")
        .args(["config", "list", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let items: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    let source = |key: &str| {
        items
            .iter()
            .find(|i| i["key"] == key)
            .map(|i| i["source"].as_str().unwrap().to_owned())
            .unwrap_or_else(|| panic!("{key} fehlt"))
    };
    assert_eq!(source("server.allowed_hosts"), "user");
    assert_eq!(source("events.store_raw"), "env");
    assert_eq!(source("server.listen"), "default");

    let out = run(beton(home.path())
        .current_dir(work.path())
        .args(["config", "list"]));
    assert!(
        stdout(&out).contains("server.allowed_hosts = [\"beton.local\"] (user)"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn cli_008_ac2_invalid_value_is_rejected_with_schema_error() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let config = home.path().join("config.yaml");
    std::fs::write(&config, "events:\n  store_raw: true\n").unwrap();
    let out = run(beton(home.path()).current_dir(work.path()).args([
        "config",
        "set",
        "events.store_raw",
        "vielleicht",
    ]));
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("events.store_raw"), "{err}");
    assert!(err.contains("expected a boolean"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "events:\n  store_raw: true\n"
    );
    let out = run(beton(home.path()).current_dir(work.path()).args([
        "config",
        "get",
        "events.store_raw",
    ]));
    assert_eq!(stdout(&out).trim(), "true");
}

// --------------------------------------------------------------------------- AUTH

#[tokio::test]
async fn auth_001_rotate_local_takes_effect_in_the_running_daemon() {
    let serve = Serve::start();
    let old = serve.client();
    old.info().await.unwrap();
    let out = run(beton(serve.home()).args(["auth", "rotate-local"]));
    assert!(out.status.success(), "{}", stderr(&out));
    match old.info().await {
        Err(beton_sdk::Error::Problem { status: 401, .. }) => {}
        other => panic!("altes Token gilt noch: {other:?}"),
    }
    serve.client().info().await.unwrap();
    let out = run(beton(serve.home()).args(["session", "list", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
}

#[cfg(unix)]
#[tokio::test]
async fn auth_004_open_hands_a_one_time_link_to_the_browser() {
    use std::os::unix::fs::PermissionsExt;
    let serve = Serve::start();
    let seen = serve.work.path().join("browser.txt");
    let browser = serve.work.path().join("browser.sh");
    std::fs::write(
        &browser,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", seen.display()),
    )
    .unwrap();
    std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o755)).unwrap();
    let session = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C";
    let out = run(beton(serve.home())
        .env("BROWSER", &browser)
        .args(["open", session, "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let url = v["url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!(
            "http://{}/auth/local/redeem?code=",
            serve.info.http
        )),
        "{url}"
    );
    assert!(url.ends_with(&format!("&next=/s/{session}")), "{url}");
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), url);
}

// --------------------------------------------------------------------------- DATA-005

#[cfg(unix)]
#[tokio::test]
async fn data_005_admin_projections_rebuild_restores_the_dump() {
    let mut serve = Serve::start();
    serve.create_idle_session().await;
    serve.create_idle_session().await;
    assert!(serve.terminate().success(), "{}", serve.log());

    let store = open_store(serve.home()).await;
    let before = store.projection_dump(OrgId::LOCAL).await.unwrap();
    assert!(before.len() >= 2);
    store.close().await;

    // Projektion beschädigen.
    let db = serve.home().join("beton.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET title = 'kaputt', cost_micro = 99")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let store = open_store(serve.home()).await;
    assert_ne!(store.projection_dump(OrgId::LOCAL).await.unwrap(), before);
    store.close().await;

    let out = run(beton(serve.home()).args(["admin", "projections", "rebuild", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["sessions"], 2);
    let store = open_store(serve.home()).await;
    assert_eq!(store.projection_dump(OrgId::LOCAL).await.unwrap(), before);
    store.close().await;
}
