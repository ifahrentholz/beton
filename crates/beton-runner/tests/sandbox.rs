//! Harness-Prozess (Fake-CLI hinter dem Claude-Adapter) und Sandbox im Runner: SBX-001 AC4
//! (genau ein `sandbox.started`, mit `backend: none` sichtbar als `degraded`), der
//! `PreToolUse`-Hook aus Stufe 1 (#206) und YOLO in der Sandbox mit durchgesetzter `deny`-Policy
//! (HAR-027 AC2). Keine echten Vendor-CLIs, kein Netz.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod support;

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
    daemon_with(
        dir,
        "turns:\n  - emit:\n      - { message: \"ok\" }\n",
        support::unsandboxed(),
    )
    .await
}

async fn daemon_with(dir: &Path, scenario: &str, sandbox: beton_sandbox::SandboxSpec) -> D {
    let sc = dir.join("scenario.yaml");
    std::fs::write(&sc, scenario).unwrap();
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
        r.sessions.sandbox = sandbox;
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
/// #206, HAR-005: Claude (Fake-CLI) läuft in Stufe 1, der `PreToolUse`-Hook erreicht seinen
/// Socket und entscheidet; Bash läuft über `bt-exec` in Stufe 2. HAR-027 AC2: Im Modus `yolo`
/// setzt beton eine `deny`-Policy auf `git push --force` weiter durch.
#[tokio::test]
async fn har_027_ac2_yolo_in_the_sandbox_hook_decides_and_force_push_stays_denied() {
    let dir = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let scenario = "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"echo stufe=$BETON_STAGE\" } }, gate: true, run: true }\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"git push --force origin main\" } }, gate: true, run: true }\n      - { message: \"fertig\" }\n";
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join(".beton/policies")).unwrap();
    std::fs::write(
        work.join(".beton/policies/git.yaml"),
        "spec_version: 1\nname: git\nrules:\n  - id: no-force\n    type: git_guard\n    params: { force_push: deny, push: allow }\n",
    )
    .unwrap();
    let d = daemon_with(
        dir.path(),
        scenario,
        beton_sandbox::SandboxSpec::from_yaml("allow_network: false").unwrap(),
    )
    .await;
    let (status, body) = d
        .http(
            "POST",
            "/v1/sessions",
            Some(json!({"target": "claude", "cwd": work, "permission_mode": "yolo"})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    d.wait_status(&id, "idle").await;
    let (status, _) = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "los"})),
        )
        .await;
    assert_eq!(status, 202);
    let start = Instant::now();
    let events = loop {
        let events = d.events(&id).await;
        if events
            .iter()
            .any(|e| e["type"] == "turn.completed" || e["type"] == "turn.failed")
        {
            break events;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "{:?}",
            events.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let text = serde_json::to_string(&events).unwrap();
    // Harness in Stufe 1 mit echtem Backend, das erlaubte Kommando lief in Stufe 2.
    assert!(
        events.iter().any(|e| e["type"] == "sandbox.started"
            && e["payload"]["stage"] == "harness"
            && e["payload"]["backend"] != "none"),
        "{text}"
    );
    let completed: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "tool.call.completed")
        .collect();
    assert_eq!(completed.len(), 2, "{text}");
    assert_eq!(completed[0]["payload"]["status"], "ok", "{text}");
    assert!(
        completed[0]["payload"].to_string().contains("stufe=tools"),
        "{text}"
    );
    // Der Force-Push: vom Hook (volles Gate) per Policy abgelehnt, nicht ausgeführt.
    assert_eq!(completed[1]["payload"]["status"], "denied", "{text}");
    let decisions: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "policy.decision" && e["payload"]["phase"] == "tool_call")
        .collect();
    assert!(
        decisions.iter().any(|d| d["payload"]["outcome"] == "deny"
            && d["payload"]["enforcement"] == "full"
            && d["payload"].to_string().contains("no-force")),
        "{text}"
    );
    assert!(!text.contains("policy_integrity_violation"), "{text}");
    assert!(!text.contains("sandbox_integrity"), "{text}");
    d.daemon.shutdown().await;
}
