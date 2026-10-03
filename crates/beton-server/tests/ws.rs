//! WebSocket-Protokoll gegen einen echten Daemon (PROTO-004 … PROTO-009).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use beton_core::event::{
    Actor, Event, EventPayload, Notice, SessionKind, SessionTrigger, TextDelta,
};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_server::app::Runtime;
use beton_server::commands::{Command, CommandCtx, CommandRegistry};
use beton_server::hub::EventService;
use beton_server::ws::{Authorizer, WsConfig};
use beton_server::{Daemon, Problem, ServerConfig, start_with};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
    daemon: Daemon,
    _dir: tempfile::TempDir,
    store: Store,
    token: String,
    session: SessionRecord,
}

impl Fixture {
    fn events(&self) -> EventService {
        EventService {
            store: self.store.clone(),
            hub: self.daemon.runtime.hub.clone(),
        }
    }

    async fn produce(&self, n: usize, text: &str) {
        let head = self
            .store
            .session(OrgId::LOCAL, self.session.id)
            .await
            .unwrap()
            .head_seq;
        let events = (0..n)
            .map(|i| notice(self.session.id, &format!("{text}{i}")))
            .collect();
        self.events()
            .append(OrgId::LOCAL, self.session.id, head, 1, events)
            .await
            .unwrap();
    }

    async fn connect(&self) -> Ws {
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
        let (ws, res) = tokio_tungstenite::connect_async(req).await.unwrap();
        assert_eq!(res.headers()["sec-websocket-protocol"], "beton.v1");
        ws
    }

    async fn hello(&self) -> Ws {
        let mut ws = self.connect().await;
        send(
            &mut ws,
            json!({"t": "hello", "protocol": "1.0", "client": {"kind": "test", "version": "0"}}),
        )
        .await;
        let welcome = recv(&mut ws).await.unwrap();
        assert_eq!(welcome["t"], "welcome", "{welcome}");
        ws
    }
}

fn notice(session: SessionId, text: &str) -> Event {
    Event::new(
        session,
        0,
        Actor::default(),
        EventPayload::Notice(Notice {
            text: text.into(),
            ..Notice::default()
        }),
    )
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
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
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

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// Nächste Steuernachricht; `Err` mit Close-Code und Grund.
async fn recv(ws: &mut Ws) -> Result<Value, (u16, String)> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return Ok(serde_json::from_str(&t).unwrap()),
            Ok(Some(Ok(Message::Close(Some(f))))) => {
                return Err((u16::from(f.code), f.reason.to_string()));
            }
            Ok(Some(Ok(Message::Close(None)))) | Ok(None) => return Err((0, String::new())),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => return Err((0, e.to_string())),
            Err(_) => panic!("keine Nachricht innerhalb von 10 s"),
        }
    }
}

/// Liest Events bis einschließlich `until_seq` und liefert alle dauerhaften seqs.
async fn collect_until(ws: &mut Ws, until_seq: u64) -> Vec<u64> {
    let mut seqs = Vec::new();
    while seqs.last().copied().unwrap_or(0) < until_seq {
        let m = recv(ws).await.unwrap();
        if m["t"] == "events" {
            for e in m["events"].as_array().unwrap() {
                if e.get("transient").is_none() {
                    seqs.push(e["seq"].as_u64().unwrap());
                }
            }
        }
    }
    seqs
}

fn short(mut r: Runtime) -> Runtime {
    r.ws = WsConfig {
        hello_timeout: Duration::from_millis(300),
        ..WsConfig::default()
    };
    r
}

#[tokio::test]
async fn proto_004_ac1_negotiates_lower_minor_or_closes_4400() {
    let f = fixture(|mut r| {
        r.ws.server_minor = 4;
        r
    })
    .await;
    let mut ws = f.connect().await;
    send(
        &mut ws,
        json!({"t": "hello", "protocol": "1.3", "client": {"kind": "test", "version": "0"}}),
    )
    .await;
    assert_eq!(recv(&mut ws).await.unwrap()["protocol"], "1.3");

    let mut ws = f.connect().await;
    send(
        &mut ws,
        json!({"t": "hello", "protocol": "1.2", "client": {"kind": "test", "version": "0"}}),
    )
    .await;
    let (code, reason) = recv(&mut ws).await.unwrap_err();
    assert_eq!(code, 4400);
    let reason: Value = serde_json::from_str(&reason).unwrap();
    assert_eq!(reason, json!({"supported": {"min": "1.3", "max": "1.4"}}));
}

