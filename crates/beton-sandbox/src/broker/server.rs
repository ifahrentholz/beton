//! Der Broker im Runner (Stufe 0): nimmt Anfragen von `bt-exec` an, prüft sie gegen die
//! Stufe-2-Policy und startet jedes Kommando in einer frischen Stufe-2-Sandbox (SBX-002).

use std::os::fd::AsRawFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use super::fdpass;
use super::proto::{ClientMsg, MAX_FRAME, Op, Reply, Request, VERSION};
use crate::command::{CommandSpec, EnvSpec, Stdio};
use crate::registry::Sandbox;
use crate::resolve::{ExecAccess, Mask, Secret};

/// Frist für die Anfrage nach dem Verbindungsaufbau.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Signale, die `bt-exec` weiterreichen darf.
const SIGNALS: [i32; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// Prüfung des Runners vor jedem `git` der Stufe 1 (SBX-003, #311): `Err` mit Begründung,
/// wenn eine Git-Steuerdatei entstanden ist oder sich geändert hat.
pub type GitCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Ein laufender Broker; endet mit `Drop` (Socket wird entfernt, laufende Kommandos enden mit
/// ihren Verbindungen).
pub struct Broker {
    socket: PathBuf,
    token: Secret,
    forward: Vec<String>,
    started: Arc<std::sync::atomic::AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.socket);
    }
}

struct Shared {
    tools: Sandbox,
    token: Secret,
    /// Konfigurierte Namen der Stufe 2, die Stufe 1 überschreiben darf (#259).
    forward: Vec<String>,
    started: Arc<std::sync::atomic::AtomicU64>,
    /// Verzeichnis des Brokers (Socket, Shims der Stufe 1), wie übergeben und kanonisch;
    /// gehört nie in den `PATH` der Stufe 2.
    dirs: [PathBuf; 2],
    /// Prüfung vor jedem `git` der Stufe 1 (SBX-003, #311); ohne sie lehnt der Broker die
    /// Vorabprüfung ab (fail closed).
    git_check: Option<GitCheck>,
}

fn token() -> std::io::Result<Secret> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| std::io::Error::other(e.to_string()))?;
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    raw.fill(0);
    Ok(Secret::new(hex))
}

impl Broker {
    /// Startet den Broker mit dem Socket `dir/exec.sock` (Verzeichnis 0700, Socket 0600) für
    /// die Stufe-2-Sandbox `tools`.
    pub async fn start(dir: &Path, tools: Sandbox) -> std::io::Result<Self> {
        Self::start_inner(dir, tools, None).await
    }

    /// Wie [`Broker::start`], beantwortet zusätzlich [`Op::GitPreflight`] des Git-Shims der
    /// Stufe 1 mit `check` (SBX-003, #311).
    pub async fn start_with_git_check(
        dir: &Path,
        tools: Sandbox,
        check: GitCheck,
    ) -> std::io::Result<Self> {
        Self::start_inner(dir, tools, Some(check)).await
    }

