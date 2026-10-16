//! Approvals, Zustand und Inbox mit echtem Daemon, Runner-Binary und Fake-Harness (POL-009,
//! POL-010, POL-014, UX-001, WEB-007 AC2 serverseitig).
//!
//! Policies liegen im Projekt (`<work>/.beton/policies/`); das echte Home des Entwicklers wird
//! nie gelesen.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod support;

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

struct Env {
    dir: tempfile::TempDir,
    daemon: Option<Daemon>,
    token: String,
    addr: SocketAddr,
}

impl Env {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work/.beton/policies")).unwrap();
        let (daemon, token, addr) = Self::boot(dir.path()).await;
        Self {
            dir,
            daemon: Some(daemon),
            token,
            addr,
        }
    }

    /// Daemon geordnet beenden und auf demselben Datenverzeichnis neu starten.
    async fn restart(&mut self) {
        if let Some(d) = self.daemon.take() {
            d.shutdown().await;
        }
        let (daemon, token, addr) = Self::boot(self.dir.path()).await;
        self.daemon = Some(daemon);
        self.token = token;
        self.addr = addr;
    }

    async fn boot(dir: &Path) -> (Daemon, String, SocketAddr) {
        let home = dir.join("home");
        let store = Store::open(dir, StoreOptions::default()).await.unwrap();
        let mut cfg = ServerConfig::local(dir.to_path_buf());
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
        let runners = dir.join("runners");
        let daemon = start_with(cfg, store, move |mut r| {
            r.sessions.sandbox = support::unsandboxed();
            r.sessions.provider = std::sync::Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r
        })
        .await
        .unwrap();
        let token = std::fs::read_to_string(dir.join("auth/local.token"))
            .unwrap()
            .trim()
            .to_owned();
        let addr = daemon.addrs[0];
        (daemon, token, addr)
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    /// Schreibt eine Projekt-Policy.
    fn project_policy(&self, name: &str, body: &str) -> PathBuf {
        let path = self.work().join(format!(".beton/policies/{name}.yaml"));
        std::fs::write(&path, format!("spec_version: 1\nname: {name}\n{body}")).unwrap();
        path
    }

    fn scenario(&self, yaml: &str) -> PathBuf {
        let p = self.dir.path().join(format!("s{}.yaml", fastrand_name()));
        std::fs::write(&p, yaml).unwrap();
        p
    }

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
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        (
            status,
            serde_json::from_str(&payload).unwrap_or(Value::Null),
        )
    }

    async fn create(&self, scenario: &Path) -> String {
        let (status, body) = self
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": "fake", "cwd": self.work(), "harness_opts": {"scenario": scenario}})),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
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
                start.elapsed() < Duration::from_secs(30),
                "Status {want} kommt nicht (ist {}): {}",
                self.status(id).await,
                serde_json::to_string_pretty(&self.events(id).await).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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

    async fn input(&self, id: &str, text: &str) -> (u16, Value) {
        self.http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": text})),
        )
        .await
    }

    /// Eingabe und Turn-Ende; liefert die Events dieses Turns.
    async fn turn(&self, id: &str, text: &str) -> Vec<Value> {
        let before = self.events(id).await.len();
        let (status, body) = self.input(id, text).await;
        assert_eq!(status, 202, "{body}");
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            let new = events[before.min(events.len())..].to_vec();
            if new.iter().any(|e| {
                matches!(
                    e["type"].as_str(),
                    Some("turn.completed" | "turn.failed" | "turn.interrupted")
                )
            }) {
                return new;
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

fn fastrand_name() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
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

fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

fn decisions<'a>(events: &'a [Value], phase: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e["type"] == "policy.decision" && e["payload"]["phase"] == phase)
        .collect()
}

fn is(t: &'static str) -> impl Fn(&Value) -> bool {
    move |e| e["type"] == t
}

/// Ein Push je Turn; erlaubt → Ergebnis, abgelehnt → Hinweis.
fn pushes(n: usize, command: &str) -> String {
    let mut s = String::from("turns:\n");
    for _ in 0..n {
        s.push_str(&format!(
            "  - emit:\n      - {{ tool_call: {{ name: Bash, kind: shell, args: {{ command: \"{command}\" }} }} }}\n      - {{ on_gate: {{ allow: [{{ tool_result: \"gepusht\" }}, {{ message: \"fertig\" }}], deny: [{{ message: \"abgelehnt\" }}] }} }}\n"
        ));
    }
    s
}

