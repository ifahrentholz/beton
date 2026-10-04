//! Automatische Session-Titel (SES-010) mit echtem Runner, Fake-Harness bzw. Fake-CLI.
//! Keine echten Vendor-CLIs, kein Netz.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_server::app::Runtime;
use beton_server::titles::TitleGenerator;
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
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        (
            status,
            serde_json::from_str(&payload).unwrap_or(Value::Null),
        )
    }

    async fn create(&self, body: Value) -> String {
        let (status, body) = self.http("POST", "/v1/sessions", Some(body)).await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn create_fake(&self, cwd: &Path, scenario: &Path) -> String {
        self.create(json!({"target": "fake", "cwd": cwd, "harness_opts": {"scenario": scenario}}))
            .await
    }

    async fn input(&self, id: &str, text: &str) {
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({"text": text})),
            )
            .await;
        assert_eq!(status, 202, "{body}");
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

    async fn summary(&self, id: &str) -> Value {
        self.http("GET", &format!("/v1/sessions/{id}"), None)
            .await
            .1
    }

    async fn wait_status(&self, id: &str, want: &str) {
        let start = Instant::now();
        while self.summary(id).await["status"] != want {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Status {want} fehlt"
            );
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

fn is(t: &str) -> impl Fn(&Value) -> bool + '_ {
    move |e: &Value| e["type"] == t
}

