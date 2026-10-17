//! Daemon als Sidecar der Desktop-App (DESK-002) mit dem echten `beton`-Binary: Start ohne
//! laufenden Daemon, Nutzung eines vorhandenen aus `beton serve`, Sessions laufen ohne App
//! weiter. Datenverzeichnis, Port und Vendor-CLIs sind isoliert (temporär, Port 0, keine echte
//! CLI).

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use beton_desktop::daemon::{Manager, Sidecar, Step};
use beton_sdk::DaemonInfo;
use common::{NO_REAL_VENDOR_CLI, Serve, UNSANDBOXED_CONFIG};
use serde_json::json;

fn manager(home: &Path) -> Manager {
    let mut m = Manager::new(
        home.to_path_buf(),
        Some(Sidecar {
            binary: PathBuf::from(env!("CARGO_BIN_EXE_beton")),
            web_dir: None,
        }),
        env!("CARGO_PKG_VERSION"),
    );
    m.serve_args = vec!["--port".into(), "0".into()];
    m.serve_env = vec![
        ("BETON_CLAUDE_PATH".into(), NO_REAL_VENDOR_CLI.into()),
        ("BETON_CODEX_PATH".into(), NO_REAL_VENDOR_CLI.into()),
    ];
    m
}

/// Beendet den von der App gestarteten Daemon am Testende.
struct Stop(PathBuf);

impl Drop for Stop {
    fn drop(&mut self) {
        if let Some(info) = DaemonInfo::read(&self.0)
            && let Some(pid) = rustix::process::Pid::from_raw(info.pid as i32)
        {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
    }
}

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.yaml"), UNSANDBOXED_CONFIG).unwrap();
    home
}

fn alive(pid: u32) -> bool {
    rustix::process::Pid::from_raw(pid as i32)
        .is_some_and(|p| rustix::process::test_kill_process(p).is_ok())
}

#[tokio::test]
async fn desk_002_ac1_without_daemon_the_app_starts_one_within_three_seconds() {
    let home = home();
    let _stop = Stop(home.path().to_path_buf());
    let m = manager(home.path());
    // Erster Start eines frisch gebauten Binarys: macOS prüft es einmal (XProtect); das
    // gehört nicht zum Start des Daemons.
    assert!(
        common::run(common::beton(home.path()).arg("--version"))
            .status
            .success()
    );
    let steps = Mutex::new(Vec::new());
    let started = Instant::now();
    let conn = m.connect(|s| steps.lock().unwrap().push(s)).await.unwrap();
    let took = started.elapsed();
    assert!(took <= Duration::from_secs(3), "Start dauerte {took:?}");
    assert!(conn.status.started_by_app);
    assert_eq!(
        steps.lock().unwrap().last(),
        Some(&Step::Connected { started: true }),
        "zeigt „verbunden“"
    );
    assert!(steps.lock().unwrap().contains(&Step::Starting));
    // Der Daemon antwortet mit dem lokalen Token aus dem Datenverzeichnis.
    let info = conn.client.info().await.unwrap();
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    // Die WebView meldet sich mit einem Einmal-Code an, das Token steht in keiner URL.
    let url = Manager::login_url(&conn.client, "/s/ses_x").await.unwrap();
    let token = std::fs::read_to_string(home.path().join("auth/local.token")).unwrap();
    assert!(!url.contains(token.trim()), "{url}");
    assert!(url.contains("/auth/local/redeem?code="), "{url}");
    m.stop().await.unwrap();
    assert!(m.probe().await.is_none(), "Stoppen beendet den Daemon");
}

#[tokio::test]
async fn desk_002_ac2_running_beton_serve_is_used_and_no_second_daemon_starts() {
    let serve = Serve::start();
    let m = manager(serve.home());
    let steps = Mutex::new(Vec::new());
    let conn = m.connect(|s| steps.lock().unwrap().push(s)).await.unwrap();
    assert!(!conn.status.started_by_app);
    assert!(!steps.lock().unwrap().contains(&Step::Starting));
    assert_eq!(conn.status.base_url, format!("http://{}", serve.info.http));
    let serve_pid = serve.child.as_ref().unwrap().id();
    assert_eq!(conn.status.pid, Some(serve_pid));
    assert_eq!(DaemonInfo::read(serve.home()).unwrap().pid, serve_pid);
    // Nur `beton serve` hält den Lock.
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .open(serve.home().join("daemon.lock"))
        .unwrap();
    assert!(
        matches!(
            fs4::FileExt::try_lock(&lock),
            Err(fs4::TryLockError::WouldBlock)
        ),
        "daemon.lock gehört dem laufenden Daemon"
    );
}

#[tokio::test]
async fn desk_002_ac4_closing_the_app_does_not_end_running_sessions() {
    let home = home();
    let _stop = Stop(home.path().to_path_buf());
    let work = tempfile::tempdir().unwrap();
    let scenario = work.path().join("hang.yaml");
    std::fs::write(
        &scenario,
        "turns:\n  - expect_input: \"los\"\n    emit:\n      - { message_delta: \"arbeitet \", chunk: 5 }\n      - { hang: true }\n",
    )
    .unwrap();
    let pid;
    let id;
    {
        // „App“: verbindet bzw. startet den Daemon, startet einen Turn und schließt dann.
        let m = manager(home.path());
        let conn = m.connect(|_| {}).await.unwrap();
        pid = conn.status.pid.unwrap();
        let created = conn
            .client
            .create_session(&json!({
                "target": "fake",
                "cwd": work.path(),
                "harness_opts": {"scenario": scenario},
                "confirm_unsandboxed": true,
            }))
            .await
            .unwrap();
        id = created["id"].as_str().unwrap().to_owned();
        let deadline = Instant::now() + Duration::from_secs(30);
        while conn.client.session(&id).await.unwrap()["status"] != "idle" {
            assert!(Instant::now() < deadline, "Session wird nicht idle");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        conn.client.input(&id, "los").await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while conn.client.session(&id).await.unwrap()["status"] != "running" {
            assert!(Instant::now() < deadline, "Turn startet nicht");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alive(pid), "Daemon läuft nach dem Schließen weiter");
    let again = manager(home.path())
        .probe()
        .await
        .expect("Daemon erreichbar")
        .0;
    assert_eq!(
        again.session(&id).await.unwrap()["status"],
        "running",
        "der Turn läuft weiter"
    );
}
