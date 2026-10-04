//! Relay-Hub im Runner (HAR-009).
//!
//! Jeder MCP-Server einer Session erreicht den Harness als stdio-Relay (`beton mcp serve` bzw.
//! `beton mcp proxy --server <name>`). Das Relay verbindet sich über einen Unix-Socket mit
//! diesem Hub und weist sich mit einem session-gebundenen Token aus; der Hub bedient dann den
//! eingebauten Server `beton` (System-Tools) oder startet bzw. erreicht den konfigurierten
//! Server und filtert dessen Tools laut `allow`.
//!
//! Sicherheit:
//! - Socket und Token-Datei liegen im privaten Laufzeitverzeichnis (0700) und sind 0600.
//!   Harness-Konfiguration und argv enthalten nur Pfade, nie das Token, Env-Werte oder Header.
//! - Das Token ist an die `session_id` gebunden: Ein Relay mit Token oder Session-ID einer
//!   anderen Session wird abgelehnt (Vergleich in konstanter Zeit).
//! - Fail closed: Unbekannte Server, fehlerhafte Handshakes und nicht freigeschaltete Tools
//!   werden abgelehnt. Startfehler eines Servers erzeugen `mcp.server_failed`, die Session
//!   läuft weiter.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{EventPayload, McpServerFailed};
use beton_core::id::SessionId;
use beton_harness::McpLaunch;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::config::{ConfiguredServer, Endpoint, SYSTEM_SERVER};
use crate::protocol::{self, Kind, codes};
use crate::system::SystemServer;

/// Version des Relay-Handshakes.
pub const RELAY_VERSION: u32 = 1;
/// Höchstlänge einer Zeile (Handshake bzw. JSON-RPC-Nachricht).
pub const MAX_LINE: usize = 16 * 1024 * 1024;
/// Frist für den Handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Erste Zeile eines Relays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    pub relay: u32,
    pub session_id: String,
    pub token: String,
    pub server: String,
}

/// Antwort des Hubs auf den Handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandshakeReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Wie Relays gestartet werden: Programm (das `beton`-Binary) und Argumente davor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCommand {
    pub program: PathBuf,
    pub prefix: Vec<String>,
}

/// Startparameter des Hubs.
pub struct HubOptions {
    pub session_id: SessionId,
    /// Privates Laufzeitverzeichnis (0700) für Socket und Token-Datei, z. B. `~/.beton/run`.
    pub run_dir: PathBuf,
    /// Arbeitsverzeichnis der Session; stdio-Server starten darin.
    pub workdir: PathBuf,
    /// Konfigurierte Server (AGT-006). Ungültige Einträge melden sich beim Start.
    pub servers: Vec<ConfiguredServer>,
    /// Der Server `beton`; `None` bei `mcp_bridge: false`.
    pub system: Option<Arc<SystemServer>>,
    pub relay: RelayCommand,
}

/// Laufender Hub. Beim Drop werden Socket und Token-Datei entfernt.
pub struct Hub {
    session_id: SessionId,
    socket: PathBuf,
    token_file: PathBuf,
    relay: RelayCommand,
    names: Vec<String>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("socket", &self.socket)
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

struct Shared {
    session_id: String,
    token: Vec<u8>,
    workdir: PathBuf,
    servers: BTreeMap<String, ConfiguredServer>,
    system: Option<Arc<SystemServer>>,
    events: mpsc::UnboundedSender<EventPayload>,
}

fn random_hex(n: usize) -> std::io::Result<String> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(hex::encode(bytes))
}

/// Schreibt eine Datei mit Modus 0600; schlägt fehl, wenn sie schon existiert.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents)?;
    f.sync_all()
}

