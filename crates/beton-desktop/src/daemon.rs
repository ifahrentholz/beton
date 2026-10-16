//! Lokaler Daemon als Sidecar (DESK-002).
//!
//! - Die App bringt dasselbe `beton`-Binary mit (neben dem App-Binary). Läuft für das
//!   Datenverzeichnis schon ein Daemon (z. B. aus `beton serve`), nutzt sie ihn; sonst startet
//!   sie ihn über `beton serve --json` (abgelöst, eigene Prozessgruppe). Den Lock auf
//!   `daemon.lock` hält immer der Daemon selbst, nie die App (AC2).
//! - Kompatibel ist ein Daemon mit derselben Protokoll-Hauptversion und höchstens einer
//!   Minor-Version Abstand zur App (AC3, PROTO-004); sonst liefert [`Manager::connect`]
//!   [`DaemonError::Incompatible`] mit beiden Versionen für den Dialog mit Upgrade-Option.
//! - Der Daemon läuft unabhängig von der App: Fenster schließen oder die App beenden beendet
//!   keine Sessions (AC4). Gestoppt wird er nur auf ausdrücklichen Wunsch ([`Manager::stop`]).
//! - Das lokale Token liest das SDK aus `auth/local.token`; es erscheint weder in Logs noch in
//!   URLs. Die WebView meldet sich mit einem Einmal-Code an (siehe [`Manager::login_url`]).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use beton_sdk::{Client, DaemonInfo};
use serde::Deserialize;
use serde_json::Value;

/// Protokoll-Hauptversion, die diese App spricht (`/v1/info.protocol_version`).
pub const APP_PROTOCOL: u32 = 1;

/// So lange darf der Start des Daemons dauern, bevor die App aufgibt (Ziel: 3 s, AC1).
pub const START_TIMEOUT: Duration = Duration::from_secs(20);

/// So lange wartet [`Manager::stop`], bis der Daemon nicht mehr antwortet.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(15);

/// Das mitgelieferte `beton`-Binary und die gebündelte Web-UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    pub binary: PathBuf,
    /// Verzeichnis mit `index.html`; der Daemon liefert es als Web-UI aus (`BETON_WEB_DIR`).
    pub web_dir: Option<PathBuf>,
}

impl Sidecar {
    /// Name des Binarys auf dieser Plattform.
    pub fn binary_name() -> &'static str {
        if cfg!(windows) { "beton.exe" } else { "beton" }
    }

    /// `beton` neben dem App-Binary (Bundle: `Contents/MacOS`, deb/AppImage: `usr/bin`,
    /// Entwicklung: `target/<profil>`), Web-UI unter `<resources>/web`.
    pub fn locate(app_exe: &Path, resources: Option<&Path>) -> Option<Self> {
        let binary = app_exe.parent()?.join(Self::binary_name());
        if !binary.is_file() {
            return None;
        }
        let web_dir = resources
            .map(|r| r.join("web"))
            .filter(|d| d.join("index.html").is_file());
        Some(Self { binary, web_dir })
    }
}

/// Versionen von App und Daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    pub app: String,
    pub app_protocol: u32,
    pub daemon: String,
    pub daemon_protocol: u32,
}

/// Passt der laufende Daemon zur App? Gleiche Protokoll-Hauptversion, gleiche Hauptversion
/// und höchstens eine Minor-Version Abstand (DESK-002 AC3). Unlesbare Versionen gelten als
/// inkompatibel (fail closed).
pub fn compatible(v: &Versions) -> bool {
    if v.app_protocol != v.daemon_protocol {
        return false;
    }
    let (Ok(app), Ok(daemon)) = (
        semver::Version::parse(&v.app),
        semver::Version::parse(&v.daemon),
    ) else {
        return false;
    };
    app.major == daemon.major && app.minor.abs_diff(daemon.minor) <= 1
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("Das mitgelieferte beton-Binary fehlt neben der App")]
    NoSidecar,
    #[error("Der laufende Daemon passt nicht zur App (Daemon {}, App {})", .0.daemon, .0.app)]
    Incompatible(Versions),
    #[error("Daemon-Start fehlgeschlagen: {0}")]
    Start(String),
    #[error("Der Daemon antwortet nicht binnen {} s", .0.as_secs())]
    Timeout(Duration),
    #[error("Daemon nicht erreichbar: {0}")]
    Unreachable(String),
    #[error("{0}")]
    Unsupported(&'static str),
}

/// Wie die App mit dem Daemon verbunden ist.
#[derive(Debug, Clone)]
pub struct Connection {
    pub client: Client,
    pub status: DaemonStatus,
}

/// Was Tray und Menü über den Daemon zeigen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// z. B. `http://127.0.0.1:7420`.
    pub base_url: String,
    pub pid: Option<u32>,
    pub version: String,
    /// Die App hat ihn gestartet (sonst lief er schon, etwa aus `beton serve`).
    pub started_by_app: bool,
    /// Ungefährer Startzeitpunkt (`run/daemon.json`).
    pub since: Option<SystemTime>,
}

