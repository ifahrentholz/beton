//! Workspace-Terminals gegen einen echten Daemon mit dem Runner-Binary (SES-019, PROTO-007,
//! DATA-011, WEB-010, SBX-017, #202). Clients sind WebSocket-Verbindungen wie die Web-UI; den
//! Bildschirm rekonstruiert der Test mit demselben VT-Emulator (`beton-pty`).
//!
//! Homes mit Secrets sind Fixtures; kein Netz außer Loopback, keine Vendor-CLIs.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

mod support;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_proto::channel::{Frame, HEADER_LEN, flags};
use beton_pty::Screen;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

const IDLE: &str = "turns:\n  - emit:\n      - { message: \"ok\" }\n";

struct D {
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
    work: PathBuf,
}

#[derive(Default)]
struct Opts {
    sandboxed: bool,
    env: Vec<(&'static str, String)>,
}

async fn daemon(opts: Opts) -> D {
    let dir = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let work = dir.path().join("work").canonicalize().unwrap_or_else(|_| {
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        dir.path().join("work").canonicalize().unwrap()
    });
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let runners = dir.path().join("runners");
    let mut inherit: std::collections::BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| !k.ends_with("_API_KEY") && k != "ANTHROPIC_AUTH_TOKEN")
        .collect();
    for (k, v) in &opts.env {
        inherit.insert((*k).into(), v.clone());
    }
    let sandboxed = opts.sandboxed;
    let daemon = start_with(cfg, store, move |mut r| {
        if sandboxed {
            r.sessions.proxy_ca = Some(std::sync::Arc::new(support::TestCa));
            r.sessions.sandbox =
                beton_sandbox::SandboxSpec::from_yaml("allow_network: true").unwrap();
        } else {
            r.sessions.sandbox = support::unsandboxed();
        }
        r.sessions.sandbox_launcher = Some(runner_bin().into());
        r.sessions.provider = std::sync::Arc::new(
            LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
        );
        r.sessions.dev = true;
        r
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    let addr = daemon.addrs[0];
    D {
        daemon,
        token,
        addr,
        _dir: dir,
        work,
    }
}

impl D {
    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value, String) {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let body = support::confirm_unsandboxed(method, path, body)
            .map(|b| b.to_string())
            .unwrap_or_default();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            port = self.addr.port(),
            token = self.token,
            len = body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let payload = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            let mut out = String::new();
            let mut rest = payload;
            while let Some((size, tail)) = rest.split_once("\r\n") {
                let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                if n == 0 {
                    break;
                }
                out.push_str(&tail[..n.min(tail.len())]);
                rest = tail.get(n + 2..).unwrap_or("");
            }
            out
        } else {
            payload.to_owned()
        };
        (
            status,
            serde_json::from_str(&payload).unwrap_or(Value::Null),
            head.to_owned(),
        )
    }

