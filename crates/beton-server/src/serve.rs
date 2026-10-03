//! Start des lokalen Daemons: TCP nur auf Loopback, dazu ein Unix-Socket mit 0600
//! (AUTH-001).

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use beton_store::Store;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::app::{self, AppParts};
use crate::config::{ConfigError, ServerConfig};
use crate::local_auth::{AuthError, BrowserLogins, LocalToken};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Store(#[from] beton_store::Error),
    #[error("{addr} konnte nicht gebunden werden: {source}")]
    Bind { addr: String, source: io::Error },
}

/// Ein laufender Daemon.
pub struct Daemon {
    pub addrs: Vec<SocketAddr>,
    pub socket: Option<PathBuf>,
    pub token: Arc<LocalToken>,
    pub runtime: crate::app::Runtime,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Daemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Daemon")
            .field("addrs", &self.addrs)
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl Daemon {
    /// Beendet alle Listener und wartet auf laufende Requests.
    pub async fn shutdown(self) {
        crate::sessions::shutdown_all(&self.runtime.sessions, &self.runtime.runners).await;
        let _ = self.stop.send(true);
        for t in self.tasks {
            let _ = t.await;
        }
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// Bindet alle Listener und startet den Server.
pub async fn start(config: ServerConfig, store: Store) -> Result<Daemon, StartError> {
    start_with(config, store, |r| r).await
}

/// Wie [`start`], mit anpassbarer Laufzeit (Kommandos, WebSocket-Grenzen).
pub async fn start_with(
    config: ServerConfig,
    store: Store,
    customize: impl FnOnce(crate::app::Runtime) -> crate::app::Runtime,
) -> Result<Daemon, StartError> {
    config.validate()?;
    let token = Arc::new(LocalToken::load_or_create(&config.data_dir)?);
    let local = store.ensure_local().await?;

    let mut listeners = Vec::new();
    let mut last_err = None;
    for addr in &config.listen {
        match TcpListener::bind(addr).await {
            Ok(l) => listeners.push(l),
            // IPv6-Loopback fehlt auf manchen Systemen; IPv4 reicht dann.
            Err(e) if addr.is_ipv6() => {
                tracing::warn!(%addr, "IPv6-Loopback nicht verfügbar: {e}");
                last_err = Some((addr.to_string(), e));
            }
            Err(source) => {
                return Err(StartError::Bind {
                    addr: addr.to_string(),
                    source,
                });
            }
        }
    }
    if listeners.is_empty()
        && config.socket.is_none()
        && let Some((addr, source)) = last_err
    {
        return Err(StartError::Bind { addr, source });
    }
    let addrs: Vec<SocketAddr> = listeners
        .iter()
        .filter_map(|l| l.local_addr().ok())
        .collect();
    let mut ports: Vec<u16> = addrs.iter().map(SocketAddr::port).collect();
    ports.dedup();
    let primary_host = addrs
        .first()
        .map(|a| format!("127.0.0.1:{}", a.port()))
        .unwrap_or_else(|| "localhost".into());

    let (stop, stop_rx) = watch::channel(false);
    let runtime = customize(
        crate::app::Runtime::for_config(&config, stop_rx.clone()).with_default_commands(),
    );
    let store_for_tunnel = store.clone();
    let router = app::build(AppParts {
        store,
        local,
        token: token.clone(),
        logins: Arc::new(BrowserLogins::default()),
        hosts: config.host_allowlist(&ports),
        origins: config.origin_allowlist(&ports),
        primary_host,
        runtime: runtime.clone(),
    });

    let mut tasks = Vec::new();
    for listener in listeners {
        let app = router
            .clone()
            .into_make_service_with_connect_info::<SocketAddr>();
        let mut rx = stop_rx.clone();
        tasks.push(tokio::spawn(async move {
            let shutdown = async move {
                let _ = rx.wait_for(|s| *s).await;
            };
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                tracing::error!("HTTP-Listener beendet: {e}");
            }
        }));
    }
    #[cfg(unix)]
    if let Some(path) = &config.tunnel_socket {
        let tstate = crate::tunnel::TunnelState {
            events: crate::hub::EventService {
                store: store_for_tunnel.clone(),
                hub: runtime.hub.clone(),
            },
            local,
            runners: runtime.runners.clone(),
            config: runtime.tunnel,
        };
        tasks.push(tokio::spawn(crate::tunnel::idle_reaper(
            tstate.clone(),
            runtime.hub.clone(),
            stop_rx.clone(),
        )));
        let tunnel = Router::new()
            .route(
                crate::tunnel::TUNNEL_PATH,
                axum::routing::get(crate::tunnel::tunnel_upgrade),
            )
            .with_state(tstate);
        tasks.push(serve_unix(path, tunnel, stop_rx.clone())?);
    }
    let socket = match &config.socket {
        Some(path) => {
            #[cfg(unix)]
            let router = router.layer(axum::Extension(crate::security::ViaSocket));
            tasks.push(serve_unix(path, router, stop_rx)?);
            Some(path.clone())
        }
        None => None,
    };
    tracing::info!(?addrs, ?socket, "beton-server gestartet");
    Ok(Daemon {
        addrs,
        socket,
        token,
        runtime,
        stop,
        tasks,
    })
}

#[cfg(unix)]
fn serve_unix(
    path: &std::path::Path,
    app: Router,
    mut stop: watch::Receiver<bool>,
) -> Result<JoinHandle<()>, StartError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let bind_err = |source| StartError::Bind {
        addr: path.display().to_string(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(bind_err)?;
        crate::local_auth::check_private(dir, 0o700)?;
    }
    // Veralteten Socket eines abgestürzten Daemons entfernen.
    if path.exists() {
        std::fs::remove_file(path).map_err(bind_err)?;
    }
    let listener = tokio::net::UnixListener::bind(path).map_err(bind_err)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(bind_err)?;
    Ok(tokio::spawn(async move {
        let shutdown = async move {
            let _ = stop.wait_for(|s| *s).await;
        };
        if let Err(e) = axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!("Socket-Listener beendet: {e}");
        }
    }))
}

#[cfg(not(unix))]
fn serve_unix(
    _path: &std::path::Path,
    _router: Router,
    _stop: watch::Receiver<bool>,
) -> Result<JoinHandle<()>, StartError> {
    Ok(tokio::spawn(async {}))
}
