//! Workspace-Terminals im Runner (SES-019, SBX-017, DATA-011).
//!
//! Das PTY hält der Runner in Stufe 0 (`beton-pty`); der Prozess am Slave startet über die
//! Tool-Sandbox der Session (`Sandbox::spawn` in Stufe 2, je Start genau ein
//! `sandbox.started {stage: tools}`). Ohne Stufenmodell (`backend: none`, bestätigt) startet er
//! über den Provider `none`, wie jedes Tool.
//!
//! Je Terminal liest ein Thread die Ausgabe in den VT-Emulator (Scrollback 10 000 Zeilen) und
//! schickt sie als Binärframes über den Tunnel, nur im Kredit des Servers (PROTO-007): Fehlt
//! Kredit, pausiert das Lesen, und das Kind blockiert beim Schreiben wie an einem echten
//! Terminal. Ohne Verbindung liest der Thread frei weiter (nur in den Emulator). Neue Zuschauer
//! bekommen ein Keyframe aus dem Emulator. Ins Session-Log gehen nur `terminal.opened`,
//! `terminal.renamed`, `terminal.snapshot` (Blob, höchstens alle 10 s bei Aktivität und beim
//! Schließen) und `terminal.closed`; nie die Bytes (DATA-011).
//!
//! Terminals überleben Client-Disconnects; sie enden mit `terminal.close`, mit dem Ende ihres
//! Prozesses oder mit dem Runner.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, EventPayload, TerminalClosed, TerminalKind, TerminalOpened, TerminalRenamed,
    TerminalSnapshot,
};
use beton_proto::channel::{ChannelKind, Frame, flags};
use beton_proto::tunnel::TunnelUp;
use beton_pty::{Pty, Screen};
use beton_sandbox::{CommandSpec, Sandbox};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};

use crate::mcp::SystemRequest;

/// Höchstzahl gleichzeitiger Terminals einer Session.
pub const MAX_TERMINALS: usize = 16;
/// Snapshots bei Aktivität höchstens so oft (DATA-011: höchstens 6 je Minute).
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(10);
/// Lesepuffer je Frame.
const READ_CHUNK: usize = 16 * 1024;
/// Schließen: nach `SIGHUP` so lange auf das Ende warten, dann `SIGKILL`.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Längster Anzeigename.
const MAX_NAME: usize = 80;

/// Was die Terminals an den Runner melden.
#[derive(Debug)]
pub enum Out {
    /// Dauerhaftes Event (mit Akteur, sonst `system/runner`).
    Event(Box<EventPayload>, Option<Actor>),
    /// Binärframe an den Server (schon kodiert).
    Frame(Vec<u8>),
    /// Steuernachricht an den Server (`channel.open`/`channel.close`).
    Up(TunnelUp),
}

/// Fehler eines Terminal-Kommandos als `{code, detail}` für `cmd.result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermError {
    pub code: &'static str,
    pub detail: String,
}

impl TermError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn to_problem(&self) -> Value {
        json!({"code": self.code, "detail": self.detail})
    }
}

/// Was ein neues Terminal starten soll.
#[derive(Debug, Clone, Default)]
pub struct OpenSpec {
    pub name: Option<String>,
    /// Programm und Argumente; leer: die Shell des Benutzers.
    pub command: Vec<String>,
    /// Zusätzliche Variablen (nur erlaubte, siehe [`beton_sandbox::broker::override_rejected`]).
    pub env: Vec<(String, String)>,
    /// Arbeitsverzeichnis innerhalb des Workspace; Default: Workspace.
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub kind: TerminalKind,
    /// Rohausgabe für `terminal/output` (ACP, #326) bis zu dieser Größe aufheben.
    pub output_limit: Option<usize>,
}

/// Sicht auf ein laufendes Terminal (`terminal.list`, Antwort von `terminal.open`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct View {
    pub terminal_id: String,
    pub channel_id: u32,
    pub name: String,
    pub kind: TerminalKind,
    pub cmd: String,
    pub cols: u16,
    pub rows: u16,
}

/// Ende des Prozesses eines Terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Exit {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

/// Kredit des Servers für die Ausgabe eines Terminals (Bytes). `None`: ohne Verbindung frei.
struct Credit {
    state: Mutex<CreditState>,
    cv: Condvar,
}

struct CreditState {
    avail: Option<i64>,
    closed: bool,
}

