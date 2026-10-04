//! Gemeinsame Helfer der CLI-Integrationstests: Binary aufrufen, Daemon im Vordergrund
//! starten, Fake-CLI finden.

#![allow(dead_code, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use beton_sdk::{Client, DaemonInfo};
use serde_json::json;

pub fn beton(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_beton"));
    c.env("BETON_HOME", home)
        .env("NO_COLOR", "1")
        .env_remove("BETON_SERVER")
        .env_remove("BETON_TOKEN")
        .env_remove("BETON_LOG")
        .stdin(Stdio::null());
    for (k, _) in std::env::vars() {
        if k.starts_with("BETON_CFG_") {
            c.env_remove(k);
        }
    }
    c
}

pub fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("beton starten")
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub struct Serve {
    pub child: Option<Child>,
    pub home: tempfile::TempDir,
    pub work: tempfile::TempDir,
    pub info: DaemonInfo,
}

impl Serve {
    pub fn start() -> Self {
        Self::start_with(&[])
    }

    pub fn start_with(env: &[(&str, &str)]) -> Self {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let log = std::fs::File::create(work.path().join("serve.log")).unwrap();
        let mut cmd = beton(home.path());
        cmd.args(["serve", "--foreground", "--port", "0", "--dev"])
            .current_dir(work.path())
            .stdout(Stdio::null())
            .stderr(log);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let info = loop {
            if let Some(info) = DaemonInfo::read(home.path()) {
                break info;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "serve endete mit {status}: {}",
                    std::fs::read_to_string(work.path().join("serve.log")).unwrap_or_default()
                );
            }
            assert!(Instant::now() < deadline, "daemon.json erscheint nicht");
            std::thread::sleep(Duration::from_millis(50));
        };
        Self {
            child: Some(child),
            home,
            work,
            info,
        }
    }

    pub fn home(&self) -> &Path {
        self.home.path()
    }

    pub fn client(&self) -> Client {
        Client::local(self.home()).unwrap()
    }

    pub fn scenario(&self, yaml: &str) -> PathBuf {
        let p = self.work.path().join(format!("{}.yaml", ulid_like()));
        std::fs::write(&p, yaml).unwrap();
        p
    }

    pub async fn create_idle_session(&self) -> String {
        let scenario = self.scenario("turns: []");
        let created = self
            .client()
            .create_session(&json!({
                "target": "fake",
                "cwd": self.work.path(),
                "harness_opts": {"scenario": scenario},
            }))
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let sessions = self.client().all_sessions(false).await.unwrap();
            if sessions
                .iter()
                .any(|s| s["id"] == id.as_str() && s["status"] == "idle")
            {
                return id;
            }
            assert!(
                Instant::now() < deadline,
                "Session wird nicht idle: {sessions:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Sendet SIGTERM und wartet auf das Ende.
    #[cfg(unix)]
    pub fn terminate(&mut self) -> std::process::ExitStatus {
        let mut child = self.child.take().unwrap();
        let status = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "serve endet nicht nach SIGTERM");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.work.path().join("serve.log")).unwrap_or_default()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn ulid_like() -> String {
    beton_core::id::RunnerId::new().to_string()
}

pub async fn open_store(home: &Path) -> beton_store::Store {
    beton_store::Store::open(home, beton_store::StoreOptions::default())
        .await
        .unwrap()
}

/// Pfad der Fake-Vendor-CLI (QA-002); baut sie bei Bedarf einmal.
pub fn fake_cli() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let beton = PathBuf::from(env!("CARGO_BIN_EXE_beton"));
        let name = format!("beton-fake-cli{}", std::env::consts::EXE_SUFFIX);
        let path = beton.with_file_name(&name);
        if !path.is_file() {
            let status = Command::new(env!("CARGO"))
                .args(["build", "-q", "-p", "beton-fake-cli"])
                .status()
                .unwrap();
            assert!(status.success(), "beton-fake-cli bauen");
        }
        path
    })
    .clone()
}

/// Wert für `BETON_CLAUDE_PATH`, der die Fake-CLI mit einem Szenario startet.
pub fn fake_claude(scenario: &Path) -> String {
    format!(
        "{} --protocol stream-json --scenario {}",
        fake_cli().display(),
        scenario.display()
    )
}

/// Datenverzeichnis für einen Daemon, den die CLI selbst im Hintergrund startet: freier Port
/// statt 7420, damit parallele Tests sich nicht stören.
pub fn background_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.yaml"),
        "server:\n  listen: [\"127.0.0.1:0\"]\n",
    )
    .unwrap();
    home
}

/// Beendet einen im Hintergrund gestarteten Daemon (aus `run/daemon.json`) beim Drop.
pub struct BackgroundDaemon(pub PathBuf);

impl BackgroundDaemon {
    pub fn pid(&self) -> Option<u32> {
        DaemonInfo::read(&self.0).map(|i| i.pid)
    }
}

impl Drop for BackgroundDaemon {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            let deadline = Instant::now() + Duration::from_secs(10);
            while DaemonInfo::read(&self.0).is_some() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Wartet, bis `check` wahr ist.
pub async fn wait_until<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check().await {
        assert!(Instant::now() < deadline, "Zeitüberschreitung: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Status einer Session über das SDK.
pub async fn status(client: &Client, id: &str) -> String {
    client.session(id).await.unwrap()["status"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Dauerhafte Events einer Session.
pub async fn events(client: &Client, id: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut after = 0;
    loop {
        let page = client.events(id, after, 200).await.unwrap();
        if page.items.is_empty() {
            return out;
        }
        after = page.items.last().unwrap()["seq"].as_u64().unwrap();
        out.extend(page.items);
    }
}
