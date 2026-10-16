//! Binärkanäle im selben WebSocket gegen einen echten Daemon (PROTO-007, DATA-011 AC3,
//! SES-019 AC2). Quellen sind synthetische Kanäle (Terminal, Browser, Audio); die echten
//! Quellen (PTY des Runners, Browser BRW-005, Spracheingabe VOI-004) nutzen denselben Weg.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_proto::channel::{ChannelAccess, ChannelKind, Frame, flags};
use beton_server::app::Runtime;
use beton_server::channels::ChannelBackend;
use beton_server::security::Authenticated;
use beton_server::ws::Authorizer;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
    daemon: Daemon,
    dir: tempfile::TempDir,
    token: String,
    session: SessionRecord,
}

async fn fixture(customize: impl FnOnce(Runtime) -> Runtime) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let daemon = start_with(cfg, store.clone(), customize).await.unwrap();
    let local = store.ensure_local().await.unwrap();
    let session = store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    Fixture {
        daemon,
        dir,
        token,
        session,
    }
}

impl Fixture {
    async fn hello(&self) -> Ws {
        let port = self.daemon.addrs[0].port();
        let mut req = format!("ws://127.0.0.1:{port}/v1/ws")
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
        let welcome = text(&mut ws).await;
        assert_eq!(welcome["t"], "welcome", "{welcome}");
        assert_eq!(welcome["protocol"], "1.1");
        ws
    }

    fn source(
        &self,
        kind: ChannelKind,
        target: &str,
    ) -> (Arc<Source>, Arc<beton_server::channels::Relay>) {
        let source = Arc::new(Source::default());
        let relay =
            self.daemon
                .runtime
                .channels
                .register(self.session.id, kind, target, source.clone());
        (source, relay)
    }

    async fn open(&self, ws: &mut Ws, kind: &str, target: &str) -> Value {
        send(
            ws,
            json!({"t": "open_channel", "id": "o1", "session_id": self.session.id, "kind": kind, "ref": target}),
        )
        .await;
        text(ws).await
    }
}

/// Synthetische Quelle: zählt Kredit, Keyframe-Anforderungen und Eingaben.
#[derive(Default)]
struct Source {
    credit: AtomicU64,
    keyframes: AtomicUsize,
    input: Mutex<Vec<u8>>,
}

impl ChannelBackend for Source {
    fn credit(&self, bytes: u64) {
        self.credit.fetch_add(bytes, Ordering::SeqCst);
    }
    fn keyframe(&self) {
        self.keyframes.fetch_add(1, Ordering::SeqCst);
    }
    fn input(&self, payload: Vec<u8>) -> bool {
        self.input.lock().unwrap().extend(payload);
        true
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// Nächste Steuernachricht (Binärframes dazwischen werden übersprungen).
async fn text(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(&t).unwrap(),
            Ok(Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_)))) => continue,
            other => panic!("keine Steuernachricht: {other:?}"),
        }
    }
}

/// Nächstes Binärframe.
async fn binary(ws: &mut Ws) -> Frame {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => return Frame::decode(&b).unwrap(),
            Ok(Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_)))) => continue,
            other => panic!("kein Binärframe: {other:?}"),
        }
    }
}

/// Nur lesen (z. B. Rolle `view` bzw. `comment_approve` ab M4, COL-001).
struct ReadOnly;

impl Authorizer for ReadOnly {
    fn can_read(&self, _: Authenticated, _: &SessionRecord) -> bool {
        true
    }
    fn channel_access(
        &self,
        _: Authenticated,
        _: &SessionRecord,
        _: ChannelKind,
    ) -> Option<ChannelAccess> {
        Some(ChannelAccess::Read)
    }
    fn can_drive(&self, _: Authenticated, _: &SessionRecord) -> bool {
        false
    }
}

struct Nobody;

impl Authorizer for Nobody {
    fn can_read(&self, _: Authenticated, _: &SessionRecord) -> bool {
        false
    }
}