fn titles(events: &[Value]) -> Vec<(String, String)> {
    events
        .iter()
        .filter(|e| e["type"] == "session.title_changed")
        .map(|e| {
            (
                e["payload"]["title"].as_str().unwrap().to_owned(),
                e["payload"]["source"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn title_costs(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["type"] == "cost.delta" && e["payload"]["purpose"] == "title")
        .cloned()
        .collect()
}

const ONE_TURN: &str = "turns:\n  - emit:\n      - { message: \"Erledigt.\" }\n";

fn with_generator(g: TitleGenerator) -> impl FnOnce(Runtime) -> Runtime {
    move |mut r| {
        r.titles.generator = g;
        r
    }
}

#[tokio::test]
async fn ses_010_ac1_generator_off_uses_heuristic_without_model_call() {
    let dir = tmp();
    let d = daemon(dir.path(), with_generator(TitleGenerator::Off)).await;
    // Würde der Fake gefragt, schlüge der Aufruf fehl – und es gäbe ein `cost.delta`.
    let sc = scenario(
        dir.path(),
        &format!("one_shot: {{ fail: \"kein Modellaufruf erwartet\" }}\n{ONE_TURN}"),
    );
    let id = d.create_fake(dir.path(), &sc).await;
    d.wait_status(&id, "idle").await;
    d.input(
        &id,
        "Die Login-Route braucht einen Rate-Limiter.\nBitte mit Tests.",
    )
    .await;
    let events = d.wait_for(&id, is("session.title_changed")).await;
    assert_eq!(
        titles(&events),
        vec![(
            "Die Login-Route braucht einen Rate-Limiter.".to_owned(),
            "generated".to_owned()
        )]
    );
    assert!(title_costs(&events).is_empty(), "kein Modellaufruf");
    let s = d.summary(&id).await;
    assert_eq!(s["title"], "Die Login-Route braucht einen Rate-Limiter.");
    assert_eq!(s["title_source"], "generated");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_010_ac3_generation_via_harness_emits_cost_delta_purpose_title() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    assert_eq!(
        beton_server::titles::TitlesConfig::default().generator,
        TitleGenerator::Auto
    );
    let sc = scenario(
        dir.path(),
        &format!("one_shot: {{ reply: \"„Rate-Limiter für Login“\" }}\n{ONE_TURN}"),
    );
    let id = d.create_fake(dir.path(), &sc).await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Die Login-Route braucht einen Rate-Limiter.")
        .await;
    let events = d.wait_for(&id, is("session.title_changed")).await;
    assert_eq!(
        titles(&events),
        vec![("Rate-Limiter für Login".to_owned(), "generated".to_owned())]
    );
    let costs = title_costs(&events);
    assert_eq!(costs.len(), 1, "{costs:?}");
    assert_eq!(costs[0]["payload"]["harness"], "fake");
    assert!(costs[0]["payload"]["input_tokens"].as_u64().unwrap() > 0);
    // Der Titel kommt nach dem ersten abgeschlossenen Turn.
    let turn_end = events.iter().position(is("turn.completed")).unwrap();
    let title_at = events.iter().position(is("session.title_changed")).unwrap();
    assert!(title_at > turn_end);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_010_auto_falls_back_to_heuristic_but_harness_mode_does_not() {
    let dir = tmp();
    let failing = format!("one_shot: {{ fail: \"Kontingent erschöpft\" }}\n{ONE_TURN}");
    let d = daemon(dir.path(), |r| r).await;
    let id = d
        .create_fake(dir.path(), &scenario(dir.path(), &failing))
        .await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Parser für YAML-Anker reparieren").await;
    let events = d.wait_for(&id, is("session.title_changed")).await;
    assert_eq!(
        titles(&events),
        vec![(
            "Parser für YAML-Anker reparieren".to_owned(),
            "generated".to_owned()
        )]
    );
    d.daemon.shutdown().await;

    let dir = tmp();
    let d = daemon(dir.path(), with_generator(TitleGenerator::Harness)).await;
    let id = d
        .create_fake(dir.path(), &scenario(dir.path(), &failing))
        .await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Parser für YAML-Anker reparieren").await;
    d.wait_for(&id, is("turn.completed")).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(titles(&d.events(&id).await).is_empty());
    assert_eq!(d.summary(&id).await["title"], "");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_010_ac2_after_user_rename_no_title_is_generated() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    // Der Turn läuft lange genug, um währenddessen umzubenennen.
    let sc = scenario(
        dir.path(),
        "turns:\n  - emit:\n      - { message: \"Erledigt.\", delay_ms: 800 }\n  - emit:\n      - { message: \"Auch erledigt.\" }\n",
    );
    let id = d.create_fake(dir.path(), &sc).await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Erste Aufgabe").await;
    let (status, body) = d
        .http(
            "PATCH",
            &format!("/v1/sessions/{id}"),
            Some(json!({"title": "Mein Titel"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    d.wait_for(&id, is("turn.completed")).await;
    d.input(&id, "Zweite Aufgabe").await;
    let start = Instant::now();
    while d
        .events(&id)
        .await
        .iter()
        .filter(|e| e["type"] == "turn.completed")
        .count()
        < 2
    {
        assert!(start.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let events = d.events(&id).await;
    assert_eq!(
        titles(&events),
        vec![("Mein Titel".to_owned(), "user".to_owned())]
    );
    assert!(title_costs(&events).is_empty());
    let s = d.summary(&id).await;
    assert_eq!(
        (s["title"].clone(), s["title_source"].clone()),
        (json!("Mein Titel"), json!("user"))
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_010_sessions_with_a_title_keep_it() {
    let dir = tmp();
    let d = daemon(dir.path(), |r| r).await;
    let sc = scenario(dir.path(), ONE_TURN);
    let id = d
        .create(json!({"target": "fake", "cwd": dir.path(), "title": "Vorgegeben", "harness_opts": {"scenario": sc}}))
        .await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Etwas tun").await;
    d.wait_for(&id, is("turn.completed")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        titles(&d.events(&id).await),
        vec![("Vorgegeben".to_owned(), "user".to_owned())]
    );
    d.daemon.shutdown().await;
}

/// Claude-Session über die Fake-CLI; `env` kommt zur Umgebung des Daemons hinzu.
async fn claude_title(env: Vec<(&str, &str)>) -> (Vec<Value>, Vec<Value>) {
    let dir = tmp();
    let fake = PathBuf::from(runner_bin()).with_file_name("beton-fake-cli");
    if !fake.is_file() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "beton-fake-cli"])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let sc = scenario(
        dir.path(),
        "one_shot: { reply: \"Rate-Limiter für die Login-Route\" }\nturns:\n  - expect_input: \"Die Login-Route braucht einen Rate-Limiter.\"\n    emit:\n      - { message: \"Mache ich.\" }\n",
    );
    let record = dir.path().join("record.jsonl");
    // Ohne API-Keys und ohne `providers`-Konfiguration (ADR-0034).
    let mut inherit: std::collections::BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| !k.ends_with("_API_KEY") && k != "ANTHROPIC_AUTH_TOKEN")
        .collect();
    inherit.insert(
        "BETON_CLAUDE_PATH".into(),
        format!(
            "{} --protocol stream-json --scenario {} --record {}",
            fake.display(),
            sc.display(),
            record.display()
        ),
    );
    for (k, v) in env {
        inherit.insert(k.into(), v.into());
    }
    let runners = dir.path().join("runners");
    let d = daemon(dir.path(), move |mut r| {
        r.sessions.provider = std::sync::Arc::new(
            LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
        );
        r
    })
    .await;
    let id = d
        .create(json!({"target": "claude", "cwd": dir.path()}))
        .await;
    d.wait_status(&id, "idle").await;
    d.input(&id, "Die Login-Route braucht einen Rate-Limiter.")
        .await;
    let events = d.wait_for(&id, is("session.title_changed")).await;
    let calls: Vec<Value> = std::fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    d.daemon.shutdown().await;
    (events, calls)
}

fn args_of(call: &Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn ses_010_ac4_claude_title_via_cli_one_shot_without_api_key() {
    let (events, calls) = claude_title(Vec::new()).await;
    assert_eq!(
        titles(&events),
        vec![(
            "Rate-Limiter für die Login-Route".to_owned(),
            "generated".to_owned()
        )]
    );
    // Genau ein Einmal-Aufruf der `claude`-CLI im nicht-interaktiven Modus.
    assert_eq!(calls.len(), 1, "{calls:?}");
    let args = args_of(&calls[0]);
    let joined = args.join(" ");
    for want in [
        "-p",
        "--output-format json",
        "--model haiku",
        "--no-session-persistence",
    ] {
        assert!(joined.contains(want), "{want} fehlt: {joined}");
    }
    assert!(
        !joined.contains("Rate-Limiter"),
        "Inhalt nicht in argv: {joined}"
    );
    assert!(
        calls[0]["stdin_bytes"].as_u64().unwrap() > 0,
        "Inhalt über stdin"
    );
    // Bereinigte Umgebung: keine API-Keys im CLI-Prozess.
    assert_eq!(calls[0]["api_key_vars"], json!([]));
    // Verbrauch zählt zur Subscription, nicht als Ausgabe.
    let costs = title_costs(&events);
    assert_eq!(costs.len(), 1);
    assert_eq!(costs[0]["payload"]["harness"], "claude");
    assert_eq!(costs[0]["payload"]["auth_source"], "vendor_cli");
    assert_eq!(costs[0]["payload"]["source"], "subscription");
    assert!(costs[0]["payload"].get("cost_micro").is_none());
}

#[tokio::test]
async fn ses_010_ac4_api_keys_in_the_daemon_env_never_reach_the_one_shot() {
    let (events, calls) = claude_title(vec![
        ("ANTHROPIC_API_KEY", "sk-ant-test-nicht-verwenden"),
        ("ANTHROPIC_AUTH_TOKEN", "test-token"),
    ])
    .await;
    assert_eq!(titles(&events).len(), 1);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["api_key_vars"], json!([]), "{calls:?}");
    assert_eq!(
        title_costs(&events)[0]["payload"]["auth_source"],
        "vendor_cli"
    );
}