    async fn blob(&self, session: &str, blob: &str) -> Vec<u8> {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let req = format!(
            "GET /v1/sessions/{session}/blobs/{blob} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n",
            port = self.addr.port(),
            token = self.token,
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let at = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let head = String::from_utf8_lossy(&buf[..at]).to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(!head.contains("chunked"), "{head}");
        buf[at..].to_vec()
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let mut all = Vec::new();
        let mut after = 0;
        loop {
            let (status, page, _) = self
                .http(
                    "GET",
                    &format!("/v1/sessions/{id}/events?after_seq={after}&limit=200"),
                    None,
                )
                .await;
            assert_eq!(status, 200, "{page}");
            let items = page["items"].as_array().unwrap().clone();
            if items.is_empty() {
                return all;
            }
            after = items.last().unwrap()["seq"].as_u64().unwrap();
            all.extend(items);
        }
    }

    async fn wait_event(&self, id: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let events = self.events(id).await;
            if let Some(e) = events.iter().find(|e| pred(e)) {
                return e.clone();
            }
            assert!(
                Instant::now() < deadline,
                "Event fehlt; Log: {:?}",
                events
                    .iter()
                    .map(|e| format!("{} {}", e["type"], e["payload"]))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Neue Session mit dem Fake-Harness; wartet auf `idle`.
    async fn session(&self) -> String {
        let sc = self.work.join(".scenario.yaml");
        std::fs::write(&sc, IDLE).unwrap();
        let (status, body, _) = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": "fake", "cwd": self.work, "harness_opts": {"scenario": sc}})),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        let id = body["id"].as_str().unwrap().to_owned();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let s = self
                .http("GET", &format!("/v1/sessions/{id}"), None)
                .await
                .1;
            if s["status"] == "idle" {
                return id;
            }
            assert!(
                Instant::now() < deadline && s["status"] != "failed",
                "Session startet nicht: {s}\n{:#?}",
                self.events(&id).await
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn open(&self, id: &str, body: Value) -> Value {
        let (status, t, head) = self
            .http("POST", &format!("/v1/sessions/{id}/terminals"), Some(body))
            .await;
        assert_eq!(status, 201, "{t}");
        let location = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("location")
                    .then(|| v.trim().to_owned())
            })
            .unwrap_or_default();
        assert_eq!(
            location,
            format!(
                "/v1/sessions/{id}/terminals/{}",
                t["terminal_id"].as_str().unwrap()
            ),
            "{head}"
        );
        t
    }

    async fn ws(&self) -> Ws {
        let mut req = format!("ws://127.0.0.1:{}/v1/ws", self.addr.port())
            .into_client_request()
            .unwrap();
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        req.headers_mut()
            .insert("sec-websocket-protocol", "beton.v1".parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        send(
            &mut ws,
            json!({"t": "hello", "protocol": "1.1", "client": {"kind": "test", "version": "0"}}),
        )
        .await;
        assert_eq!(text(&mut ws).await["t"], "welcome");
        ws
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

async fn text(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(20), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(&t).unwrap(),
            Ok(Some(Ok(_))) => continue,
            other => panic!("keine Steuernachricht: {other:?}"),
        }
    }
}

/// Ein Zuschauer eines Terminals mit eigenem Emulator.
struct Viewer {
    ws: Ws,
    channel: u32,
    screen: Screen,
    raw: Vec<u8>,
    keyframes: usize,
}

impl Viewer {
    async fn attach(d: &D, session: &str, terminal: &str, cols: u16, rows: u16) -> Self {
        let mut ws = d.ws().await;
        send(
            &mut ws,
            json!({"t": "open_channel", "id": "o", "session_id": session, "kind": "terminal", "ref": terminal}),
        )
        .await;
        let opened = text(&mut ws).await;
        assert_eq!(opened["t"], "channel_opened", "{opened}");
        let channel = opened["channel_id"].as_u64().unwrap() as u32;
        send(
            &mut ws,
            json!({"t": "credit", "channel_id": channel, "bytes": 64 * 1024 * 1024}),
        )
        .await;
        Self {
            ws,
            channel,
            screen: Screen::new(cols, rows),
            raw: Vec::new(),
            keyframes: 0,
        }
    }

    fn take(&mut self, f: Frame) {
        if f.has(flags::KEYFRAME) {
            self.keyframes += 1;
        }
        self.screen.feed(&f.payload);
        self.raw.extend_from_slice(&f.payload);
    }

    /// Liest Frames, bis `pred` auf dem Bildschirm (samt Scrollback) gilt.
    async fn until(&mut self, what: &str, pred: impl Fn(&mut Screen) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !pred(&mut self.screen) {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, self.ws.next()).await {
                Ok(Some(Ok(Message::Binary(b)))) => {
                    let f = Frame::decode(&b).unwrap();
                    assert_eq!(f.channel_id, self.channel);
                    self.take(f);
                }
                Ok(Some(Ok(_))) => {}
                other => panic!(
                    "{what} fehlt ({other:?}); Bildschirm:\n{}",
                    self.screen.text()
                ),
            }
        }
    }

    async fn input(&mut self, data: &[u8]) {
        self.ws
            .send(Message::Binary(
                Frame::new(self.channel, flags::INPUT | flags::FIN, data.to_vec())
                    .encode()
                    .into(),
            ))
            .await
            .unwrap();
    }
}

fn all_text(s: &mut Screen) -> String {
    let mut lines = s.scrollback_text();
    lines.push(s.text());
    lines.join("\n")
}

/// SES-019 AC1: Ein im Terminal gestarteter Langläufer (Dev-Server) läuft weiter, wenn alle
/// Clients trennen, und ist beim Re-Attach mit Scrollback sichtbar; neue Ausgabe folgt live.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ses_019_ac1_long_runner_survives_disconnect_and_reattach_shows_scrollback() {
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    let t = d
        .open(
            &id,
            json!({"name": "Dev-Server", "command": ["/bin/sh", "-c", "i=0; while :; do echo \"tick $i\"; i=$((i+1)); sleep 0.05; done"], "cols": 80, "rows": 10}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    assert_eq!(t["name"], "Dev-Server");
    {
        let mut v = Viewer::attach(&d, &id, &term, 80, 10).await;
        v.until("tick 5", |s| all_text(s).contains("tick 5")).await;
        assert!(v.keyframes >= 1, "erst ein Keyframe");
    }
    // Alle Clients weg; der Prozess läuft weiter.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (_, list, _) = d
        .http("GET", &format!("/v1/sessions/{id}/terminals"), None)
        .await;
    assert_eq!(list["items"][0]["terminal_id"], term.as_str(), "{list}");
    let mut v = Viewer::attach(&d, &id, &term, 80, 10).await;
    // Das Keyframe bringt den Verlauf (Scrollback) mit, danach geht es live weiter.
    v.until("Verlauf", |s| all_text(s).contains("tick 1\n"))
        .await;
    let seen = all_text(&mut v.screen);
    let last: u32 = seen
        .lines()
        .filter_map(|l| l.trim().strip_prefix("tick ")?.parse().ok())
        .max()
        .unwrap();
    assert!(last >= 20, "lief während der Trennung weiter: {last}");
    v.until("neue Ausgabe", move |s| {
        all_text(s).contains(&format!("tick {}", last + 3))
    })
    .await;
    // Terminal-Bytes stehen nie im Log.
    let events = d.events(&id).await;
    assert!(!events.iter().any(|e| e["type"] == "terminal.output"));
    let opened = events
        .iter()
        .find(|e| e["type"] == "terminal.opened")
        .unwrap();
    assert_eq!(opened["actor"]["kind"], "user");
    assert_eq!(opened["payload"]["kind"], "user");
    d.daemon.shutdown().await;
}

/// DATA-011 AC2: Ein später attachender Client sieht für ein geschlossenes Terminal den letzten
/// Bildschirmzustand (`terminal.snapshot` als Blob); `terminal.closed` trägt den Exit-Code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_011_ac2_late_client_sees_the_last_screen_of_a_closed_terminal() {
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/sh", "-c", "printf '\\033[1mfinaler Zustand\\033[0m\\n'; exit 3"], "cols": 60, "rows": 8}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    let closed = d
        .wait_event(&id, |e| {
            e["type"] == "terminal.closed" && e["payload"]["terminal_id"] == term.as_str()
        })
        .await;
    assert_eq!(closed["payload"]["exit_code"], 3);
    let events = d.events(&id).await;
    let snap = events
        .iter()
        .rev()
        .find(|e| e["type"] == "terminal.snapshot" && e["payload"]["terminal_id"] == term.as_str())
        .expect("Snapshot beim Schließen");
    assert!(
        snap["seq"].as_u64() < closed["seq"].as_u64(),
        "Snapshot vor dem Ende"
    );
    let p = &snap["payload"];
    assert_eq!(
        (p["cols"].as_u64(), p["rows"].as_u64()),
        (Some(60), Some(8))
    );
    let bytes = d.blob(&id, p["blob_ref"].as_str().unwrap()).await;
    let mut screen = Screen::new(60, 8);
    screen.feed(&bytes);
    assert!(
        screen.text().contains("finaler Zustand"),
        "{}",
        screen.text()
    );
    // Der Kanal eines beendeten Terminals ist zu.
    let mut ws = d.ws().await;
    send(
        &mut ws,
        json!({"t": "open_channel", "id": "o", "session_id": id, "kind": "terminal", "ref": term}),
    )
    .await;
    let r = text(&mut ws).await;
    assert_eq!(r["t"], "nack", "{r}");
    d.daemon.shutdown().await;
}

/// DATA-011 AC1: Nach 100 MB Terminal-Ausgabe enthält das Log keine `terminal.output`-Events
/// und höchstens 6 Snapshots je Minute Aktivität (plus den beim Schließen).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_011_ac1_100_mb_output_leaves_no_bytes_and_few_snapshots_in_the_log() {
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    let started = Instant::now();
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/sh", "-c", "yes AUSGABEZEILEDIENIEINSLOGGEHT | tr A-Z a-z | head -c 100000000; echo; echo fertig"], "cols": 120, "rows": 40}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    d.wait_event(&id, |e| {
        e["type"] == "terminal.closed" && e["payload"]["terminal_id"] == term.as_str()
    })
    .await;
    let minutes = started.elapsed().as_secs_f64() / 60.0;
    let events = d.events(&id).await;
    assert!(!events.iter().any(|e| e["type"] == "terminal.output"));
    let snapshots = events
        .iter()
        .filter(|e| e["type"] == "terminal.snapshot")
        .count();
    let allowed = (6.0 * minutes.max(1.0 / 6.0)).ceil() as usize + 1;
    println!("100 MB in {minutes:.2} min, {snapshots} Snapshots (erlaubt {allowed})");
    assert!(snapshots >= 1, "mindestens der beim Schließen");
    assert!(snapshots <= allowed, "{snapshots} > {allowed}");
    // Kein Event trägt die Ausgabe.
    let log = serde_json::to_string(&events).unwrap();
    assert!(!log.contains("ausgabezeiledienieinsloggeht"));
    d.daemon.shutdown().await;
}

