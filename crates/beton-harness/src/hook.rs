//! Hook-Brücke für Vendor-Hooks (HAR-005 ab M2, HAR-013).
//!
//! Claude Code ruft vor jedem Tool-Call das Hook-Kommando `beton hook claude --event
//! PreToolUse --socket <pfad> --token-file <pfad>` auf (Settings per `--settings`). Das
//! Kommando liest das Hook-JSON von stdin, verbindet sich über einen Unix-Socket mit dem
//! [`HookServer`] des Adapters und gibt dessen Antwort als Hook-Ausgabe zurück.
//!
//! Sicherheit:
//! - Socket (0600) und Token-Datei (0600) liegen in einem privaten Verzeichnis (0700). Argv und
//!   Settings enthalten nur Pfade, nie das Token.
//! - Fail closed: Jeder Fehler des Kommandos (Socket weg, falsches Token, ungültige Antwort)
//!   endet mit Exit-Code 2; Claude Code wertet das als Blockade des Tool-Calls.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version des Hook-Protokolls.
pub const HOOK_VERSION: u32 = 1;
/// Höchstlänge einer Zeile (Hook-JSON bzw. Antwort).
pub const MAX_LINE: usize = 16 * 1024 * 1024;
/// Exit-Code, mit dem Claude Code einen Tool-Call blockiert.
pub const EXIT_BLOCK: i32 = 2;

/// Wie das Hook-Kommando gestartet wird: das `beton`-Binary und Argumente davor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCommand {
    pub program: PathBuf,
    pub prefix: Vec<String>,
}

impl HookCommand {
    /// Das laufende Binary (im Runner: `beton`).
    pub fn current_exe() -> Option<Self> {
        Some(Self {
            program: std::env::current_exe().ok()?,
            prefix: Vec::new(),
        })
    }
}

/// Was ein Adapter für einen Vendor-Hook braucht (vom Runner gesetzt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSetup {
    pub command: HookCommand,
    /// Privates Laufzeitverzeichnis (0700), z. B. `~/.beton/run`.
    pub run_dir: PathBuf,
}

/// Erste Zeile des Kommandos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookHandshake {
    pub hook: u32,
    pub token: String,
    /// Name des Vendor-Hooks, z. B. `PreToolUse`.
    pub event: String,
}

/// Antwort des Servers: Exit-Code, Hook-Ausgabe (stdout, JSON) und Text für stderr.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookReply {
    pub exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

impl HookReply {
    /// Keine Entscheidung: der Vendor macht normal weiter.
    pub fn pass() -> Self {
        Self::default()
    }

    /// Blockiert (Exit-Code 2) mit Begründung auf stderr.
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            exit: EXIT_BLOCK,
            stdout: None,
            stderr: Some(reason.into()),
        }
    }
}

/// Beantwortet Hook-Aufrufe.
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn handle(&self, event: &str, payload: Value) -> HookReply;
}

/// Lauscht auf Hook-Aufrufe einer Session. Beim Drop werden Socket, Token-Datei und
/// Verzeichnis entfernt.
pub struct HookServer {
    dir: PathBuf,
    socket: PathBuf,
    token_file: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for HookServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookServer")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
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

/// Shell-Quoting für die Kommandozeile in den Hook-Settings (Claude führt sie per Shell aus).
pub fn shell_quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));
    if safe {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

impl HookServer {
    /// Startet den Server in einem neuen privaten Verzeichnis unter `run_dir`.
    #[cfg(unix)]
    pub async fn start(run_dir: &Path, handler: Arc<dyn HookHandler>) -> std::io::Result<Self> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::create_dir_all(run_dir)?;
        let dir = run_dir.join(format!("hook-{}", random_hex(4)?));
        std::fs::create_dir(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let socket = dir.join("hook.sock");
        let token_file = dir.join("hook.token");
        let token = random_hex(32)?;
        let cleanup = |e: std::io::Error| {
            let _ = std::fs::remove_dir_all(&dir);
            e
        };
        write_private(&token_file, token.as_bytes()).map_err(cleanup)?;
        let listener = tokio::net::UnixListener::bind(&socket).map_err(cleanup)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(cleanup)?;
        let token = Arc::new(token.into_bytes());
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                let handler = handler.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let (r, w) = stream.into_split();
                    serve(handler, &token, tokio::io::BufReader::new(r), w).await;
                });
            }
        });
        Ok(Self {
            dir,
            socket,
            token_file,
            task,
        })
    }

    #[cfg(not(unix))]
    pub async fn start(_run_dir: &Path, _handler: Arc<dyn HookHandler>) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Hook-Brücke über Unix-Sockets gibt es unter Windows noch nicht",
        ))
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn token_file(&self) -> &Path {
        &self.token_file
    }

    /// Kommandozeile für einen Hook, shell-gequotet: `<beton> hook <harness> --event <event>
    /// --socket <pfad> --token-file <pfad>`.
    pub fn command_line(&self, cmd: &HookCommand, harness: &str, event: &str) -> String {
        let mut words = vec![cmd.program.display().to_string()];
        words.extend(cmd.prefix.iter().cloned());
        words.extend([
            "hook".to_owned(),
            harness.to_owned(),
            "--event".to_owned(),
            event.to_owned(),
            "--socket".to_owned(),
            self.socket.display().to_string(),
            "--token-file".to_owned(),
            self.token_file.display().to_string(),
        ]);
        words
            .iter()
            .map(|w| shell_quote(w))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Drop for HookServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(r: &mut R) -> std::io::Result<String> {
    use tokio::io::AsyncBufReadExt as _;
    let mut buf = Vec::new();
    let mut limited = tokio::io::AsyncReadExt::take(r, MAX_LINE as u64 + 1);
    limited.read_until(b'\n', &mut buf).await?;
    if buf.len() > MAX_LINE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Zeile zu lang",
        ));
    }
    while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| std::io::Error::other("keine UTF-8-Zeile"))
}