impl Hub {
    /// Startet den Hub. Liefert ihn und den Strom von `mcp.server_failed`-Events.
    #[cfg(unix)]
    pub async fn start(
        opts: HubOptions,
    ) -> std::io::Result<(Self, mpsc::UnboundedReceiver<EventPayload>)> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::create_dir_all(&opts.run_dir)?;
        let meta = std::fs::metadata(&opts.run_dir)?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} ist nicht privat (Modus {:o}); erwartet 0700",
                    opts.run_dir.display(),
                    meta.permissions().mode() & 0o777
                ),
            ));
        }
        let id = random_hex(4)?;
        let socket = opts.run_dir.join(format!("mcp-{id}.sock"));
        let token_file = opts.run_dir.join(format!("mcp-{id}.token"));
        let token = random_hex(32)?;
        write_private(&token_file, token.as_bytes())?;
        let listener = match tokio::net::UnixListener::bind(&socket) {
            Ok(l) => l,
            Err(e) => {
                let _ = std::fs::remove_file(&token_file);
                return Err(e);
            }
        };
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let (events, rx) = mpsc::unbounded_channel();
        let mut names = Vec::new();
        if opts.system.is_some() {
            names.push(SYSTEM_SERVER.to_owned());
        }
        let mut servers = BTreeMap::new();
        for s in opts.servers {
            // Ungültige Einträge gar nicht erst an den Harness geben (fail closed).
            if let Err(error) = s.endpoint() {
                let _ = events.send(EventPayload::McpServerFailed(McpServerFailed {
                    name: s.name.clone(),
                    error,
                }));
                continue;
            }
            names.push(s.name.clone());
            servers.insert(s.name.clone(), s);
        }
        let shared = Arc::new(Shared {
            session_id: opts.session_id.to_string(),
            token: token.into_bytes(),
            workdir: opts.workdir,
            servers,
            system: opts.system,
            events,
        });
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let (r, w) = stream.into_split();
                    serve_connection(shared, BufReader::new(r), w).await;
                });
            }
        });
        Ok((
            Self {
                session_id: opts.session_id,
                socket,
                token_file,
                relay: opts.relay,
                names,
                task,
            },
            rx,
        ))
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn token_file(&self) -> &Path {
        &self.token_file
    }

    /// Namen der Server, die der Harness bekommt (`beton` zuerst).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Relay-Kommandos für den Harness (HAR-009).
    pub fn launches(&self) -> Vec<McpLaunch> {
        self.names
            .iter()
            .map(|name| {
                let mut args = self.relay.prefix.clone();
                args.extend(["mcp".to_owned()]);
                if name == SYSTEM_SERVER {
                    args.push("serve".into());
                } else {
                    args.extend(["proxy".to_owned(), "--server".to_owned(), name.clone()]);
                }
                args.extend([
                    "--session".to_owned(),
                    self.session_id.to_string(),
                    "--socket".to_owned(),
                    self.socket.display().to_string(),
                    "--token-file".to_owned(),
                    self.token_file.display().to_string(),
                ]);
                McpLaunch {
                    name: name.clone(),
                    command: self.relay.program.display().to_string(),
                    args,
                }
            })
            .collect()
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.token_file);
    }
}

/// Liest eine Zeile mit Längenbegrenzung; `None` am Ende des Stroms.
pub async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    max: usize,
) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&buf).into_owned())
            });
        }
        let (take, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        buf.extend_from_slice(&available[..take]);
        r.consume(take);
        if buf.len() > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Zeile zu lang",
            ));
        }
        if done {
            while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
    }
}

async fn write_line<W: AsyncWrite + Unpin>(w: &mut W, v: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(v).unwrap_or_default();
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await
}

/// Prüft den Handshake gegen Session und Token (HAR-009 AC3).
fn authorize(shared: &Shared, h: &Handshake) -> Result<(), &'static str> {
    if h.relay != RELAY_VERSION {
        return Err("relay_version");
    }
    let token_ok: bool = h.token.as_bytes().ct_eq(&shared.token).into();
    let session_ok: bool = h
        .session_id
        .as_bytes()
        .ct_eq(shared.session_id.as_bytes())
        .into();
    // Beide Prüfungen immer ausführen; keine Auskunft, welche scheiterte.
    if !(token_ok & session_ok) {
        return Err("unauthorized");
    }
    let known = if h.server == SYSTEM_SERVER {
        shared.system.is_some()
    } else {
        shared.servers.contains_key(&h.server)
    };
    if !known {
        return Err("unknown_server");
    }
    Ok(())
}