fn approvals_of(events: &[Value]) -> Vec<&Value> {
    of_type(events, "approval.requested")
}

impl Env {
    async fn open_approval(&self, id: &str) -> Value {
        let start = Instant::now();
        loop {
            let (_, approvals) = self
                .http("GET", &format!("/v1/sessions/{id}/approvals"), None)
                .await;
            if let Some(a) = approvals["items"].as_array().and_then(|a| a.first()) {
                return a.clone();
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "keine Freigabe offen"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn resolve_with(&self, id: &str, body: Value) -> (u16, Value) {
        let a = self.open_approval(id).await;
        let aid = a["id"].as_str().unwrap().to_owned();
        self.http(
            "POST",
            &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
            Some(body),
        )
        .await
    }

    /// Startet einen Turn ohne auf sein Ende zu warten.
    async fn start_turn(&self, id: &str, text: &str) -> usize {
        let before = self.events(id).await.len();
        let (status, body) = self.input(id, text).await;
        assert_eq!(status, 202, "{body}");
        before
    }

    async fn turn_end(&self, id: &str, before: usize) -> Vec<Value> {
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            let new = events[before.min(events.len())..].to_vec();
            if new.iter().any(|e| {
                matches!(
                    e["type"].as_str(),
                    Some("turn.completed" | "turn.failed" | "turn.interrupted")
                )
            }) {
                return new;
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

// --------------------------------------------------------------------------------- POL-010

#[tokio::test]
async fn pol_010_ac1_two_ask_rules_make_one_card_with_two_reasons() {
    let env = Env::start().await;
    env.project_policy(
        "git-and-budget",
        "rules:\n  - id: confirm-destructive-shell\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: ask\n    reason: \"Destruktives oder veröffentlichendes Shell-Kommando\"\n    approval: { timeout: 30m, on_timeout: deny }\n",
    );
    env.project_policy(
        "guard",
        "rules:\n  - id: git-guard\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: ask\n    reason: \"Push verlässt den Rechner\"\n    approval: { timeout: 2h, on_timeout: abort }\n",
    );
    let id = env
        .create(&env.scenario(&pushes(1, "git push origin main")))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    let card = env.open_approval(&id).await;
    assert_eq!(card["kind"], "policy");
    assert_eq!(
        card["subject"]["reasons"].as_array().unwrap().len(),
        2,
        "{card}"
    );
    assert_eq!(card["subject"]["tool"]["name"], "Bash");
    assert_eq!(
        card["options"],
        json!(["approve", "approve_session", "deny"])
    );
    env.wait_status(&id, "waiting_approval").await;
    let (status, body) = env.resolve_with(&id, json!({"decision": "allow"})).await;
    assert_eq!(status, 200, "{body}");
    let turn = env.turn_end(&id, before).await;
    assert_eq!(approvals_of(&turn).len(), 1, "genau eine Card");
    let requested = approvals_of(&turn)[0];
    assert_eq!(requested["payload"]["on_timeout"], "abort");
    assert_eq!(of_type(&turn, "turn.completed").len(), 1);
}

const ASK_PUSH_SESSION: &str = "rules:\n  - id: confirm-push\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: ask\n    reason: \"Push bestätigen\"\n    approval: { remember: session }\n";

#[tokio::test]
async fn pol_010_ac2_approve_session_allows_the_same_command_without_a_card() {
    let env = Env::start().await;
    env.project_policy("push", ASK_PUSH_SESSION);
    let mut scenario = pushes(2, "git push origin feature/x");
    scenario.push_str(&pushes(1, "git push --tags origin").replacen("turns:\n", "", 1));
    let id = env.create(&env.scenario(&scenario)).await;
    env.wait_status(&id, "idle").await;

    // Turn 1: Card, „für diese Session erlauben“.
    let before = env.start_turn(&id, "eins").await;
    let (status, body) = env
        .resolve_with(&id, json!({"decision": "allow", "remember": "session"}))
        .await;
    assert_eq!(status, 200, "{body}");
    let turn = env.turn_end(&id, before).await;
    let resolved = of_type(&turn, "approval.resolved")[0];
    assert_eq!(resolved["payload"]["remember"], "session");

    // Turn 2: derselbe Befehl ohne Card; policy.decision nennt den Grant.
    let turn = env.turn(&id, "zwei").await;
    assert!(approvals_of(&turn).is_empty(), "{turn:#?}");
    let d = decisions(&turn, "tool_call");
    assert_eq!(d[0]["payload"]["outcome"], "allow");
    assert_eq!(
        d[0]["payload"]["grants_used"].as_array().unwrap().len(),
        1,
        "{d:#?}"
    );

    // Turn 3: anderer Befehl, wieder eine Card.
    let before = env.start_turn(&id, "drei").await;
    let card = env.open_approval(&id).await;
    assert_eq!(card["subject"]["summary"], "git push --tags origin");
    env.resolve_with(&id, json!({"decision": "deny", "reason": "nein"}))
        .await;
    env.turn_end(&id, before).await;
}

#[tokio::test]
async fn pol_010_resolve_is_checked_against_the_card() {
    let env = Env::start().await;
    env.project_policy(
        "push",
        "rules:\n  - id: confirm-push\n    on: tool_call\n    action: ask\n    approval: { remember: none }\n",
    );
    let id = env.create(&env.scenario(&pushes(1, "git push"))).await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    // Merken bietet diese Card nicht an.
    let (status, body) = env
        .resolve_with(&id, json!({"decision": "allow", "remember": "session"}))
        .await;
    assert_eq!(status, 409, "{body}");
    // Antworten gibt es nur für Fragen.
    let (status, body) = env
        .resolve_with(&id, json!({"decision": "answer", "answer": "ja"}))
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, _) = env.resolve_with(&id, json!({"decision": "allow"})).await;
    assert_eq!(status, 200);
    let turn = env.turn_end(&id, before).await;
    // Wer entschieden hat, steht im Log (WEB-007 AC2).
    let resolved = of_type(&turn, "approval.resolved")[0];
    assert_eq!(resolved["payload"]["actor"]["kind"], "user", "{resolved}");
    assert_eq!(resolved["payload"]["via"], "user");
}

#[tokio::test]
async fn pol_010_ac5_timeout_applies_abort_and_ends_the_turn() {
    let env = Env::start().await;
    env.project_policy(
        "timeouts",
        "rules:\n  - id: rule-a\n    on: tool_call\n    action: ask\n    approval: { timeout: 1s, on_timeout: deny }\n  - id: rule-b\n    on: tool_call\n    action: ask\n    approval: { timeout: 2h, on_timeout: abort }\n",
    );
    let id = env.create(&env.scenario(&pushes(1, "rm -rf dist"))).await;
    env.wait_status(&id, "idle").await;
    let turn = env.turn(&id, "los").await;
    let requested = approvals_of(&turn)[0];
    assert_eq!(requested["payload"]["on_timeout"], "abort");
    let resolved = of_type(&turn, "approval.resolved")[0];
    assert_eq!(resolved["payload"]["via"], "timeout");
    assert_eq!(resolved["payload"]["decision"], "abort");
    assert_eq!(resolved["payload"]["on_timeout_applied"], "abort");
    // Kein Ergebnis des Tool-Calls; der Turn endet.
    assert!(
        !of_type(&turn, "tool.call.completed")
            .iter()
            .any(|c| c["payload"]["status"] == "ok"),
        "{turn:#?}"
    );
    env.wait_status(&id, "idle").await;
}

#[tokio::test]
async fn pol_010_ac4_open_approval_survives_a_daemon_restart() {
    let mut env = Env::start().await;
    env.project_policy("push", ASK_PUSH_SESSION);
    let id = env
        .create(&env.scenario(&pushes(1, "git push origin main")))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    let card = env.open_approval(&id).await;
    env.wait_status(&id, "waiting_approval").await;

    env.restart().await;

    // Weiterhin sichtbar: in der Session und in der Inbox.
    let again = env.open_approval(&id).await;
    assert_eq!(again["id"], card["id"]);
    let (_, inbox) = env.http("GET", "/v1/inbox?status=open", None).await;
    assert_eq!(inbox["open_count"], 1, "{inbox}");
    // Und entscheidbar: der Tool-Call läuft, der Turn endet.
    let (status, body) = env.resolve_with(&id, json!({"decision": "allow"})).await;
    assert_eq!(status, 200, "{body}");
    let turn = env.turn_end(&id, before).await;
    assert!(
        of_type(&turn, "tool.call.completed")
            .iter()
            .any(|c| c["payload"]["status"] == "ok"),
        "{turn:#?}"
    );
    assert_eq!(of_type(&turn, "turn.completed").len(), 1, "{turn:#?}");
    assert!(
        !of_type(&turn, "turn.failed")
            .iter()
            .any(|t| t["payload"]["problem"]["code"] == "runner_restarted"),
        "{turn:#?}"
    );
}

#[tokio::test]
async fn pol_010_ac4_idle_runner_ends_with_the_daemon() {
    // Ohne offene Freigabe bleibt nichts zurück: der Runner endet mit dem Daemon.
    let mut env = Env::start().await;
    let id = env.create(&env.scenario(&pushes(1, "ls"))).await;
    env.wait_status(&id, "idle").await;
    env.restart().await;
    let (status, body) = env.input(&id, "los").await;
    assert_eq!(status, 202, "{body}");
    let events = env.wait_for(&id, is("turn.completed")).await;
    // Ein neuer Runner hat die Session fortgesetzt.
    assert_eq!(of_type(&events, "session.started").len(), 2, "{events:#?}");
}

// --------------------------------------------------------------------------------- POL-014

#[tokio::test]
async fn pol_014_require_approval_asks_for_file_writes() {
    let env = Env::start().await;
    env.project_policy(
        "writes",
        "rules:\n  - id: needs-human\n    type: require_approval\n    params: { kinds: [file_write, file_edit], remember: none, timeout: 10m }\n",
    );
    let scenario = "turns:\n  - emit:\n      - { tool_call: { name: Write, kind: file_write, args: { file_path: \"a.txt\", content: \"x\" } } }\n      - { on_gate: { allow: [{ tool_result: \"ok\" }], deny: [{ message: \"nein\" }] } }\n";
    let id = env.create(&env.scenario(scenario)).await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    let card = env.open_approval(&id).await;
    assert_eq!(card["subject"]["reasons"][0]["rule_id"], "needs-human");
    assert_eq!(card["options"], json!(["approve", "deny"]));
    env.resolve_with(&id, json!({"decision": "deny"})).await;
    env.turn_end(&id, before).await;
}

// --------------------------------------------------------------------------------- POL-009

const COUNT_PUSHES: &str = "rules:\n  - id: count-pushes\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: notify\n    notify: { channels: [inbox], message: 'Push Nr. {{ has(state.session.pushes) ? state.session.pushes + 1 : 1 }}' }\n    effects:\n      - { op: increment, scope: session, key: pushes }\n";

fn state_values(events: &[Value], key: &str) -> Vec<Value> {
    of_type(events, "policy.state_changed")
        .into_iter()
        .filter(|e| e["payload"]["key"] == key)
        .map(|e| e["payload"]["value"].clone())
        .collect()
}

#[tokio::test]
async fn pol_009_ac1_fork_takes_the_state_up_to_its_seq() {
    let env = Env::start().await;
    env.project_policy("count", COUNT_PUSHES);
    let id = env.create(&env.scenario(&pushes(4, "git push"))).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    let second = env.turn(&id, "zwei").await;
    let fork_at = second
        .iter()
        .find(|e| e["type"] == "turn.completed")
        .unwrap()["seq"]
        .as_u64()
        .unwrap();
    let third = env.turn(&id, "drei").await;
    let events = env.events(&id).await;
    assert_eq!(
        state_values(&events, "pushes"),
        [json!(1), json!(2), json!(3)]
    );
    let notified = of_type(&third, "policy.notified")[0];
    assert_eq!(notified["payload"]["message"], "Push Nr. 3");

    // Fork nach dem zweiten Push: Zustand 2, der nächste Push zählt als dritter.
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/fork"),
            Some(json!({"at_seq": fork_at + 1})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let child = body["session"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .unwrap()
        .to_owned();
    let child_events = env.events(&child).await;
    assert_eq!(state_values(&child_events, "pushes"), [json!(1), json!(2)]);
    env.wait_status(&child, "idle").await;
    let turn = env.turn(&child, "vier").await;
    assert_eq!(
        of_type(&turn, "policy.notified")[0]["payload"]["message"],
        "Push Nr. 3",
        "{turn:#?}"
    );
    assert_eq!(state_values(&turn, "pushes"), [json!(3)]);
}

#[tokio::test]
async fn pol_009_ac2_effects_of_an_ask_rule_follow_its_resolution() {
    let env = Env::start().await;
    env.project_policy(
        "asked",
        "rules:\n  - id: confirm\n    on: tool_call\n    action: ask\n    approval: { timeout: 2s }\n    effects:\n      - { op: increment, scope: session, key: asked }\n",
    );
    let id = env.create(&env.scenario(&pushes(3, "ls"))).await;
    env.wait_status(&id, "idle").await;
    // Freigabe: angewendet.
    let before = env.start_turn(&id, "eins").await;
    env.resolve_with(&id, json!({"decision": "allow"})).await;
    let turn = env.turn_end(&id, before).await;
    assert_eq!(state_values(&turn, "asked"), [json!(1)]);
    // Ablehnung: mit Default effects_on ebenfalls.
    let before = env.start_turn(&id, "zwei").await;
    env.resolve_with(&id, json!({"decision": "deny"})).await;
    let turn = env.turn_end(&id, before).await;
    assert_eq!(state_values(&turn, "asked"), [json!(2)]);
    // Ablauf: nicht.
    let turn = env.turn(&id, "drei").await;
    assert_eq!(
        of_type(&turn, "approval.resolved")[0]["payload"]["via"],
        "timeout"
    );
    assert!(state_values(&turn, "asked").is_empty(), "{turn:#?}");
}

// ---------------------------------------------------------------------------------- UX-001

#[tokio::test]
async fn ux_001_ac1_approval_appears_in_the_inbox_and_is_done_after_resolve() {
    let env = Env::start().await;
    env.project_policy("push", ASK_PUSH_SESSION);
    let id = env
        .create(&env.scenario(&pushes(1, "git push origin main")))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    let requested_at = env.wait_for(&id, is("approval.requested")).await;
    let _ = requested_at;
    let start = Instant::now();
    let item = loop {
        let (status, page) = env.http("GET", "/v1/inbox?status=open", None).await;
        assert_eq!(status, 200, "{page}");
        if let Some(item) = page["items"].as_array().and_then(|i| i.first()) {
            assert_eq!(page["open_count"], 1);
            break item.clone();
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "nicht binnen 1 s in der Inbox"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(item["kind"], "approval");
    assert_eq!(item["session_id"], id.as_str());
    assert_eq!(item["summary"], "git push origin main");
    // Ein anderer Client entscheidet in der Session.
    env.resolve_with(&id, json!({"decision": "allow"})).await;
    env.turn_end(&id, before).await;
    let (_, done) = env
        .http(
            "GET",
            &format!("/v1/inbox/{}", item["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["resolution"]["decision"], "allow");
    let (_, page) = env.http("GET", "/v1/inbox?status=open", None).await;
    assert_eq!(page["open_count"], 0);
}

#[tokio::test]
async fn ux_001_ac2_approve_from_the_inbox_resumes_the_paused_session() {
    let env = Env::start().await;
    env.project_policy("push", ASK_PUSH_SESSION);
    let id = env
        .create(&env.scenario(&pushes(1, "git push origin main")))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "los").await;
    env.wait_status(&id, "waiting_approval").await;
    let (_, page) = env
        .http("GET", "/v1/inbox?status=open&kind=approval", None)
        .await;
    let item = page["items"][0]["id"].as_str().unwrap().to_owned();
    // Hinweise und Unbekanntes lassen sich nicht entscheiden.
    let (status, _) = env
        .http(
            "POST",
            "/v1/inbox/inb_unbekannt/resolve",
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!(status, 404);
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/inbox/{item}/resolve"),
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let turn = env.turn_end(&id, before).await;
    assert!(
        of_type(&turn, "tool.call.completed")
            .iter()
            .any(|c| c["payload"]["status"] == "ok"),
        "{turn:#?}"
    );
    // Schon entschieden: 409.
    let (status, _) = env
        .http(
            "POST",
            &format!("/v1/inbox/{item}/resolve"),
            Some(json!({"decision": "deny"})),
        )
        .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn ux_001_notify_inbox_becomes_a_notice() {
    let env = Env::start().await;
    env.project_policy("count", COUNT_PUSHES);
    let id = env.create(&env.scenario(&pushes(1, "git push"))).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "los").await;
    let (_, page) = env.http("GET", "/v1/inbox?kind=notice", None).await;
    let notice = &page["items"][0];
    assert_eq!(notice["status"], "info");
    assert_eq!(notice["title"], "Push Nr. 1");
    assert_eq!(page["open_count"], 0);
    let (status, _) = env
        .http(
            "POST",
            &format!("/v1/inbox/{}/resolve", notice["id"].as_str().unwrap()),
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!(status, 409);
    let (status, _) = env.http("GET", "/v1/inbox?status=kaputt", None).await;
    assert_eq!(status, 400);
}

// ------------------------------------------------- Security-Review M5: Fail-closed-Pfade

/// Session mit „für diese Session erlaubt“ für `git push origin feature/x`.
async fn session_with_grant(env: &Env, turns: usize) -> String {
    env.project_policy("push", ASK_PUSH_SESSION);
    let id = env
        .create(&env.scenario(&pushes(turns, "git push origin feature/x")))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.start_turn(&id, "eins").await;
    let (status, body) = env
        .resolve_with(&id, json!({"decision": "allow", "remember": "session"}))
        .await;
    assert_eq!(status, 200, "{body}");
    env.turn_end(&id, before).await;
    id
}

#[tokio::test]
async fn pol_010_ac2_grant_is_restored_from_the_log_after_a_runner_restart() {
    let mut env = Env::start().await;
    let id = session_with_grant(&env, 2).await;
    // Ohne offene Freigabe endet der Runner mit dem Daemon; ein neuer Runner lädt die Grants
    // aus dem Log (policy.state).
    env.restart().await;
    let turn = env.turn(&id, "zwei").await;
    assert_eq!(
        of_type(&turn, "session.started").len(),
        1,
        "neuer Runner: {turn:#?}"
    );
    assert!(approvals_of(&turn).is_empty(), "{turn:#?}");
    let d = decisions(&turn, "tool_call");
    assert_eq!(
        d[0]["payload"]["grants_used"].as_array().unwrap().len(),
        1,
        "{d:#?}"
    );
}

#[tokio::test]
async fn pol_010_ac2_a_fork_does_not_inherit_grants() {
    let env = Env::start().await;
    let id = session_with_grant(&env, 2).await;
    let (status, body) = env
        .http("POST", &format!("/v1/sessions/{id}/fork"), Some(json!({})))
        .await;
    assert_eq!(status, 201, "{body}");
    let child = body["session"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .unwrap()
        .to_owned();
    env.wait_status(&child, "idle").await;
    let before = env.start_turn(&child, "zwei").await;
    let card = env.open_approval(&child).await;
    assert_eq!(card["subject"]["summary"], "git push origin feature/x");
    env.resolve_with(&child, json!({"decision": "deny"})).await;
    env.turn_end(&child, before).await;
}

#[tokio::test]
async fn pol_010_resolving_twice_or_after_the_timeout_is_rejected() {
    let env = Env::start().await;
    env.project_policy(
        "push",
        "rules:\n  - id: confirm\n    on: tool_call\n    action: ask\n    approval: { timeout: 1s }\n",
    );
    let id = env.create(&env.scenario(&pushes(2, "git push"))).await;
    env.wait_status(&id, "idle").await;
    // Zweimal entscheiden: die zweite Entscheidung findet nichts mehr.
    let before = env.start_turn(&id, "eins").await;
    let card = env.open_approval(&id).await;
    let aid = card["id"].as_str().unwrap().to_owned();
    let url = format!("/v1/sessions/{id}/approvals/{aid}/resolve");
    let (first, _) = env
        .http("POST", &url, Some(json!({"decision": "deny"})))
        .await;
    assert_eq!(first, 200);
    env.turn_end(&id, before).await;
    let (second, body) = env
        .http("POST", &url, Some(json!({"decision": "allow"})))
        .await;
    assert_eq!(second, 404, "{body}");
    // Nach Ablauf: ebenfalls nichts mehr zu entscheiden, es bleibt bei deny.
    let turn = env.turn(&id, "zwei").await;
    let aid = approvals_of(&turn)[0]["payload"]["approval_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (late, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!(late, 404, "{body}");
    assert!(
        !of_type(&env.events(&id).await, "tool.call.completed")
            .iter()
            .any(|c| c["payload"]["status"] == "ok"),
        "nie ausgeführt"
    );
}
