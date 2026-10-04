//! Agent-Ausführung über die echte Prozesskette (AGT-004, AGT-005, AGT-010): Daemon im
//! Prozess, Runner-Binary, Fake-Harness bzw. Fake-Vendor-CLIs (QA-002). Keine echten
//! Vendor-CLIs, kein Netz außer Loopback.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
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

/// Testumgebung: eigenes Home (keine Dateien des Entwicklers), Projekt, Daemon.
struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

impl Env {
    fn home(dir: &Path) -> PathBuf {
        dir.join("home")
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    async fn start(vars: &[(&str, String)], prepare: impl FnOnce(&Path)) -> Self {
        // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = Self::home(dir.path());
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
            .filter(|(k, _)| !k.starts_with("BETON_"))
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
        let daemon = start_with(cfg, store.clone(), move |mut r| {
            r.sessions.provider = std::sync::Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
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

    async fn turn(&self, id: &str, text: &str) -> Vec<Value> {
        self.input(id, text).await;
        self.wait_for(id, |e| {
            matches!(
                e["type"].as_str(),
                Some("turn.completed" | "turn.failed" | "session.status")
            ) && (e["type"] != "session.status" || e["payload"]["status"] == "failed")
        })
        .await
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

/// Szenario: ein MCP-Tool aufrufen, dann antworten.
fn mcp_scenario(server: &str, tool: &str, args: Value) -> String {
    format!(
        "turns:\n  - expect_input: \"los\"\n    emit:\n      - {{ mcp_call: {{ server: {server}, tool: {tool}, args: {args} }} }}\n      - {{ message: \"fertig\" }}\n"
    )
}

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
    // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
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

/// Ergebnis des einzigen abgeschlossenen Tool-Calls als Text.
fn tool_result(events: &[Value]) -> String {
    let done = of_type(events, "tool.call.completed");
    assert_eq!(
        done.len(),
        1,
        "{}",
        serde_json::to_string_pretty(events).unwrap()
    );
    done[0]["payload"]["result"].to_string()
}

fn sc(dir: &Path, name: &str, text: &str) -> PathBuf {
    write(&dir.join(name), text)
}

impl Env {
    /// `POST /v1/sessions` mit Agent; liefert Status und Antwort.
    async fn create_agent(&self, body: Value) -> (u16, Value) {
        let mut body = body;
        body["cwd"] = json!(self.work());
        self.http("POST", "/v1/sessions", Some(body)).await
    }

    async fn sessions(&self) -> Vec<Value> {
        let (status, list) = self.http("GET", "/v1/sessions?limit=50", None).await;
        assert_eq!(status, 200, "{list}");
        list["items"].as_array().unwrap().clone()
    }
}

fn resolved(events: &[Value]) -> Value {
    let r = of_type(events, "agent.resolved");
    assert_eq!(
        r.len(),
        1,
        "{}",
        serde_json::to_string_pretty(events).unwrap()
    );
    r[0]["payload"].clone()
}

/// Text aller Assistant-Nachrichten.
fn answers(events: &[Value]) -> String {
    of_type(events, "message.completed")
        .iter()
        .filter(|e| e["payload"]["role"] == "assistant")
        .map(|e| e["payload"]["content"].to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

const ECHO: &str = "turns:\n  - emit: [{ echo_input: true }]\n  - emit: [{ echo_input: true }]\n";

fn fake_agent(dir: &Path, extra: &str) {
    write(
        &dir.join("work/.beton/agents/pr-fixer/agent.yaml"),
        &format!(
            "spec_version: 1\nname: pr-fixer\nversion: 0.3.0\nexecutor: {{ harness: fake }}\ninstructions:\n  file: prompts/system.md\n  append: \"Branch: {{{{ params.branch }}}}\"\n  project_files: none\nparams:\n  branch: {{ type: string, default: main }}\n  max_attempts: {{ type: integer, default: 3, minimum: 1, maximum: 10 }}\n{extra}"
        ),
    );
    write(
        &dir.join("work/.beton/agents/pr-fixer/prompts/system.md"),
        "Behebe die CI. Version A.",
    );
}

// --------------------------------------------------------------------------- AGT-004

#[tokio::test]
async fn agt_004_ac1_same_files_same_hash_and_a_prompt_change_changes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| fake_agent(dir, "")).await;
    let body = json!({"agent": "pr-fixer", "harness_opts": {"scenario": scenario}});
    let mut hashes = Vec::new();
    for _ in 0..2 {
        let (status, s) = env.create_agent(body.clone()).await;
        assert_eq!(status, 201, "{s}");
        let events = env.events(s["id"].as_str().unwrap()).await;
        let r = resolved(&events);
        assert_eq!(r["name"], "pr-fixer");
        assert_eq!(r["version"], "0.3.0");
        assert_eq!(r["source"], "project");
        assert!(
            r["blob_ref"].as_str().unwrap().starts_with("sha256:"),
            "{r}"
        );
        // Der Snapshot liegt als Blob der Session vor (DATA-006).
        let hex = r["blob_ref"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:");
        let (status, _) = env
            .http(
                "GET",
                &format!("/v1/sessions/{}/blobs/{hex}", s["id"].as_str().unwrap()),
                None,
            )
            .await;
        assert_eq!(status, 200);
        hashes.push(r["hash"].as_str().unwrap().to_owned());
        // Harness und Modell kommen aus `executor`.
        assert_eq!(s["harness"], "fake");
    }
    assert_eq!(hashes[0], hashes[1]);
    write(
        &env.work().join(".beton/agents/pr-fixer/prompts/system.md"),
        "Behebe die CI. Version B.",
    );
    let (_, s) = env.create_agent(body).await;
    let r = resolved(&env.events(s["id"].as_str().unwrap()).await);
    assert_ne!(r["hash"].as_str().unwrap(), hashes[0]);
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_004_ac2_fork_uses_the_snapshot_although_the_agent_files_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| fake_agent(dir, "")).await;
    let (status, s) = env
        .create_agent(json!({"agent": "pr-fixer", "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 201, "{s}");
    let id = s["id"].as_str().unwrap().to_owned();
    let events = env.turn(&id, "erste Aufgabe").await;
    // Fake-Harness: `first_message_prefix` (AGT-005).
    assert!(
        answers(&events).contains("Version A"),
        "{}",
        answers(&events)
    );
    let source = resolved(&events);

    // Agent-Dateien ändern, dann forken.
    write(
        &env.work().join(".beton/agents/pr-fixer/prompts/system.md"),
        "Behebe die CI. Version B.",
    );
    let (status, forked) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/fork"),
            Some(json!({"workspace": "shared"})),
        )
        .await;
    assert_eq!(status, 201, "{forked}");
    let fork = forked["session"]["id"].as_str().unwrap().to_owned();
    let events = env.turn(&fork, "zweite Aufgabe").await;
    let r = resolved(&events);
    assert_eq!(r["hash"], source["hash"]);
    assert_eq!(r["params"], source["params"]);
    let text = answers(&events);
    assert!(text.contains("Version A"), "{text}");
    assert!(!text.contains("Version B"), "{text}");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_004_ac3_timeout_interrupts_the_turn_and_the_run_ends_timed_out() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(
        tmp.path(),
        "hang.yaml",
        "turns:\n  - emit: [{ message_delta: \"Ich arbeite\" }, { hang: true }]\n",
    );
    let env = Env::start(&[], |dir| {
        write(
            &dir.join("work/.beton/agents/slow/agent.yaml"),
            "spec_version: 1\nname: slow\nexecutor: { harness: fake, timeout: 1s }\n",
        );
    })
    .await;
    let (status, s) = env
        .create_agent(json!({"agent": "slow", "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 201, "{s}");
    let id = s["id"].as_str().unwrap().to_owned();
    let started = Instant::now();
    env.input(&id, "los").await;
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "session.status" && e["payload"]["reason"] == "timed_out"
        })
        .await;
    assert!(started.elapsed() < Duration::from_secs(15));
    let interrupted = of_type(&events, "turn.interrupted");
    assert_eq!(
        interrupted.len(),
        1,
        "{}",
        serde_json::to_string_pretty(&events).unwrap()
    );
    assert_eq!(interrupted[0]["payload"]["reason"], "timed_out");
    let last = of_type(&events, "session.status").last().unwrap()["payload"].clone();
    assert_eq!(last["status"], "stopped");
    assert_eq!(last["reason"], "timed_out");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_004_harness_override_without_needed_capability_is_rejected_at_start() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| {
        // Sub-Agents brauchen MCP-Injektion; der Fake-Harness hat keine.
        write(
            &dir.join("work/.beton/agents/lead/agent.yaml"),
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  helper:\n    executor: { harness: codex }\nspawn: { agents: [helper] }\n",
        );
    })
    .await;
    let (status, problem) = env
        .create_agent(
            json!({"agent": "lead", "target": "fake", "harness_opts": {"scenario": scenario}}),
        )
        .await;
    assert_eq!(status, 422, "{problem}");
    assert_eq!(problem["code"], "harness_incompatible");
    assert!(env.sessions().await.is_empty());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_004_cli_overrides_are_recorded_in_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| {
        write(
            &dir.join("work/.beton/agents/x/agent.yaml"),
            "spec_version: 1\nname: x\nexecutor: { harness: claude, model: sonnet }\n",
        );
    })
    .await;
    let (status, s) = env
        .create_agent(json!({"agent": "x", "target": "fake", "model": "fake-large", "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 201, "{s}");
    assert_eq!(s["harness"], "fake");
    let r = resolved(&env.events(s["id"].as_str().unwrap()).await);
    assert_eq!(
        r["overrides"],
        json!({"harness": "fake", "model": "fake-large"})
    );
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-005

#[tokio::test]
async fn agt_005_instructions_and_rendered_params_reach_the_harness_once() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| fake_agent(dir, "")).await;
    let (_, s) = env
        .create_agent(json!({"agent": "pr-fixer", "params": {"branch": "develop"}, "harness_opts": {"scenario": scenario}}))
        .await;
    let id = s["id"].as_str().unwrap().to_owned();
    env.turn(&id, "eins").await;
    let events = env.turn(&id, "zwei").await;
    let text = answers(&events);
    assert_eq!(
        text.matches("Behebe die CI. Version A.").count(),
        1,
        "{text}"
    );
    assert!(text.contains("Branch: develop"), "{text}");
    // Im Log steht die Nutzer-Nachricht ohne Instructions.
    let users: Vec<String> = of_type(&events, "message.completed")
        .iter()
        .filter(|e| e["payload"]["role"] == "user")
        .map(|e| e["payload"]["content"].to_string())
        .collect();
    assert!(users.iter().all(|u| !u.contains("Version A")), "{users:?}");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_005_ac2_codex_gets_claude_md_and_not_agents_md() {
    let tmp = tempfile::tempdir().unwrap();
    let record = tmp.path().join("kontext.jsonl");
    let scenario = sc(
        tmp.path(),
        "c.yaml",
        "turns:\n  - emit: [{ message: \"ok\" }]\n",
    );
    let (key, mut value) = codex_path(&scenario);
    value.push_str(&format!(" --record {}", record.display()));
    let env = Env::start(&[(key, value)], |dir| {
        write(&dir.join("work/AGENTS.md"), "Regel aus AGENTS.md");
        write(&dir.join("work/CLAUDE.md"), "Regel aus CLAUDE.md");
        write(
            &dir.join("work/.beton/agents/rev/agent.yaml"),
            "spec_version: 1\nname: rev\nexecutor: { harness: codex }\ninstructions: { text: \"Reviewe knapp.\", project_files: auto }\n",
        );
    })
    .await;
    let (status, s) = env.create_agent(json!({"agent": "rev"})).await;
    assert_eq!(status, 201, "{s}");
    env.turn(s["id"].as_str().unwrap(), "los").await;
    let context = std::fs::read_to_string(&record).unwrap();
    let dev: Vec<Value> = context
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["source"] == "developer_instructions")
        .collect();
    assert_eq!(dev.len(), 1, "{context}");
    let text = dev[0]["text"].as_str().unwrap();
    assert!(text.contains("Reviewe knapp."), "{text}");
    assert_eq!(text.matches("Regel aus CLAUDE.md").count(), 1, "{text}");
    // AGENTS.md liest Codex selbst; beton liefert sie nicht doppelt.
    assert!(!context.contains("Regel aus AGENTS.md"), "{context}");
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-010

#[tokio::test]
async fn agt_010_ac1_value_above_maximum_is_rejected_before_the_start() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| fake_agent(dir, "")).await;
    let (status, problem) = env
        .create_agent(json!({"agent": "pr-fixer", "params": {"max_attempts": "20"}, "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 422, "{problem}");
    assert_eq!(problem["code"], "invalid_param");
    assert_eq!(problem["errors"][0]["pointer"], "/params/max_attempts");
    // Keine Session, kein Runner.
    assert!(env.sessions().await.is_empty());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_010_ac2_missing_required_param_is_reported_for_the_form() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| {
        fake_agent(
            dir,
            "  ticket: { type: string, required: true, description: Ticket-ID }\n",
        );
    })
    .await;
    let (status, problem) = env
        .create_agent(json!({"agent": "pr-fixer", "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 422, "{problem}");
    assert_eq!(problem["code"], "params_required");
    assert_eq!(
        problem["errors"],
        json!([{"pointer": "/params/ticket", "detail": "Pflichtparameter ohne Default fehlt (type: string)"}])
    );
    assert!(env.sessions().await.is_empty());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_010_ac3_resolved_params_are_in_agent_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let scenario = sc(tmp.path(), "echo.yaml", ECHO);
    let env = Env::start(&[], |dir| fake_agent(dir, "")).await;
    let (status, s) = env
        .create_agent(json!({"agent": "pr-fixer", "params": {"max_attempts": "5"}, "harness_opts": {"scenario": scenario}}))
        .await;
    assert_eq!(status, 201, "{s}");
    let r = resolved(&env.events(s["id"].as_str().unwrap()).await);
    // Aufgelöst: Text → integer, Default eingesetzt.
    assert_eq!(r["params"], json!({"branch": "main", "max_attempts": 5}));
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_010_session_spawn_passes_params_and_instructions_to_the_child() {
    let tmp = tempfile::tempdir().unwrap();
    let record = tmp.path().join("kind.jsonl");
    let parent = sc(
        tmp.path(),
        "parent.yaml",
        &mcp_scenario(
            "beton",
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "prüfe den Diff", "params": {"depth": "2"}}),
        ),
    );
    let child = sc(
        tmp.path(),
        "child.yaml",
        "turns:\n  - expect_input: \"prüfe den Diff\"\n    emit:\n      - { message: \"Keine Befunde.\" }\n",
    );
    let (key, mut value) = codex_path(&child);
    value.push_str(&format!(" --record {}", record.display()));
    let env = Env::start(&[claude_path(&parent), (key, value)], |dir| {
        write(
            &dir.join("work/.beton/agents/lead/agent.yaml"),
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  quick-check:\n    executor: { harness: codex, permission_mode: plan }\n    params: { depth: { type: integer, default: 1 } }\n    instructions: { text: \"Reviewe nur den Diff.\", append: \"Tiefe {{ params.depth }}\", project_files: none }\nspawn: { agents: [quick-check] }\n",
        );
    })
    .await;
    let (status, s) = env.create_agent(json!({"agent": "lead"})).await;
    assert_eq!(status, 201, "{s}");
    let id = s["id"].as_str().unwrap().to_owned();
    let events = env.turn(&id, "los").await;
    assert!(
        tool_result(&events).contains("Keine Befunde."),
        "{}",
        tool_result(&events)
    );
    let child = env
        .sessions()
        .await
        .into_iter()
        .find(|s| s["id"] != json!(id))
        .unwrap();
    let r = resolved(&env.events(child["id"].as_str().unwrap()).await);
    assert_eq!(r["name"], "quick-check");
    assert_eq!(r["source"], "subagent");
    assert_eq!(r["params"], json!({"depth": 2}));
    let context = std::fs::read_to_string(&record).unwrap();
    assert!(
        context.contains("Reviewe nur den Diff.\\n\\nTiefe 2"),
        "{context}"
    );
    env.daemon.shutdown().await;
}
