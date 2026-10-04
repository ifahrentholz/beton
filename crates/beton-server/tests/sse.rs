//! Read-only SSE-Stream `GET /v1/sessions/{id}/events/stream` gegen einen echten Daemon
//! (PROTO-012, API-003). Der Client hier liest wie `curl -N`: rohes HTTP/1.1 mit
//! `Transfer-Encoding: chunked`, Frames nach `text/event-stream`.

#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Event, EventPayload, Notice, SessionKind, SessionTrigger, TextDelta,
};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_server::app::Runtime;
use beton_server::hub::EventService;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Fixture {
    daemon: Daemon,
    _dir: tempfile::TempDir,
    store: Store,
    token: String,
    session: SessionRecord,
}

impl Fixture {
    fn addr(&self) -> SocketAddr {
        self.daemon.addrs[0]
    }

    fn events(&self) -> EventService {
        EventService {
            store: self.store.clone(),
            hub: self.daemon.runtime.hub.clone(),
        }
    }

    async fn head(&self) -> u64 {
        self.store
            .session(OrgId::LOCAL, self.session.id)
            .await
            .unwrap()
            .head_seq
    }

    async fn produce(&self, n: usize) {
        let head = self.head().await;
        let events = (0..n)
            .map(|i| {
                Event::new(
                    self.session.id,
                    0,
                    Actor::default(),
                    EventPayload::Notice(Notice {
                        text: format!("n{i}"),
                        ..Notice::default()
                    }),
                )
            })
            .collect();
        self.events()
            .append(OrgId::LOCAL, self.session.id, head, 1, events)
            .await
            .unwrap();
    }

    fn path(&self, query: &str) -> String {
        format!("/v1/sessions/{}/events/stream{query}", self.session.id)
    }

    /// Öffnet den Strom; `headers` sind zusätzliche Zeilen (`Name: Wert`).
    async fn open(&self, query: &str, headers: &[&str]) -> SseClient {
        let mut req = format!(
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\n",
            self.path(query),
            self.addr().port(),
            self.token
        );
        for h in headers {
            req.push_str(h);
            req.push_str("\r\n");
        }
        req.push_str("\r\n");
        let mut stream = TcpStream::connect(self.addr()).await.unwrap();
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut client = SseClient {
            stream,
            raw: Vec::new(),
            body: Vec::new(),
            status: 0,
            head: String::new(),
        };
        client.read_head().await;
        client
    }

    /// Einfacher Request ohne Strom (für Fehlerfälle).
    async fn request(&self, method: &str, query: &str, headers: &[&str]) -> (u16, String) {
        let mut req = format!(
            "{method} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Length: 0\r\nConnection: close\r\n",
            self.path(query),
            self.addr().port(),
            self.token
        );
        for h in headers {
            req.push_str(h);
            req.push_str("\r\n");
        }
        req.push_str("\r\n");
        let mut stream = TcpStream::connect(self.addr()).await.unwrap();
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (status, text)
    }
}

/// Ein SSE-Frame: Felder in Reihenfolge, Kommentare als Feld mit leerem Namen.
#[derive(Debug, Clone, Default)]
struct Frame {
    id: Option<String>,
    event: Option<String>,
    data: Option<String>,
    comments: Vec<String>,
}

impl Frame {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(self.data.as_deref().unwrap_or("null")).unwrap()
    }
}

struct SseClient {
    stream: TcpStream,
    /// Ungelesene Bytes der Verbindung (chunked).
    raw: Vec<u8>,
    /// Entpackter Body, noch nicht in Frames zerlegt.
    body: Vec<u8>,
    status: u16,
    head: String,
}

