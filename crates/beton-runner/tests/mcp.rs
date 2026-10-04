//! MCP-Injektion, System-Tools und Skills über die echte Prozesskette (HAR-009, AGT-006,
//! AGT-007, AGT-008): Daemon im Prozess, Runner-Binary, Fake-Vendor-CLI (QA-002) als Harness
//! und das Runner-Binary als MCP-Relay. Keine echten Vendor-CLIs, kein Netz außer Loopback.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_host::LocalProvider;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, Store, StoreOptions};
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
    store: Store,
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
            store,
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

/// ACP-Agent `fake` im Projekt registrieren (HAR-008).
fn acp_config(work: &Path, scenario: &Path, extra: &str) {
    write(
        &work.join(".beton/config.yaml"),
        &format!(
            "harnesses:\n  acp:\n    agents:\n      fake:\n        command: {}\n        args: [\"--protocol\", \"acp\", \"--scenario\", \"{}\"]\n{extra}",
            fake_cli().display(),
            scenario.display()
        ),
    );
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

// --------------------------------------------------------------------------- HAR-009 AC1

#[tokio::test]
async fn har_009_ac1_policy_query_answers_from_the_runner_in_claude() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "c.yaml",
        &mcp_scenario("beton", "policy_query", json!({"action": "git push"})),
    );
    let env = Env::start(&[claude_path(&s)], |_| {}).await;
    let id = env.create("claude").await;
    let events = env.turn(&id, "los").await;
    let req = of_type(&events, "tool.call.requested");
    assert_eq!(req[0]["payload"]["tool"], "policy_query");
    assert_eq!(req[0]["payload"]["mcp_server"], "beton");
    assert_eq!(req[0]["payload"]["source"], "beton_mcp");
    let result = tool_result(&events);
    assert!(
        result.contains("decision") && result.contains("ask"),
        "{result}"
    );
    let ready = of_type(&events, "harness.ready");
    assert!(
        ready[0]["payload"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "mcp__beton__policy_query"),
        "{}",
        ready[0]
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_009_ac1_policy_query_answers_from_the_runner_in_codex() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "x.yaml",
        &mcp_scenario("beton", "policy_query", json!({"action": "git push"})),
    );
    let env = Env::start(&[codex_path(&s)], |_| {}).await;
    let id = env.create("codex").await;
    let events = env.turn(&id, "los").await;
    let req = of_type(&events, "tool.call.requested");
    assert_eq!(req[0]["payload"]["source"], "beton_mcp");
    let result = tool_result(&events);
    assert!(
        result.contains("decision") && result.contains("ask"),
        "{result}"
    );
    let ready = of_type(&events, "harness.ready");
    assert_eq!(ready[0]["payload"]["mcp_servers"], json!(["beton"]));
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_009_ac1_policy_query_answers_from_the_runner_in_acp() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "a.yaml",
        &mcp_scenario("beton", "policy_query", json!({"action": "git push"})),
    );
    let env = Env::start(&[], |dir| acp_config(&dir.join("work"), &s, "")).await;
    let id = env.create("acp:fake").await;
    let events = env.turn(&id, "los").await;
    let result = tool_result(&events);
    assert!(
        result.contains("decision") && result.contains("ask"),
        "{result}"
    );
    let ready = of_type(&events, "harness.ready");
    assert_eq!(ready[0]["payload"]["mcp_servers"], json!(["beton"]));
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- HAR-009 AC2

#[tokio::test]
async fn har_009_ac2_without_bridge_the_beton_server_is_not_passed() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(tmp.path(), "a.yaml", &mcp_scenario("demo", "a", json!({})));
    let fake = fake_cli();
    let env = Env::start(&[], |dir| {
        let work = dir.join("work");
        acp_config(&work, &s, "        mcp_bridge: false\n");
        write(
            &work.join(".beton/mcp.yaml"),
            &format!(
                "servers:\n  demo:\n    command: {}\n    args: [\"--protocol\", \"mcp-server\", \"--tools\", \"a\"]\n",
                fake.display()
            ),
        );
    })
    .await;
    let id = env.create("acp:fake").await;
    let events = env.turn(&id, "los").await;
    let ready = of_type(&events, "harness.ready");
    assert_eq!(ready[0]["payload"]["mcp_servers"], json!(["demo"]));
    // Andere Server bleiben nutzbar.
    assert!(tool_result(&events).contains("a: {}"), "{events:?}");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_009_ac2_claude_without_bridge_gets_no_beton_server() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "c.yaml",
        "turns:\n  - emit:\n      - { message: \"ok\" }\n",
    );
    let env = Env::start(&[claude_path(&s)], |dir| {
        write(
            &dir.join("work/.beton/config.yaml"),
            "harnesses:\n  claude:\n    mcp_bridge: false\n",
        );
    })
    .await;
    let id = env.create("claude").await;
    let events = env.turn(&id, "hallo").await;
    let ready = of_type(&events, "harness.ready");
    assert_eq!(
        ready[0]["payload"]["mcp_servers"],
        json!([]),
        "{}",
        ready[0]
    );
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-006

