//! Session-Lebenszyklus über REST und WebSocket mit echtem Runner und Fake-Harness
//! (SES-001, SES-002, SES-003, SES-005, PROTO-006, HAR-002, DATA-006, DATA-008).

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_server::app::Runtime;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

struct Daemonized {
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

async fn daemon(dir: &Path, customize: impl FnOnce(Runtime) -> Runtime) -> Daemonized {
    let store = Store::open(dir, StoreOptions::default()).await.unwrap();
    let mut cfg = ServerConfig::local(dir.to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let runners = dir.join("runners");
    let daemon = start_with(cfg, store, move |mut r| {
        r.sessions.provider =
            std::sync::Arc::new(LocalProvider::new(vec![runner_bin().into()], runners));
        r.sessions.dev = true;
        customize(r)
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(dir.join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    let addr = daemon.addrs[0];
    Daemonized {
        daemon,
        token,
        addr,
    }
}

impl Daemonized {
    /// Einfacher HTTP/1.1-Client (keine weitere Abhängigkeit nötig).
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
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let payload = if head
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

    async fn ws(
        &self,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
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
        ws.send(Message::Text(
            json!({"t": "hello", "protocol": "1.0", "client": {"kind": "test", "version": "0"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let _welcome = ws.next().await;
        ws
    }

    async fn create(&self, cwd: &Path, scenario: &Path) -> String {
        let (status, body) = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": "fake", "cwd": cwd, "harness_opts": {"scenario": scenario}})),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let mut after = 0;
        loop {
            let (status, page) = self
                .http(
                    "GET",
                    &format!("/v1/sessions/{id}/events?after_seq={after}&limit=200"),
                    None,
                )
                .await;
            assert_eq!(status, 200, "{page}");
            let items = page["items"].as_array().unwrap().clone();
            if items.is_empty() {
                return out;
            }
            after = items.last().unwrap()["seq"].as_u64().unwrap();
            out.extend(items);
        }
    }

    async fn wait_for(&self, id: &str, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            if events.iter().any(&pred) {
                return events;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Event fehlt: {:?}",
                events.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn status(&self, id: &str) -> String {
        self.http("GET", &format!("/v1/sessions/{id}"), None)
            .await
            .1["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    async fn wait_status(&self, id: &str, want: &str) {
        let start = Instant::now();
        while self.status(id).await != want {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Status {want} kommt nicht (ist {})",
                self.status(id).await
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

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap()
}

fn scenario(dir: &Path, yaml: &str) -> PathBuf {
    let p = dir.join(format!("{}.yaml", beton_core::id::RunnerId::new()));
    std::fs::write(&p, yaml).unwrap();
    p
}

fn is(t: &str) -> impl Fn(&Value) -> bool + '_ {
    move |e: &Value| e["type"] == t
}

#[tokio::test]
async fn ses_001_ac1_created_then_started_and_idle() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    let events = d
        .wait_for(&id, |e| {
            e["type"] == "session.status" && e["payload"]["status"] == "idle"
        })
        .await;
    assert_eq!(events[0]["type"], "session.created");
    assert_eq!(events[0]["seq"], 1);
    assert_eq!(events[1]["type"], "session.started");
    d.wait_status(&id, "idle").await;
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_001_ac2_archive_stops_runner_and_hides_session() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    d.wait_status(&id, "idle").await;
    let (status, body) = d
        .http("POST", &format!("/v1/sessions/{id}/archive"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    d.wait_status(&id, "stopped").await;
    assert!(!d.daemon.runtime.runners.connected(id.parse().unwrap()));
    let (_, list) = d.http("GET", "/v1/sessions", None).await;
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["id"] != id.as_str())
    );
    let (_, all) = d
        .http("GET", "/v1/sessions?include_archived=true", None)
        .await;
    assert!(
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == id.as_str() && s["archived"] == true)
    );
    // AC4: genau ein Event mit dem Auslöser als Akteur.
    let events = d.events(&id).await;
    let archived: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "session.archived")
        .collect();
    assert_eq!(archived.len(), 1);
    assert_eq!(
        archived[0]["actor"],
        json!({"kind": "user", "id": "usr_local"})
    );
    assert_eq!(
        d.http("POST", &format!("/v1/sessions/{id}/unarchive"), None)
            .await
            .0,
        200
    );
    let unarchived = d
        .events(&id)
        .await
        .into_iter()
        .filter(|e| e["type"] == "session.unarchived")
        .count();
    assert_eq!(unarchived, 1);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_001_ac3_delete_removes_everything_and_leaves_tombstone() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    d.wait_status(&id, "idle").await;
    let blob = {
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        store
            .put_session_blob(beton_core::id::OrgId::LOCAL, id.parse().unwrap(), b"anhang")
            .await
            .unwrap()
    };
    assert_eq!(
        d.http("GET", &format!("/v1/sessions/{id}/blobs/{blob}"), None)
            .await
            .0,
        200
    );
    assert_eq!(
        d.http("DELETE", &format!("/v1/sessions/{id}"), None)
            .await
            .0,
        204
    );
    assert_eq!(
        d.http("GET", &format!("/v1/sessions/{id}"), None).await.0,
        404
    );
    assert_eq!(
        d.http("GET", &format!("/v1/sessions/{id}/events"), None)
            .await
            .0,
        404
    );
    assert_eq!(
        d.http("GET", &format!("/v1/sessions/{id}/blobs/{blob}"), None)
            .await
            .0,
        404
    );
    let (_, tombstones) = d.http("GET", "/v1/tombstones", None).await;
    assert!(
        tombstones["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id.as_str())
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_002_ac1_three_clients_see_identical_streams() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let text = "x".repeat(1000);
    let id = d
        .create(
            dir.path(),
            &scenario(
                dir.path(),
                &format!("turns:\n  - emit: [{{ message_delta: \"{text}\", chunk: 1 }}]\n"),
            ),
        )
        .await;
    d.wait_status(&id, "idle").await;
    let mut clients = Vec::new();
    for _ in 0..3 {
        let mut ws = d.ws().await;
        ws.send(Message::Text(
            json!({"t": "attach", "id": "a", "session_id": id, "from_seq": 0})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        clients.push(ws);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        d.http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "los"}))
        )
        .await
        .0,
        202
    );
    let mut streams = Vec::new();
    for ws in &mut clients {
        let mut deltas = Vec::new();
        let start = Instant::now();
        while deltas.len() < 1000 && start.elapsed() < Duration::from_secs(20) {
            let Ok(Some(Ok(Message::Text(t)))) =
                tokio::time::timeout(Duration::from_secs(5), ws.next()).await
            else {
                break;
            };
            let m: Value = serde_json::from_str(&t).unwrap();
            for e in m["events"].as_array().into_iter().flatten() {
                if e["type"] == "message.delta" && e["payload"]["snapshot"] != true {
                    deltas.push((
                        e["tseq"].as_u64().unwrap(),
                        e["payload"]["text"].as_str().unwrap().to_owned(),
                    ));
                }
            }
        }
        assert_eq!(deltas.len(), 1000);
        streams.push(deltas);
    }
    assert_eq!(streams[0], streams[1]);
    assert_eq!(streams[1], streams[2]);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_002_ac2_reconnect_from_seq_500_gets_exactly_the_rest() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let mut yaml = String::from("turns:\n  - emit:\n");
    for i in 0..600 {
        yaml.push_str(&format!("      - {{ message: \"m{i}\" }}\n"));
    }
    let id = d.create(dir.path(), &scenario(dir.path(), &yaml)).await;
    d.wait_status(&id, "idle").await;
    d.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "los"})),
    )
    .await;
    let events = d.wait_for(&id, is("turn.completed")).await;
    let head = events.last().unwrap()["seq"].as_u64().unwrap();
    let mut ws = d.ws().await;
    ws.send(Message::Text(
        json!({"t": "attach", "id": "a", "session_id": id, "from_seq": 500})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut seqs = Vec::new();
    while seqs.last().copied().unwrap_or(500) < head {
        let Some(Ok(Message::Text(t))) = ws.next().await else {
            panic!()
        };
        let m: Value = serde_json::from_str(&t).unwrap();
        for e in m["events"].as_array().into_iter().flatten() {
            if e.get("transient").is_none() {
                seqs.push(e["seq"].as_u64().unwrap());
            }
        }
    }
    assert_eq!(seqs, (501..=head).collect::<Vec<_>>());
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_003_ac1_resume_after_daemon_restart_uses_native_ref() {
    let dir = tmp();
    let s = scenario(
        dir.path(),
        "turns: [{ emit: [{ message: eins }] }, { emit: [{ message: zwei }] }]",
    );
    let id = {
        let d = daemon(dir.path(), |r| r).await;
        let id = d.create(dir.path(), &s).await;
        d.wait_status(&id, "idle").await;
        d.daemon.shutdown().await;
        id
    };
    let d = daemon(dir.path(), |r| r).await;
    let (status, body) = d
        .http("POST", &format!("/v1/sessions/{id}/resume"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    let events = d.wait_for(&id, is("session.resumed")).await;
    let resumed = events
        .iter()
        .find(|e| e["type"] == "session.resumed")
        .unwrap();
    assert_eq!(resumed["payload"]["mode"], "native");
    let started: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "session.started")
        .collect();
    assert_eq!(started.len(), 2);
    assert_eq!(
        started[1]["payload"]["harness_session_ref"], "fake-session",
        "gespeicherte Vendor-Session-ID"
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_003_ac3_input_to_stopped_session_resumes_it() {
    let dir = tmp();
    let d = daemon(dir.path(), |mut r| {
        r.tunnel.idle_timeout = Duration::from_millis(300);
        r.tunnel.idle_check = Duration::from_millis(100);
        r
    })
    .await;
    let id = d
        .create(
            dir.path(),
            &scenario(dir.path(), "turns: [{ emit: [{ message: Antwort }] }]"),
        )
        .await;
    d.wait_status(&id, "stopped").await;
    let (status, body) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "hallo"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let events = d.wait_for(&id, is("turn.completed")).await;
    assert!(events.iter().any(
        |e| e["type"] == "message.completed" && e["payload"]["content"][0]["text"] == "Antwort"
    ));
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_005_ac1_ac3_interrupt_during_tool_call_and_idempotent() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(dir.path(), &scenario(dir.path(), "turns:\n  - emit: [{ tool_call: { name: Bash, kind: shell, args: { command: \"sleep 60\" } } }, { hang: true }]\n"))
        .await;
    d.wait_status(&id, "idle").await;
    d.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "los"})),
    )
    .await;
    d.wait_for(&id, is("tool.call.requested")).await;
    let start = Instant::now();
    assert_eq!(
        d.http("POST", &format!("/v1/sessions/{id}/interrupt"), None)
            .await
            .0,
        200
    );
    d.wait_for(&id, is("turn.interrupted")).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    d.wait_status(&id, "idle").await;
    let before = d.events(&id).await.len();
    assert_eq!(
        d.http("POST", &format!("/v1/sessions/{id}/interrupt"), None)
            .await
            .0,
        200
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(d.events(&id).await.len(), before, "kein neues Event");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn proto_006_ac1_duplicate_input_submit_starts_one_turn() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(
            dir.path(),
            &scenario(
                dir.path(),
                "turns: [{ emit: [{ message: a }] }, { emit: [{ message: b }] }]",
            ),
        )
        .await;
    d.wait_status(&id, "idle").await;
    let mut ws = d.ws().await;
    let mut ids = Vec::new();
    for c in ["c1", "c2"] {
        ws.send(Message::Text(json!({"t": "cmd", "id": c, "session_id": id, "name": "input.submit", "args": {"text": "x"}, "idempotency_key": "k1"}).to_string().into())).await.unwrap();
        loop {
            let Some(Ok(Message::Text(t))) = ws.next().await else {
                panic!()
            };
            let m: Value = serde_json::from_str(&t).unwrap();
            if m["t"] == "ack" {
                ids.push(m["result"]["input_id"].clone());
                break;
            }
            assert_ne!(m["t"], "nack", "{m}");
        }
    }
    assert_eq!(ids[0], ids[1]);
    let events = d.wait_for(&id, is("turn.completed")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = if events.len() < d.events(&id).await.len() {
        d.events(&id).await
    } else {
        events
    };
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "turn.started")
            .count(),
        1
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn har_002_ac3_action_without_capability_is_rejected() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(
            dir.path(),
            &scenario(
                dir.path(),
                "capabilities: { model_switch: none }\nturns: []",
            ),
        )
        .await;
    d.wait_status(&id, "idle").await;
    let (status, problem) = d
        .http(
            "PATCH",
            &format!("/v1/sessions/{id}"),
            Some(json!({"model": "opus"})),
        )
        .await;
    assert_eq!(status, 422, "{problem}");
    assert_eq!(problem["code"], "capability_unsupported");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn har_002_ac1_catalog_lists_harnesses_with_capabilities() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let (status, page) = d.http("GET", "/v1/harnesses?host=hst_local", None).await;
    assert_eq!(status, 200, "{page}");
    let items = page["items"].as_array().unwrap();
    for id in ["claude", "fake"] {
        let entry = items
            .iter()
            .find(|h| h["id"] == id)
            .unwrap_or_else(|| panic!("{id} fehlt"));
        // Gegen das generierte Schema: als HarnessInfo lesbar.
        let parsed: beton_harness::registry::HarnessInfo =
            serde_json::from_value(entry.clone()).unwrap();
        assert!(!parsed.capabilities.is_empty());
    }
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn web_018_approval_via_rest_resolves_gate() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let s = scenario(
        dir.path(),
        "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"git push\" } }, gate: true }\n      - { on_gate: { allow: [{ tool_result: ok }], deny: [{ message: abgelehnt }] } }\n",
    );
    let id = d.create(dir.path(), &s).await;
    d.wait_status(&id, "idle").await;
    d.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "los"})),
    )
    .await;
    d.wait_status(&id, "waiting_approval").await;
    let (_, approvals) = d
        .http("GET", &format!("/v1/sessions/{id}/approvals"), None)
        .await;
    let aid = approvals["items"][0]["id"].as_str().unwrap().to_owned();
    let (status, body) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
            Some(json!({"decision": "deny", "reason": "nicht jetzt"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let events = d.wait_for(&id, is("turn.completed")).await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "tool.call.completed" && e["payload"]["status"] == "denied")
    );
    assert!(
        d.http("GET", &format!("/v1/sessions/{id}/approvals"), None)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_001_ac4_lifecycle_actions_emit_one_event_by_the_actor() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    d.wait_status(&id, "idle").await;
    for action in ["archive", "unarchive"] {
        assert_eq!(
            d.http("POST", &format!("/v1/sessions/{id}/{action}"), None)
                .await
                .0,
            200
        );
    }
    let events = d.events(&id).await;
    let user = json!({"kind": "user", "id": "usr_local"});
    for t in ["session.created", "session.archived", "session.unarchived"] {
        let found: Vec<&Value> = events.iter().filter(|e| e["type"] == t).collect();
        assert_eq!(found.len(), 1, "{t}");
        assert_eq!(found[0]["actor"], user, "{t}");
    }
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn har_002_ac2_incompatible_cli_fails_the_session_with_event() {
    let dir = tmp();
    // Eine "claude"-CLI mit zu alter Version.
    let cli = dir.path().join("claude-alt");
    std::fs::write(&cli, "#!/bin/sh\necho '1.0.0 (Claude Code)'\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut inherit: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    inherit.insert("BETON_CLAUDE_PATH".into(), cli.display().to_string());
    let runners = dir.path().join("runners");
    let d = daemon(dir.path(), move |mut r| {
        r.sessions.provider = std::sync::Arc::new(
            LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
        );
        r
    })
    .await;
    let (status, body) = d
        .http(
            "POST",
            "/v1/sessions",
            Some(json!({"target": "claude", "cwd": dir.path()})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    let events = d.wait_for(&id, is("harness.incompatible")).await;
    let e = events
        .iter()
        .find(|e| e["type"] == "harness.incompatible")
        .unwrap();
    assert_eq!(e["payload"]["detected_version"], "1.0.0");
    d.wait_status(&id, "failed").await;
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn proto_001_ac4_large_payload_reaches_clients_via_blob_api() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let big = "z".repeat(70 * 1024);
    let id = d
        .create(
            dir.path(),
            &scenario(
                dir.path(),
                &format!("turns: [{{ emit: [{{ message: \"{big}\" }}] }}]"),
            ),
        )
        .await;
    d.wait_status(&id, "idle").await;
    d.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "los"})),
    )
    .await;
    let events = d.wait_for(&id, is("turn.completed")).await;
    let offloaded = events
        .iter()
        .find(|e| e["type"] == "message.completed" && e["actor"]["kind"] != "user")
        .unwrap();
    assert!(offloaded.get("payload").is_none(), "Payload ausgelagert");
    let blob = offloaded["payload_ref"].as_str().unwrap();
    let (status, content) = d
        .http("GET", &format!("/v1/sessions/{id}/blobs/{blob}"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        content["content"][0]["text"].as_str().unwrap().len(),
        big.len()
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn har_004_claude_adapter_runs_a_turn_through_the_runner() {
    let dir = tmp();
    let fake = std::path::PathBuf::from(runner_bin()).with_file_name("beton-fake-cli");
    if !fake.is_file() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "beton-fake-cli"])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let sc = scenario(
        dir.path(),
        "turns:\n  - expect_input: \"sag hallo\"\n    emit:\n      - { message_delta: \"Hallo!\", chunk: 3 }\n",
    );
    let mut inherit: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    inherit.insert(
        "BETON_CLAUDE_PATH".into(),
        format!(
            "{} --protocol stream-json --scenario {}",
            fake.display(),
            sc.display()
        ),
    );
    let runners = dir.path().join("runners");
    let d = daemon(dir.path(), move |mut r| {
        r.sessions.provider = std::sync::Arc::new(
            LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
        );
        r
    })
    .await;
    let (status, body) = d
        .http(
            "POST",
            "/v1/sessions",
            Some(json!({"target": "claude", "cwd": dir.path()})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    d.wait_status(&id, "idle").await;
    let (status, body) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "sag hallo"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let events = d.wait_for(&id, is("turn.completed")).await;
    let text = events
        .iter()
        .find(|e| e["type"] == "message.completed" && e["payload"]["role"] == "assistant")
        .unwrap();
    assert_eq!(text["payload"]["content"][0]["text"], "Hallo!");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn proto_015_second_runner_in_the_same_daemon_loses_no_events() {
    // Ein neuer Runner zählt `rseq` wieder ab 1; der Server darf seine Events nicht als
    // Duplikate des Vorgängers verwerfen.
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(
            dir.path(),
            &scenario(dir.path(), "turns: [{ emit: [{ message: Antwort }] }]"),
        )
        .await;
    d.wait_status(&id, "idle").await;
    let (status, body) = d
        .http("POST", &format!("/v1/sessions/{id}/archive"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    d.wait_status(&id, "stopped").await;
    d.http("POST", &format!("/v1/sessions/{id}/unarchive"), None)
        .await;
    let (status, body) = d
        .http("POST", &format!("/v1/sessions/{id}/resume"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    d.wait_status(&id, "idle").await;
    let events = d.events(&id).await;
    let count = |t: &str| events.iter().filter(|e| e["type"] == t).count();
    assert_eq!(count("session.started"), 2, "{events:#?}");
    assert_eq!(count("session.resumed"), 1);
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn session_list_delta_contains_only_changed_sessions() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let a = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    let b = d
        .create(dir.path(), &scenario(dir.path(), "turns: []"))
        .await;
    d.wait_status(&a, "idle").await;
    d.wait_status(&b, "idle").await;
    let (_, list) = d.http("GET", "/v1/sessions", None).await;
    let since = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["last_activity_at"].as_str())
        .max()
        .unwrap()
        .to_owned();
    let (_, body) = d
        .http("POST", &format!("/v1/sessions/{a}/archive"), None)
        .await;
    assert_eq!(body["archived"], true, "{body}");
    let (status, delta) = d
        .http("GET", &format!("/v1/sessions?updated_after={since}"), None)
        .await;
    assert_eq!(status, 200, "{delta}");
    // Nur geänderte Sessions: a (archiviert) ist dabei; jede gelieferte Session hat sich nach
    // `since` geändert. b darf auftauchen, wenn nach dem Listenabruf noch ein spätes Event
    // (z. B. Usage nach Turn-Ende) ihre Aktivität fortgeschrieben hat.
    let items = delta["items"].as_array().unwrap();
    let archived = items
        .iter()
        .find(|s| s["id"] == a.as_str())
        .unwrap_or_else(|| panic!("archivierte Session fehlt: {delta}"));
    assert_eq!(archived["archived"], true);
    for s in items {
        assert!(
            s["last_activity_at"].as_str().unwrap() > since.as_str(),
            "unverändert geliefert: {s}"
        );
    }
    assert!(items.len() <= 2, "{delta}");
    let (status, _) = d
        .http("GET", "/v1/sessions?updated_after=gestern", None)
        .await;
    assert_eq!(status, 400);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_002_user_input_is_part_of_the_event_log() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create(
            dir.path(),
            &scenario(dir.path(), "turns: [{ emit: [{ message: Antwort }] }]"),
        )
        .await;
    d.wait_status(&id, "idle").await;
    d.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "Bitte prüfen"})),
    )
    .await;
    let events = d.wait_for(&id, is("turn.completed")).await;
    let user = events
        .iter()
        .position(|e| e["type"] == "message.completed" && e["payload"]["role"] == "user")
        .expect("Nachricht des Nutzers fehlt");
    assert_eq!(
        events[user]["payload"]["content"][0]["text"],
        "Bitte prüfen"
    );
    assert_eq!(events[user]["payload"]["author"], "usr_local");
    let turn = events
        .iter()
        .position(|e| e["type"] == "turn.started")
        .unwrap();
    assert!(user < turn, "Eingabe steht vor dem Turn");
    d.daemon.shutdown().await;
}