impl SseClient {
    async fn fill(&mut self, deadline: Instant) -> bool {
        let mut buf = [0u8; 8192];
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, self.stream.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => false,
            Ok(Ok(n)) => {
                self.raw.extend_from_slice(&buf[..n]);
                true
            }
        }
    }

    async fn read_head(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pos) = find(&self.raw, b"\r\n\r\n") {
                self.head = String::from_utf8_lossy(&self.raw[..pos]).into_owned();
                self.raw.drain(..pos + 4);
                self.status = self
                    .head
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                return;
            }
            assert!(self.fill(deadline).await, "kein HTTP-Kopf");
        }
    }

    /// Entpackt vollständige Chunks aus `raw` nach `body`.
    fn dechunk(&mut self) {
        loop {
            let Some(pos) = find(&self.raw, b"\r\n") else {
                return;
            };
            let size = usize::from_str_radix(String::from_utf8_lossy(&self.raw[..pos]).trim(), 16)
                .unwrap();
            if self.raw.len() < pos + 2 + size + 2 {
                return;
            }
            self.body
                .extend_from_slice(&self.raw[pos + 2..pos + 2 + size]);
            self.raw.drain(..pos + 2 + size + 2);
        }
    }

    /// Nächster Frame oder `None` nach `within`.
    async fn next(&mut self, within: Duration) -> Option<Frame> {
        let deadline = Instant::now() + within;
        loop {
            self.dechunk();
            if let Some(pos) = find(&self.body, b"\n\n") {
                let text = String::from_utf8_lossy(&self.body[..pos]).into_owned();
                self.body.drain(..pos + 2);
                let mut f = Frame::default();
                for line in text.lines() {
                    let (name, value) = line.split_once(':').unwrap_or((line, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value).to_owned();
                    match name {
                        "" => f.comments.push(value),
                        "id" => f.id = Some(value),
                        "event" => f.event = Some(value),
                        "data" => f.data = Some(value),
                        _ => {}
                    }
                }
                return Some(f);
            }
            if !self.fill(deadline).await {
                return None;
            }
        }
    }

    /// Nächster Frame mit Daten (überspringt Heartbeats).
    async fn next_event(&mut self) -> Frame {
        loop {
            let f = self
                .next(Duration::from_secs(10))
                .await
                .expect("kein Event innerhalb von 10 s");
            if f.data.is_some() {
                return f;
            }
        }
    }

    /// Liest dauerhafte Events bis einschließlich `until`; liefert ihre ids.
    async fn ids_until(&mut self, until: u64) -> Vec<u64> {
        let mut ids = Vec::new();
        while ids.last().copied().unwrap_or(0) < until {
            let f = self.next_event().await;
            let id: u64 =
                f.id.as_deref()
                    .expect("dauerhaftes Event ohne id")
                    .parse()
                    .unwrap();
            assert_eq!(f.json()["seq"].as_u64(), Some(id), "id = seq");
            assert_eq!(
                f.event.as_deref(),
                f.json()["type"].as_str(),
                "event = type"
            );
            ids.push(id);
        }
        ids
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
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
                harness_opts: serde_json::Value::Null,
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
        _dir: dir,
        store,
        token,
        session,
    }
}

fn contiguous(ids: &[u64], from: u64) -> bool {
    ids.iter().enumerate().all(|(i, id)| *id == from + i as u64)
}

#[tokio::test]
async fn proto_012_ac1_last_event_id_50_resumes_at_51_without_gaps() {
    let f = fixture(|r| r).await;
    f.produce(70).await;
    let head = f.head().await;
    let mut c = f.open("", &["Last-Event-ID: 50"]).await;
    assert_eq!(c.status, 200, "{}", c.head);
    assert!(
        c.head
            .to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "{}",
        c.head
    );
    let ids = c.ids_until(head).await;
    assert_eq!(ids.first(), Some(&51));
    assert!(contiguous(&ids, 51), "{ids:?}");
    // Danach live, weiterhin lückenlos.
    f.produce(5).await;
    let live = c.ids_until(head + 5).await;
    assert!(contiguous(&live, head + 1), "{live:?}");
}

