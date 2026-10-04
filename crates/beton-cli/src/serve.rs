//! `beton serve` (CLI-004): lokaler Daemon auf Loopback.
//!
//! - Konfiguration aus Default, User-Datei und Env (ohne Projektebene).
//! - Genau ein Daemon pro Datenverzeichnis: exklusiver Lock auf `<data_dir>/daemon.lock`.
//! - `<data_dir>/run/daemon.json` nennt Adresse und PID für Clients (SDK, Desktop).
//! - Runner starten als `beton __runner` aus demselben Binary (RUN-002).
//! - Täglicher Blob-GC (DATA-006).
//! - `SIGTERM`/`Strg+C`: Runner benachrichtigen, Listener schließen, Store schließen (AC3).

use std::fs::File;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use beton_harness::registry::HarnessLayers;
use beton_sdk::DaemonInfo;
use beton_server::ServerConfig;
use tokio::task::JoinHandle;

use crate::cli::ServeArgs;
use crate::commands::Ctx;
use crate::exit::{CliError, CliResult, Exit};

/// Erster Blob-GC nach dem Start, danach täglich.
pub const BLOB_GC_FIRST: Duration = Duration::from_secs(10 * 60);
pub const BLOB_GC_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

/// Server-Konfiguration aus Einstellungen und Flags. `--bind` ersetzt die Adressen,
/// `--port` den Port; Nicht-Loopback lehnt `ServerConfig::validate` ab (AUTH-001 AC4).
pub fn server_config(
    home: &Path,
    settings: &crate::config::Settings,
    args: &ServeArgs,
) -> Result<ServerConfig, CliError> {
    let mut listen = settings.server.listen.clone();
    if let Some(ip) = args.bind {
        let port = listen
            .first()
            .map_or(beton_server::DEFAULT_PORT, SocketAddr::port);
        listen = vec![SocketAddr::new(ip, port)];
    }
    if let Some(port) = args.port {
        for addr in &mut listen {
            addr.set_port(port);
        }
    }
    let mut config = ServerConfig::local(home.to_path_buf());
    config.listen = listen;
    config
        .allowed_hosts
        .clone_from(&settings.server.allowed_hosts);
    config
        .allowed_origins
        .clone_from(&settings.auth.ws_allowed_origins);
    config
        .validate()
        .map_err(|e| CliError::new(Exit::Usage, e))?;
    Ok(config)
}

/// Store-Einstellungen aus der Konfiguration (`events.store_raw`, PROTO-001 AC3).
pub fn store_options(settings: &crate::config::Settings) -> beton_store::StoreOptions {
    beton_store::StoreOptions {
        store_raw: settings.events.store_raw,
        ..beton_store::StoreOptions::default()
    }
}

/// Exklusiver Lock auf `<data_dir>/daemon.lock`; frei beim Drop bzw. Prozessende.
#[derive(Debug)]
pub struct DaemonLock {
    _file: File,
    path: PathBuf,
}