#[tokio::test]
async fn proto_004_ac2_missing_hello_closes_4400() {
    let f = fixture(short).await;
    let mut ws = f.connect().await;
    let started = Instant::now();
    let (code, _) = recv(&mut ws).await.unwrap_err();
    assert_eq!(code, 4400);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn proto_005_ac1_attach_during_production_is_gapless() {
    let f = Arc::new(fixture(|r| r).await);
    f.produce(200, "vorher ").await;
    let producer = {
        let f = f.clone();
        tokio::spawn(async move {
            // ~100 Events/s
            for i in 0..100 {
                f.produce(1, &format!("live {i}")).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    // session.created (1) + 200 + 100
    let seqs = collect_until(&mut ws, 301).await;
    producer.await.unwrap();
    assert_eq!(
        seqs,
        (1..=301).collect::<Vec<_>>(),
        "lückenlos und ohne Duplikate"
    );
}

#[tokio::test]
async fn proto_005_live_marker_and_tail() {
    let f = fixture(|r| r).await;
    f.produce(10, "x").await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0, "tail": 3}),
    )
    .await;
    let first = recv(&mut ws).await.unwrap();
    assert_eq!(first["has_more"], true);
    let seqs: Vec<u64> = first["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [9, 10, 11]);
    let live = recv(&mut ws).await.unwrap();
    assert_eq!(
        live,
        json!({"t": "live", "session_id": f.session.id, "head_seq": 11})
    );
}

#[tokio::test]
async fn proto_005_ac2_from_seq_ahead_is_nacked() {
    let f = fixture(|r| r).await;
    let mut ws = f.hello().await;
    send(&mut ws, json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": f.session.head_seq + 5})).await;
    let m = recv(&mut ws).await.unwrap();
    assert_eq!(m["t"], "nack");
    assert_eq!(m["problem"]["code"], "seq_ahead");
}

struct Nobody;

impl Authorizer for Nobody {
    fn can_read(&self, _: beton_server::security::Authenticated, _: &SessionRecord) -> bool {
        false
    }
}

#[tokio::test]
async fn proto_005_ac3_no_read_permission_gets_403_and_no_events() {
    let f = fixture(|mut r| {
        r.authorizer = Arc::new(Nobody);
        r
    })
    .await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    let m = recv(&mut ws).await.unwrap();
    assert_eq!(m["t"], "nack");
    assert_eq!(m["problem"]["status"], 403);
    f.produce(3, "geheim").await;
    let quiet = tokio::time::timeout(Duration::from_millis(300), ws.next()).await;
    assert!(quiet.is_err(), "keine Events ohne Leserecht");
}

#[tokio::test]
async fn proto_005_transient_snapshot_on_attach_and_live_deltas() {
    let f = fixture(|r| r).await;
    let hub = f.daemon.runtime.hub.clone();
    let delta = |text: &str| {
        let mut e = Event::new(
            f.session.id,
            0,
            Actor::default(),
            EventPayload::MessageDelta(TextDelta {
                message_id: "m1".into(),
                text: text.into(),
                snapshot: false,
            }),
        );
        e.transient = true;
        e
    };
    hub.publish_transient(delta("Hal")).unwrap();
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    let _replay = recv(&mut ws).await.unwrap();
    assert_eq!(recv(&mut ws).await.unwrap()["t"], "live");
    let snap = recv(&mut ws).await.unwrap();
    assert_eq!(snap["events"][0]["payload"]["text"], "Hal");
    assert_eq!(snap["events"][0]["payload"]["snapshot"], true);
    hub.publish_transient(delta("lo")).unwrap();
    let live = recv(&mut ws).await.unwrap();
    assert_eq!(live["events"][0]["payload"]["text"], "lo");
    assert_eq!(live["events"][0]["transient"], true);
}

/// Testkommando: zählt Aufrufe.
#[derive(Default)]
struct Count(AtomicU32);

#[async_trait]
impl Command for Count {
    fn name(&self) -> &'static str {
        "test.count"
    }
    fn rest_twin(&self) -> (&'static str, &'static str) {
        ("get", "/v1/info")
    }
    async fn run(&self, _: &CommandCtx, _: Option<SessionId>, _: Value) -> Result<Value, Problem> {
        Ok(json!({"n": self.0.fetch_add(1, Ordering::SeqCst) + 1}))
    }
}

fn with_count(counter: Arc<Count>) -> impl FnOnce(Runtime) -> Runtime {
    move |mut r| {
        let mut reg = CommandRegistry::default();
        reg.register(counter);
        r.commands = Arc::new(reg);
        r.ws.rate_violation_close = Duration::from_millis(1500);
        r
    }
}

#[tokio::test]
async fn proto_006_idempotency_key_returns_original_result() {
    let counter = Arc::new(Count::default());
    let f = fixture(with_count(counter.clone())).await;
    let mut ws = f.hello().await;
    for id in ["c1", "c2"] {
        send(&mut ws, json!({"t": "cmd", "id": id, "name": "test.count", "args": {}, "idempotency_key": "k1"})).await;
        let ack = recv(&mut ws).await.unwrap();
        assert_eq!(ack, json!({"t": "ack", "id": id, "result": {"n": 1}}));
    }
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proto_006_ac2_unknown_command_keeps_connection_open() {
    let f = fixture(|r| r).await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "cmd", "id": "c1", "name": "gibt.es.nicht", "args": {}}),
    )
    .await;
    let m = recv(&mut ws).await.unwrap();
    assert_eq!(m["problem"]["code"], "unknown_command");
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    assert_eq!(recv(&mut ws).await.unwrap()["t"], "events");
}

#[test]
fn proto_006_ac3_every_command_has_a_rest_twin() {
    let doc = serde_json::to_value(beton_server::openapi()).unwrap();
    let mut reg = CommandRegistry::default();
    reg.register(Arc::new(Count::default()));
    assert!(reg.missing_rest_twins(&doc).is_empty());

    struct Orphan;
    #[async_trait]
    impl Command for Orphan {
        fn name(&self) -> &'static str {
            "test.orphan"
        }
        fn rest_twin(&self) -> (&'static str, &'static str) {
            ("post", "/v1/gibt-es-nicht")
        }
        async fn run(
            &self,
            _: &CommandCtx,
            _: Option<SessionId>,
            _: Value,
        ) -> Result<Value, Problem> {
            Ok(Value::Null)
        }
    }
    reg.register(Arc::new(Orphan));
    assert_eq!(
        reg.missing_rest_twins(&doc),
        ["test.orphan → POST /v1/gibt-es-nicht"]
    );
    // Die eingebauten Kommandos (ab WP-09) prüft derselbe Test über die Standard-Registry.
    assert!(
        Runtime::new(tokio::sync::watch::channel(false).1)
            .commands
            .missing_rest_twins(&doc)
            .is_empty()
    );
}

