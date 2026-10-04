//! Direkt-API-Harness, Compaction und Kontextanzeige über die echte Prozesskette
//! (HAR-002, HAR-009, HAR-010, HAR-011, HAR-022, SES-011, AGT-008): Daemon im Prozess,
//! Runner-Binary, Fake-Vendor-CLIs (QA-002) und der lokale Mock-Server des
//! Direkt-API-Harness auf Loopback. Kein echter Anbieter, kein echter Key, kein Internet.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_harness::registry::HarnessLayers;
use beton_harness_direct::mock::{self, MockServer, Say};
use beton_host::LocalProvider;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Offensichtlich falscher Test-Key (kein echtes Key-Muster).
const MARKER: &str = "bt-fake-key-MARKER-e2e-5d1c";
const VAR: &str = "OPENROUTER_API_KEY";

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
    token: String,
    addr: SocketAddr,
}

/// `providers.openrouter` gegen den Mock (OpenAI-Wire-Format).
fn openrouter(server: &MockServer) -> BTreeMap<String, Value> {
    [(
        "openrouter".to_owned(),
        json!({
            "kind": "openai",
            "base_url": server.base_url("/v1"),
            "api_key_env": VAR,
            "models": [{"id": "qwen/qwen3-coder", "context_window": 262144}],
        }),
    )]
    .into()
}