/// Fortschritt für den Startbildschirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Mitgeliefertes Binary gefunden.
    SidecarFound { version: String },
    /// Kein Daemon erreichbar, ein neuer startet.
    Starting,
    /// Verbunden; `started` = von der App gestartet.
    Connected { started: bool },
}

#[derive(Debug, Deserialize)]
struct ServeOutput {
    url: String,
    #[serde(default)]
    pid: Option<u32>,
}

/// Startet, findet und stoppt den lokalen Daemon eines Datenverzeichnisses.
#[derive(Debug, Clone)]
pub struct Manager {
    home: PathBuf,
    sidecar: Option<Sidecar>,
    app_version: String,
    /// Zusätzliche Argumente für `beton serve` (Tests: `--port 0`).
    pub serve_args: Vec<String>,
    /// Zusätzliche Umgebung für den Daemon (Tests: Fake-CLIs).
    pub serve_env: Vec<(String, String)>,
}

impl Manager {
    pub fn new(home: PathBuf, sidecar: Option<Sidecar>, app_version: impl Into<String>) -> Self {
        Self {
            home,
            sidecar,
            app_version: app_version.into(),
            serve_args: Vec::new(),
            serve_env: Vec::new(),
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn sidecar(&self) -> Option<&Sidecar> {
        self.sidecar.as_ref()
    }

    /// Ein erreichbarer Daemon für das Datenverzeichnis samt `/v1/info`.
    pub async fn probe(&self) -> Option<(Client, Value)> {
        let client = Client::local(&self.home).ok()?;
        let info = client.info().await.ok()?;
        Some((client, info))
    }

    fn status(&self, client: &Client, info: &Value, started: bool) -> DaemonStatus {
        let daemon = DaemonInfo::read(&self.home);
        let since = std::fs::metadata(DaemonInfo::path_in(&self.home))
            .and_then(|m| m.modified())
            .ok();
        DaemonStatus {
            base_url: client.base_url().to_owned(),
            pid: daemon.map(|d| d.pid),
            version: info["version"].as_str().unwrap_or_default().to_owned(),
            started_by_app: started,
            since,
        }
    }

    fn check(&self, info: &Value) -> Result<(), DaemonError> {
        let versions = Versions {
            app: self.app_version.clone(),
            app_protocol: APP_PROTOCOL,
            daemon: info["version"].as_str().unwrap_or_default().to_owned(),
            daemon_protocol: info["protocol_version"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
        };
        if compatible(&versions) {
            Ok(())
        } else {
            Err(DaemonError::Incompatible(versions))
        }
    }

    /// Verbindet mit dem laufenden Daemon oder startet einen (DESK-002 AC1, AC2).
    pub async fn connect(&self, progress: impl Fn(Step)) -> Result<Connection, DaemonError> {
        if self.sidecar.is_some() {
            progress(Step::SidecarFound {
                version: self.app_version.clone(),
            });
        }
        if let Some((client, info)) = self.probe().await {
            self.check(&info)?;
            progress(Step::Connected { started: false });
            let status = self.status(&client, &info, false);
            return Ok(Connection { client, status });
        }
        progress(Step::Starting);
        let (client, info) = self.start().await?;
        self.check(&info)?;
        progress(Step::Connected { started: true });
        let status = self.status(&client, &info, true);
        Ok(Connection { client, status })
    }

    /// Startet den mitgelieferten Daemon und wartet, bis er antwortet.
    #[expect(
        clippy::disallowed_methods,
        reason = "Stufe 0: Start des eigenen Daemons (Sidecar der Desktop-App)"
    )]
    async fn start(&self) -> Result<(Client, Value), DaemonError> {
        let sidecar = self.sidecar.as_ref().ok_or(DaemonError::NoSidecar)?;
        let mut cmd = tokio::process::Command::new(&sidecar.binary);
        cmd.arg("serve")
            .arg("--json")
            .arg("-q")
            .args(&self.serve_args)
            .env("BETON_HOME", &self.home)
            .current_dir(&self.home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(web) = &sidecar.web_dir {
            cmd.env("BETON_WEB_DIR", web);
        }
        for (k, v) in &self.serve_env {
            cmd.env(k, v);
        }
        std::fs::create_dir_all(&self.home).map_err(|e| DaemonError::Start(e.to_string()))?;
        let started = Instant::now();
        let out = tokio::time::timeout(START_TIMEOUT, cmd.output())
            .await
            .map_err(|_| DaemonError::Timeout(START_TIMEOUT))?
            .map_err(|e| DaemonError::Start(e.to_string()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(DaemonError::Start(format!("{} {last}", out.status)));
        }
        let parsed: ServeOutput = serde_json::from_slice(&out.stdout)
            .map_err(|e| DaemonError::Start(format!("Ausgabe von beton serve: {e}")))?;
        tracing::info!(
            url = %parsed.url,
            pid = ?parsed.pid,
            ms = started.elapsed().as_millis() as u64,
            "Daemon gestartet"
        );
        self.probe()
            .await
            .ok_or(DaemonError::Unreachable(parsed.url))
    }

    /// Stoppt den Daemon (`SIGTERM`) und wartet, bis er nicht mehr antwortet. Laufende Turns
    /// werden abgebrochen; der Verlauf bleibt gespeichert.
    pub async fn stop(&self) -> Result<(), DaemonError> {
        let Some(_) = self.probe().await else {
            return Ok(());
        };
        let info = DaemonInfo::read(&self.home)
            .ok_or_else(|| DaemonError::Unreachable("run/daemon.json fehlt".into()))?;
        terminate(info.pid)?;
        let deadline = Instant::now() + STOP_TIMEOUT;
        while self.probe().await.is_some() {
            if Instant::now() > deadline {
                return Err(DaemonError::Timeout(STOP_TIMEOUT));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Stoppt den laufenden Daemon und startet den mitgelieferten (Neustart bzw. Upgrade).
    pub async fn restart(&self, progress: impl Fn(Step)) -> Result<Connection, DaemonError> {
        self.stop().await?;
        self.connect(progress).await
    }

    /// Einmal-Link für die WebView (AUTH-004): 60 s gültig, einmal einlösbar, leitet ohne
    /// Code auf `next` weiter. Das lokale Token selbst steht nie in einer URL.
    pub async fn login_url(client: &Client, next: &str) -> Result<String, DaemonError> {
        let code = client
            .create_login_code()
            .await
            .map_err(|e| DaemonError::Unreachable(e.to_string()))?;
        Ok(format!(
            "{}/auth/local/redeem?code={}&next={}",
            client.base_url(),
            code.code,
            encode_component(next)
        ))
    }
}

/// Prozent-Kodierung eines Query-Werts (nur unreservierte Zeichen bleiben).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<(), DaemonError> {
    let raw = i32::try_from(pid).map_err(|_| DaemonError::Unreachable(format!("PID {pid}")))?;
    let pid = rustix::process::Pid::from_raw(raw)
        .ok_or_else(|| DaemonError::Unreachable(format!("PID {raw}")))?;
    rustix::process::kill_process(pid, rustix::process::Signal::TERM)
        .map_err(|e| DaemonError::Unreachable(format!("Signal an PID {raw}: {e}")))
}

#[cfg(not(unix))]
fn terminate(_pid: u32) -> Result<(), DaemonError> {
    Err(DaemonError::Unsupported(
        "Den Daemon zu stoppen geht unter Windows (Beta) noch nicht; beende ihn mit `beton serve` im Terminal.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(app: &str, daemon: &str) -> Versions {
        Versions {
            app: app.into(),
            app_protocol: 1,
            daemon: daemon.into(),
            daemon_protocol: 1,
        }
    }

    #[test]
    fn desk_002_ac3_more_than_one_minor_apart_is_incompatible() {
        assert!(compatible(&v("0.3.2", "0.3.0")));
        assert!(compatible(&v("0.3.2", "0.2.9")));
        assert!(compatible(&v("0.3.2", "0.4.0")));
        assert!(!compatible(&v("0.3.2", "0.1.0")), "zwei Minor-Versionen");
        assert!(!compatible(&v("1.0.0", "0.9.0")), "andere Hauptversion");
        assert!(!compatible(&v("0.3.2", "kaputt")), "unlesbar: fail closed");
        let mut other = v("0.3.2", "0.3.2");
        other.daemon_protocol = 2;
        assert!(!compatible(&other), "andere Protokoll-Hauptversion");
    }

    #[test]
    fn desk_002_sidecar_is_the_beton_binary_next_to_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("beton-desktop");
        std::fs::write(&app, "").unwrap();
        assert_eq!(Sidecar::locate(&app, None), None);
        std::fs::write(dir.path().join(Sidecar::binary_name()), "").unwrap();
        let res = dir.path().join("res");
        std::fs::create_dir_all(res.join("web")).unwrap();
        assert_eq!(Sidecar::locate(&app, Some(&res)).unwrap().web_dir, None);
        std::fs::write(res.join("web/index.html"), "").unwrap();
        let s = Sidecar::locate(&app, Some(&res)).unwrap();
        assert_eq!(s.binary, dir.path().join(Sidecar::binary_name()));
        assert_eq!(s.web_dir, Some(res.join("web")));
    }

    #[test]
    fn desk_002_login_link_carries_only_the_one_time_code() {
        assert_eq!(
            encode_component("/s/ses_1?seq=2&opened=link"),
            "%2Fs%2Fses_1%3Fseq%3D2%26opened%3Dlink"
        );
    }
}