/// PROTO-007: Kanal öffnen, Keyframe vor den laufenden Frames, Frames erst nach Kredit, mit
/// der Kanal-ID des Clients; lokal hat der Owner `drive`, Eingaben erreichen die Quelle.
#[tokio::test]
async fn proto_007_owner_opens_a_terminal_channel_and_drives_it() {
    let f = fixture(|r| r).await;
    let (source, relay) = f.source(ChannelKind::Terminal, "term_1");
    let mut ws = f.hello().await;
    let opened = f.open(&mut ws, "terminal", "term_1").await;
    assert_eq!(opened["t"], "channel_opened", "{opened}");
    assert_eq!(opened["access"], "drive");
    assert_eq!(opened["window"], 262_144);
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    // Neuer Zuschauer: Keyframe angefordert.
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.keyframes.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": 65536}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    relay.push(Frame::new(1, flags::FIN, b"vor dem keyframe".to_vec()));
    relay.push(Frame::new(
        1,
        flags::KEYFRAME | flags::FIN,
        b"\x1bcBILD".to_vec(),
    ));
    relay.push(Frame::new(1, flags::FIN, b"$ ".to_vec()));
    let k = binary(&mut ws).await;
    assert_eq!(k.channel_id, cid);
    assert!(k.has(flags::KEYFRAME));
    assert_eq!(k.payload, b"\x1bcBILD");
    assert_eq!(binary(&mut ws).await.payload, b"$ ");
    // Eingabe als Binärframe mit INPUT.
    ws.send(Message::Binary(
        Frame::new(cid, flags::INPUT | flags::FIN, b"ls\r".to_vec())
            .encode()
            .into(),
    ))
    .await
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.input.lock().unwrap().as_slice() != b"ls\r" {
        assert!(Instant::now() < deadline, "Eingabe kam nicht an");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Schließen: Bestätigung, danach keine Frames mehr.
    send(&mut ws, json!({"t": "close_channel", "channel_id": cid})).await;
    let closed = loop {
        let m = text(&mut ws).await;
        if m["t"] != "credit" {
            break m;
        }
        assert_eq!(m["bytes"], 3, "Kredit für die Eingabe zurück");
    };
    assert_eq!(closed["t"], "channel_closed");
    assert_eq!(relay.subscribers(), 0);
}

/// PROTO-007 AC2, SES-019 AC2: Ein Client mit Leserecht (Rolle `view` bzw. `comment_approve`,
/// ab M4) öffnet den Terminal-Kanal lesend; Frames mit `INPUT` werden verworfen und mit
/// `nack 403` beantwortet, die Verbindung bleibt offen.
#[tokio::test]
async fn proto_007_ac2_ses_019_ac2_read_only_client_input_is_dropped_with_403() {
    let f = fixture(|mut r| {
        r.authorizer = Arc::new(ReadOnly);
        r
    })
    .await;
    let (source, relay) = f.source(ChannelKind::Terminal, "term_1");
    let mut ws = f.hello().await;
    let opened = f.open(&mut ws, "terminal", "term_1").await;
    assert_eq!(opened["t"], "channel_opened", "{opened}");
    assert_eq!(opened["access"], "read");
    assert_eq!(opened["window"], 0);
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": 65536}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    relay.push(Frame::new(1, flags::KEYFRAME | flags::FIN, b"K".to_vec()));
    assert_eq!(binary(&mut ws).await.payload, b"K", "Lesen geht");
    ws.send(Message::Binary(
        Frame::new(cid, flags::INPUT | flags::FIN, b"rm -rf ~\r".to_vec())
            .encode()
            .into(),
    ))
    .await
    .unwrap();
    let nack = text(&mut ws).await;
    assert_eq!(nack["t"], "nack", "{nack}");
    assert_eq!(nack["problem"]["status"], 403);
    assert_eq!(nack["id"], format!("channel:{cid}"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(source.input.lock().unwrap().is_empty(), "verworfen");
    // Verbindung bleibt offen.
    relay.push(Frame::new(1, flags::FIN, b"weiter".to_vec()));
    assert_eq!(binary(&mut ws).await.payload, b"weiter");
    // Terminals steuern (öffnen, Größe, schließen, umbenennen) geht nur mit `drive`.
    for (name, args) in [
        ("terminal.open", json!({"cols": 80, "rows": 24})),
        (
            "terminal.resize",
            json!({"terminal_id": "term_1", "cols": 80, "rows": 24}),
        ),
        (
            "terminal.rename",
            json!({"terminal_id": "term_1", "name": "x"}),
        ),
        ("terminal.close", json!({"terminal_id": "term_1"})),
    ] {
        send(
            &mut ws,
            json!({"t": "cmd", "id": name, "session_id": f.session.id, "name": name, "args": args}),
        )
        .await;
        let r = text(&mut ws).await;
        assert_eq!(r["t"], "nack", "{name}: {r}");
        assert_eq!(r["problem"]["status"], 403, "{name}: {r}");
    }
}

/// PROTO-007: ohne Leserecht kein Kanal, unbekannte Quelle 404, Frames ohne offenen Kanal
/// werden mit `nack` beantwortet.
#[tokio::test]
async fn proto_007_open_requires_read_and_an_existing_source() {
    let f = fixture(|mut r| {
        r.authorizer = Arc::new(Nobody);
        r
    })
    .await;
    f.source(ChannelKind::Terminal, "term_1");
    let mut ws = f.hello().await;
    let r = f.open(&mut ws, "terminal", "term_1").await;
    assert_eq!(r["t"], "nack", "{r}");
    assert_eq!(r["problem"]["status"], 403);

    let f = fixture(|r| r).await;
    let mut ws = f.hello().await;
    let r = f.open(&mut ws, "terminal", "term_gibt_es_nicht").await;
    assert_eq!(r["t"], "nack", "{r}");
    assert_eq!(r["problem"]["status"], 404);
    ws.send(Message::Binary(
        Frame::new(99, flags::INPUT | flags::FIN, b"x".to_vec())
            .encode()
            .into(),
    ))
    .await
    .unwrap();
    let r = text(&mut ws).await;
    assert_eq!(r["t"], "nack", "{r}");
    // Ein kaputtes Binärframe beendet die Verbindung nicht.
    ws.send(Message::Binary(vec![9, 9].into())).await.unwrap();
    let r = text(&mut ws).await;
    assert_eq!(r["t"], "nack", "{r}");
    assert_eq!(r["problem"]["status"], 400);
}

/// PROTO-007 AC3: Ein Client, der keine Browser-Frames bestätigt, bekommt nie mehr als zwei
/// ausstehende; mit Bestätigung kommt das neueste.
#[tokio::test]
async fn proto_007_ac3_unacked_browser_frames_never_exceed_two() {
    let f = fixture(|r| r).await;
    let (_source, relay) = f.source(ChannelKind::Browser, "brw_1");
    let mut ws = f.hello().await;
    let opened = f.open(&mut ws, "browser", "brw_1").await;
    assert_eq!(opened["t"], "channel_opened", "{opened}");
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    let meta = beton_proto::channel::BrowserFrameMeta {
        w: 1280,
        h: 720,
        ts: 0,
        format: beton_proto::channel::ImageFormat::Jpeg,
    };
    for i in 0..50u8 {
        relay.push(Frame::new(
            1,
            flags::KEYFRAME | flags::FIN,
            meta.payload(&[0xff, 0xd8, i]),
        ));
    }
    let mut got = Vec::new();
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
        if let Message::Binary(b) = m {
            got.push(Frame::decode(&b).unwrap());
        }
    }
    assert_eq!(got.len(), 2, "höchstens zwei unbestätigt");
    let last = |f: &Frame| {
        *beton_proto::channel::BrowserFrameMeta::parse(&f.payload)
            .unwrap()
            .1
            .last()
            .unwrap()
    };
    assert_eq!(last(&got[0]), 0);
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "frames": 1}),
    )
    .await;
    let newest = binary(&mut ws).await;
    assert_eq!(last(&newest), 49, "nur das neueste wartende");
}

