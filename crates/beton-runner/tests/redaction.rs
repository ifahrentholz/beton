//! Redaction im echten Daemon mit Runner und Fake-Harness (SEC-013 AC1, SES-007 AC3).
//!
//! Der Daemon bekommt die Pipeline aus `beton-secrets` (bekannte Werte über die
//! `RedactionCap`, Speicher-Backend statt Keychain); die Test-Secrets sind Fixtures.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Event, EventPayload, MessageCompleted, MessageRole, SessionKind, SessionTrigger,
    TurnCompleted, TurnStarted,
};
use beton_core::id::{PrincipalId, SessionId, TurnId, UserId};
use beton_host::LocalProvider;
use beton_secrets::backend::MemoryBackend;
use beton_secrets::{NewSecret, Redactor, SecretKind, SecretRef, SecretValue, Secrets};
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bekannter Wert ohne erkennbares Token-Format (nur der exakte Abgleich findet ihn).
const KNOWN: &str = "Zx9-known-SECRET-value-4711";
/// Ein echt aussehender GitHub-Token, wie er per `env.passthrough` in der Sandbox stünde.
const GITHUB: &str = "ghp_0123456789abcdefghijABCDEFGHIJ012345";
/// Platzhalter des Proxys; bleibt sichtbar.
const PLACEHOLDER: &str = "bt_cred_0123456789abcdef0123";

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
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

struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    /// Zweiter Zugriff auf dieselbe Datenbank ohne Redaction: ein „präpariertes“ Log, wie es
    /// vor dem Anlegen des Secrets entstanden wäre.
    unredacted: Store,
    token: String,
    addr: SocketAddr,
}