async fn serve_connection<R, W>(shared: Arc<Shared>, mut r: R, mut w: W)
where
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_line(&mut r, 4096)).await;
    let handshake = match first {
        Ok(Ok(Some(line))) => serde_json::from_str::<Handshake>(&line).ok(),
        _ => None,
    };
    let verdict = match &handshake {
        Some(h) => authorize(&shared, h),
        None => Err("bad_handshake"),
    };
    let reply = HandshakeReply {
        ok: verdict.is_ok(),
        error: verdict.err().map(str::to_owned),
    };
    let reply_value = serde_json::to_value(&reply).unwrap_or(Value::Null);
    if write_line(&mut w, &reply_value).await.is_err() {
        return;
    }
    let (Some(h), Ok(())) = (handshake, verdict) else {
        tracing::warn!(error = ?reply.error, "MCP-Relay abgelehnt");
        return;
    };
    if h.server == SYSTEM_SERVER {
        if let Some(system) = shared.system.clone() {
            serve_system(system, r, w).await;
        }
        return;
    }
    let Some(server) = shared.servers.get(&h.server).cloned() else {
        return;
    };
    match server.endpoint() {
        Ok(Endpoint::Stdio { .. }) => proxy_stdio(shared, server, r, w).await,
        Ok(Endpoint::Http { .. }) => {
            crate::http::proxy_http(shared_events(&shared), server, r, w).await
        }
        Err(_) => {}
    }
}

fn shared_events(shared: &Shared) -> mpsc::UnboundedSender<EventPayload> {
    shared.events.clone()
}

/// Schreibt Antworten aus einem Kanal; ein Writer pro Verbindung.
pub(crate) fn writer<W: AsyncWrite + Unpin + Send + 'static>(
    mut w: W,
) -> mpsc::UnboundedSender<Value> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(v) = rx.recv().await {
            if write_line(&mut w, &v).await.is_err() {
                break;
            }
        }
    });
    tx
}

async fn serve_system<R, W>(system: Arc<SystemServer>, mut r: R, w: W)
where
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let out = writer(w);
    while let Ok(Some(line)) = read_line(&mut r, MAX_LINE).await {
        if line.trim().is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Value>(&line) {
            Ok(m) => m,
            Err(_) => {
                let _ = out.send(protocol::error(
                    &Value::Null,
                    codes::PARSE_ERROR,
                    "ungültiges JSON",
                    None,
                ));
                continue;
            }
        };
        // Jede Anfrage in eigenem Task: ein langer `session_spawn` blockiert kein `ping`.
        let system = system.clone();
        let out = out.clone();
        tokio::spawn(async move {
            if let Some(reply) = system.handle(&msg).await {
                let _ = out.send(reply);
            }
        });
    }
}

/// Gemeinsame Filterlogik für weitergeleitete Server (stdio und HTTP).
#[derive(Default)]
pub(crate) struct Filter {
    /// Offene Anfragen des Harness: ID → Methode.
    pending: Mutex<HashMap<String, String>>,
}

impl Filter {
    /// Nachricht vom Harness. `Err(reply)`: nicht weiterleiten, sondern direkt antworten.
    pub(crate) fn outbound(&self, server: &ConfiguredServer, msg: &Value) -> Result<(), Value> {
        if protocol::kind(msg) != Kind::Request {
            return Ok(());
        }
        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or_default();
        if method == "tools/call" {
            let tool = msg["params"]["name"].as_str().unwrap_or_default();
            if !server.allows(tool) {
                return Err(protocol::error(
                    &id,
                    codes::INVALID_PARAMS,
                    format!("tool_not_enabled: {tool} ist laut `allow` nicht freigegeben"),
                    Some(json!({"code": "tool_not_enabled", "tool": tool})),
                ));
            }
        }
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id.to_string(), method.to_owned());
        }
        Ok(())
    }

    /// Nachricht vom Server, ggf. gefiltert. Liefert die Methode der beantworteten Anfrage.
    pub(crate) fn inbound(&self, server: &ConfiguredServer, msg: &mut Value) -> Option<String> {
        if protocol::kind(msg) != Kind::Response {
            return None;
        }
        let method = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&msg["id"].to_string()))?;
        if method == "tools/list"
            && let Some(tools) = msg["result"]["tools"].as_array_mut()
        {
            tools.retain(|t| server.allows(t["name"].as_str().unwrap_or_default()));
        }
        Some(method)
    }
}