#[tokio::test]
async fn proto_008_ac3_command_flood_is_rejected_then_closed() {
    let f = fixture(with_count(Arc::new(Count::default()))).await;
    let mut ws = f.hello().await;
    let mut rejected = 0;
    let started = Instant::now();
    let mut closed = None;
    'outer: while started.elapsed() < Duration::from_secs(5) {
        for i in 0..150 {
            if ws
                .send(Message::Text(
                    json!({"t": "cmd", "id": format!("c{i}"), "name": "test.count", "args": {}})
                        .to_string()
                        .into(),
                ))
                .await
                .is_err()
            {
                break 'outer;
            }
        }
        while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            match msg {
                Ok(Message::Text(t)) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    if v["problem"]["status"] == 429 {
                        rejected += 1;
                    }
                }
                Ok(Message::Close(Some(frame))) => {
                    closed = Some(u16::from(frame.code));
                    break 'outer;
                }
                _ => {}
            }
        }
    }
    assert!(rejected > 0, "nack 429 erwartet");
    assert_eq!(closed, Some(4429));
}

#[tokio::test]
async fn proto_008_ac1_blocked_client_gets_overflow_and_recovers() {
    let f = fixture(|mut r| {
        r.ws.overflow_bytes = 64 * 1024;
        r
    })
    .await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    // Client liest nicht, Server produziert viel.
    for _ in 0..40 {
        f.produce(50, &"x".repeat(2000)).await;
    }
    let head = f
        .store
        .session(OrgId::LOCAL, f.session.id)
        .await
        .unwrap()
        .head_seq;
    let mut seen = Vec::new();
    let resume_from = loop {
        let m = recv(&mut ws).await.unwrap();
        match m["t"].as_str().unwrap() {
            "events" => seen.extend(
                m["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["seq"].as_u64().unwrap()),
            ),
            "overflow" => break m["resume_from"].as_u64().unwrap(),
            _ => {}
        }
    };
    assert!(resume_from < head, "Overflow vor dem Ende");
    assert_eq!(
        seen.last().copied().unwrap_or(0),
        resume_from,
        "resume_from = zuletzt gesendet"
    );
    // Neu attachen ab resume_from: Zustand vollständig.
    send(
        &mut ws,
        json!({"t": "attach", "id": "r2", "session_id": f.session.id, "from_seq": resume_from}),
    )
    .await;
    let rest = collect_until(&mut ws, head).await;
    seen.extend(rest);
    assert_eq!(seen, (1..=head).collect::<Vec<_>>());
}

#[tokio::test]
async fn proto_008_ac2_slow_client_does_not_delay_fast_client() {
    let f = fixture(|r| r).await;
    let mut slow = f.hello().await;
    let mut fast = f.hello().await;
    for ws in [&mut slow, &mut fast] {
        send(
            ws,
            json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 1}),
        )
        .await;
    }
    assert_eq!(recv(&mut fast).await.unwrap()["t"], "live");
    let mut latencies = Vec::new();
    for i in 0..100 {
        let sent = Instant::now();
        f.produce(1, &format!("{i} {}", "y".repeat(4000))).await;
        loop {
            let m = recv(&mut fast).await.unwrap();
            if m["t"] == "events" {
                break;
            }
        }
        latencies.push(sent.elapsed());
    }
    latencies.sort();
    let p99 = latencies[98];
    assert!(p99 < Duration::from_millis(100), "p99 {p99:?}");
    drop(slow);
}