impl Credit {
    fn new() -> Self {
        Self {
            state: Mutex::new(CreditState {
                avail: None,
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CreditState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Wartet auf Kredit; `None` nach dem Schließen, sonst die erlaubte Lesegröße.
    fn wait(&self) -> Option<usize> {
        let mut st = self.lock();
        loop {
            if st.closed {
                return None;
            }
            match st.avail {
                None => return Some(READ_CHUNK),
                Some(n) if n > 0 => return Some((n as usize).min(READ_CHUNK)),
                Some(_) => {}
            }
            st = self
                .cv
                .wait(st)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn consume(&self, n: usize) {
        if let Some(a) = self.lock().avail.as_mut() {
            *a -= n as i64;
        }
    }

    fn add(&self, n: u64) {
        let mut st = self.lock();
        if let Some(a) = st.avail.as_mut() {
            *a = a.saturating_add(i64::try_from(n).unwrap_or(i64::MAX));
        }
        self.cv.notify_all();
    }

    /// Verbunden: ab jetzt nur im Kredit des Servers; ohne Verbindung frei.
    fn set_connected(&self, connected: bool) {
        self.lock().avail = connected.then_some(0);
        self.cv.notify_all();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
    }
}

/// Aufgehobene Rohausgabe (ACP `terminal/output`).
#[derive(Default)]
struct Captured {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl Captured {
    fn push(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
        if self.bytes.len() > self.limit {
            let cut = self.bytes.len() - self.limit;
            self.bytes.drain(..cut);
            // Nicht mitten in einem UTF-8-Zeichen beginnen.
            while self.bytes.first().is_some_and(|b| b & 0xC0 == 0x80) {
                self.bytes.remove(0);
            }
            self.truncated = true;
        }
    }
}

struct Term {
    view: View,
    pty: Arc<Pty>,
    screen: Arc<Mutex<Screen>>,
    credit: Arc<Credit>,
    input: std::sync::mpsc::Sender<Vec<u8>>,
    kill: Option<oneshot::Sender<()>>,
    exit: watch::Receiver<Option<Exit>>,
    captured: Option<Arc<Mutex<Captured>>>,
    /// Prozess beendet; Terminals eines Harness (`kind: tool`) bleiben bis zur Freigabe
    /// bekannt, damit `terminal/output` die Ausgabe noch liefert (ACP, #326).
    ended: bool,
}

struct Shared {
    terms: Mutex<HashMap<String, Term>>,
    connected: AtomicBool,
}

/// Die Terminals einer Session.
#[derive(Clone)]
pub struct Terminals {
    shared: Arc<Shared>,
    sandbox: Sandbox,
    workdir: PathBuf,
    shell: PathBuf,
    out: mpsc::UnboundedSender<Out>,
    calls: mpsc::UnboundedSender<SystemRequest>,
    next_channel: Arc<AtomicU32>,
    next_name: Arc<AtomicU32>,
    snapshot_interval: Duration,
}

impl std::fmt::Debug for Terminals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminals").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Sucht `name` in `path` (nur absolute Einträge, wie in der Sandbox).
fn which(name: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .filter(|d| d.starts_with('/'))
        .map(|d| Path::new(d).join(name))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && executable(&m)))
}

#[cfg(unix)]
fn executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    m.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_m: &std::fs::Metadata) -> bool {
    true
}

/// Exit-Code und Signal eines Prozesses.
fn exit_of(s: std::process::ExitStatus) -> Exit {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        Exit {
            exit_code: s.code(),
            signal: s.signal(),
        }
    }
    #[cfg(not(unix))]
    {
        Exit {
            exit_code: s.code(),
            signal: None,
        }
    }
}

/// PTY, Kind, Leser und Schreiber eines gestarteten Terminals.
type SpawnedPty = (
    Arc<Pty>,
    beton_sandbox::SandboxedChild,
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
);

/// Startet `cmd` am PTY über die Sandbox (Unix); liefert PTY, Kind, Leser und Schreiber.
#[cfg(unix)]
fn spawn_at_pty(
    sandbox: &Sandbox,
    cmd: CommandSpec,
    cols: u16,
    rows: u16,
) -> Result<SpawnedPty, TermError> {
    let mut pty =
        Pty::open(cols, rows).map_err(|e| TermError::new("unavailable", e.to_string()))?;
    let slave = pty
        .take_slave()
        .ok_or_else(|| TermError::new("unavailable", "PTY ohne Slave"))?;
    let tty = beton_sandbox::Tty::new(Arc::new(slave), pty.slave_path().to_path_buf());
    // Der Slave gehört danach nur dem Kind: Mit dessen Ende endet das Lesen am Master.
    let mut child = sandbox.spawn(cmd.tty(tty)).map_err(|e| {
        TermError::new(
            if e.code() == "sandbox_unavailable" {
                "sandbox_unavailable"
            } else {
                "unavailable"
            },
            e.to_string(),
        )
    })?;
    let pty = Arc::new(pty);
    match (pty.reader(), pty.writer()) {
        (Ok(r), Ok(w)) => Ok((pty, child, r, w)),
        (Err(e), _) | (_, Err(e)) => {
            let _ = child.start_kill();
            Err(TermError::new("unavailable", e.to_string()))
        }
    }
}

/// Windows (Beta): Terminals gibt es erst mit dem Sandbox-Backend (SBX-016, M5).
#[cfg(not(unix))]
fn spawn_at_pty(
    _sandbox: &Sandbox,
    _cmd: CommandSpec,
    _cols: u16,
    _rows: u16,
) -> Result<SpawnedPty, TermError> {
    Err(TermError::new(
        "sandbox_unavailable",
        "Terminals unter Windows folgen mit dem Sandbox-Backend (SBX-016)",
    ))
}

fn new_id() -> String {
    let mut bytes = [0u8; 12];
    let _ = getrandom::fill(&mut bytes);
    let mut s = String::from("term_");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

impl Terminals {
    /// `sandbox`: die Tool-Sandbox der Session (Stufe 2); `shell`: Default-Programm.
    pub fn new(
        sandbox: Sandbox,
        workdir: PathBuf,
        shell: PathBuf,
        out: mpsc::UnboundedSender<Out>,
        calls: mpsc::UnboundedSender<SystemRequest>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                terms: Mutex::default(),
                connected: AtomicBool::new(false),
            }),
            sandbox,
            workdir,
            shell,
            out,
            calls,
            next_channel: Arc::new(AtomicU32::new(0)),
            next_name: Arc::new(AtomicU32::new(0)),
            snapshot_interval: SNAPSHOT_INTERVAL,
        }
    }

    /// Kürzerer Abstand der Snapshots (Tests).
    pub fn with_snapshot_interval(mut self, every: Duration) -> Self {
        self.snapshot_interval = every;
        self
    }

    /// Die Shell des Benutzers aus `SHELL` (absolut), sonst `/bin/sh`.
    pub fn default_shell(env_shell: Option<&str>) -> PathBuf {
        env_shell
            .filter(|s| s.starts_with('/') && Path::new(s).is_file())
            .map_or_else(|| PathBuf::from("/bin/sh"), PathBuf::from)
    }

    /// Kommando `terminal.*` des Servers (SES-019).
    pub async fn command(
        &self,
        name: &str,
        args: &Value,
        actor: Option<Actor>,
    ) -> Result<Value, TermError> {
        match name {
            "terminal.open" => {
                let size = |k: &str| {
                    args[k]
                        .as_u64()
                        .and_then(|v| u16::try_from(v).ok())
                        .ok_or_else(|| TermError::new("validation_failed", format!("{k} fehlt")))
                };
                let command: Vec<String> = match &args["command"] {
                    Value::Null => Vec::new(),
                    v => serde_json::from_value(v.clone()).map_err(|e| {
                        TermError::new("validation_failed", format!("command: {e}"))
                    })?,
                };
                let spec = OpenSpec {
                    name: args["name"].as_str().map(str::to_owned),
                    command,
                    cols: size("cols")?,
                    rows: size("rows")?,
                    kind: TerminalKind::User,
                    ..OpenSpec::default()
                };
                self.open(spec, actor).map(|v| json!(v))
            }
            "terminal.list" => Ok(json!({"items": self.list()})),
            "terminal.resize" => {
                let id = args["terminal_id"].as_str().unwrap_or_default();
                let cols = args["cols"].as_u64().and_then(|v| u16::try_from(v).ok());
                let rows = args["rows"].as_u64().and_then(|v| u16::try_from(v).ok());
                let (Some(cols), Some(rows)) = (cols, rows) else {
                    return Err(TermError::new("validation_failed", "cols und rows fehlen"));
                };
                self.resize(id, cols, rows).map(|v| json!(v))
            }
            "terminal.rename" => {
                let id = args["terminal_id"].as_str().unwrap_or_default();
                let name = args["name"].as_str().unwrap_or_default();
                self.rename(id, name, actor).map(|v| json!(v))
            }
            "terminal.close" => {
                let id = args["terminal_id"].as_str().unwrap_or_default();
                self.close(id).map(|()| Value::Null)
            }
            _ => Err(TermError::new(
                "unknown_command",
                format!("`{name}` gibt es nicht"),
            )),
        }
    }

    fn check_name(name: &str) -> Result<String, TermError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(TermError::new(
                "validation_failed",
                format!("Name: 1 bis {MAX_NAME} Zeichen ohne Steuerzeichen"),
            ));
        }
        Ok(name.to_owned())
    }

    /// Öffnet ein Terminal: PTY in Stufe 0, Prozess über die Tool-Sandbox (fail closed).
    pub fn open(&self, spec: OpenSpec, actor: Option<Actor>) -> Result<View, TermError> {
        if !beton_pty::pty::valid_size(spec.cols, spec.rows) {
            return Err(TermError::new(
                "validation_failed",
                format!("ungültige Größe {} × {}", spec.cols, spec.rows),
            ));
        }
        if lock(&self.shared.terms).len() >= MAX_TERMINALS {
            return Err(TermError::new(
                "rate_limited",
                format!("höchstens {MAX_TERMINALS} Terminals je Session"),
            ));
        }
        let name = match &spec.name {
            Some(n) => Self::check_name(n)?,
            None => format!(
                "Shell {}",
                self.next_name.fetch_add(1, Ordering::Relaxed) + 1
            ),
        };
        let path = self
            .sandbox
            .policy()
            .env
            .get("PATH")
            .cloned()
            // Ohne Sandbox (`backend: none`) gilt der `PATH` des Runners.
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
        let (program, args) = match spec.command.split_first() {
            None => (self.shell.clone(), Vec::new()),
            Some((p, rest)) => {
                let program = if p.contains('/') {
                    PathBuf::from(p)
                } else {
                    which(p, &path).ok_or_else(|| {
                        TermError::new("not_found", format!("`{p}` nicht im PATH der Sandbox"))
                    })?
                };
                (program, rest.to_vec())
            }
        };
        let cwd = match &spec.cwd {
            None => self.workdir.clone(),
            Some(c) => {
                let root = self
                    .workdir
                    .canonicalize()
                    .unwrap_or_else(|_| self.workdir.clone());
                let canon = c.canonicalize().map_err(|e| {
                    TermError::new("validation_failed", format!("cwd {}: {e}", c.display()))
                })?;
                if !canon.starts_with(&root) {
                    return Err(TermError::new(
                        "validation_failed",
                        "cwd liegt außerhalb des Workspace",
                    ));
                }
                canon
            }
        };
        for (k, _) in &spec.env {
            if let Some(why) = beton_sandbox::broker::override_rejected(k, &[]) {
                return Err(TermError::new("env_denied", format!("`{k}`: {why}")));
            }
        }
        let mut cmd = CommandSpec::new(&program)
            .args(args.clone())
            .cwd(&cwd)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");
        for (k, v) in &spec.env {
            cmd = cmd.env(k, v);
        }
        // Ohne Sandbox (`backend: none`) erbt der Prozess den Runner; dessen Variablen nicht.
        cmd.env.remove = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| k.starts_with("BETON_"))
            .collect();
        let (pty, child, mut reader, mut writer) =
            spawn_at_pty(&self.sandbox, cmd, spec.cols, spec.rows)?;
        let id = new_id();
        let channel_id = self.next_channel.fetch_add(1, Ordering::Relaxed) + 1;
        let cmd_text = std::iter::once(program.display().to_string())
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");
        let view = View {
            terminal_id: id.clone(),
            channel_id,
            name: name.clone(),
            kind: spec.kind,
            cmd: cmd_text.clone(),
            cols: spec.cols,
            rows: spec.rows,
        };
        let screen = Arc::new(Mutex::new(Screen::new(spec.cols, spec.rows)));
        let credit = Arc::new(Credit::new());
        credit.set_connected(self.shared.connected.load(Ordering::SeqCst));
        let dirty = Arc::new(AtomicBool::new(false));
        let captured = spec.output_limit.map(|limit| {
            Arc::new(Mutex::new(Captured {
                limit,
                ..Captured::default()
            }))
        });
        // Lesen: Emulator, Rohausgabe (ACP) und Frames im Kredit des Servers.
        let reader_done = {
            let (screen, credit, dirty, out, captured, shared) = (
                screen.clone(),
                credit.clone(),
                dirty.clone(),
                self.out.clone(),
                captured.clone(),
                self.shared.clone(),
            );
            let (done_tx, done_rx) = oneshot::channel::<()>();
            std::thread::spawn(move || {
                let mut buf = vec![0u8; READ_CHUNK];
                while let Some(max) = credit.wait() {
                    let n = match reader.read(&mut buf[..max]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    {
                        let mut s = lock(&screen);
                        s.feed(&buf[..n]);
                        // Unter der Sperre des Emulators: Keyframes und Frames bleiben geordnet.
                        if shared.connected.load(Ordering::SeqCst) {
                            credit.consume(n);
                            let _ = out.send(Out::Frame(
                                Frame::new(channel_id, flags::FIN, buf[..n].to_vec()).encode(),
                            ));
                        }
                    }
                    if let Some(c) = &captured {
                        lock(c).push(&buf[..n]);
                    }
                    dirty.store(true, Ordering::SeqCst);
                }
                let _ = done_tx.send(());
            });
            done_rx
        };
        // Schreiben: Eingaben der Clients.
        let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Ok(data) = input_rx.recv() {
                if writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let (kill_tx, kill_rx) = oneshot::channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        lock(&self.shared.terms).insert(
            id.clone(),
            Term {
                view: view.clone(),
                pty: pty.clone(),
                screen: screen.clone(),
                credit: credit.clone(),
                input: input_tx,
                kill: Some(kill_tx),
                exit: exit_rx,
                captured,
                ended: false,
            },
        );
        let _ = self.out.send(Out::Event(
            Box::new(EventPayload::TerminalOpened(TerminalOpened {
                terminal_id: id.clone(),
                channel_id,
                kind: spec.kind,
                cmd: cmd_text,
                cols: spec.cols,
                rows: spec.rows,
                name: Some(name),
            })),
            actor,
        ));
        if self.shared.connected.load(Ordering::SeqCst) {
            let _ = self.out.send(Out::Up(TunnelUp::ChannelOpen {
                channel_id,
                kind: ChannelKind::Terminal,
                target: id.clone(),
            }));
        }
        let me = self.clone();
        tokio::spawn(async move {
            me.supervise(id, child, kill_rx, reader_done, dirty, exit_tx)
                .await;
        });
        Ok(view)
    }

    /// Wartet auf das Ende des Prozesses (bzw. `terminal.close`), macht Snapshots bei
    /// Aktivität und zum Schluss, meldet `terminal.closed`.
    async fn supervise(
        &self,
        id: String,
        mut child: beton_sandbox::SandboxedChild,
        kill: oneshot::Receiver<()>,
        reader_done: oneshot::Receiver<()>,
        dirty: Arc<AtomicBool>,
        exit_tx: watch::Sender<Option<Exit>>,
    ) {
        let mut tick = tokio::time::interval(self.snapshot_interval);
        tick.reset();
        let mut last_snapshot: Option<Instant> = None;
        let mut kill = Some(kill);
        let status = loop {
            tokio::select! {
                s = child.wait() => break s.ok(),
                r = async { match kill.as_mut() { Some(k) => k.await, None => std::future::pending().await } } => {
                    kill = None;
                    if r.is_ok() {
                        #[cfg(unix)]
                        let _ = child.signal_tree(libc::SIGHUP);
                        match tokio::time::timeout(CLOSE_GRACE, child.wait()).await {
                            Ok(s) => break s.ok(),
                            Err(_) => {
                                let _ = child.start_kill();
                                break child.wait().await.ok();
                            }
                        }
                    }
                    // Sender weg (Runner endet): weiter auf das Kind warten.
                }
                _ = tick.tick() => {
                    let due = last_snapshot.is_none_or(|t| t.elapsed() >= self.snapshot_interval);
                    if due && dirty.swap(false, Ordering::SeqCst) {
                        last_snapshot = Some(Instant::now());
                        self.snapshot(&id).await;
                    }
                }
            }
        };
        // Restliche Ausgabe lesen lassen (kurz), dann der letzte Zustand.
        let _ = tokio::time::timeout(Duration::from_secs(2), reader_done).await;
        let exit = status.map(exit_of);
        let _ = exit_tx.send(Some(exit.unwrap_or(Exit {
            exit_code: None,
            signal: None,
        })));
        self.snapshot(&id).await;
        // Nach `shutdown` meldet der Runner das Ende selbst. Terminals eines Harness bleiben
        // für `terminal/output` bis zur Freigabe.
        let t = {
            let mut terms = lock(&self.shared.terms);
            match terms.get_mut(&id) {
                None => return,
                Some(t) if t.view.kind == TerminalKind::Tool => {
                    t.ended = true;
                    t.credit.close();
                    (t.view.channel_id, None::<Term>)
                }
                Some(_) => {
                    let Some(t) = terms.remove(&id) else { return };
                    t.credit.close();
                    (t.view.channel_id, Some(t))
                }
            }
        };
        let _ = self.out.send(Out::Event(
            Box::new(EventPayload::TerminalClosed(TerminalClosed {
                terminal_id: id,
                exit_code: exit.and_then(|e| e.exit_code),
            })),
            None,
        ));
        let _ = self.out.send(Out::Up(TunnelUp::ChannelClose {
            channel_id: t.0,
            reason: Some("closed".into()),
        }));
    }

    /// Terminal eines Harness freigeben (ACP `terminal/release`): Prozess beenden, vergessen.
    pub async fn release(&self, id: &str) -> Result<(), TermError> {
        let running = self.with(id, |t| Ok(!t.ended))?;
        if running {
            self.close(id)?;
            let _ = tokio::time::timeout(CLOSE_GRACE * 3, self.wait_for_exit(id)).await;
        }
        lock(&self.shared.terms).remove(id);
        Ok(())
    }

    /// Bildschirmzustand als Blob (über den Server), dann `terminal.snapshot`.
    async fn snapshot(&self, id: &str) {
        use base64::Engine as _;
        let (bytes, cols, rows, cursor) = {
            let terms = lock(&self.shared.terms);
            let Some(t) = terms.get(id) else { return };
            let mut s = lock(&t.screen);
            let (cols, rows) = s.size();
            let c = s.cursor();
            (s.keyframe(), cols, rows, c)
        };
        let (reply, rx) = oneshot::channel();
        let req = SystemRequest {
            tool: "terminal.snapshot".into(),
            args: json!({"terminal_id": id, "data_b64": base64::engine::general_purpose::STANDARD.encode(&bytes)}),
            reply,
        };
        if self.calls.send(req).is_err() {
            return;
        }
        let Ok(Ok(v)) = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .unwrap_or(Ok(Err(Value::Null)))
        else {
            tracing::warn!(terminal = id, "Snapshot nicht gespeichert");
            return;
        };
        let Some(blob) = v["blob_ref"].as_str() else {
            return;
        };
        let _ = self.out.send(Out::Event(
            Box::new(EventPayload::TerminalSnapshot(TerminalSnapshot {
                terminal_id: id.to_owned(),
                blob_ref: blob.to_owned(),
                cols,
                rows,
                cursor: json!({"row": cursor.row, "col": cursor.col, "visible": cursor.visible}),
            })),
            None,
        ));
    }

    pub fn list(&self) -> Vec<View> {
        let mut v: Vec<View> = lock(&self.shared.terms)
            .values()
            .filter(|t| !t.ended)
            .map(|t| t.view.clone())
            .collect();
        v.sort_by_key(|t| t.channel_id);
        v
    }

    fn with<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut Term) -> Result<T, TermError>,
    ) -> Result<T, TermError> {
        let mut terms = lock(&self.shared.terms);
        let t = terms
            .get_mut(id)
            .ok_or_else(|| TermError::new("not_found", format!("Terminal {id}")))?;
        f(t)
    }

    /// Neue Fenstergröße: PTY (`SIGWINCH` an den Prozess) und Emulator.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<View, TermError> {
        self.with(id, |t| {
            t.pty
                .resize(cols, rows)
                .map_err(|e| TermError::new("validation_failed", e.to_string()))?;
            lock(&t.screen).resize(cols, rows);
            t.view.cols = cols;
            t.view.rows = rows;
            Ok(t.view.clone())
        })
    }

    pub fn rename(&self, id: &str, name: &str, actor: Option<Actor>) -> Result<View, TermError> {
        let name = Self::check_name(name)?;
        let view = self.with(id, |t| {
            t.view.name.clone_from(&name);
            Ok(t.view.clone())
        })?;
        let _ = self.out.send(Out::Event(
            Box::new(EventPayload::TerminalRenamed(TerminalRenamed {
                terminal_id: id.to_owned(),
                name,
            })),
            actor,
        ));
        Ok(view)
    }

    /// Schließen: `SIGHUP`, nach kurzer Frist `SIGKILL`; Snapshot und `terminal.closed` folgen.
    pub fn close(&self, id: &str) -> Result<(), TermError> {
        self.with(id, |t| {
            if let Some(k) = t.kill.take() {
                let _ = k.send(());
            }
            Ok(())
        })
    }

    /// Eingabe eines Clients (Binärframe mit `INPUT` und der Kanal-ID des Runners).
    pub fn input_frame(&self, bytes: &[u8]) {
        let Ok(frame) = Frame::decode(bytes) else {
            return;
        };
        if !frame.has(flags::INPUT) {
            return;
        }
        let terms = lock(&self.shared.terms);
        if let Some(t) = terms
            .values()
            .find(|t| t.view.channel_id == frame.channel_id)
        {
            let _ = t.input.send(frame.payload);
        }
    }

    /// Eingabe per Terminal-ID (ACP, Tests).
    pub fn write(&self, id: &str, data: &[u8]) -> Result<(), TermError> {
        self.with(id, |t| {
            t.input
                .send(data.to_vec())
                .map_err(|_| TermError::new("not_found", "Terminal beendet"))
        })
    }

    pub fn credit(&self, channel_id: u32, bytes: u64) {
        let terms = lock(&self.shared.terms);
        if let Some(t) = terms.values().find(|t| t.view.channel_id == channel_id) {
            t.credit.add(bytes);
        }
    }

    /// Keyframe für neue Zuschauer, in der Reihenfolge der Frames.
    pub fn keyframe(&self, channel_id: u32) {
        let terms = lock(&self.shared.terms);
        let Some(t) = terms.values().find(|t| t.view.channel_id == channel_id) else {
            return;
        };
        let mut s = lock(&t.screen);
        let bytes = s.keyframe();
        t.credit.consume(bytes.len());
        for f in Frame::fragments(channel_id, flags::KEYFRAME, &bytes) {
            let _ = self.out.send(Out::Frame(f.encode()));
        }
    }

    /// Nach dem (Wieder-)Verbinden: Kanäle anmelden; Frames erst im neuen Kredit.
    pub fn connected(&self) -> Vec<TunnelUp> {
        self.shared.connected.store(true, Ordering::SeqCst);
        let terms = lock(&self.shared.terms);
        terms
            .values()
            .map(|t| {
                t.credit.set_connected(true);
                TunnelUp::ChannelOpen {
                    channel_id: t.view.channel_id,
                    kind: ChannelKind::Terminal,
                    target: t.view.terminal_id.clone(),
                }
            })
            .collect()
    }

    /// Verbindung verloren: Terminals lesen frei weiter (nur in den Emulator).
    pub fn disconnected(&self) {
        self.shared.connected.store(false, Ordering::SeqCst);
        for t in lock(&self.shared.terms).values() {
            t.credit.set_connected(false);
        }
    }

    /// Rohausgabe (ACP `terminal/output`): Text, gekürzt?, Ende.
    pub fn output(&self, id: &str) -> Result<(String, bool, Option<Exit>), TermError> {
        self.with(id, |t| {
            let exit = *t.exit.borrow();
            let Some(c) = &t.captured else {
                return Ok((lock(&t.screen).text(), false, exit));
            };
            let c = lock(c);
            Ok((
                String::from_utf8_lossy(&c.bytes).into_owned(),
                c.truncated,
                exit,
            ))
        })
    }

    /// Wartet auf das Ende des Prozesses (ACP `terminal/wait_for_exit`).
    pub async fn wait_for_exit(&self, id: &str) -> Result<Exit, TermError> {
        let mut rx = self.with(id, |t| Ok(t.exit.clone()))?;
        let exit = rx
            .wait_for(Option::is_some)
            .await
            .map_err(|_| TermError::new("not_found", "Terminal beendet"))?;
        Ok(exit.unwrap_or(Exit {
            exit_code: None,
            signal: None,
        }))
    }

    /// Ist das Terminal (noch) bekannt?
    pub fn contains(&self, id: &str) -> bool {
        lock(&self.shared.terms).contains_key(id)
    }

    /// Alle Terminals beenden (Runner endet): sofort `SIGKILL`, `terminal.closed` ohne
    /// Snapshot (der letzte periodische bleibt).
    pub fn shutdown(&self) -> Vec<EventPayload> {
        let terms: Vec<(String, Term)> = lock(&self.shared.terms).drain().collect();
        terms
            .into_iter()
            .map(|(id, mut t)| {
                t.credit.close();
                if let Some(k) = t.kill.take() {
                    let _ = k.send(());
                }
                EventPayload::TerminalClosed(TerminalClosed {
                    terminal_id: id,
                    exit_code: None,
                })
            })
            .collect()
    }
}