pub(crate) fn failed(
    events: &mpsc::UnboundedSender<EventPayload>,
    name: &str,
    error: impl Into<String>,
) {
    let error = error.into();
    tracing::warn!(server = name, "MCP-Server nicht verfügbar: {error}");
    let _ = events.send(EventPayload::McpServerFailed(McpServerFailed {
        name: name.to_owned(),
        error,
    }));
}

/// Beantwortet alle weiteren Anfragen des Harness mit einem Fehler, bis er die Verbindung
/// schließt; so meldet der Harness den Server als fehlgeschlagen statt zu hängen.
pub(crate) async fn refuse_rest<R: AsyncBufRead + Unpin>(
    mut r: R,
    out: mpsc::UnboundedSender<Value>,
    message: String,
) {
    while let Ok(Some(line)) = read_line(&mut r, MAX_LINE).await {
        if let Ok(msg) = serde_json::from_str::<Value>(&line)
            && protocol::kind(&msg) == Kind::Request
        {
            let _ = out.send(protocol::error(
                &msg["id"],
                codes::SERVER_UNAVAILABLE,
                message.clone(),
                None,
            ));
        }
    }
}

async fn proxy_stdio<R, W>(shared: Arc<Shared>, server: ConfiguredServer, mut r: R, w: W)
where
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let out = writer(w);
    let Ok(Endpoint::Stdio { command, args, env }) = server.endpoint() else {
        return;
    };
    let mut cmd = tokio::process::Command::new(command);
    // Interne Variablen des Runners (Tunnel-Socket, Session, Konfiguration) gehen nicht an
    // fremde Server.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("BETON_") {
            cmd.env_remove(key);
        }
    }
    cmd.args(args)
        .envs(env)
        .current_dir(&shared.workdir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        // stderr kann Secrets enthalten: nicht in Events oder Logs übernehmen.
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Start fehlgeschlagen: {e}");
            failed(&shared.events, &server.name, msg.clone());
            refuse_rest(r, out, msg).await;
            return;
        }
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return;
    };
    let filter = Arc::new(Filter::default());
    let initialized = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Server → Harness.
    let from_server = {
        let filter = filter.clone();
        let out = out.clone();
        let server = server.clone();
        let initialized = initialized.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout);
            while let Ok(Some(line)) = read_line(&mut lines, MAX_LINE).await {
                let Ok(mut msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if filter.inbound(&server, &mut msg).as_deref() == Some("initialize") {
                    initialized.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if out.send(msg).is_err() {
                    break;
                }
            }
        })
    };
    // Harness → Server, bis eine Seite endet.
    let mut from_server = from_server;
    let mut server_gone = false;
    loop {
        tokio::select! {
            line = read_line(&mut r, MAX_LINE) => {
                let Ok(Some(line)) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Err(reply) = filter.outbound(&server, &msg) {
                    let _ = out.send(reply);
                    continue;
                }
                let mut bytes = serde_json::to_vec(&msg).unwrap_or_default();
                bytes.push(b'\n');
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    server_gone = true;
                    break;
                }
            }
            _ = &mut from_server => {
                server_gone = true;
                break;
            }
        }
    }
    drop(stdin);
    let exit = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    let _ = child.kill().await;
    if server_gone {
        // Offene Anfragen beantworten, damit der Harness nicht hängt.
        let open: Vec<String> = filter
            .pending
            .lock()
            .map(|mut p| p.drain().map(|(id, _)| id).collect())
            .unwrap_or_default();
        for id in open {
            let id: Value = serde_json::from_str(&id).unwrap_or(Value::Null);
            let _ = out.send(protocol::error(
                &id,
                codes::SERVER_UNAVAILABLE,
                format!("MCP-Server {} beendet", server.name),
                None,
            ));
        }
    } else {
        from_server.abort();
    }
    // War der Server nie initialisiert, ist er gescheitert (HAR-009 AC4).
    if server_gone && !initialized.load(std::sync::atomic::Ordering::SeqCst) {
        let detail = match exit {
            Ok(Ok(status)) => format!("Server endete vor initialize ({status})"),
            _ => "Server antwortete nicht auf initialize".to_owned(),
        };
        failed(&shared.events, &server.name, detail);
    }
}

#[cfg(test)]
mod tests;
