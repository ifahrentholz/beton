//! Sub-Agents über Harness-Grenzen und die Built-in-Agents über die echte Prozesskette
//! (AGT-009, AGT-011, AGT-012): Daemon im Prozess, Runner-Binary, Fake-Vendor-CLIs (QA-002)
//! für Claude Code und Codex. Ein Szenario mit `select: by_input` bedient alle Sessions eines
//! Harness (Parent und Childs); `${mcp.<n>.…}` setzt Ergebnisse früherer Tool-Aufrufe ein.
//! Keine echten Vendor-CLIs, keine API-Keys, kein Netz außer Loopback.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use beton_harness::registry::{HarnessAuth, HarnessCommandConfig, HarnessesConfig};
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

const WAIT: Duration = Duration::from_secs(60);

/// Testumgebung: eigenes Home, Projekt als Git-Repository mit einem Commit, Daemon.
struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

/// Vorbereitung im Testverzeichnis vor dem Start (z. B. Projekt-Agents anlegen).
type Prepare = Box<dyn FnOnce(&Path)>;

#[derive(Default)]
struct Options {
    vars: Vec<(&'static str, String)>,
    /// `harnesses:` der User-Konfiguration (z. B. `auth: api_key` für Kosten).
    harnesses_user: HarnessesConfig,
    prepare: Option<Prepare>,
}

impl Env {
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    async fn start(opts: Options) -> Self {
        // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("README.md"), "# Projekt\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "Start"]);
        if let Some(prepare) = opts.prepare {
            prepare(dir.path());
        }
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        let mut cfg = ServerConfig::local(dir.path().to_path_buf());
        cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
        cfg.socket = None;
        // Ohne `*_API_KEY` (AGT-011 AC5, AGT-012 AC4) und ohne Dateien des Entwicklers.
        let mut inherit: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| !k.starts_with("BETON_") && !k.ends_with("_API_KEY"))
            .collect();
        inherit.insert("HOME".into(), home.display().to_string());
        inherit.insert(
            "BETON_HOME".into(),
            home.join(".beton").display().to_string(),
        );
        for (k, v) in &opts.vars {
            inherit.insert((*k).to_owned(), v.clone());
        }
        let runners = dir.path().join("runners");
        let worktrees = dir.path().join("worktrees");
        let user = opts.harnesses_user;
        let daemon = start_with(cfg, store.clone(), move |mut r| {
            r.sessions.provider = std::sync::Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r.sessions.worktrees_root = worktrees;
            r.sessions.harnesses_user = user;
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

    /// Startet einen Agent (`POST /v1/sessions {agent}`, AGT-004) im Projekt.
    async fn run_agent(&self, agent: &str, prompt: &str) -> String {
        let (status, body) = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"agent": agent, "cwd": self.work()})),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        let id = body["id"].as_str().unwrap().to_owned();
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({"text": prompt})),
            )
            .await;
        assert_eq!(status, 202, "{body}");
        id
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

    async fn wait_for(&self, id: &str, what: &str, pred: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            if pred(&events) {
                return events;
            }
            assert!(
                start.elapsed() < WAIT,
                "{what} fehlt: {}",
                serde_json::to_string_pretty(&events).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wartet auf das Ende des ersten Turns (oder einen Fehler).
    async fn first_turn(&self, id: &str) -> Vec<Value> {
        self.wait_for(id, "Turn-Ende", |events| {
            events.iter().any(|e| {
                matches!(e["type"].as_str(), Some("turn.completed" | "turn.failed"))
                    || (e["type"] == "session.status" && e["payload"]["status"] == "failed")
            })
        })
        .await
    }

    async fn tree(&self, id: &str) -> Value {
        let (status, tree) = self
            .http("GET", &format!("/v1/sessions/{id}/subagents"), None)
            .await;
        assert_eq!(status, 200, "{tree}");
        tree
    }

    /// Wartet, bis keine Child-Session mehr läuft (sauberes Herunterfahren).
    async fn settle(&self, id: &str) {
        let start = Instant::now();
        loop {
            let tree = self.tree(id).await;
            let busy = tree["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["task"] == "running");
            if !busy || start.elapsed() > WAIT {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

fn assistant_text(events: &[Value]) -> String {
    of_type(events, "message.completed")
        .into_iter()
        .filter(|e| e["payload"]["role"] == "assistant")
        .map(|e| {
            e["payload"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_texts(events: &[Value]) -> Vec<String> {
    of_type(events, "message.completed")
        .into_iter()
        .filter(|e| e["payload"]["role"] == "user")
        .map(|e| {
            e["payload"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect()
}

/// Ergebnisse der Tool-Calls eines Tools (in Reihenfolge) als Text.
fn tool_results(events: &[Value], tool: &str) -> Vec<String> {
    let ids: Vec<&Value> = of_type(events, "tool.call.requested")
        .into_iter()
        .filter(|e| e["payload"]["tool"] == tool)
        .map(|e| &e["payload"]["call_id"])
        .collect();
    of_type(events, "tool.call.completed")
        .into_iter()
        .filter(|e| ids.contains(&&e["payload"]["call_id"]))
        .map(|e| match &e["payload"]["result"] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

// --------------------------------------------------------------------------- Szenarien

/// Szenario (JSON ist gültiges YAML), Turn-Auswahl nach Eingabe.
fn scenario(dir: &Path, name: &str, turns: Value) -> PathBuf {
    let path = dir.join(name);
    write(
        &path,
        &json!({"select": "by_input", "turns": turns}).to_string(),
    );
    path
}

fn turn(input: Option<&str>, emit: Vec<Value>) -> Value {
    match input {
        Some(i) => json!({"expect_input": i, "emit": emit}),
        None => json!({"emit": emit}),
    }
}

fn call(tool: &str, args: Value) -> Value {
    json!({"mcp_call": {"server": "beton", "tool": tool, "args": args}})
}

fn msg(text: &str) -> Value {
    json!({"message": text})
}

fn usage(cost_usd: Option<f64>) -> Value {
    match cost_usd {
        Some(c) => json!({"usage": {"input_tokens": 100, "output_tokens": 20, "cost_usd": c}}),
        None => json!({"usage": {"input_tokens": 100, "output_tokens": 20}}),
    }
}

fn claude(scenario: &Path) -> (&'static str, String) {
    (
        "BETON_CLAUDE_PATH",
        format!(
            "{} --protocol stream-json --scenario {}",
            fake_cli().display(),
            scenario.display()
        ),
    )
}

fn codex(scenario: &Path) -> (&'static str, String) {
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

/// Projekt-Agent unter `.beton/agents/<name>/agent.yaml`.
fn project_agent(name: &str, yaml: &str) -> Prepare {
    let (name, yaml) = (name.to_owned(), yaml.to_owned());
    Box::new(move |dir: &Path| {
        write(
            &dir.join(format!("work/.beton/agents/{name}/agent.yaml")),
            &yaml,
        );
    })
}

fn nodes(tree: &Value) -> Vec<Value> {
    tree["nodes"].as_array().unwrap().clone()
}

// --------------------------------------------------------------------------- AGT-009

#[tokio::test]
async fn agt_009_ac1_claude_parent_starts_codex_child_in_its_own_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(
            Some("los"),
            vec![
                call(
                    "session_spawn",
                    json!({"agent": "rev", "prompt": "prüfe", "worktree": "new"})
                ),
                msg("fertig"),
            ]
        )]),
    );
    let child = scenario(
        tmp.path(),
        "codex.yaml",
        json!([turn(
            Some("prüfe"),
            vec![
                json!({"write_file": {"path": "child.txt", "content": "vom Child"}}),
                msg("geprüft"),
            ]
        )]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&parent), codex(&child)],
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  rev:\n    executor: { harness: codex }\nspawn: { agents: [rev] }\n",
        )),
        ..Options::default()
    })
    .await;
    let head = git(&env.work(), &["rev-parse", "HEAD"]);
    let id = env.run_agent("lead", "los").await;
    let events = env.first_turn(&id).await;
    let result = tool_results(&events, "session_spawn").join("");
    assert!(result.contains("geprüft"), "{result}");

    let tree = env.tree(&id).await;
    let all = nodes(&tree);
    assert_eq!(all.len(), 2, "{tree}");
    let kid = &all[1];
    assert_eq!(kid["harness"], "codex");
    assert_eq!(kid["parent_id"], json!(id));
    let wt = &kid["worktree"];
    let path = PathBuf::from(wt["path"].as_str().unwrap());
    // Verschiedene Verzeichnisse; der Child-Branch basiert auf dem HEAD des Parents.
    assert_ne!(
        path.canonicalize().unwrap(),
        env.work().canonicalize().unwrap()
    );
    assert_eq!(wt["base_sha"], json!(head));
    let branch = wt["branch"].as_str().unwrap();
    assert!(branch.starts_with("beton/rev-"), "{branch}");
    assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    assert_eq!(
        git(&env.work(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    assert!(path.join("child.txt").is_file());
    assert!(!env.work().join("child.txt").exists());
    assert!(git(&env.work(), &["worktree", "list"]).contains(path.to_str().unwrap()));
    // Das Ergebnis nennt den Worktree, damit der Parent den Branch weitergeben kann.
    assert!(result.contains(branch), "{result}");
    env.daemon.shutdown().await;
}

/// HAR-017, HAR-027 bei Sub-Agents: Effort und Permission-Mode kommen aus dem `executor` des
/// Sub-Agents; `yolo` ohne Sandbox wird vor dem Anlegen abgelehnt (`sandbox_required`).
#[tokio::test]
async fn agt_009_child_takes_effort_and_mode_from_its_executor_and_never_yolo() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(
            Some("los"),
            vec![
                call("session_spawn", json!({"agent": "wild", "prompt": "w"})),
                call("session_spawn", json!({"agent": "careful", "prompt": "c"})),
                msg("fertig"),
            ]
        )]),
    );
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([turn(Some("c"), vec![msg("gelesen"), usage(None)])]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  wild:\n    executor: { harness: codex, permission_mode: yolo }\n  careful:\n    executor: { harness: codex, permission_mode: plan, reasoning_effort: high }\nspawn: { agents: [wild, careful] }\n",
        )),
        ..Options::default()
    })
    .await;
    let id = env.run_agent("lead", "los").await;
    let events = env.first_turn(&id).await;
    let results = tool_results(&events, "session_spawn");
    assert!(results[0].contains("sandbox_required"), "{}", results[0]);
    assert!(results[1].contains("gelesen"), "{}", results[1]);
    let all = nodes(&env.tree(&id).await);
    assert_eq!(all.len(), 2, "kein Child für yolo");
    let child = all[1]["id"].as_str().unwrap();
    let created = &env.events(child).await[0]["payload"];
    assert_eq!(created["permission_mode"], "plan");
    assert_eq!(created["effort"], "high");
    // Sub-Agents haben einen Titel (ihr Agent) und bekommen keinen generierten (SES-010).
    assert_eq!(all[1]["title"], "careful");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_009_ac2_max_depth_two_stops_the_great_grandchild() {
    let tmp = tempfile::tempdir().unwrap();
    // Claude spielt Wurzel und Enkel, Codex Kind und (nie erreicht) Urenkel.
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([
            turn(
                Some("los"),
                vec![
                    call("session_spawn", json!({"agent": "kind", "prompt": "k"})),
                    msg("Wurzel fertig")
                ]
            ),
            turn(
                Some("e"),
                vec![
                    call("session_spawn", json!({"agent": "urenkel", "prompt": "u"})),
                    msg("Enkel fertig")
                ]
            ),
        ]),
    );
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([
            turn(
                Some("k"),
                vec![
                    call("session_spawn", json!({"agent": "enkel", "prompt": "e"})),
                    msg("Kind fertig")
                ]
            ),
            turn(Some("u"), vec![msg("Urenkel – darf es nicht geben")]),
        ]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  kind:\n    executor: { harness: codex }\n    agents:\n      enkel:\n        executor: { harness: claude }\n        agents:\n          urenkel: { executor: { harness: codex } }\n        spawn: { agents: [urenkel] }\n    spawn: { agents: [enkel] }\nspawn: { agents: [kind], max_depth: 2 }\n",
        )),
        ..Options::default()
    })
    .await;
    let id = env.run_agent("lead", "los").await;
    env.first_turn(&id).await;
    let tree = env.tree(&id).await;
    let all = nodes(&tree);
    let depths: Vec<u64> = all.iter().map(|n| n["depth"].as_u64().unwrap()).collect();
    assert_eq!(depths, [0, 1, 2], "{tree}");
    let enkel = all[2]["id"].as_str().unwrap();
    let events = env.events(enkel).await;
    let denied = tool_results(&events, "session_spawn").join("");
    assert!(denied.contains("spawn_denied: max_depth"), "{denied}");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_009_ac3_fourth_concurrent_spawn_is_denied() {
    let tmp = tempfile::tempdir().unwrap();
    let spawn = || {
        call(
            "session_spawn",
            json!({"agent": "worker", "prompt": "w", "async": true}),
        )
    };
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(
            Some("los"),
            vec![spawn(), spawn(), spawn(), spawn(), msg("verteilt")]
        )]),
    );
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([turn(
            Some("w"),
            vec![json!({"message": "erledigt", "delay_ms": 6000})]
        )]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  worker:\n    executor: { harness: codex }\nspawn: { agents: [worker], max_concurrent: 3 }\n",
        )),
        ..Options::default()
    })
    .await;
    let id = env.run_agent("lead", "los").await;
    let events = env.first_turn(&id).await;
    let results = tool_results(&events, "session_spawn");
    assert_eq!(results.len(), 4, "{results:?}");
    assert!(
        results[..3]
            .iter()
            .all(|r| r.contains("\"status\":\"running\"")),
        "{results:?}"
    );
    assert!(
        results[3].contains("spawn_denied: max_concurrent"),
        "{}",
        results[3]
    );
    assert_eq!(nodes(&env.tree(&id).await).len(), 4, "Wurzel + 3 Childs");
    assert_eq!(of_type(&events, "agent.spawned").len(), 3);
    env.settle(&id).await;
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_009_ac4_agent_completed_carries_status_and_cost_and_counts_for_the_parent() {
    let tmp = tempfile::tempdir().unwrap();
    // Parent auf Codex, Child auf Claude mit API-Key-Abrechnung: nur dann meldet Claude Kosten.
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([turn(
            Some("los"),
            vec![
                call(
                    "session_spawn",
                    json!({"agent": "helfer", "prompt": "rechne"})
                ),
                msg("fertig")
            ]
        )]),
    );
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(Some("rechne"), vec![msg("42"), usage(Some(0.25))])]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        harnesses_user: HarnessesConfig {
            entries: [(
                "claude".to_owned(),
                HarnessCommandConfig {
                    auth: Some(HarnessAuth::ApiKey),
                    ..HarnessCommandConfig::default()
                },
            )]
            .into(),
            ..HarnessesConfig::default()
        },
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: codex }\nagents:\n  helfer:\n    executor: { harness: claude }\nspawn: { agents: [helfer] }\n",
        )),
    })
    .await;
    let id = env.run_agent("lead", "los").await;
    let events = env
        .wait_for(&id, "agent.completed", |ev| {
            !of_type(ev, "agent.completed").is_empty() && !of_type(ev, "turn.completed").is_empty()
        })
        .await;
    let spawned = of_type(&events, "agent.spawned");
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0]["payload"]["harness"], "claude");
    assert_eq!(spawned[0]["payload"]["async"], false);
    let child = spawned[0]["payload"]["child_session_id"].clone();
    let done = of_type(&events, "agent.completed");
    assert_eq!(done[0]["payload"]["child_session_id"], child);
    assert_eq!(done[0]["payload"]["status"], "completed");
    assert_eq!(done[0]["payload"]["summary"], "42");
    assert_eq!(done[0]["payload"]["cost_micro"], 250_000);
    // Die Kosten des Childs zählen zum Teilbaum des Parents.
    let tree = env.tree(&id).await;
    let root = &nodes(&tree)[0];
    assert_eq!(
        root["subtree_cost_micro"].as_i64().unwrap(),
        root["cost_micro"].as_i64().unwrap() + 250_000,
        "{tree}"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_009_ac5_cancelling_the_parent_ends_running_children() {
    let tmp = tempfile::tempdir().unwrap();
    let spawn = || {
        call(
            "session_spawn",
            json!({"agent": "worker", "prompt": "w", "async": true}),
        )
    };
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(
            Some("los"),
            vec![spawn(), spawn(), msg("zwei laufen")]
        )]),
    );
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([turn(Some("w"), vec![json!({"hang": true})])]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        prepare: Some(project_agent(
            "lead",
            "spec_version: 1\nname: lead\nexecutor: { harness: claude }\nagents:\n  worker:\n    executor: { harness: codex }\nspawn: { agents: [worker] }\n",
        )),
        ..Options::default()
    })
    .await;
    let id = env.run_agent("lead", "los").await;
    env.first_turn(&id).await;
    let kids: Vec<String> = nodes(&env.tree(&id).await)[1..]
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kids.len(), 2);
    for k in &kids {
        env.wait_for(k, "laufender Turn", |ev| {
            !of_type(ev, "turn.started").is_empty()
        })
        .await;
    }
    // Der Mensch bricht den Parent ab.
    let (status, body) = env
        .http("POST", &format!("/v1/sessions/{id}/interrupt"), None)
        .await;
    assert!(matches!(status, 200 | 202 | 204), "{status} {body}");
    for k in &kids {
        let ev = env
            .wait_for(k, "Stopp mit parent_cancelled", |ev| {
                ev.iter().any(|e| {
                    e["type"] == "session.status"
                        && e["payload"]["status"] == "stopped"
                        && e["payload"]["reason"] == "parent_cancelled"
                })
            })
            .await;
        assert!(!of_type(&ev, "turn.started").is_empty());
    }
    let events = env
        .wait_for(&id, "agent.completed", |ev| {
            of_type(ev, "agent.completed").len() == 2
        })
        .await;
    for done in of_type(&events, "agent.completed") {
        assert_eq!(done["payload"]["status"], "cancelled");
        assert_eq!(done["payload"]["cancel_reason"], "parent_cancelled");
    }
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-011 maestra