/// PROTO-007 AC1 (über den Runner): 50 MB Ausgabe an einen auf 100 KB/s gedrosselten Client
/// erhöhen den Speicher des Servers um weniger als 2 MB; das PTY-Lesen pausiert (der Prozess
/// kommt nicht weiter, solange der Client nicht liest). Danach liest er schnell und bekommt
/// alles.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proto_007_ac1_slow_client_pauses_pty_reading_and_bounds_server_memory() {
    const TOTAL: usize = 50 * 1024 * 1024;
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    // Erst auf den Zuschauer warten, dann ausgeben (ohne Zuschauer liest der Runner frei).
    let marker = d.work.join("los");
    let script = format!(
        "while [ ! -e {m} ]; do sleep 0.05; done; head -c {TOTAL} /dev/zero | tr '\\000' x; echo; echo ENDE",
        m = marker.display()
    );
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/sh", "-c", script], "cols": 200, "rows": 50}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    let hub = d.daemon.runtime.channels.clone();
    let mut ws = d.ws().await;
    send(
        &mut ws,
        json!({"t": "open_channel", "id": "o", "session_id": id, "kind": "terminal", "ref": term}),
    )
    .await;
    let opened = text(&mut ws).await;
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": 64 * 1024}),
    )
    .await;
    std::fs::write(&marker, "").unwrap();
    let mut received = 0usize;
    let mut peak = 0usize;
    let slow_until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < slow_until {
        send(
            &mut ws,
            json!({"t": "credit", "channel_id": cid, "bytes": 10_000}),
        )
        .await;
        let tick = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(Ok(m))) = tokio::time::timeout_at(tick, ws.next()).await {
            if let Message::Binary(b) = m {
                received += b.len() - HEADER_LEN;
            }
        }
        peak = peak.max(hub.buffered_bytes());
    }
    println!("gedrosselt: {received} Bytes, Spitze im Server {peak} Bytes");
    assert!(peak < 2 * 1024 * 1024, "Server hielt {peak} Bytes");
    assert!(
        received < 2 * 1024 * 1024,
        "Kredit bremst den Sender: {received} Bytes in 4 s"
    );
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": 4 * TOTAL as u64}),
    )
    .await;
    let mut tail = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    while !String::from_utf8_lossy(&tail).contains("ENDE") {
        assert!(
            Instant::now() < deadline,
            "bei {received} Bytes stehen geblieben"
        );
        match tokio::time::timeout(Duration::from_secs(30), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                received += b.len() - HEADER_LEN;
                tail.extend_from_slice(&b[HEADER_LEN..]);
                if tail.len() > 64 {
                    tail.drain(..tail.len() - 64);
                }
                peak = peak.max(hub.buffered_bytes());
            }
            Ok(Some(Ok(_))) => {}
            other => panic!("Strom endete bei {received}: {other:?}"),
        }
    }
    // PTY-Zeilenumbrüche (\r\n) und das Keyframe kommen dazu.
    assert!(received >= TOTAL, "{received}");
    assert!(peak < 2 * 1024 * 1024, "Server hielt {peak} Bytes");
    d.daemon.shutdown().await;
}