/// PROTO-007 AC1 (Server): 50 MB an einen auf 100 KB/s gedrosselten Client erhöhen den
/// Speicher des Servers um weniger als 2 MB: Kredit stoppt den Sender (hier eine Quelle, die
/// sich wie das PTY-Lesen des Runners nur im gewährten Fenster bewegt). Danach liest der Client
/// schnell, und alle Bytes kommen vollständig an. Den Weg über den Runner prüft
/// `beton-runner/tests/terminals.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proto_007_ac1_slow_client_bounds_server_memory() {
    const TOTAL: u64 = 50 * 1024 * 1024;
    let f = fixture(|r| r).await;
    let (source, relay) = f.source(ChannelKind::Terminal, "term_1");
    let hub = f.daemon.runtime.channels.clone();
    let mut ws = f.hello().await;
    let opened = f.open(&mut ws, "terminal", "term_1").await;
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    // Produzent (startet mit dem Zuschauer): sendet nur im gewährten Kredit (16 KiB je Frame), wie der Runner.
    let producer = {
        let source = source.clone();
        let relay = relay.clone();
        tokio::spawn(async move {
            let mut sent = 0u64;
            let mut keyframes = 0;
            while sent < TOTAL {
                let k = source.keyframes.load(Ordering::SeqCst);
                if k > keyframes {
                    keyframes = k;
                    relay.push(Frame::new(1, flags::KEYFRAME | flags::FIN, b"K".to_vec()));
                    sent += 1;
                    continue;
                }
                let credit = source.credit.load(Ordering::SeqCst);
                if credit < sent + 16 * 1024 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    continue;
                }
                let n = (16 * 1024).min(TOTAL - sent);
                relay.push(Frame::new(1, flags::FIN, vec![b'x'; n as usize]));
                sent += n;
            }
        })
    };
    let mut received = 0u64;
    let mut peak = 0usize;
    // 5 s lang 100 KB/s: alle 100 ms 10 KB Kredit.
    let slow_until = Instant::now() + Duration::from_secs(5);
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": 64 * 1024}),
    )
    .await;
    while Instant::now() < slow_until {
        send(
            &mut ws,
            json!({"t": "credit", "channel_id": cid, "bytes": 10_000}),
        )
        .await;
        let tick = Instant::now() + Duration::from_millis(100);
        while let Ok(Some(Ok(m))) = tokio::time::timeout_at(tick.into(), ws.next()).await {
            if let Message::Binary(b) = m {
                received += (b.len() - beton_proto::channel::HEADER_LEN) as u64;
            }
        }
        peak = peak.max(hub.buffered_bytes());
    }
    println!("gedrosselt: {received} Bytes empfangen, Spitze im Server {peak} Bytes");
    assert!(peak < 2 * 1024 * 1024, "Server hielt {peak} Bytes");
    assert!(
        received < 2 * 1024 * 1024,
        "Kredit begrenzt den Fluss: {received}"
    );
    // Jetzt schnell lesen: alles kommt an.
    send(
        &mut ws,
        json!({"t": "credit", "channel_id": cid, "bytes": TOTAL * 2}),
    )
    .await;
    while received < TOTAL {
        match tokio::time::timeout(Duration::from_secs(20), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                received += (b.len() - beton_proto::channel::HEADER_LEN) as u64;
                peak = peak.max(hub.buffered_bytes());
            }
            Ok(Some(Ok(_))) => {}
            other => panic!(
                "Strom endete bei {received}: {other:?}; Kredit des Senders {}, wartend {}, Zuschauer {}",
                source.credit.load(Ordering::SeqCst),
                relay.queued_bytes(),
                relay.subscribers()
            ),
        }
    }
    producer.await.unwrap();
    assert_eq!(received, TOTAL);
    assert!(peak < 2 * 1024 * 1024, "Server hielt {peak} Bytes");
}