impl Env {
    async fn start(vars: impl FnOnce(&Path) -> Vec<(&'static str, String)>) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        let redactor = Arc::new(Redactor::pending());
        let store = Store::open(
            dir.path(),
            StoreOptions {
                redactor: redactor.clone(),
                ..StoreOptions::default()
            },
        )
        .await
        .unwrap();
        let unredacted = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        // Ein Secret, das schon vor dem Start des Daemons angelegt war.
        let backend = Arc::new(MemoryBackend::default());
        let local = store.ensure_local().await.unwrap();
        let (secrets, _) = Secrets::new(store.clone(), local, backend.clone());
        secrets
            .put(NewSecret {
                secret_ref: SecretRef::user("custom:db", "default", None).unwrap(),
                kind: SecretKind::Token,
                value: SecretValue::from(KNOWN),
            })
            .await
            .unwrap();

        let mut cfg = ServerConfig::local(dir.path().to_path_buf());
        cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
        cfg.socket = None;
        let mut inherit: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| {
                !k.starts_with("BETON_") && k != "CLAUDE_CONFIG_DIR" && k != "CODEX_HOME"
            })
            .collect();
        inherit.insert("HOME".into(), home.display().to_string());
        inherit.insert(
            "BETON_HOME".into(),
            home.join(".beton").display().to_string(),
        );
        for (k, v) in vars(dir.path()) {
            inherit.insert(k.to_owned(), v);
        }
        let runners = dir.path().join("runners");
        let daemon = start_with(cfg, store, move |mut r| {
            r.sessions.provider = Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r.secret_backend = backend;
            r.redactor = Some(redactor);
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
            unredacted,
            token,
            addr,
        }
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> (u16, String) {
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
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        (status, payload)
    }

    async fn json(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let (status, text) = self.http(method, path, body).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let (status, page) = self
            .json(
                "GET",
                &format!("/v1/sessions/{id}/events?after_seq=0&limit=200"),
                None,
            )
            .await;
        assert_eq!(status, 200, "{page}");
        page["items"].as_array().unwrap().clone()
    }

    async fn wait_for(&self, id: &str, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            if events.iter().any(&pred) {
                return events;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "Event fehlt: {}",
                serde_json::to_string_pretty(&events).unwrap()
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

fn leaks(text: &str) -> Vec<&'static str> {
    [KNOWN, GITHUB]
        .into_iter()
        .filter(|v| text.contains(v))
        .collect()
}

#[tokio::test]
async fn sec_013_ac1_tool_output_with_a_real_token_is_redacted_in_log_and_live() {
    let env = Env::start(|_| Vec::new()).await;
    // Das Tool gibt aus, was per `env.passthrough` in der Sandbox steht (GITHUB_TOKEN) und
    // einen bekannten Wert aus dem Secret-Store; der Agent streamt den Token auch als Text.
    let scenario = env.write(
        "leak.yaml",
        &format!(
            "turns:\n  - emit:\n      - {{ message_delta: \"Token: {GITHUB} und {KNOWN}\", chunk: 500 }}\n      - {{ tool_call: {{ name: Bash, kind: shell, args: {{ command: \"echo $GITHUB_TOKEN; cat .env\" }} }} }}\n      - {{ tool_result: \"{GITHUB}\\nDB_PASSWORD={KNOWN}\\nPROXY={PLACEHOLDER}\" }}\n      - {{ message: fertig }}\n"
        ),
    );
    let (status, body) = env
        .json(
            "POST",
            "/v1/sessions",
            Some(json!({"target": "fake", "cwd": env.work(), "harness_opts": {"scenario": scenario}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    env.wait_for(&id, |e| e["type"] == "session.started").await;

    // Live-Client: alles, was der Hub verteilt (dauerhaft und transient).
    let session: SessionId = id.parse().unwrap();
    let (mut live, _) = env.daemon.runtime.hub.subscribe(session);
    let (status, body) = env
        .json(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "Zeig die Umgebung"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let e = tokio::time::timeout_at(deadline.into(), live.recv())
            .await
            .expect("Turn endet nicht")
            .unwrap();
        let done = e.type_name() == "turn.completed";
        seen.push(serde_json::to_value(&*e).unwrap());
        if done {
            break;
        }
    }
    let live_text = serde_json::to_string(&seen).unwrap();
    assert!(leaks(&live_text).is_empty(), "Live: {live_text}");
    assert!(
        seen.iter().any(|e| e["type"] == "message.delta"
            && e["payload"]["text"]
                .as_str()
                .unwrap_or_default()
                .contains("[REDACTED:github]")),
        "transientes Delta geschwärzt: {live_text}"
    );

    // Persistiert: `tool.call.completed` enthält nur Ersetzungen, der Platzhalter bleibt.
    let events = env.events(&id).await;
    let stored = serde_json::to_string(&events).unwrap();
    assert!(leaks(&stored).is_empty(), "Log: {stored}");
    let completed = events
        .iter()
        .find(|e| e["type"] == "tool.call.completed")
        .unwrap();
    let result = completed["payload"]["result"].to_string();
    assert!(result.contains("[REDACTED:github]"), "{result}");
    assert!(result.contains("[REDACTED:secret]"), "{result}");
    assert!(result.contains(PLACEHOLDER), "{result}");

    // Auch Export und Datenbank enthalten die Werte nicht.
    let (status, export) = env
        .http(
            "GET",
            &format!("/v1/sessions/{id}/export?with_raw=true"),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert!(leaks(&export).is_empty(), "Export: {export}");
    env.daemon.shutdown().await;
    let db = std::fs::read(env.dir.path().join("beton.db")).unwrap();
    let wal = std::fs::read(env.dir.path().join("beton.db-wal")).unwrap_or_default();
    for bytes in [db, wal] {
        for v in [KNOWN, GITHUB] {
            assert!(
                !bytes.windows(v.len()).any(|w| w == v.as_bytes()),
                "{v} in der DB"
            );
        }
    }
}

fn codex_cmd(scenario: &Path) -> String {
    let link = scenario.with_file_name("codex");
    if !link.exists() {
        std::os::unix::fs::symlink(fake_cli(), &link).unwrap();
    }
    format!(
        "{} --protocol app-server --scenario {}",
        link.display(),
        scenario.display()
    )
}

#[tokio::test]
async fn ses_007_ac3_handover_contains_no_secret_values_only_placeholders() {
    let env = Env::start(|dir| {
        let codex = dir.join("codex.yaml");
        std::fs::write(&codex, "turns: [{ emit: [{ echo_input: true }] }]\n").unwrap();
        vec![("BETON_CODEX_PATH", codex_cmd(&codex))]
    })
    .await;
    // Präpariertes Log: geschrieben, bevor der Wert als Secret bekannt war (ohne Redaction).
    let local = env.unredacted.ensure_local().await.unwrap();
    let id = SessionId::new();
    let scenario = env.write("fake.yaml", "turns: []\n");
    env.unredacted
        .create_session(
            local.org,
            NewSession {
                id,
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: env.work().display().to_string(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::Api,
                home_node: local.node,
                harness_opts: json!({ "scenario": scenario }),
                max_cost_usd: None,
            },
        )
        .await
        .unwrap();
    let turn = TurnId::new();
    let user = Actor::User {
        id: PrincipalId::User(UserId::LOCAL),
        device_id: None,
    };
    let message = |role, text: String| {
        EventPayload::MessageCompleted(MessageCompleted {
            message_id: format!("m_{}", TurnId::new()),
            role,
            content: vec![json!({ "type": "text", "text": text })],
            author: (role == MessageRole::User).then_some(PrincipalId::User(UserId::LOCAL)),
        })
    };
    let payloads = vec![
        (
            user.clone(),
            message(
                MessageRole::User,
                format!("Nutze das DB-Passwort {KNOWN} und den Proxy-Platzhalter {PLACEHOLDER}"),
            ),
        ),
        (
            Actor::default(),
            EventPayload::TurnStarted(TurnStarted {
                turn_id: turn,
                input_id: None,
                author: PrincipalId::User(UserId::LOCAL),
            }),
        ),
        (
            Actor::default(),
            message(
                MessageRole::Assistant,
                format!("Erledigt mit {KNOWN}, Token {GITHUB}"),
            ),
        ),
        (
            Actor::default(),
            EventPayload::TurnCompleted(TurnCompleted {
                turn_id: turn,
                stop_reason: "end_turn".into(),
                usage_summary: json!({}),
            }),
        ),
    ];
    let batch: Vec<Event> = payloads
        .into_iter()
        .map(|(actor, p)| {
            let mut e = Event::new(id, 0, actor, p);
            e.turn_id = Some(turn);
            e
        })
        .collect();
    let record = env.unredacted.session(local.org, id).await.unwrap();
    env.unredacted
        .append(local.org, id, record.head_seq, record.epoch, batch)
        .await
        .unwrap();

    let (status, body) = env
        .json(
            "POST",
            &format!("/v1/sessions/{id}/fork"),
            Some(json!({"harness": "codex"})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    env.wait_for(&child, |e| e["type"] == "session.started")
        .await;
    let doc = std::fs::read_to_string(
        env.work()
            .join(".beton/handover")
            .join(format!("{child}.md")),
    )
    .unwrap();
    assert!(leaks(&doc).is_empty(), "Handover: {doc}");
    assert!(doc.contains(PLACEHOLDER), "Platzhalter bleibt: {doc}");
    assert!(doc.contains("[REDACTED:"), "{doc}");
    // Auch der Plan für den Runner (Verlauf samt raw) ist geschwärzt.
    let plans: Vec<_> = walk(env.dir.path())
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter(|t| t.contains("from_session"))
        .collect();
    for plan in &plans {
        assert!(leaks(plan).is_empty(), "Fork-Plan: {plan}");
    }
    env.daemon.shutdown().await;
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