impl Env {
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    /// Daemon mit `providers` (User-Ebene) und Umgebung `vars` für die Runner.
    async fn start(
        providers: BTreeMap<String, Value>,
        vars: &[(&str, String)],
        prepare: impl FnOnce(&Path),
    ) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        prepare(dir.path());
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        let mut cfg = ServerConfig::local(dir.path().to_path_buf());
        cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
        cfg.socket = None;
        let mut inherit: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| !k.starts_with("BETON_") && !k.ends_with("_API_KEY"))
            .collect();
        inherit.insert("HOME".into(), home.display().to_string());
        inherit.insert(
            "BETON_HOME".into(),
            home.join(".beton").display().to_string(),
        );
        for (k, v) in vars {
            inherit.insert((*k).to_owned(), v.clone());
        }
        let runners = dir.path().join("runners");
        let layers = HarnessLayers {
            providers: providers.clone(),
            ..HarnessLayers::default()
        };
        let daemon = start_with(cfg, store, move |mut r| {
            r.sessions.provider = Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r.sessions.providers = providers;
            r.harnesses = beton_runner::builtin_registry_with_options(
                &layers,
                true,
                &beton_harness_direct::DirectOptions::default(),
            )
            .0;
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

    async fn create(&self, target: &str) -> String {
        let (status, body) = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": target, "cwd": self.work()})),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
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
                start.elapsed() < Duration::from_secs(30),
                "Event fehlt: {}",
                serde_json::to_string_pretty(&events).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wartet auf das Turn-Ende und gibt dabei jede offene Freigabe frei (WEB-018).
    async fn turn_approving(&self, id: &str, text: &str) -> Vec<Value> {
        self.input(id, text).await;
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            if events.iter().any(|e| {
                matches!(e["type"].as_str(), Some("turn.completed" | "turn.failed"))
                    || (e["type"] == "session.status" && e["payload"]["status"] == "failed")
            }) {
                return events;
            }
            let (_, approvals) = self
                .http("GET", &format!("/v1/sessions/{id}/approvals"), None)
                .await;
            for a in approvals["items"].as_array().into_iter().flatten() {
                let aid = a["id"].as_str().unwrap();
                self.http(
                    "POST",
                    &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
                    Some(json!({"decision": "allow"})),
                )
                .await;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "Turn endet nicht: {}",
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

fn write(path: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    path.to_owned()
}

fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

/// Alle Dateien unter `root` (rekursiv), die `needle` enthalten.
fn files_containing(root: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file()
                && let Ok(bytes) = std::fs::read(&p)
                && bytes.windows(needle.len()).any(|w| w == needle)
            {
                out.push(p);
            }
        }
    }
    out
}

fn tool(id: &str, name: &str, input: Value) -> Say {
    Say::Tool {
        id: id.into(),
        name: name.into(),
        input,
    }
}

const PATH: &str = "/v1/chat/completions";

// ------------------------------------------------------------------------- HAR-002 AC1

#[tokio::test]
async fn har_002_ac1_catalog_lists_the_configured_direct_provider() {
    let server = MockServer::start().await.unwrap();
    let env = Env::start(openrouter(&server), &[], |_| {}).await;
    let (status, page) = env.http("GET", "/v1/harnesses?host=hst_local", None).await;
    assert_eq!(status, 200, "{page}");
    let entry = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == "direct:openrouter")
        .unwrap_or_else(|| panic!("direct:openrouter fehlt: {page}"))
        .clone();
    // Gegen das generierte Schema: als HarnessInfo lesbar, vollständige Capabilities.
    let info: beton_harness::registry::HarnessInfo = serde_json::from_value(entry).unwrap();
    let caps = &info.capabilities[0];
    assert_eq!(caps.transport, beton_harness::Transport::InProc);
    assert_eq!(caps.compaction, beton_harness::CompactionSupport::Native);
    assert_eq!(caps.models, ["qwen/qwen3-coder"]);
    assert!(caps.mcp_injection);
    env.daemon.shutdown().await;
    // ADR-0034: Ohne `providers` gibt es keinen Direkt-API-Harness.
    let plain = Env::start(BTreeMap::new(), &[], |_| {}).await;
    let (_, page) = plain
        .http("GET", "/v1/harnesses?host=hst_local", None)
        .await;
    assert!(
        !page["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["id"].as_str().unwrap().starts_with("direct:")),
        "{page}"
    );
    plain.daemon.shutdown().await;
}

// ---------------------------------------------------------------- HAR-011 AC5 (Sicherheit)

#[tokio::test]
async fn har_011_ac5_daemon_key_reaches_only_the_provider() {
    let server = MockServer::start().await.unwrap();
    server.push(
        PATH,
        mock::openai(
            "chatcmpl-1",
            &[
                Say::Text("Ich schaue nach.".into(), 4),
                tool(
                    "call_mock000000000001",
                    "fs_glob",
                    json!({"pattern": "*.md"}),
                ),
            ],
        ),
    );
    server.push(
        PATH,
        mock::openai(
            "chatcmpl-2",
            &[
                Say::Text("Eine Datei.".into(), 4),
                Say::Usage {
                    input: 50,
                    output: 5,
                },
            ],
        ),
    );
    let env = Env::start(openrouter(&server), &[(VAR, MARKER.to_owned())], |dir| {
        write(&dir.join("work/README.md"), "# Projekt\n");
    })
    .await;
    let id = env.create("direct:openrouter").await;
    let events = env
        .turn_approving(&id, "Welche Markdown-Dateien gibt es?")
        .await;
    assert!(!of_type(&events, "turn.completed").is_empty(), "{events:?}");
    let done = of_type(&events, "tool.call.completed");
    assert_eq!(done[0]["payload"]["result"], "README.md");
    // Der Key ging an den Anbieter …
    let reqs = server.requests_to(PATH);
    assert_eq!(reqs.len(), 2);
    for r in &reqs {
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {MARKER}").as_str())
        );
        assert_eq!(r.body["model"], "qwen/qwen3-coder");
    }
    // … aber nicht ins Event-Log und in keine Datei von Daemon, Runner oder Workspace.
    let log = serde_json::to_string(&events).unwrap();
    assert!(!log.contains(MARKER));
    env.daemon.shutdown().await;
    let leaks = files_containing(env.dir.path(), MARKER.as_bytes());
    assert!(leaks.is_empty(), "Key in Dateien: {leaks:?}");
}

#[tokio::test]
async fn har_011_ac5_missing_key_variable_fails_the_session_start_with_a_hint() {
    let server = MockServer::start().await.unwrap();
    let env = Env::start(openrouter(&server), &[], |_| {}).await;
    let id = env.create("direct:openrouter").await;
    env.http(
        "POST",
        &format!("/v1/sessions/{id}/input"),
        Some(json!({"text": "hallo"})),
    )
    .await;
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "session.status" && e["payload"]["status"] == "failed"
        })
        .await;
    let text = serde_json::to_string(&events).unwrap();
    assert!(
        text.contains(VAR) && text.contains("providers.openrouter.api_key_env"),
        "{text}"
    );
    assert!(server.requests().is_empty(), "kein Request ohne Key");
    env.daemon.shutdown().await;
}

// ------------------------------------------------------------------ HAR-009 und AGT-008

