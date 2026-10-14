//! OpenTelemetry-Traces einer Fake-Harness-Session (OBS-003) gegen einen OTLP-Empfänger im
//! Prozess auf Loopback (die Linux-CI hat kein Netzwerk, ein Collector-Container entfällt).

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use beton_host::LocalProvider;
use beton_server::otel::{OtelConfig, SessionTracer};
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PROMPT: &str = "MARKER-PROMPT-4711 bitte lies die Datei";
const MARKER: &str = "MARKER-";

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

/// OTLP/HTTP-Empfänger: sammelt die JSON-Rümpfe und Header von `POST /v1/traces`.
#[derive(Clone, Default)]
struct Receiver {
    bodies: Arc<Mutex<Vec<Value>>>,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
}

impl Receiver {
    async fn start(&self) -> SocketAddr {
        let me = self.clone();
        let app = axum::Router::new().route(
            "/v1/traces",
            axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let me = me.clone();
                async move {
                    me.headers.lock().unwrap().push(headers);
                    me.bodies
                        .lock()
                        .unwrap()
                        .push(serde_json::from_slice(&body).unwrap());
                    axum::Json(json!({}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    /// Alle empfangenen Spans.
    fn spans(&self) -> Vec<Value> {
        self.bodies
            .lock()
            .unwrap()
            .iter()
            .flat_map(|b| {
                b["resourceSpans"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|rs| rs["scopeSpans"].as_array().into_iter().flatten())
                    .flat_map(|ss| ss["spans"].as_array().cloned().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

impl Env {
    async fn start(receiver: SocketAddr, capture_content: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        let mut cfg = ServerConfig::local(dir.path().to_path_buf());
        cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
        cfg.socket = None;
        let mut inherit: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| {
                !k.starts_with("BETON_")
                    && !k.starts_with("OTEL_")
                    && k != "CLAUDE_CONFIG_DIR"
                    && k != "CODEX_HOME"
            })
            .collect();
        inherit.insert("HOME".into(), home.display().to_string());
        inherit.insert(
            "BETON_HOME".into(),
            home.join(".beton").display().to_string(),
        );
        let mut headers = BTreeMap::new();
        headers.insert("x-beton-test".to_owned(), "otlp".to_owned());
        let config = OtelConfig::resolve(
            Some(&format!("http://{receiver}")),
            &headers,
            capture_content,
            &|_| None,
        )
        .unwrap()
        .unwrap();
        let config = OtelConfig {
            flush_interval: Duration::from_millis(100),
            ..config
        };
        let tracer = SessionTracer::start(config, Arc::new(beton_store::NoRedaction));
        let runners = dir.path().join("runners");
        let daemon = start_with(cfg, store, move |mut r| {
            r.sessions.provider = Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r.tracer = Some(tracer);
            r
        })
        .await
        .unwrap();
        let token = std::fs::read_to_string(dir.path().join("auth/local.token"))
            .unwrap()
            .trim()
            .to_owned();
        let addr = daemon.addrs[0];
        Self {
            dir,
            daemon,
            token,
            addr,
        }
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            port = self.addr.port(),
            token = self.token,
            len = body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let payload = text.split_once("\r\n\r\n").map(|(_, p)| p).unwrap_or("");
        let payload = if text
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        (
            status,
            serde_json::from_str(&payload).unwrap_or(Value::Null),
        )
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let (status, page) = self
            .http(
                "GET",
                &format!("/v1/sessions/{id}/events?after_seq=0&limit=200"),
                None,
            )
            .await;
        assert_eq!(status, 200, "{page}");
        page["items"].as_array().unwrap().clone()
    }

    async fn turn(&self, id: &str, text: &str) {
        let done = |events: &[Value]| {
            events
                .iter()
                .filter(|e| e["type"] == "turn.completed")
                .count()
        };
        let before = done(&self.events(id).await);
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({ "text": text })),
            )
            .await;
        assert_eq!(status, 202, "{body}");
        let start = Instant::now();
        while done(&self.events(id).await) == before {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "Turn endet nicht: {:#?}",
                self.events(id).await
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&tail[..n.min(tail.len())]);
        rest = tail.get(n + 2..).unwrap_or("");
    }
    out
}

fn attr<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|a| a["key"] == key)
        .map(|a| &a["value"])
}

const SCENARIO: &str = "turns:\n  - emit:\n      - { tool_call: { name: Read, kind: file_read, args: { file_path: MARKER-ARGS-4711.md } }, gate: true }\n      - { on_gate: { allow: [{ tool_result: inhalt }], deny: [{ message: abgelehnt }] } }\n      - { message: fertig }\n  - emit:\n      - { tool_call: { name: Read, kind: file_read, args: { file_path: MARKER-ARGS-4712.md } }, gate: true }\n      - { on_gate: { allow: [{ tool_result: inhalt }], deny: [{ message: abgelehnt }] } }\n      - { message: fertig }\n";

/// Deckt OBS-003 AC1 (Hierarchie), OBS-003 AC2 (eine Trace-ID über alle Turns) und
/// OBS-003 AC3 (ohne `capture_content` kein Marker in Attributen) Ende zu Ende ab.
#[tokio::test]
async fn obs_003_ac1_fake_session_trace_reaches_an_otlp_receiver() {
    let receiver = Receiver::default();
    let addr = receiver.start().await;
    let env = Env::start(addr, false).await;
    let scenario = env.dir.path().join("trace.yaml");
    std::fs::write(&scenario, SCENARIO).unwrap();
    let (status, body) = env
        .http(
            "POST",
            "/v1/sessions",
            Some(json!({"target": "fake", "cwd": env.work(), "harness_opts": {"scenario": scenario}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    env.turn(&id, PROMPT).await;
    env.turn(&id, PROMPT).await;

    let start = Instant::now();
    let spans = loop {
        let spans = receiver.spans();
        let turns = spans.iter().filter(|s| s["name"] == "turn").count();
        if turns >= 2 {
            break spans;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Spans fehlen: {spans:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let named = |n: &str| -> Vec<&Value> { spans.iter().filter(|s| s["name"] == n).collect() };
    let turns = named("turn");
    let tools = named("tool.call");
    let policies = named("policy.evaluate");
    assert_eq!(turns.len(), 2, "{spans:#?}");
    assert_eq!(tools.len(), 2, "{spans:#?}");

    // AC1: Hierarchie turn → tool.call → policy.evaluate (Phase tool_call).
    for tool in &tools {
        let parent = tool["parentSpanId"].as_str().unwrap();
        assert!(turns.iter().any(|t| t["spanId"] == parent), "{tool}");
        let call = attr(tool, "beton.tool.call_id").unwrap()["stringValue"].clone();
        let decision = policies
            .iter()
            .find(|p| p["parentSpanId"] == tool["spanId"])
            .unwrap_or_else(|| panic!("keine Entscheidung unter {call}: {spans:#?}"));
        assert_eq!(
            attr(decision, "beton.policy.phase").unwrap()["stringValue"],
            "tool_call"
        );
    }
    assert!(
        policies
            .iter()
            .any(|p| turns.iter().any(|t| t["spanId"] == p["parentSpanId"])),
        "Entscheidung der Phase turn unter dem Turn: {spans:#?}"
    );

    // AC2: alle Spans der Session, über beide Turns, in einem Trace.
    let trace = turns[0]["traceId"].as_str().unwrap();
    assert_eq!(trace.len(), 32);
    assert!(spans.iter().all(|s| s["traceId"] == trace));
    assert_eq!(
        attr(turns[1], "beton.session.id").unwrap()["stringValue"],
        id.as_str()
    );

    // AC3: ohne `capture_content` weder Prompt noch Tool-Argumente in Attributen.
    let all = serde_json::to_string(&spans).unwrap();
    assert!(!all.contains(MARKER), "{all}");

    // Header aus der Konfiguration gehen mit.
    let headers = receiver.headers.lock().unwrap().clone();
    assert!(headers.iter().all(|h| h["x-beton-test"] == "otlp"));
    assert_eq!(headers[0]["content-type"], "application/json");
    env.daemon.shutdown().await;
}