#[tokio::test]
async fn agt_006_ac1_tools_outside_allow_are_invisible_and_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(tmp.path(), "c.yaml", &mcp_scenario("demo", "b", json!({})));
    let fake = fake_cli();
    let env = Env::start(&[claude_path(&s)], |dir| {
        write(
            &dir.join("work/.beton/mcp.yaml"),
            &format!(
                "servers:\n  demo:\n    command: {}\n    args: [\"--protocol\", \"mcp-server\", \"--tools\", \"a,b,c\"]\n    allow: [a]\n",
                fake.display()
            ),
        );
    })
    .await;
    let id = env.create("claude").await;
    let events = env.turn(&id, "los").await;
    let ready = of_type(&events, "harness.ready");
    let tools: Vec<&str> = ready[0]["payload"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|t| t.starts_with("mcp__demo__"))
        .collect();
    assert_eq!(tools, ["mcp__demo__a"]);
    let done = of_type(&events, "tool.call.completed");
    assert_eq!(done[0]["payload"]["status"], "error");
    assert!(
        done[0]["payload"]["result"]
            .to_string()
            .contains("tool_not_enabled"),
        "{}",
        done[0]
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_006_ac4_har_009_ac4_unstartable_server_fails_without_stopping_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "c.yaml",
        "turns:\n  - expect_input: \"los\"\n    emit:\n      - { message: \"läuft weiter\" }\n",
    );
    let env = Env::start(&[claude_path(&s)], |dir| {
        write(
            &dir.join("home/.beton/mcp.yaml"),
            "servers:\n  kaputt:\n    command: /nicht/vorhanden/mcp-server\n",
        );
    })
    .await;
    let id = env.create("claude").await;
    let events = env.turn(&id, "los").await;
    let failed = of_type(&events, "mcp.server_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["payload"]["name"], "kaputt");
    assert!(
        failed[0]["payload"]["error"]
            .as_str()
            .unwrap()
            .contains("Start fehlgeschlagen")
    );
    assert!(!of_type(&events, "turn.completed").is_empty());
    let text = of_type(&events, "message.completed")
        .into_iter()
        .find(|e| e["payload"]["role"] == "assistant")
        .unwrap();
    assert_eq!(text["payload"]["content"][0]["text"], "läuft weiter");
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-007 AC2

#[tokio::test]
async fn agt_007_ac2_session_spawn_runs_a_codex_child_and_returns_its_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = sc(
        tmp.path(),
        "parent.yaml",
        &mcp_scenario(
            "beton",
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "prüfe den Diff", "async": false}),
        ),
    );
    let child = sc(
        tmp.path(),
        "child.yaml",
        "turns:\n  - expect_input: \"prüfe den Diff\"\n    emit:\n      - { message: \"Keine Befunde.\" }\n",
    );
    let env = Env::start(&[claude_path(&parent), codex_path(&child)], |dir| {
        write(
            &dir.join("work/.beton/agents/lead/agent.yaml"),
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  quick-check:\n    executor: { harness: codex, permission_mode: plan }\n    instructions: { text: \"Reviewe nur den Diff.\" }\nspawn: { agents: [quick-check] }\n",
        );
    })
    .await;
    // Eine Session mit Agent (`agent_ref`); `beton run <agent>` folgt mit AGT-004.
    let local = env.store.ensure_local().await.unwrap();
    let session = env
        .store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "claude".into(),
                cwd: env.work().display().to_string(),
                model: None,
                agent_ref: Some("lead".into()),
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
            },
        )
        .await
        .unwrap();
    let id = session.id.to_string();
    let events = env.turn(&id, "los").await;
    let ready = of_type(&events, "harness.ready");
    assert!(
        ready[0]["payload"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "mcp__beton__session_spawn"),
        "{}",
        ready[0]
    );
    let result = tool_result(&events);
    assert!(result.contains("Keine Befunde."), "{result}");
    // Die Child-Session läuft auf Codex und hängt am Parent.
    let (status, list) = env.http("GET", "/v1/sessions?limit=50", None).await;
    assert_eq!(status, 200, "{list}");
    let child = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] != json!(id))
        .unwrap()
        .clone();
    assert_eq!(child["harness"], "codex");
    let child_events = env.events(child["id"].as_str().unwrap()).await;
    let created = &child_events[0]["payload"];
    assert_eq!(created["parent_session_id"], json!(id));
    assert_eq!(created["kind"], "subagent");
    assert_eq!(created["trigger"], "spawn");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_007_ac1_session_without_spawn_agents_cannot_call_session_spawn() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "c.yaml",
        &mcp_scenario(
            "beton",
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "p"}),
        ),
    );
    let env = Env::start(&[claude_path(&s)], |_| {}).await;
    let id = env.create("claude").await;
    let events = env.turn(&id, "los").await;
    let ready = of_type(&events, "harness.ready");
    assert!(
        !ready[0]["payload"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "mcp__beton__session_spawn")
    );
    let done = of_type(&events, "tool.call.completed");
    assert_eq!(done[0]["payload"]["status"], "error");
    assert!(
        done[0]["payload"]["result"]
            .to_string()
            .contains("tool_not_enabled")
    );
    // Keine Child-Session.
    let (_, list) = env.http("GET", "/v1/sessions?limit=50", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-008

#[tokio::test]
async fn agt_008_ac2_skill_load_returns_the_body_without_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let s = sc(
        tmp.path(),
        "a.yaml",
        &mcp_scenario("beton", "skill_load", json!({"name": "fix-ci"})),
    );
    let env = Env::start(&[], |dir| {
        let work = dir.join("work");
        acp_config(&work, &s, "");
        write(
            &work.join(".beton/skills/fix-ci/SKILL.md"),
            "---\nname: fix-ci\ndescription: Behebt CI\n---\n# Fix CI\n\nLogs lesen.\n",
        );
    })
    .await;
    // ACP hat keine nativen Skills: Inhalte kommen über `skill_load`.
    let id = env.create("acp:fake").await;
    let events = env.turn(&id, "los").await;
    let result = tool_result(&events);
    assert!(result.contains("# Fix CI"), "{result}");
    assert!(!result.contains("description: Behebt CI"), "{result}");
    env.daemon.shutdown().await;
}