#[tokio::test]
async fn agt_008_ac2_har_009_ac1_skill_load_and_policy_query_on_the_direct_harness() {
    let server = MockServer::start().await.unwrap();
    server.push(
        PATH,
        mock::openai(
            "chatcmpl-1",
            &[tool(
                "call_mock000000000001",
                "mcp__beton__skill_load",
                json!({"name": "fix-ci"}),
            )],
        ),
    );
    server.push(
        PATH,
        mock::openai(
            "chatcmpl-2",
            &[tool(
                "call_mock000000000002",
                "mcp__beton__policy_query",
                json!({"action": "git push"}),
            )],
        ),
    );
    server.push(
        PATH,
        mock::openai("chatcmpl-3", &[Say::Text("fertig".into(), 3)]),
    );
    let env = Env::start(openrouter(&server), &[(VAR, MARKER.to_owned())], |dir| {
        write(
            &dir.join("work/.beton/skills/fix-ci/SKILL.md"),
            "---\nname: fix-ci\ndescription: Behebt CI\n---\n# Fix CI\n\nLogs lesen.\n",
        );
    })
    .await;
    let id = env.create("direct:openrouter").await;
    let events = env.turn_approving(&id, "los").await;
    let ready = of_type(&events, "harness.ready");
    let tools = ready[0]["payload"]["tools"].to_string();
    assert!(
        tools.contains("mcp__beton__skill_load") && tools.contains("fs_read"),
        "{tools}"
    );
    let requested = of_type(&events, "tool.call.requested");
    assert_eq!(requested[0]["payload"]["mcp_server"], "beton");
    assert_eq!(requested[0]["payload"]["source"], "beton_mcp");
    let done = of_type(&events, "tool.call.completed");
    assert_eq!(done.len(), 2, "{events:?}");
    let skill = done[0]["payload"]["result"].as_str().unwrap();
    assert!(skill.contains("# Fix CI"), "{skill}");
    assert!(!skill.contains("description: Behebt CI"), "{skill}");
    let policy = done[1]["payload"]["result"].as_str().unwrap();
    assert!(
        policy.contains("decision") && policy.contains("ask"),
        "{policy}"
    );
    // Das Modell bekam den Skill-Text als Tool-Ergebnis.
    let second = &server.requests_to(PATH)[1].body;
    assert!(second.to_string().contains("Fix CI"), "{second}");
    env.daemon.shutdown().await;
}

// ------------------------------------------------------------------------------ SES-011

fn claude_path(scenario: &Path) -> (&'static str, String) {
    (
        "BETON_CLAUDE_PATH",
        format!(
            "{} --protocol stream-json --scenario {}",
            fake_cli().display(),
            scenario.display()
        ),
    )
}

fn codex_path(scenario: &Path) -> (&'static str, String) {
    let link = scenario.with_file_name("codex");
    if !link.exists() {
        std::os::unix::fs::symlink(fake_cli(), &link).unwrap();
    }
    (
        "BETON_CODEX_PATH",
        format!(
            "{} --protocol app-server --scenario {}",
            link.display(),
            scenario.display()
        ),
    )
}

