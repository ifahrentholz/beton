//! SBX-001 AC4 im Runner: Der Start des Harness-Prozesses (Fake-CLI hinter dem Claude-Adapter)
//! meldet genau ein `sandbox.started`. Bis SBX-006 (WP-49) läuft er ausdrücklich über
//! `backend: none`, sichtbar als `degraded`. Keine echten Vendor-CLIs, kein Netz.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

struct D {
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

impl D {
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

    async fn wait_status(&self, id: &str, want: &str) {
        let start = Instant::now();
        loop {
            let s = self
                .http("GET", &format!("/v1/sessions/{id}"), None)
                .await
                .1;
            if s["status"] == want {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Status {want} fehlt: {s}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn fake_cli() -> PathBuf {
    let fake = PathBuf::from(runner_bin()).with_file_name("beton-fake-cli");
    if !fake.is_file() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "beton-fake-cli"])
            .status()
            .unwrap();
        assert!(status.success());
    }
    fake
}

async fn daemon(dir: &Path) -> D {
    let sc = dir.join("scenario.yaml");
    std::fs::write(&sc, "turns:\n  - emit:\n      - { message: \"ok\" }\n").unwrap();
    let mut inherit: std::collections::BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| !k.ends_with("_API_KEY") && k != "ANTHROPIC_AUTH_TOKEN")
        .collect();
    inherit.insert(
        "BETON_CLAUDE_PATH".into(),
        format!(
            "{} --protocol stream-json --scenario {}",
            fake_cli().display(),
            sc.display()
        ),
    );
    let store = Store::open(dir, StoreOptions::default()).await.unwrap();
    let mut cfg = ServerConfig::local(dir.to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let runners = dir.join("runners");
    let daemon = start_with(cfg, store, move |mut r| {
        r.sessions.provider = std::sync::Arc::new(
            LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
        );
        r
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(dir.join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    let addr = daemon.addrs[0];
    D {
        daemon,
        token,
        addr,
    }
}

#[tokio::test]
async fn sbx_001_ac4_harness_start_emits_exactly_one_sandbox_started() {
    let dir = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let d = daemon(dir.path()).await;
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
    let events = d.events(&id).await;
    let started: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "sandbox.started")
        .collect();
    assert_eq!(
        started.len(),
        1,
        "{:?}",
        events.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
    );
    let p = &started[0]["payload"];
    assert_eq!(p["stage"], "harness");
    assert_eq!(p["backend"], "none");
    assert_eq!(p["caps"], json!([]));
    assert_eq!(p["degraded"], true);
    d.daemon.shutdown().await;
}