/// Turns der Fake-CLIs für einen maestra-Lauf: Plan → 2 Implementierungen parallel in eigenen
/// Worktrees → Cross-Review → Zusammenfassung.
fn maestra_turns() -> (Value, Value) {
    let wait = |ids: Vec<&str>| {
        call(
            "session_wait",
            json!({"ids": ids, "mode": "all", "timeout": "50s"}),
        )
    };
    let maestra = turn(
        Some("Baue den Rate-Limiter"),
        vec![
            call("session_list", json!({})),
            msg("Plan: 1. Limiter (impl-claude) 2. Tests (impl-codex)"),
            call(
                "session_spawn",
                json!({"agent": "impl-claude", "prompt": "Teilaufgabe 1: Limiter", "async": true, "worktree": "new"}),
            ),
            call(
                "session_spawn",
                json!({"agent": "impl-codex", "prompt": "Teilaufgabe 2: Tests", "async": true, "worktree": "new"}),
            ),
            wait(vec!["${mcp.1.session_id}", "${mcp.2.session_id}"]),
            call(
                "session_spawn",
                json!({"agent": "review-codex", "prompt": "Reviewe ${mcp.1.worktree.branch} (Base ${mcp.1.worktree.base_sha})", "async": true}),
            ),
            call(
                "session_spawn",
                json!({"agent": "review-claude", "prompt": "Reviewe ${mcp.2.worktree.branch} (Base ${mcp.2.worktree.base_sha})", "async": true}),
            ),
            wait(vec!["${mcp.4.session_id}", "${mcp.5.session_id}"]),
            msg(
                "| 1 | impl-claude | review-codex | approved |\n| 2 | impl-codex | review-claude | approved |",
            ),
        ],
    );
    let claude_turns = json!([
        maestra,
        turn(
            Some("Teilaufgabe 1: Limiter"),
            vec![
                json!({"write_file": {"path": "limiter.rs", "content": "// Limiter\n"}}),
                json!({"message": "BRANCH: limiter", "delay_ms": 6000}),
            ]
        ),
        // Review ohne feste Eingabe (der Branch-Name ist dynamisch).
        turn(None, vec![msg("VERDICT: approve\nBLOCKING:\n- keine")]),
    ]);
    let codex_turns = json!([
        turn(
            Some("Teilaufgabe 2: Tests"),
            vec![
                json!({"write_file": {"path": "limiter_test.rs", "content": "// Tests\n"}}),
                json!({"message": "BRANCH: tests", "delay_ms": 1500}),
                usage(None),
            ]
        ),
        turn(
            None,
            vec![msg("VERDICT: approve\nBLOCKING:\n- keine"), usage(None)]
        ),
    ]);
    (claude_turns, codex_turns)
}