/// WEB-010 AC2: Eine neue Fenstergröße erreicht das PTY (`stty size`), und ein Vollbild-
/// programm (`vim`) zeichnet in der neuen Größe (Statuszeile in der letzten Zeile).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_010_ac2_resize_reaches_the_pty_and_vim_redraws() {
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    let t = d
        .open(&id, json!({"command": ["/bin/sh"], "cols": 80, "rows": 24}))
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    let mut v = Viewer::attach(&d, &id, &term, 80, 24).await;
    v.input(b"stty size\r").await;
    v.until("24 80", |s| s.text().contains("24 80")).await;
    let (status, r, _) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/terminals/{term}/resize"),
            Some(json!({"cols": 100, "rows": 30})),
        )
        .await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(
        (r["cols"].as_u64(), r["rows"].as_u64()),
        (Some(100), Some(30))
    );
    v.screen.resize(100, 30);
    v.input(b"clear; stty size\r").await;
    v.until("30 100", |s| s.text().contains("30 100")).await;
    // vim in der neuen Größe; nach einem weiteren Resize zeichnet es neu.
    let vim = ["/usr/bin/vim", "/bin/vim", "/usr/local/bin/vim"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .expect("vim fehlt (auf macOS und den Ubuntu-Runnern vorhanden)");
    std::fs::write(d.work.join("notiz.txt"), "Zeile eins\n").unwrap();
    v.input(format!("{vim} -u NONE -N -n -c 'set laststatus=2' notiz.txt\r").as_bytes())
        .await;
    v.until("vim", |s| s.text().contains("Zeile eins")).await;
    let (status, _, _) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/terminals/{term}/resize"),
            Some(json!({"cols": 60, "rows": 20})),
        )
        .await;
    assert_eq!(status, 200);
    v.screen.resize(60, 20);
    // Statuszeile (Dateiname) in Zeile 19 von 20, Text oben.
    v.until("Statuszeile nach dem Resize", |s| {
        let text = s.text();
        let lines: Vec<&str> = text.lines().collect();
        lines.len() >= 19 && lines[18].contains("notiz.txt") && lines[0].starts_with("Zeile eins")
    })
    .await;
    v.input(b"\x1b:q!\r").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    v.input(b"echo zurueck\r").await;
    v.until("Shell", |s| s.text().contains("zurueck")).await;
    d.daemon.shutdown().await;
}

