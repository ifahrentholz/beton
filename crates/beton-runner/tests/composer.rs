//! Composer-Erweiterungen mit echtem Runner und Fake-Harness (WEB-006): Anhänge als Blob mit
//! der Eingabe, Grenzen und Typen.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_server::app::Runtime;
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

async fn daemon(dir: &Path, customize: impl FnOnce(Runtime) -> Runtime) -> D {
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
    D {
        daemon,
        token,
        addr,
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
        let n =
            usize::from_str_radix(String::from_utf8_lossy(&rest[..pos]).trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        let tail = &rest[pos + 2..];
        out.extend_from_slice(&tail[..n.min(tail.len())]);
        rest = tail.get(n + 2..).unwrap_or(&[]);
    }
    out
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

impl D {
    async fn raw(&self, method: &str, path: &str, content_type: &str, body: &[u8]) -> Reply {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
            port = self.addr.port(),
            token = self.token,
            len = body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        // Bei Ablehnung schließt der Server evtl. vor dem Ende des Bodys.
        let _ = stream.write_all(body).await;
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
        let split = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(buf.len());
        let head = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let payload = buf.get(split + 4..).unwrap_or(&[]);
        let body = if head.contains("transfer-encoding: chunked") {
            dechunk(payload)
        } else {
            payload.to_vec()
        };
        Reply { status, body }
    }

    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        self.raw(method, path, "application/json", body.as_bytes())
            .await
    }

    async fn create(&self, cwd: &Path, scenario: &Path) -> String {
        let r = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": "fake", "cwd": cwd, "harness_opts": {"scenario": scenario}})),
            )
            .await;
        assert_eq!(r.status, 201, "{}", r.json());
        r.json()["id"].as_str().unwrap().to_owned()
    }

    async fn upload(&self, id: &str, name: &str, mime: &str, bytes: &[u8]) -> Reply {
        self.raw(
            "POST",
            &format!("/v1/sessions/{id}/attachments?name={name}"),
            mime,
            bytes,
        )
        .await
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let r = self
            .http(
                "GET",
                &format!("/v1/sessions/{id}/events?after_seq=0&limit=200"),
                None,
            )
            .await;
        r.json()["items"].as_array().cloned().unwrap_or_default()
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
                events
                    .iter()
                    .map(|e| format!("{} {}", e["type"], e["payload"]))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn wait_idle(&self, id: &str) {
        let start = Instant::now();
        while self
            .http("GET", &format!("/v1/sessions/{id}"), None)
            .await
            .json()["status"]
            != "idle"
        {
            assert!(start.elapsed() < Duration::from_secs(20));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
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

/// Ein kleines PNG (Signatur und IHDR genügen für den Test).
fn png() -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(std::iter::repeat_n(7u8, 300));
    b
}

const ECHO: &str = "turns:\n  - emit:\n      - { echo_input: true }\n";

fn agent_text(events: &[Value]) -> String {
    events
        .iter()
        .filter(|e| e["type"] == "message.completed" && e["payload"]["role"] == "assistant")
        .map(|e| {
            e["payload"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn web_006_ac1_pasted_screenshot_is_a_blob_sent_with_the_input() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let sc = scenario(
        dir.path(),
        &format!("capabilities: {{ images: true }}\n{ECHO}"),
    );
    let id = d.create(dir.path(), &sc).await;
    d.wait_idle(&id).await;
    let bytes = png();
    let up = d
        .upload(&id, "Bildschirmfoto.png", "image/png", &bytes)
        .await;
    assert_eq!(up.status, 201, "{}", up.json());
    let att = up.json();
    assert!(att["blob"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(att["name"], "Bildschirmfoto.png");
    assert_eq!(att["mime"], "image/png");
    assert_eq!(att["size"], bytes.len());

    let r = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "Was zeigt der Screenshot?", "attachments": [att]})),
        )
        .await;
    assert_eq!(r.status, 202, "{}", r.json());
    let events = d.wait_for(&id, |e| e["type"] == "turn.completed").await;
    // Die Nachricht des Users trägt den Anhang als Blob-Referenz.
    let user = events
        .iter()
        .find(|e| e["type"] == "message.completed" && e["payload"]["role"] == "user")
        .unwrap();
    let content = user["payload"]["content"].as_array().unwrap();
    assert_eq!(
        content[0],
        json!({"type": "text", "text": "Was zeigt der Screenshot?"})
    );
    assert_eq!(content[1]["type"], "attachment");
    assert_eq!(content[1]["blob"], att["blob"]);
    assert_eq!(content[1]["mime"], "image/png");
    // Der Blob ist über die Session abrufbar.
    let hash = att["blob"].as_str().unwrap();
    let blob = d
        .http("GET", &format!("/v1/sessions/{id}/blobs/{hash}"), None)
        .await;
    assert_eq!(blob.status, 200);
    assert_eq!(blob.body, bytes);
    // Und er kam beim Harness an.
    assert!(
        agent_text(&events).contains(&format!(
            "[Anhang: Bildschirmfoto.png, image/png, {} Bytes]",
            bytes.len()
        )),
        "{}",
        agent_text(&events)
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn web_006_limits_types_and_capabilities_are_enforced() {
    let dir = tmp();
    let d = daemon(dir.path(), |mut r| {
        r.attachments.max_file_bytes = 1024;
        r.attachments.max_files = 2;
        r
    })
    .await;
    let info = d.http("GET", "/v1/info", None).await.json();
    assert_eq!(
        info["attachments"],
        json!({"max_file_bytes": 1024, "max_files": 2})
    );
    // Fake ohne Capability `images`.
    let id = d.create(dir.path(), &scenario(dir.path(), ECHO)).await;
    d.wait_idle(&id).await;

    let big = d
        .upload(&id, "login-trace.har", "text/plain", &vec![b'x'; 4096])
        .await;
    assert_eq!(big.status, 413, "{}", big.json());
    assert_eq!(big.json()["code"], "payload_too_large");
    assert!(
        big.json()["detail"]
            .as_str()
            .unwrap()
            .contains("login-trace.har nicht angehängt"),
        "{}",
        big.json()
    );
    let zip = d.upload(&id, "a.zip", "application/zip", b"PK").await;
    assert_eq!(zip.status, 415);
    assert_eq!(zip.json()["code"], "unsupported_media_type");

    let image = d.upload(&id, "bild.png", "image/png", &png()).await.json();
    let r = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "Bild?", "attachments": [image]})),
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.json());
    assert_eq!(r.json()["code"], "capability_unsupported");

    let text = d
        .upload(
            &id,
            "notizen.md",
            "text/markdown",
            "# Notiz\nInhalt".as_bytes(),
        )
        .await
        .json();
    let r = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "Drei?", "attachments": [text, text, text]})),
        )
        .await;
    assert_eq!(r.status, 400, "{}", r.json());

    // Textdateien gehen an jeden Harness, als Teil des Texts.
    let r = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "Lies das", "attachments": [text]})),
        )
        .await;
    assert_eq!(r.status, 202, "{}", r.json());
    let events = d.wait_for(&id, |e| e["type"] == "turn.completed").await;
    let echoed = agent_text(&events);
    assert!(echoed.starts_with("Lies das"), "{echoed}");
    assert!(
        echoed.contains("<datei name=\"notizen.md\">\n# Notiz\nInhalt\n</datei>"),
        "{echoed}"
    );

    // Blobs anderer Sessions lassen sich nicht anhängen (DATA-006 AC1).
    let other = d.create(dir.path(), &scenario(dir.path(), ECHO)).await;
    let r = d
        .http(
            "POST",
            &format!("/v1/sessions/{other}/input"),
            Some(json!({"text": "fremd", "attachments": [text]})),
        )
        .await;
    assert_eq!(r.status, 404, "{}", r.json());
    d.daemon.shutdown().await;
}