async fn write_reply<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, reply: &HookReply) {
    use tokio::io::AsyncWriteExt as _;
    let mut line = serde_json::to_vec(reply).unwrap_or_default();
    line.push(b'\n');
    let _ = w.write_all(&line).await;
    let _ = w.flush().await;
}

async fn serve<R, W>(handler: Arc<dyn HookHandler>, token: &[u8], mut r: R, mut w: W)
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use subtle::ConstantTimeEq as _;
    let first = tokio::time::timeout(Duration::from_secs(10), read_line(&mut r)).await;
    let handshake = match first {
        Ok(Ok(line)) => serde_json::from_str::<HookHandshake>(&line).ok(),
        _ => None,
    };
    let Some(h) = handshake.filter(|h| h.hook == HOOK_VERSION) else {
        write_reply(&mut w, &HookReply::block("beton: ungültiger Hook-Aufruf")).await;
        return;
    };
    let ok: bool = h.token.as_bytes().ct_eq(token).into();
    if !ok {
        tracing::warn!("Hook-Aufruf mit falschem Token abgelehnt");
        write_reply(
            &mut w,
            &HookReply::block("beton: Hook-Aufruf nicht autorisiert"),
        )
        .await;
        return;
    }
    let payload = match tokio::time::timeout(Duration::from_secs(10), read_line(&mut r)).await {
        Ok(Ok(line)) => serde_json::from_str::<Value>(&line).ok(),
        _ => None,
    };
    let Some(payload) = payload else {
        write_reply(&mut w, &HookReply::block("beton: ungültige Hook-Daten")).await;
        return;
    };
    let reply = handler.handle(&h.event, payload).await;
    write_reply(&mut w, &reply).await;
}

/// Ergebnis des Hook-Kommandos: Exit-Code, stdout, stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOutput {
    pub exit: i32,
    pub stdout: String,
    pub stderr: String,
}

fn blocked(message: &str) -> ClientOutput {
    ClientOutput {
        exit: EXIT_BLOCK,
        stdout: String::new(),
        stderr: format!("beton: {message}"),
    }
}

/// Das Hook-Kommando (`beton hook <harness> --event <e> --socket <p> --token-file <p>`):
/// schickt `input` (Hook-JSON von stdin) an den Server und wartet auf die Antwort (ohne eigene
/// Frist: die setzt der Vendor über den Hook-`timeout`). Jeder Fehler blockiert (Exit 2).
#[cfg(unix)]
pub fn client(args: &[String], input: &[u8]) -> ClientOutput {
    use std::io::{BufRead as _, Read as _, Write as _};
    let mut event = None;
    let mut socket = None;
    let mut token_file = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--event" => event = it.next().cloned(),
            "--socket" => socket = it.next().map(PathBuf::from),
            "--token-file" => token_file = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    let (Some(event), Some(socket), Some(token_file)) = (event, socket, token_file) else {
        return blocked(
            "Aufruf: beton hook <harness> --event <name> --socket <pfad> --token-file <pfad>",
        );
    };
    let Ok(token) = std::fs::read_to_string(&token_file) else {
        return blocked("Token-Datei nicht lesbar; Runner nicht erreichbar");
    };
    if serde_json::from_slice::<Value>(input).is_err() {
        return blocked("Hook-Daten sind kein JSON");
    }
    let Ok(stream) = std::os::unix::net::UnixStream::connect(&socket) else {
        return blocked("Runner nicht erreichbar (Hook-Socket)");
    };
    let handshake = HookHandshake {
        hook: HOOK_VERSION,
        token: token.trim().to_owned(),
        event,
    };
    let mut first = serde_json::to_vec(&handshake).unwrap_or_default();
    first.push(b'\n');
    // JSON-Strings enthalten keine rohen Zeilenumbrüche; so wird die Eingabe eine Zeile.
    let mut body: Vec<u8> = input
        .iter()
        .copied()
        .filter(|b| *b != b'\n' && *b != b'\r')
        .collect();
    body.push(b'\n');
    let mut writer = &stream;
    if writer.write_all(&first).is_err() || writer.write_all(&body).is_err() {
        return blocked("Hook-Daten nicht gesendet");
    }
    let mut line = String::new();
    let mut reader = std::io::BufReader::new(&stream).take(MAX_LINE as u64);
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return blocked("keine Antwort vom Runner");
    }
    let Ok(reply) = serde_json::from_str::<HookReply>(line.trim_end()) else {
        return blocked("ungültige Antwort vom Runner");
    };
    ClientOutput {
        exit: reply.exit,
        stdout: reply.stdout.map(|v| v.to_string()).unwrap_or_default(),
        stderr: reply.stderr.unwrap_or_default(),
    }
}

#[cfg(not(unix))]
pub fn client(_args: &[String], _input: &[u8]) -> ClientOutput {
    blocked("Hook-Brücke gibt es unter Windows noch nicht")
}