/// #202, SBX-001 AC4: Jeder Start eines Terminals meldet genau ein
/// `sandbox.started {stage: tools}` im Session-Log; SES-019 AC3, SBX-017 AC2: In einem
/// User-Terminal mit Default-Sandbox scheitert `cat ~/.ssh/id_rsa` (Fixture-Home), Netz geht
/// nur über den Proxy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ses_019_ac3_sbx_017_ac2_user_terminal_runs_in_stage_two() {
    let home = tempfile::Builder::new()
        .prefix("bthome")
        .tempdir_in("/tmp")
        .unwrap();
    let home_path = home.path().canonicalize().unwrap();
    std::fs::create_dir_all(home_path.join(".ssh")).unwrap();
    std::fs::write(home_path.join(".ssh/id_rsa"), "FIXTURE-SSH-RSA-KEY").unwrap();
    // Ein Dienst auf Loopback, den Stufe 2 nicht direkt erreichen darf.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let _ = std::io::Write::write_all(&mut s, b"DIREKT-ERREICHT\n");
        }
    });
    let d = daemon(Opts {
        sandboxed: true,
        env: vec![("HOME", home_path.display().to_string())],
    })
    .await;
    let id = d.session().await;
    let before = d
        .events(&id)
        .await
        .iter()
        .filter(|e| e["type"] == "sandbox.started" && e["payload"]["stage"] == "tools")
        .count();
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/sh"], "cols": 100, "rows": 30}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    let mut v = Viewer::attach(&d, &id, &term, 100, 30).await;
    v.input(
        format!(
            "cat {}/.ssh/id_rsa; echo \"rc=$?\"; curl -s --noproxy '*' --max-time 3 http://127.0.0.1:{port}/; echo \"net=$?\"\r",
            home_path.display()
        )
        .as_bytes(),
    )
    .await;
    v.until("net=", |s| s.text().lines().any(|l| l.starts_with("net=")))
        .await;
    let text = all_text(&mut v.screen);
    assert!(!text.contains("FIXTURE-SSH-RSA-KEY"), "{text}");
    assert!(text.lines().any(|l| l == "rc=1"), "{text}");
    assert!(!text.contains("DIREKT-ERREICHT"), "{text}");
    assert!(!text.lines().any(|l| l == "net=0"), "{text}");
    let started: Vec<Value> = d
        .events(&id)
        .await
        .into_iter()
        .filter(|e| e["type"] == "sandbox.started" && e["payload"]["stage"] == "tools")
        .collect();
    assert_eq!(
        started.len(),
        before + 1,
        "genau ein Start für das Terminal"
    );
    let p = &started.last().unwrap()["payload"];
    assert_ne!(p["backend"], "none");
    assert_eq!(p["degraded"], false, "{p}");
    d.daemon.shutdown().await;
}