impl DaemonLock {
    pub fn acquire(home: &Path) -> Result<Self, CliError> {
        std::fs::create_dir_all(home).with_context(|| home.display().to_string())?;
        let path = home.join("daemon.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| path.display().to_string())?;
        let locked = match fs4::FileExt::try_lock(&file) {
            Ok(()) => true,
            Err(fs4::TryLockError::WouldBlock) => false,
            Err(fs4::TryLockError::Error(e)) => {
                return Err(CliError::new(
                    Exit::General,
                    anyhow::Error::new(e).context(path.display().to_string()),
                ));
            }
        };
        if !locked {
            let running = DaemonInfo::read(home)
                .map(|d| format!(" (PID {}, {})", d.pid, d.http))
                .unwrap_or_default();
            return Err(CliError::new(
                Exit::General,
                anyhow::anyhow!("Für {} läuft bereits ein Daemon{running}", home.display()),
            ));
        }
        Ok(Self { _file: file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Führt `task` zuerst nach `first`, danach alle `period` aus.
pub fn spawn_periodic<F, Fut>(first: Duration, period: Duration, mut task: F) -> JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        tokio::time::sleep(first).await;
        loop {
            task().await;
            tokio::time::sleep(period).await;
        }
    })
}

fn write_daemon_info(home: &Path, daemon: &beton_server::Daemon) -> anyhow::Result<PathBuf> {
    let http = daemon
        .addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| daemon.addrs.first())
        .map(ToString::to_string)
        .unwrap_or_default();
    let info = DaemonInfo {
        pid: std::process::id(),
        http,
        version: crate::VERSION.into(),
    };
    let path = DaemonInfo::path_in(home);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&info)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub async fn serve(ctx: &Ctx, args: ServeArgs) -> CliResult {
    let layers = ctx.layers()?;
    let settings = layers
        .daemon_settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let config = server_config(&ctx.home, &settings, &args)?;
    let dev = args.dev || cfg!(debug_assertions);

    let _log = crate::logging::init(crate::logging::LogConfig {
        stderr_human: !ctx.global.quiet,
        ..crate::logging::LogConfig::for_component(crate::logging::Component::Daemon)
    })
    .map_err(|e| ctx.note(format!("Logging nicht verfügbar: {e:#}")))
    .ok();

    let lock = DaemonLock::acquire(&ctx.home)?;
    let store = beton_store::Store::open(&ctx.home, store_options(&settings))
        .await
        .context("Datenbank öffnen")?;

    let exe = std::env::current_exe().context("eigenes Binary")?;
    let runner_command = vec![exe.display().to_string(), "__runner".to_owned()];
    let runners_dir = ctx.home.join("runners");
    let harnesses_user = settings.harnesses.clone();
    let registry = beton_runner::builtin_registry(
        &HarnessLayers {
            user: harnesses_user.clone(),
            project: Default::default(),
        },
        dev,
    );
    let daemon = beton_server::start_with(config, store.clone(), move |mut r| {
        r.sessions.provider = Arc::new(beton_host::LocalProvider::new(runner_command, runners_dir));
        r.sessions.dev = dev;
        r.sessions.harnesses_user = harnesses_user;
        r.harnesses = registry;
        r
    })
    .await
    .map_err(|e| CliError::new(Exit::General, e))?;
    let info_path = write_daemon_info(&ctx.home, &daemon).context("daemon.json schreiben")?;

    let gc_store = store.clone();
    let gc = spawn_periodic(BLOB_GC_FIRST, BLOB_GC_PERIOD, move || {
        let store = gc_store.clone();
        async move {
            match store.gc_blobs(SystemTime::now()).await {
                Ok(r) => tracing::info!(
                    rows = r.rows_deleted,
                    files = r.files_deleted,
                    "Blob-GC abgeschlossen"
                ),
                Err(e) => tracing::warn!("Blob-GC fehlgeschlagen: {e}"),
            }
        }
    });

    let urls: Vec<String> = daemon.addrs.iter().map(|a| format!("http://{a}")).collect();
    tracing::info!(addrs = ?daemon.addrs, dev, "Daemon gestartet");
    ctx.note(format!(
        "beton {} lauscht auf {} (Strg+C beendet)",
        crate::VERSION,
        urls.join(", ")
    ));

    shutdown_signal().await;
    ctx.note("Beende: Runner werden benachrichtigt …");
    gc.abort();
    daemon.shutdown().await;
    store.close().await;
    let _ = std::fs::remove_file(&info_path);
    tracing::info!(lock = %lock.path().display(), "Daemon beendet");
    drop(lock);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn args(bind: Option<&str>, port: Option<u16>) -> ServeArgs {
        ServeArgs {
            bind: bind.map(|b| b.parse().unwrap()),
            port,
            dev: false,
        }
    }

    #[test]
    fn cli_004_ac1_default_binds_only_loopback() {
        let cfg =
            server_config(Path::new("/tmp/b"), &Settings::default(), &args(None, None)).unwrap();
        assert!(!cfg.listen.is_empty());
        assert!(cfg.listen.iter().all(|a| a.ip().is_loopback()));
        assert!(cfg.listen.iter().all(|a| a.port() == 7420));
    }

    #[test]
    fn cli_004_ac1_bind_all_interfaces_is_refused() {
        for bind in ["0.0.0.0", "::", "192.168.1.20"] {
            let err = server_config(
                Path::new("/tmp/b"),
                &Settings::default(),
                &args(Some(bind), None),
            )
            .unwrap_err();
            assert_eq!(err.exit, Exit::Usage, "{bind}");
            assert!(err.to_string().contains("AUTH-009"), "{err}");
        }
        let ok = server_config(
            Path::new("/tmp/b"),
            &Settings::default(),
            &args(Some("127.0.0.1"), Some(0)),
        )
        .unwrap();
        assert_eq!(ok.listen, vec!["127.0.0.1:0".parse().unwrap()]);
    }

    #[test]
    fn proto_001_ac3_store_raw_setting_reaches_the_store() {
        assert!(store_options(&Settings::default()).store_raw);
        let mut settings = Settings::default();
        settings.events.store_raw = false;
        assert!(!store_options(&settings).store_raw);
    }

    #[test]
    fn second_daemon_for_the_same_data_dir_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let first = DaemonLock::acquire(dir.path()).unwrap();
        let err = DaemonLock::acquire(dir.path()).unwrap_err();
        assert!(err.to_string().contains("läuft bereits"), "{err}");
        drop(first);
        DaemonLock::acquire(dir.path()).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn data_006_blob_gc_runs_after_first_delay_then_periodically() {
        let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = count.clone();
        let handle = spawn_periodic(BLOB_GC_FIRST, BLOB_GC_PERIOD, move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let load = || count.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(BLOB_GC_FIRST - Duration::from_secs(1)).await;
        assert_eq!(load(), 0);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(load(), 1);
        tokio::time::sleep(BLOB_GC_PERIOD).await;
        assert_eq!(load(), 2);
        handle.abort();
    }
}