/// Signalname für ACP (`SIGKILL` …).
fn signal_name(n: i32) -> String {
    match n {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        3 => "SIGQUIT".into(),
        9 => "SIGKILL".into(),
        13 => "SIGPIPE".into(),
        15 => "SIGTERM".into(),
        n => format!("SIG{n}"),
    }
}

/// Die Terminals als Host für Harnesses (ACP `terminal/*`, #326): Kommandos laufen am PTY in
/// Stufe 2 und erscheinen als Terminal `kind: tool` mit dem Agent als Akteur.
pub struct ToolTerminals {
    pub terminals: Terminals,
    pub actor: Actor,
}

/// Rohausgabe eines Harness-Terminals höchstens (ohne eigene Angabe).
const TOOL_OUTPUT_DEFAULT: usize = 1024 * 1024;

#[async_trait::async_trait]
impl beton_harness::terminal::TerminalHost for ToolTerminals {
    async fn create(&self, req: beton_harness::terminal::ToolTerminal) -> Result<String, String> {
        let name: String = std::iter::once(req.command.as_str())
            .chain(req.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME)
            .collect();
        let spec = OpenSpec {
            name: Some(if name.trim().is_empty() {
                "Agent".into()
            } else {
                name
            }),
            command: std::iter::once(req.command).chain(req.args).collect(),
            env: req.env,
            cwd: req.cwd,
            cols: 120,
            rows: 32,
            kind: TerminalKind::Tool,
            output_limit: Some(
                req.output_limit
                    .unwrap_or(TOOL_OUTPUT_DEFAULT)
                    .min(TOOL_OUTPUT_DEFAULT * 16),
            ),
        };
        self.terminals
            .open(spec, Some(self.actor.clone()))
            .map(|v| v.terminal_id)
            .map_err(|e| e.detail)
    }