/// Eine Spracheingabe als Senke: Audio kommt an, wird nie gespeichert (DATA-011 AC3).
#[derive(Default)]
struct Transcriber {
    bytes: AtomicUsize,
}

impl ChannelBackend for Transcriber {
    fn credit(&self, _: u64) {}
    fn keyframe(&self) {}
    fn input(&self, payload: Vec<u8>) -> bool {
        self.bytes.fetch_add(payload.len(), Ordering::SeqCst);
        true
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// DATA-011 AC3: Nach einer Spracheingabe über den Audio-Kanal enthalten weder DB noch
/// Blob-Store Audio-Daten (Suche nach WAV- und Opus-Signaturen über alle Dateien).
#[tokio::test]
async fn data_011_ac3_audio_never_reaches_db_or_blob_store() {
    let f = fixture(|r| r).await;
    let sink = Arc::new(Transcriber::default());
    f.daemon
        .runtime
        .channels
        .register(f.session.id, ChannelKind::Audio, "mic", sink.clone());
    let mut ws = f.hello().await;
    let opened = f.open(&mut ws, "audio", "mic").await;
    assert_eq!(opened["t"], "channel_opened", "{opened}");
    let cid = opened["channel_id"].as_u64().unwrap() as u32;
    let mut wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    wav.extend(vec![0x11; 2000]);
    let mut opus = b"OggS\x00\x02".to_vec();
    opus.extend(b"OpusHead");
    opus.extend(vec![0x22; 2000]);
    let mut sent = 0;
    for chunk in [wav, opus] {
        sent += chunk.len();
        ws.send(Message::Binary(
            Frame::new(cid, flags::INPUT | flags::FIN, chunk)
                .encode()
                .into(),
        ))
        .await
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while sink.bytes.load(Ordering::SeqCst) < sent {
        assert!(Instant::now() < deadline, "Audio kam nicht an");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    send(&mut ws, json!({"t": "close_channel", "channel_id": cid})).await;
    text(&mut ws).await;
    drop(ws);
    // Alles unter dem Datenverzeichnis (Datenbank samt WAL, Blobs).
    let mut stack = vec![f.dir.path().to_path_buf()];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                files += 1;
                for sig in [&b"WAVEfmt"[..], b"RIFF\x24", b"OpusHead", b"OggS\x00"] {
                    assert!(!contains(&bytes, sig), "{} enthält {sig:?}", p.display());
                }
            }
        }
    }
    assert!(files > 0, "Datenverzeichnis durchsucht");
}