    async fn start_inner(
        dir: &Path,
        tools: Sandbox,
        git_check: Option<GitCheck>,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let socket = dir.join("exec.sock");
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let token = token()?;
        let started = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let forward = super::forward_names(tools.policy());
        let shared = Arc::new(Shared {
            tools,
            token: token.clone(),
            forward: forward.clone(),
            started: started.clone(),
            dirs: [dir.to_path_buf(), dir.canonicalize()?],
            git_check,
        });
        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((conn, _)) => {
                        let shared = shared.clone();
                        tokio::spawn(async move { serve(conn, shared).await });
                    }
                    Err(e) => {
                        tracing::warn!("Exec-Broker: accept: {e}");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }
        });
        Ok(Self {
            socket,
            token,
            forward,
            started,
            task,
        })
    }

    /// Kommandos, die der Broker bisher in Stufe 2 gestartet hat (Integritätsprüfung des
    /// Runners, SBX-002).
    pub fn started(&self) -> u64 {
        self.started.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Der Zähler hinter [`Broker::started`] (für Zeitpunkte der Gate-Entscheidungen, #270).
    pub fn start_counter(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.started.clone()
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn token(&self) -> &Secret {
        &self.token
    }

    /// Zugang für die Auflösung der Stufe 1 ([`crate::ResolveCtx::exec`]).
    pub fn access(&self, shim_dir: PathBuf) -> ExecAccess {
        ExecAccess {
            socket: self.socket.clone(),
            shim_dir,
            token: self.token.clone(),
            forward_env: self.forward.clone(),
        }
    }
}

async fn read_frame<T: for<'de> serde::Deserialize<'de>>(
    r: &mut (impl tokio::io::AsyncRead + Unpin),
) -> std::io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("Rahmen zu groß"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

async fn write_frame(
    w: &mut (impl tokio::io::AsyncWrite + Unpin),
    msg: &Reply,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

fn reject(code: &str, message: impl Into<String>) -> Reply {
    Reply::Error {
        code: code.into(),
        message: message.into(),
    }
}

/// Token der Session und Protokollversion (jede Anfrage).
fn authorize(req: &Request, shared: &Shared) -> Result<(), Reply> {
    if !super::same(req.token.as_bytes(), shared.token.expose().as_bytes()) {
        return Err(reject(
            "unauthorized",
            "Token des Exec-Brokers fehlt oder ist falsch",
        ));
    }
    if req.v != VERSION {
        return Err(reject(
            "invalid_request",
            format!("Protokollversion {} (erwartet {VERSION})", req.v),
        ));
    }
    Ok(())
}

/// Vorabprüfung des Git-Shims (SBX-003, #311): startet nichts.
fn git_preflight(req: &Request, shared: &Shared) -> Reply {
    if let Err(reply) = authorize(req, shared) {
        return reply;
    }
    match &shared.git_check {
        None => reject(
            "unsupported",
            "keine Prüfung der Git-Steuerdateien in dieser Session",
        ),
        Some(check) => match check() {
            Ok(()) => Reply::Allowed,
            Err(why) => reject("git_control_changed", why),
        },
    }
}

/// `PATH` für Stufe 2 ohne Einträge im Verzeichnis des Brokers (Shims der Stufe 1): Stufe 2
/// sieht es nicht, und `execvp` bräche an einem nicht lesbaren Eintrag ab, statt weiter zu
/// suchen. `None`: nichts übrig, Stufe 2 behält ihren eigenen `PATH`.
fn stage_two_path(path: &str, dirs: &[PathBuf]) -> Option<String> {
    let kept: Vec<&str> = path
        .split(':')
        .filter(|d| !dirs.iter().any(|dir| Path::new(d).starts_with(dir)))
        .collect();
    (!kept.is_empty()).then(|| kept.join(":"))
}

/// Prüft eine Anfrage gegen Token und Stufe-2-Policy (AC3); liefert das kanonische `cwd`.
fn validate(req: &Request, shared: &Shared) -> Result<PathBuf, Reply> {
    authorize(req, shared)?;
    if req.tty {
        return Err(reject(
            "unsupported",
            "PTY über den Exec-Broker folgt mit SBX-017 (M3)",
        ));
    }
    let Some(program) = req.argv.first() else {
        return Err(reject("invalid_request", "argv ist leer"));
    };
    if !Path::new(program).is_absolute() {
        return Err(reject(
            "invalid_request",
            "das Programm muss ein absoluter Pfad sein (bt-exec löst es auf)",
        ));
    }
    if req.argv.iter().any(|a| a.contains('\0')) {
        return Err(reject("invalid_request", "NUL-Byte in argv"));
    }
    let policy = shared.tools.policy();
    for (k, v) in &req.env_overrides {
        if policy.credential_env.contains(k) {
            return Err(reject(
                "env_denied",
                format!("`{k}`: trägt den Credential-Platzhalter der Stufe 2"),
            ));
        }
        if let Some(why) = super::override_rejected(k, &shared.forward) {
            return Err(reject("env_denied", format!("`{k}`: {why}")));
        }
        if k == "PATH" && !super::path_ok(v) {
            return Err(reject(
                "env_denied",
                "`PATH`: nur absolute Einträge (SBX-002)",
            ));
        }
        if v.contains('\0') {
            return Err(reject("env_denied", format!("`{k}`: NUL-Byte im Wert")));
        }
    }
    let denied = || {
        reject(
            "cwd_denied",
            format!(
                "{} liegt nicht in den Wurzeln der Stufe 2",
                req.cwd.display()
            ),
        )
    };
    let cwd = std::fs::canonicalize(&req.cwd).map_err(|_| denied())?;
    let inside = policy
        .read_roots
        .iter()
        .chain(&policy.write_roots)
        .any(|r| cwd.starts_with(r));
    let masked = policy.masks.iter().any(|m| match m {
        Mask::Path { path, .. } => cwd.starts_with(path),
        Mask::Pattern { .. } => false,
    });
    if !inside || masked || !cwd.is_dir() {
        return Err(denied());
    }
    Ok(cwd)
}

async fn serve(mut conn: UnixStream, shared: Arc<Shared>) {
    let received = tokio::time::timeout(REQUEST_TIMEOUT, async {
        let fds = loop {
            conn.readable().await?;
            match conn.try_io(tokio::io::Interest::READABLE, || {
                fdpass::recv_fds(conn.as_raw_fd())
            }) {
                Ok(fds) => break fds,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
        };
        let req: Option<Request> = read_frame(&mut conn).await?;
        Ok::<_, std::io::Error>((fds, req))
    })
    .await;
    let (fds, req) = match received {
        Ok(Ok((fds, Some(req)))) => (fds, req),
        Ok(Ok((_, None))) | Ok(Err(_)) | Err(_) => return,
    };
    if req.op == Op::GitPreflight {
        drop(fds);
        let reply = git_preflight(&req, &shared);
        if let Reply::Error { code, .. } = &reply {
            tracing::debug!(code = %code, "Exec-Broker: git in Stufe 1 abgelehnt");
        }
        let _ = write_frame(&mut conn, &reply).await;
        return;
    }
    let cwd = match validate(&req, &shared) {
        Ok(cwd) => cwd,
        Err(reply) => {
            if let Reply::Error { code, .. } = &reply {
                tracing::debug!(code = %code, "Exec-Broker lehnt ab");
            }
            let _ = write_frame(&mut conn, &reply).await;
            return;
        }
    };
    let mut fds = fds.into_iter().map(Arc::new);
    let (Some(stdin), Some(stdout), Some(stderr)) = (fds.next(), fds.next(), fds.next()) else {
        return;
    };
    let set = req
        .env_overrides
        .iter()
        .filter_map(|(k, v)| {
            if k == "PATH" {
                stage_two_path(v, &shared.dirs).map(|p| (k.clone(), p))
            } else {
                Some((k.clone(), v.clone()))
            }
        })
        .collect();
    let cmd = CommandSpec {
        program: PathBuf::from(&req.argv[0]),
        args: req.argv[1..].to_vec(),
        env: EnvSpec {
            clear: true,
            set,
            remove: Vec::new(),
        },
        cwd: Some(cwd),
        stdin: Stdio::Fd(stdin),
        stdout: Stdio::Fd(stdout),
        stderr: Stdio::Fd(stderr),
        // `tty: true` lehnt der Broker ab (Terminals laufen über `beton-pty`, SBX-017).
        tty: None,
    };
    // Die Deskriptoren des Clients hält nur noch das Kind (sonst sähe der Harness nie EOF).
    let spawned = shared.tools.spawn(cmd);
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = write_frame(&mut conn, &reject(e.code(), e.to_string())).await;
            return;
        }
    };
    shared
        .started
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let pid = child.id().unwrap_or(0);
    tracing::debug!(pid, "Exec-Broker: Stufe 2 gestartet");
    let (mut rd, mut wr) = conn.into_split();
    if write_frame(&mut wr, &Reply::Started { pid }).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return;
    }
    loop {
        tokio::select! {
            status = child.wait() => {
                let reply = match status {
                    Ok(s) => Reply::Exited { code: s.code(), signal: s.signal() },
                    Err(e) => reject("spawn_failed", e.to_string()),
                };
                let _ = write_frame(&mut wr, &reply).await;
                return;
            }
            msg = read_frame::<ClientMsg>(&mut rd) => match msg {
                Ok(Some(ClientMsg::Signal { signal })) if SIGNALS.contains(&signal) => {
                    let _ = child.signal_tree(signal);
                }
                Ok(Some(ClientMsg::Signal { .. })) => {}
                // bt-exec ist weg: der Baum endet mit ihm.
                Ok(None) | Err(_) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return;
                }
            }
        }
    }
}