    async fn output(&self, id: &str) -> Result<beton_harness::terminal::TerminalOutput, String> {
        let (output, truncated, exit) = self.terminals.output(id).map_err(|e| e.detail)?;
        Ok(beton_harness::terminal::TerminalOutput {
            output,
            truncated,
            exit: exit.map(|e| beton_harness::terminal::TerminalExit {
                exit_code: e.exit_code,
                signal: e.signal.map(signal_name),
            }),
        })
    }

    async fn wait_for_exit(
        &self,
        id: &str,
    ) -> Result<beton_harness::terminal::TerminalExit, String> {
        let e = self
            .terminals
            .wait_for_exit(id)
            .await
            .map_err(|e| e.detail)?;
        Ok(beton_harness::terminal::TerminalExit {
            exit_code: e.exit_code,
            signal: e.signal.map(signal_name),
        })
    }

    async fn kill(&self, id: &str) -> Result<(), String> {
        self.terminals.close(id).map_err(|e| e.detail)
    }

    async fn release(&self, id: &str) -> Result<(), String> {
        self.terminals.release(id).await.map_err(|e| e.detail)
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use beton_harness::terminal::TerminalHost as _;

    fn host() -> (ToolTerminals, mpsc::UnboundedReceiver<Out>) {
        let (out, rx) = mpsc::unbounded_channel();
        let (calls, _calls_rx) = mpsc::unbounded_channel();
        let dir = std::env::temp_dir();
        let terminals = Terminals::new(
            Sandbox::unsandboxed(beton_sandbox::Stage::Tools, None),
            dir,
            PathBuf::from("/bin/sh"),
            out,
            calls,
        );
        (
            ToolTerminals {
                terminals,
                actor: Actor::Agent {
                    id: None,
                    harness: "acp:test".into(),
                    agent_ref: None,
                },
            },
            rx,
        )
    }

    /// #326, SES-019: Ein Kommando des Agents läuft als Terminal `kind: tool`; Ausgabe und Ende
    /// bleiben bis zur Freigabe abrufbar, die Ausgabe ist begrenzt.
    #[tokio::test]
    async fn ses_019_agent_terminal_keeps_output_until_release() {
        let (h, mut rx) = host();
        let id = h
            .create(beton_harness::terminal::ToolTerminal {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "printf 'abcdefghij'; exit 4".into()],
                output_limit: Some(5),
                ..Default::default()
            })
            .await
            .unwrap();
        let exit = h.wait_for_exit(&id).await.unwrap();
        assert_eq!(exit.exit_code, Some(4));
        let out = h.output(&id).await.unwrap();
        assert_eq!(out.output, "fghij");
        assert!(out.truncated);
        assert_eq!(out.exit.unwrap().exit_code, Some(4));
        assert!(
            h.terminals.list().is_empty(),
            "beendet, nicht mehr gelistet"
        );
        h.release(&id).await.unwrap();
        assert!(h.output(&id).await.is_err());
        let mut opened = None;
        while let Ok(o) = rx.try_recv() {
            if let Out::Event(p, a) = o
                && let EventPayload::TerminalOpened(t) = *p
            {
                opened = Some((t, a));
            }
        }
        let (t, a) = opened.unwrap();
        assert_eq!(t.kind, TerminalKind::Tool);
        assert!(matches!(a, Some(Actor::Agent { .. })));
    }

    /// #326: `terminal/kill` beendet den Prozess, die Ausgabe bleibt.
    #[tokio::test]
    async fn ses_019_agent_terminal_kill_ends_the_process() {
        let (h, _rx) = host();
        let id = h
            .create(beton_harness::terminal::ToolTerminal {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "echo bereit; sleep 600".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        h.kill(&id).await.unwrap();
        let exit = tokio::time::timeout(Duration::from_secs(10), h.wait_for_exit(&id))
            .await
            .unwrap()
            .unwrap();
        assert!(exit.exit_code.is_some() || exit.signal.is_some());
        assert!(h.output(&id).await.unwrap().output.contains("bereit"));
    }
}