/// AGT-011 AC1 und AC5: kompletter Lauf ohne API-Keys und ohne `providers`.
#[tokio::test]
async fn agt_011_ac1_ac5_maestra_runs_plan_parallel_worktrees_cross_review_and_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let (claude_turns, codex_turns) = maestra_turns();
    let claude_s = scenario(tmp.path(), "claude.yaml", claude_turns);
    let codex_s = scenario(tmp.path(), "codex.yaml", codex_turns);
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        ..Options::default()
    })
    .await;
    let id = env.run_agent("maestra", "Baue den Rate-Limiter").await;
    let events = env.first_turn(&id).await;
    assert!(of_type(&events, "turn.failed").is_empty(), "{events:?}");
    let resolved = of_type(&events, "agent.resolved");
    assert_eq!(resolved[0]["payload"]["source"], "builtin");

    // Plan → 2 Implementierungen parallel: beide gestartet, bevor eine fertig war.
    let order: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["type"] == "agent.spawned" || e["type"] == "agent.completed")
        .map(|e| {
            (
                e["type"].as_str().unwrap().to_owned(),
                e["payload"]["child_session_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(order[0].0, "agent.spawned");
    assert_eq!(order[1].0, "agent.spawned", "{order:?}");
    let spawned = of_type(&events, "agent.spawned");
    let agents: Vec<(&str, &str)> = spawned
        .iter()
        .map(|e| {
            (
                e["payload"]["agent_ref"].as_str().unwrap(),
                e["payload"]["harness"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        agents,
        [
            ("maestra#impl-claude", "claude"),
            ("maestra#impl-codex", "codex"),
            ("maestra#review-codex", "codex"),
            ("maestra#review-claude", "claude"),
        ]
    );
    // Jede Implementierung in eigenem Worktree mit eigenem Branch.
    let tree = env.tree(&id).await;
    let all = nodes(&tree);
    let by_id = |id: &str| all.iter().find(|n| n["id"] == id).unwrap().clone();
    let impl_ids: Vec<String> = spawned[..2]
        .iter()
        .map(|e| {
            e["payload"]["child_session_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let impls: Vec<Value> = impl_ids.iter().map(|i| by_id(i)).collect();
    let paths: Vec<&str> = impls
        .iter()
        .map(|n| n["worktree"]["path"].as_str().unwrap())
        .collect();
    assert_ne!(paths[0], paths[1]);
    assert!(Path::new(paths[0]).join("limiter.rs").is_file());
    assert!(Path::new(paths[1]).join("limiter_test.rs").is_file());
    assert!(
        !env.work().join("limiter.rs").exists(),
        "maestra schreibt nicht selbst"
    );
    // Jedes Review kam von einem anderen Harness als die Implementierung, die es prüft.
    for review in &spawned[2..] {
        let rid = review["payload"]["child_session_id"].as_str().unwrap();
        let prompt = user_texts(&env.events(rid).await).join("\n");
        let reviewed = impls
            .iter()
            .find(|n| prompt.contains(n["worktree"]["branch"].as_str().unwrap()))
            .unwrap_or_else(|| panic!("Review ohne Branch: {prompt}"));
        assert_ne!(
            reviewed["harness"], review["payload"]["harness"],
            "{prompt}"
        );
    }
    let summary = assistant_text(&events);
    assert!(summary.contains("approved"), "{summary}");
    assert_eq!(of_type(&events, "agent.completed").len(), 4);
    // AC5: keine API-Keys im Env (Env::start filtert sie), keine `providers`; alle
    // Child-Sessions laufen über den Login der Vendor-CLI.
    assert!(!env.dir.path().join("home/.beton/config.yaml").exists());
    for n in &all[1..] {
        assert_eq!(n["auth_source"], "vendor_cli", "{n}");
        assert_eq!(n["cost_micro"], 0, "Subscription ist keine Ausgabe: {n}");
    }
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_011_ac3_blocking_review_retasks_the_implementer_until_needs_human() {
    let tmp = tempfile::tempdir().unwrap();
    let wait = |id: &str| call("session_wait", json!({"ids": [id], "timeout": "50s"}));
    let send = |id: &str, text: &str| call("session_send", json!({"session_id": id, "text": text}));
    // Ergebnisse: 0 impl, 1 wait, 2 review, 3 wait, 4 send impl, 5 wait, 6 send review, 7 wait,
    // 8 send impl, 9 wait, 10 send review, 11 wait.
    let maestra = turn(
        Some("Baue den Cache"),
        vec![
            call(
                "session_spawn",
                json!({"agent": "impl-claude", "prompt": "Teilaufgabe: Cache", "async": true}),
            ),
            wait("${mcp.0.session_id}"),
            call(
                "session_spawn",
                json!({"agent": "review-codex", "prompt": "Reviewe ${mcp.0.worktree.branch}, Runde 1 von 3", "async": true}),
            ),
            wait("${mcp.2.session_id}"),
            send("${mcp.0.session_id}", "Behebe: Race Condition (Runde 1)"),
            wait("${mcp.0.session_id}"),
            send("${mcp.2.session_id}", "Runde 2 von 3"),
            wait("${mcp.2.session_id}"),
            send("${mcp.0.session_id}", "Behebe: Race Condition (Runde 2)"),
            wait("${mcp.0.session_id}"),
            send("${mcp.2.session_id}", "Runde 3 von 3"),
            wait("${mcp.2.session_id}"),
            msg("| Cache | impl-claude | review-codex | 3 | needs_human |\nOffen: Race Condition"),
        ],
    );
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([
            maestra,
            turn(Some("Teilaufgabe: Cache"), vec![msg("BRANCH: cache")]),
            turn(
                Some("Behebe: Race Condition (Runde 1)"),
                vec![msg("versucht 1")]
            ),
            turn(
                Some("Behebe: Race Condition (Runde 2)"),
                vec![msg("versucht 2")]
            ),
        ]),
    );
    let blocking = || msg("VERDICT: changes_requested\nBLOCKING:\n- Race Condition");
    let codex_s = scenario(
        tmp.path(),
        "codex.yaml",
        json!([
            turn(None, vec![blocking()]),
            turn(Some("Runde 2 von 3"), vec![blocking()]),
            turn(Some("Runde 3 von 3"), vec![blocking()]),
        ]),
    );
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        ..Options::default()
    })
    .await;
    let id = env.run_agent("maestra", "Baue den Cache").await;
    let events = env.first_turn(&id).await;
    assert!(of_type(&events, "turn.failed").is_empty(), "{events:?}");
    let spawned = of_type(&events, "agent.spawned");
    assert_eq!(
        spawned.len(),
        2,
        "derselbe Implementer und Reviewer, keine neuen Sessions"
    );
    let implementer = spawned[0]["payload"]["child_session_id"].as_str().unwrap();
    // Der Implementer wurde zweimal erneut beauftragt (3 Aufträge insgesamt).
    let tasks = user_texts(&env.events(implementer).await);
    assert_eq!(tasks.len(), 3, "{tasks:?}");
    let messages = of_type(&events, "agent.message");
    assert_eq!(messages.len(), 4);
    assert!(
        messages
            .iter()
            .filter(|m| m["payload"]["to_session"] == implementer)
            .all(|m| m["payload"]["text"].as_str().unwrap().starts_with("Behebe"))
    );
    // Drei blockierende Review-Runden, dann needs_human.
    let reviews: Vec<&Value> = of_type(&events, "agent.completed")
        .into_iter()
        .filter(|e| e["payload"]["child_session_id"] == spawned[1]["payload"]["child_session_id"])
        .collect();
    assert_eq!(reviews.len(), 3);
    assert!(reviews.iter().all(|r| {
        r["payload"]["summary"]
            .as_str()
            .unwrap()
            .starts_with("VERDICT: changes_requested")
    }));
    assert!(assistant_text(&events).contains("needs_human"));
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_011_ac4_without_codex_maestra_reviews_same_vendor_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let maestra = turn(
        Some("Baue den Parser"),
        vec![
            call("session_list", json!({})),
            // Abgelehnt, bevor eine Session entsteht (kein Ergebnis-Index).
            call(
                "session_spawn",
                json!({"agent": "impl-codex", "prompt": "Teilaufgabe: Parser", "async": true}),
            ),
            call(
                "session_spawn",
                json!({"agent": "impl-claude", "prompt": "Teilaufgabe: Parser", "async": true}),
            ),
            call(
                "session_wait",
                json!({"ids": ["${mcp.1.session_id}"], "timeout": "50s"}),
            ),
            call(
                "session_spawn",
                json!({"agent": "review-claude", "prompt": "Reviewe ${mcp.1.worktree.branch}", "async": true}),
            ),
            call(
                "session_wait",
                json!({"ids": ["${mcp.3.session_id}"], "timeout": "50s"}),
            ),
            msg(
                "| Parser | impl-claude | review-claude | approved |\nHinweis: same-vendor review (Codex nicht eingerichtet)",
            ),
        ],
    );
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([
            maestra,
            turn(Some("Teilaufgabe: Parser"), vec![msg("BRANCH: parser")]),
            turn(None, vec![msg("VERDICT: approve")]),
        ]),
    );
    let env = Env::start(Options {
        vars: vec![
            claude(&claude_s),
            ("BETON_CODEX_PATH", "/nicht/vorhanden/codex".to_owned()),
        ],
        ..Options::default()
    })
    .await;
    let id = env.run_agent("maestra", "Baue den Parser").await;
    let events = env.first_turn(&id).await;
    let listing = tool_results(&events, "session_list").join("");
    assert!(listing.contains("impl-codex"), "{listing}");
    assert!(listing.contains("nicht installiert"), "{listing}");
    let spawns = tool_results(&events, "session_spawn");
    assert!(spawns[0].contains("harness_unavailable"), "{}", spawns[0]);
    let harnesses: Vec<&str> = of_type(&events, "agent.spawned")
        .iter()
        .map(|e| e["payload"]["harness"].as_str().unwrap())
        .collect();
    assert_eq!(harnesses, ["claude", "claude"]);
    assert!(assistant_text(&events).contains("same-vendor review"));
    env.daemon.shutdown().await;
}

// --------------------------------------------------------------------------- AGT-012 duetto

fn duetto_question() -> Vec<Value> {
    vec![
        call("session_list", json!({})),
        call(
            "session_spawn",
            json!({"agent": "voce-claude", "prompt": "Frage: Tabs oder Spaces?", "async": true}),
        ),
        call(
            "session_spawn",
            json!({"agent": "voce-codex", "prompt": "Frage: Tabs oder Spaces?", "async": true}),
        ),
        call(
            "session_wait",
            json!({"ids": ["${mcp.1.session_id}", "${mcp.2.session_id}"], "mode": "all", "timeout": "50s"}),
        ),
    ]
}

/// AGT-012 AC1, AC2 und AC4: zwei Stimmen parallel, 2 Kritikrunden je Stimme, Synthese.
#[tokio::test]
async fn agt_012_ac1_ac2_ac4_duetto_debates_two_rounds_and_synthesizes() {
    let tmp = tempfile::tempdir().unwrap();
    let both = || {
        call(
            "session_wait",
            json!({"ids": ["${mcp.1.session_id}", "${mcp.2.session_id}"], "mode": "all", "timeout": "50s"}),
        )
    };
    let critique = |id: &str, round: u32| {
        call(
            "session_send",
            json!({"session_id": id, "text": format!("Runde {round} von 2: Kritisiere die andere Antwort")}),
        )
    };
    let mut steps = duetto_question();
    for round in 1..=2 {
        steps.push(critique("${mcp.1.session_id}", round));
        steps.push(critique("${mcp.2.session_id}", round));
        steps.push(both());
    }
    steps.push(msg(
        "## Konsens\n- Konsistenz zählt\n\n## Dissens\n- Tabs vs. Spaces\n\n## Urteil\nFormatter entscheidet.\n\n2 Kritikrunden je Stimme · Claude Code und Codex",
    ));
    let voice = |name: &str| {
        json!([
            turn(
                Some("Frage: Tabs oder Spaces?"),
                vec![
                    msg(&format!("{name}: Spaces")),
                    json!({"delay_ms": 4000, "message": format!("{name}: Spaces, weil …")})
                ]
            ),
            turn(
                Some("Runde 1 von 2: Kritisiere die andere Antwort"),
                vec![msg(&format!("{name}: Kritik 1"))]
            ),
            turn(
                Some("Runde 2 von 2: Kritisiere die andere Antwort"),
                vec![msg(&format!("{name}: Kritik 2"))]
            ),
        ])
    };
    let mut claude_turns = vec![turn(Some("/debate rounds=2 Tabs oder Spaces?"), steps)];
    claude_turns.extend(voice("Claude").as_array().unwrap().clone());
    let claude_s = scenario(tmp.path(), "claude.yaml", Value::Array(claude_turns));
    let mut codex_turns = voice("Codex").as_array().unwrap().clone();
    for t in &mut codex_turns {
        t["emit"].as_array_mut().unwrap().push(usage(None));
    }
    let codex_s = scenario(tmp.path(), "codex.yaml", Value::Array(codex_turns));
    let env = Env::start(Options {
        vars: vec![claude(&claude_s), codex(&codex_s)],
        ..Options::default()
    })
    .await;
    let id = env
        .run_agent("duetto", "/debate rounds=2 Tabs oder Spaces?")
        .await;
    let events = env.first_turn(&id).await;
    assert!(of_type(&events, "turn.failed").is_empty(), "{events:?}");

    // AC1: zwei Childs auf verschiedenen Harnesses, parallel (beide gestartet, bevor eine
    // Antwort kam), eingesammelt mit session_wait(mode: all).
    let spawned = of_type(&events, "agent.spawned");
    let harnesses: Vec<&str> = spawned
        .iter()
        .map(|e| e["payload"]["harness"].as_str().unwrap())
        .collect();
    assert_eq!(harnesses, ["claude", "codex"]);
    let first_done = events
        .iter()
        .position(|e| e["type"] == "agent.completed")
        .unwrap();
    let second_spawn = events
        .iter()
        .rposition(|e| e["type"] == "agent.spawned")
        .unwrap();
    assert!(second_spawn < first_done, "beide Stimmen laufen parallel");
    let waits = tool_results(&events, "session_wait");
    assert!(waits[0].contains("\"done\":true"), "{}", waits[0]);

    // AC2: genau 2 Kritikrunden pro Stimme, Synthese mit Konsens und Dissens.
    for s in &spawned {
        let voice = s["payload"]["child_session_id"].as_str().unwrap();
        let texts = user_texts(&env.events(voice).await);
        let rounds = texts.iter().filter(|t| t.starts_with("Runde")).count();
        assert_eq!(rounds, 2, "{texts:?}");
    }
    let synthesis = assistant_text(&events);
    assert!(synthesis.contains("## Konsens") && synthesis.contains("## Dissens"));

    // AC4: alle beteiligten Sessions über den Login der Vendor-CLI, ohne API-Key.
    let tree = env.tree(&id).await;
    for n in nodes(&tree) {
        assert_eq!(n["auth_source"], "vendor_cli", "{n}");
    }
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn agt_012_ac3_duetto_aborts_without_the_second_voice() {
    let tmp = tempfile::tempdir().unwrap();
    // Vorab-Prüfung: Codex fehlt → klare Meldung, keine Stimme gestartet.
    let claude_s = scenario(
        tmp.path(),
        "claude.yaml",
        json!([turn(
            Some("Tabs oder Spaces?"),
            vec![
                call("session_list", json!({})),
                msg(
                    "duetto braucht zwei Stimmen auf verschiedenen Harnesses. Codex ist hier nicht eingerichtet (nicht installiert)."
                ),
            ]
        )]),
    );
    let env = Env::start(Options {
        vars: vec![
            claude(&claude_s),
            ("BETON_CODEX_PATH", "/nicht/vorhanden/codex".to_owned()),
        ],
        ..Options::default()
    })
    .await;
    let id = env.run_agent("duetto", "Tabs oder Spaces?").await;
    let events = env.first_turn(&id).await;
    let listing = tool_results(&events, "session_list").join("");
    assert!(listing.contains("voce-codex"), "{listing}");
    assert!(listing.contains("nicht installiert"), "{listing}");
    assert!(of_type(&events, "agent.spawned").is_empty());
    assert!(assistant_text(&events).contains("braucht zwei Stimmen"));
    env.daemon.shutdown().await;
}