/// Zahl der `context.usage` je Turn (zwischen `turn.started` und dem Turn-Ende).
fn context_per_turn(events: &[Value]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut current: Option<usize> = None;
    for e in events {
        match e["type"].as_str() {
            Some("turn.started") => current = Some(0),
            Some("context.usage") => {
                if let Some(c) = current.as_mut() {
                    *c += 1;
                }
            }
            Some("turn.completed" | "turn.failed" | "turn.interrupted") => {
                out.extend(current.take());
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn ses_011_ac1_exactly_one_context_usage_per_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = write(
        &tmp.path().join("c.yaml"),
        "turns:\n  - emit:\n      - { message: \"eins\" }\n      - { usage: { input_tokens: 1000, output_tokens: 10, context_window: 200000 } }\n  - emit:\n      - { message: \"zwei\" }\n      - { usage: { input_tokens: 2000, output_tokens: 10 } }\n",
    );
    // Codex meldet den Kontext je Usage-Update; der Runner bündelt auf einen je Turn.
    let codex = write(
        &tmp.path().join("x.yaml"),
        "turns:\n  - emit:\n      - { usage: { input_tokens: 10, output_tokens: 1 } }\n      - { message: \"eins\" }\n      - { usage: { input_tokens: 30, output_tokens: 2 } }\n",
    );
    let env = Env::start(
        BTreeMap::new(),
        &[claude_path(&claude), codex_path(&codex)],
        |_| {},
    )
    .await;
    let id = env.create("claude").await;
    env.turn_approving(&id, "eins").await;
    env.wait_for(&id, |e| {
        e["type"] == "session.status" && e["payload"]["status"] == "idle"
    })
    .await;
    env.input(&id, "zwei").await;
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "message.completed" && e["payload"]["content"][0]["text"] == "zwei"
        })
        .await;
    let _ = events;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = env.events(&id).await;
    assert_eq!(context_per_turn(&events), [1, 1], "{events:#?}");
    let id = env.create("codex").await;
    env.turn_approving(&id, "eins").await;
    let events = env.events(&id).await;
    assert_eq!(context_per_turn(&events), [1]);
    // Der gebündelte Wert ist der letzte Stand des Turns.
    let ctx = of_type(&events, "context.usage");
    assert_eq!(ctx[0]["payload"]["used_tokens"], 32);
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_011_ac2_compact_api_sends_compact_to_claude() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = write(
        &tmp.path().join("c.yaml"),
        "turns:\n  - emit:\n      - { message: \"lang\" }\n      - { usage: { input_tokens: 120000, output_tokens: 50, context_window: 200000 } }\n",
    );
    let env = Env::start(BTreeMap::new(), &[claude_path(&claude)], |_| {}).await;
    let id = env.create("claude").await;
    env.turn_approving(&id, "viel").await;
    let (status, body) = env
        .http("POST", &format!("/v1/sessions/{id}/compact"), None)
        .await;
    assert_eq!(status, 202, "{body}");
    let events = env
        .wait_for(&id, |e| e["type"] == "compaction.completed")
        .await;
    let started = of_type(&events, "compaction.started");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["payload"]["before_tokens"], 120_000);
    let done = of_type(&events, "compaction.completed");
    assert!(done[0]["payload"]["after_tokens"].as_u64().unwrap() < 120_000);
    let ctx = env
        .wait_for(&id, |e| {
            e["type"] == "context.usage" && e["payload"]["used_tokens"].as_u64() < Some(120_000)
        })
        .await;
    assert!(!of_type(&ctx, "context.usage").is_empty());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_011_ac3_har_022_ac2_compact_without_capability_is_409() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = write(
        &tmp.path().join("x.yaml"),
        "turns:\n  - emit:\n      - { message: \"ok\" }\n",
    );
    let env = Env::start(BTreeMap::new(), &[codex_path(&codex)], |_| {}).await;
    // Codex: `compaction: none` bis zur Durchreichung (Folge-Issue); die API lehnt ab.
    let id = env.create("codex").await;
    env.turn_approving(&id, "hallo").await;
    let (status, problem) = env
        .http("POST", &format!("/v1/sessions/{id}/compact"), None)
        .await;
    assert_eq!(status, 409, "{problem}");
    assert_eq!(problem["code"], "capability_unsupported");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_011_compact_api_on_the_direct_harness_runs_its_own_compaction() {
    let server = MockServer::start().await.unwrap();
    server.push(
        PATH,
        mock::openai(
            "chatcmpl-1",
            &[
                Say::Text("Antwort".into(), 3),
                Say::Usage {
                    input: 400,
                    output: 20,
                },
            ],
        ),
    );
    server.push(
        PATH,
        mock::openai("chatcmpl-2", &[Say::Text("Zusammenfassung".into(), 50)]),
    );
    let env = Env::start(openrouter(&server), &[(VAR, MARKER.to_owned())], |_| {}).await;
    let id = env.create("direct:openrouter").await;
    env.turn_approving(&id, "erste Frage").await;
    let (status, body) = env
        .http("POST", &format!("/v1/sessions/{id}/compact"), None)
        .await;
    assert_eq!(status, 202, "{body}");
    let events = env
        .wait_for(&id, |e| e["type"] == "compaction.completed")
        .await;
    assert_eq!(of_type(&events, "compaction.started").len(), 1);
    assert_eq!(server.requests_to(PATH).len(), 2, "Summary-Request");
    env.daemon.shutdown().await;
}