#[tokio::test]
async fn proto_009_ac1_missing_pong_disconnects_and_frees_resources() {
    let f = fixture(|mut r| {
        r.ws.ping_interval = Duration::from_millis(100);
        r.ws.pong_timeout = Duration::from_millis(400);
        r
    })
    .await;
    let mut ws = f.hello().await;
    send(
        &mut ws,
        json!({"t": "attach", "id": "r1", "session_id": f.session.id, "from_seq": 0}),
    )
    .await;
    let _ = recv(&mut ws).await;
    assert_eq!(f.daemon.runtime.hub.subscribers(f.session.id), 1);
    // Nicht lesen → keine Pongs.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let mut code = None;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
        if let Ok(Message::Close(Some(frame))) = msg {
            code = Some(frame.code);
            break;
        }
    }
    assert_eq!(code, Some(CloseCode::from(4408)));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        f.daemon.runtime.hub.subscribers(f.session.id),
        0,
        "Abo freigegeben"
    );
}

#[tokio::test]
async fn proto_009_shutdown_closes_with_4503() {
    let f = fixture(|r| r).await;
    let mut ws = f.hello().await;
    let Fixture { daemon, .. } = f;
    let shutdown = tokio::spawn(daemon.shutdown());
    let (code, _) = recv(&mut ws).await.unwrap_err();
    assert_eq!(code, 4503);
    shutdown.await.unwrap();
}

#[tokio::test]
async fn auth_003_real_websocket_endpoint_rejects_foreign_origin() {
    let f = fixture(|r| r).await;
    let port = f.daemon.addrs[0].port();
    let mut req = format!("ws://127.0.0.1:{port}/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", f.token).parse().unwrap(),
    );
    req.headers_mut()
        .insert("origin", "https://evil.test".parse().unwrap());
    let err = tokio_tungstenite::connect_async(req).await.unwrap_err();
    assert!(err.to_string().contains("403"), "{err}");
}