/// WEB-010 AC1: Tastendruck-zu-Echo-Latenz lokal ≤ 30 ms (p95), gemessen vom Client über
/// Server, Tunnel, Runner und PTY zurück. Läuft im Release-Build als eigener CI-Schritt
/// (`cargo test -p beton-runner --release --test terminals web_010_ac1 -- --ignored`). Eine
/// erste Runde wärmt auf und zählt nicht; gewertet wird das p95 über alle Tastendrücke der
/// folgenden fünf Runden (5 × 100), damit ein kurzer Stillstand des geteilten Runners weniger
/// als 5 % trifft, eine echte Verlangsamung aber alle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Benchmark; im Release-Build ausführen"]
async fn web_010_ac1_keystroke_to_echo_p95_at_most_30ms() {
    const ROUNDS: usize = 5;
    const KEYS: usize = 100;
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    // `cat` im kanonischen Modus: das Echo kommt von der Line-Discipline des PTY.
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/cat"], "cols": 80, "rows": 24}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    let mut v = Viewer::attach(&d, &id, &term, 80, 24).await;
    v.until("Keyframe", |_| true).await;
    let mut all = Vec::new();
    for round in 0..=ROUNDS {
        let mut lat = Vec::with_capacity(KEYS);
        for i in 0..KEYS {
            let key = [b'a' + (i % 26) as u8];
            let sent = Instant::now();
            v.input(&key).await;
            loop {
                match tokio::time::timeout(Duration::from_secs(5), v.ws.next()).await {
                    Ok(Some(Ok(Message::Binary(b)))) => {
                        if b[HEADER_LEN..].contains(&key[0]) {
                            break;
                        }
                    }
                    Ok(Some(Ok(_))) => {}
                    other => panic!("kein Echo: {other:?}"),
                }
            }
            lat.push(sent.elapsed());
            if i % 40 == 39 {
                v.input(b"\r").await;
            }
        }
        lat.sort();
        let p95 = lat[lat.len() * 95 / 100];
        if round == 0 {
            println!("Aufwärmen (nicht gewertet): p95 {p95:?}");
        } else {
            println!(
                "Runde {round}: Median {:?}, p95 {p95:?}",
                lat[lat.len() / 2]
            );
            all.extend(lat);
        }
    }
    all.sort();
    let p95 = all[all.len() * 95 / 100];
    println!("p95 über {ROUNDS} Runden: {p95:?}");
    assert!(p95 <= Duration::from_millis(30), "p95 {p95:?} > 30 ms");
    d.daemon.shutdown().await;
}

/// SES-019: Umbenennen und Schließen; beides steht im Log, der Prozessbaum endet.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ses_019_rename_and_close_a_terminal() {
    let d = daemon(Opts::default()).await;
    let id = d.session().await;
    let t = d
        .open(
            &id,
            json!({"command": ["/bin/sh", "-c", "sleep 600"], "cols": 80, "rows": 24}),
        )
        .await;
    let term = t["terminal_id"].as_str().unwrap().to_owned();
    assert!(t["name"].as_str().unwrap().starts_with("Shell"), "{t}");
    let (status, r, _) = d
        .http(
            "PATCH",
            &format!("/v1/sessions/{id}/terminals/{term}"),
            Some(json!({"name": "Build"})),
        )
        .await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["name"], "Build");
    let (status, _, _) = d
        .http(
            "PATCH",
            &format!("/v1/sessions/{id}/terminals/{term}"),
            Some(json!({"name": "  "})),
        )
        .await;
    assert_eq!(status, 400, "validation_failed");
    let (status, _, _) = d
        .http(
            "DELETE",
            &format!("/v1/sessions/{id}/terminals/{term}"),
            None,
        )
        .await;
    assert_eq!(status, 204);
    d.wait_event(&id, |e| e["type"] == "terminal.closed").await;
    let events = d.events(&id).await;
    let renamed = events
        .iter()
        .find(|e| e["type"] == "terminal.renamed")
        .unwrap();
    assert_eq!(renamed["payload"]["name"], "Build");
    let (_, list, _) = d
        .http("GET", &format!("/v1/sessions/{id}/terminals"), None)
        .await;
    assert_eq!(list["items"], json!([]));
    // Unbekannte Terminals: 404.
    let (status, _, _) = d
        .http(
            "DELETE",
            &format!("/v1/sessions/{id}/terminals/{term}"),
            None,
        )
        .await;
    assert_eq!(status, 404);
    d.daemon.shutdown().await;
}