#[tokio::test]
async fn api_003_ac1_from_seq_0_delivers_history_then_live() {
    let f = fixture(|r| r).await;
    f.produce(30).await;
    let head = f.head().await;
    let mut c = f.open("?from_seq=0", &[]).await;
    assert_eq!(c.status, 200, "{}", c.head);
    let ids = c.ids_until(head).await;
    assert!(contiguous(&ids, 1), "{ids:?}");
    let first = ids.len();
    assert_eq!(first as u64, head);
    // Live-Events kommen ohne neue Anfrage.
    let producer = {
        let events = f.events();
        let session = f.session.id;
        tokio::spawn(async move {
            for i in 0..10u64 {
                let e = Event::new(
                    session,
                    0,
                    Actor::default(),
                    EventPayload::Notice(Notice {
                        text: format!("live {i}"),
                        ..Notice::default()
                    }),
                );
                events
                    .append(OrgId::LOCAL, session, head + i, 1, vec![e])
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let live = c.ids_until(head + 10).await;
    producer.await.unwrap();
    assert!(contiguous(&live, head + 1), "{live:?}");
}

#[tokio::test]
async fn api_003_ac2_reconnect_with_last_event_id_42_starts_at_43() {
    let f = fixture(|r| r).await;
    f.produce(60).await;
    let head = f.head().await;
    // `Last-Event-ID` (Reconnect eines EventSource) gilt vor `from_seq` aus der URL.
    let mut c = f.open("?from_seq=0", &["Last-Event-ID: 42"]).await;
    let ids = c.ids_until(head).await;
    assert_eq!(ids.first(), Some(&43));
    assert!(contiguous(&ids, 43), "{ids:?}");
}

#[tokio::test]
async fn proto_012_ac2_only_get_no_actions() {
    let f = fixture(|r| r).await;
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let (status, text) = f.request(method, "", &[]).await;
        assert_eq!(status, 405, "{method}: {text}");
        assert!(text.contains("method_not_allowed"), "{text}");
    }
    // Auch Ereignisse nach einem POST-Versuch: nichts ist passiert.
    assert_eq!(f.head().await, 1);
}

#[tokio::test]
async fn proto_012_ac3_heartbeat_comment_every_15_s_without_events() {
    assert_eq!(
        beton_server::sse::SseConfig::default().heartbeat,
        Duration::from_secs(15)
    );
    let f = fixture(|mut r| {
        r.sse.heartbeat = Duration::from_millis(200);
        r
    })
    .await;
    let mut c = f.open("", &["Last-Event-ID: 1"]).await;
    let started = Instant::now();
    let mut beats = 0;
    while beats < 3 {
        let frame = c
            .next(Duration::from_secs(5))
            .await
            .expect("kein Heartbeat");
        assert!(frame.data.is_none(), "unerwartetes Event: {frame:?}");
        assert_eq!(frame.comments, vec!["hb".to_owned()]);
        beats += 1;
    }
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
}

#[tokio::test]
async fn proto_012_transient_events_only_on_request_and_without_id() {
    let f = fixture(|r| r).await;
    let head = f.head().await;
    let mut plain = f.open(&format!("?from_seq={head}"), &[]).await;
    let mut with = f
        .open(&format!("?from_seq={head}&transient=true"), &[])
        .await;
    // Beide Ströme stehen (Abo vor dem ersten Event).
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut delta = Event::new(
        f.session.id,
        0,
        Actor::default(),
        EventPayload::MessageDelta(TextDelta {
            message_id: "m1".into(),
            text: "Hal".into(),
            snapshot: false,
        }),
    );
    delta.transient = true;
    f.daemon.runtime.hub.publish_transient(delta).unwrap();
    f.produce(1).await;
    let t = with.next_event().await;
    assert_eq!(t.event.as_deref(), Some("message.delta"));
    assert!(t.id.is_none(), "transiente Events tragen keine id: {t:?}");
    assert_eq!(t.json()["transient"], true);
    assert_eq!(with.ids_until(head + 1).await, vec![head + 1]);
    // Ohne `transient=true` nur das dauerhafte Event.
    let first = plain.next_event().await;
    assert_eq!(first.id.as_deref(), Some(&*(head + 1).to_string()));
}

#[tokio::test]
async fn proto_012_errors_are_problems() {
    let f = fixture(|r| r).await;
    let (status, text) = f.request("GET", "", &["Last-Event-ID: 99"]).await;
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("seq_ahead"), "{text}");
    let (status, text) = f.request("GET", "?from_seq=abc", &[]).await;
    assert_eq!(status, 400, "{text}");
    let (status, text) = f.request("GET", "", &["Last-Event-ID: nicht-zahl"]).await;
    assert_eq!(status, 400, "{text}");
}
