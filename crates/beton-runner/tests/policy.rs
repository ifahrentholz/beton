//! Policy-Gate im Runner mit echtem Daemon, Runner-Binary und Fake-Harness (POL-001 AC2/AC3,
//! POL-003, POL-004, POL-006, POL-024, POL-025, SES-004 #101, #182).
//!
//! Policies liegen im Projekt (`<work>/.beton/policies/`) bzw. beim User
//! (`$BETON_HOME/policies/`); das echte Home des Entwicklers wird nie gelesen.

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

struct Env {
    dir: tempfile::TempDir,
    _daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

impl Env {
    async fn start() -> Self {
        Self::start_in(None).await
    }

    /// Daemon mit `user.timezone` (POL-004 AC4); `None` = Default UTC.
    async fn start_in(timezone: Option<beton_core::timezone::UserTimeZone>) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work/.beton/policies")).unwrap();
        let store = Store::open(dir.path(), StoreOptions::default())
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
        let runners = dir.path().join("runners");
        let daemon = start_with(cfg, store, move |mut r| {
            r.sessions.provider = std::sync::Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            if let Some(tz) = timezone {
                r.timezone = tz;
            }
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
            _daemon: daemon,
            token,
            addr,
        }
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    /// Schreibt eine User-Policy (`$BETON_HOME/policies/`).
    fn user_policy(&self, name: &str, body: &str) -> PathBuf {
        let dir = self.dir.path().join("home/.beton/policies");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.yaml"));
        std::fs::write(&path, format!("spec_version: 1\nname: {name}\n{body}")).unwrap();
        path
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

    async fn create(&self, scenario: &Path) -> String {
        self.create_with(scenario, json!({})).await
    }

    /// Wie [`Env::create`], mit weiteren Feldern (z. B. `model`).
    async fn create_with(&self, scenario: &Path, extra: Value) -> String {
        let mut body =
            json!({"target": "fake", "cwd": self.work(), "harness_opts": {"scenario": scenario}});
        if let (Some(b), Value::Object(e)) = (body.as_object_mut(), extra) {
            b.extend(e);
        }
        let (status, body) = self.http("POST", "/v1/sessions", Some(body)).await;
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

    /// Wie [`Env::turn`], gibt aber jede offene Freigabe frei.
    async fn turn_approving(&self, id: &str, text: &str) -> Vec<Value> {
        let before = self.events(id).await.len();
        let (status, body) = self.input(id, text).await;
        assert_eq!(status, 202, "{body}");
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            let new = events[before.min(events.len())..].to_vec();
            if new.iter().any(|e| e["type"] == "turn.completed") {
                return new;
            }
            let (_, approvals) = self
                .http("GET", &format!("/v1/sessions/{id}/approvals"), None)
                .await;
            if let Some(aid) = approvals["items"][0]["id"].as_str() {
                self.http(
                    "POST",
                    &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
                    Some(json!({"decision": "allow"})),
                )
                .await;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "{events:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn resolve(&self, id: &str, decision: &str) {
        let start = Instant::now();
        let aid = loop {
            let (_, approvals) = self
                .http("GET", &format!("/v1/sessions/{id}/approvals"), None)
                .await;
            if let Some(a) = approvals["items"][0]["id"].as_str() {
                break a.to_owned();
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "keine Freigabe offen"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/approvals/{aid}/resolve"),
                Some(json!({"decision": decision, "reason": "Test"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
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

const PUSH: &str = "turns:\n  - emit:\n      - { message: \"Ich pushe.\" }\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"git push origin main\" } } }\n      - { on_gate: { allow: [{ tool_result: \"gepusht\" }, { message: \"fertig\" }], deny: [{ message: \"abgelehnt\" }] } }\n";

// ------------------------------------------------------------------- POL-003 session_start

#[tokio::test]
async fn pol_003_ac1_ac3_session_start_deny_starts_no_harness() {
    let env = Env::start().await;
    env.project_policy(
        "start",
        "rules:\n  - id: no-start\n    on: session_start\n    when: 'start.harness == \"fake\"'\n    action: deny\n    reason: \"Fake-Sessions sind hier verboten\"\n",
    );
    let s = env.scenario(PUSH);
    let id = env.create(&s).await;
    let events = env.wait_for(&id, is("session.start_denied")).await;
    let denied = of_type(&events, "session.start_denied")[0];
    assert!(
        denied["payload"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("verboten")
    );
    assert_eq!(denied["payload"]["rules"], json!(["no-start"]));
    let d = decisions(&events, "session_start");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["payload"]["outcome"], "deny");
    // Kein Harness gestartet.
    assert!(
        of_type(&events, "session.started").is_empty(),
        "{events:#?}"
    );
    assert!(of_type(&events, "harness.ready").is_empty());
}

#[tokio::test]
async fn pol_003_ac3_session_start_ask_waits_for_the_approval() {
    let env = Env::start().await;
    env.project_policy(
        "start",
        "rules:\n  - id: confirm-start\n    on: session_start\n    action: ask\n    reason: \"Start bestätigen\"\n",
    );
    let s = env.scenario(PUSH);
    let id = env.create(&s).await;
    let events = env.wait_for(&id, is("approval.requested")).await;
    assert_eq!(
        of_type(&events, "approval.requested")[0]["payload"]["kind"],
        "policy"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = env.events(&id).await;
    assert!(
        of_type(&events, "session.started").is_empty(),
        "wartet auf Freigabe"
    );
    env.resolve(&id, "allow").await;
    env.wait_for(&id, is("session.started")).await;
    env.wait_status(&id, "idle").await;
}

// ------------------------------------------------------------------ POL-003 model_request

#[tokio::test]
async fn pol_003_ac1_model_request_deny_blocks_the_turn() {
    let env = Env::start().await;
    env.project_policy(
        "prompts",
        "rules:\n  - id: no-secrets\n    on: model_request\n    when: 'request.prompt.contains(\"geheim\")'\n    action: deny\n    reason: \"Keine Geheimnisse an das Modell\"\n",
    );
    let s = env.scenario("turns:\n  - emit: [{ message: \"ok\" }]\n");
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let (status, body) = env.input(&id, "das ist geheim").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["code"], "policy_denied");
    let events = env.events(&id).await;
    let d = decisions(&events, "model_request");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["payload"]["outcome"], "deny");
    assert!(of_type(&events, "turn.started").is_empty());
    // Erlaubte Eingabe läuft.
    let turn = env.turn(&id, "harmlos").await;
    assert_eq!(of_type(&turn, "turn.completed").len(), 1);
    assert_eq!(
        decisions(&turn, "model_request")[0]["payload"]["outcome"],
        "allow"
    );
}

// ---------------------------------------------------------------------- POL-003 tool_call

#[tokio::test]
async fn pol_003_ac1_pol_006_ac1_tool_call_deny_is_enforced_with_reason() {
    let env = Env::start().await;
    env.project_policy(
        "git",
        "rules:\n  - id: no-push\n    on: tool_call\n    when: 'tool.kind == \"shell\" && shell.has_command(tool.argv, \"git\", \"push\")'\n    action: deny\n    reason: \"Push nur nach Review\"\n",
    );
    let id = env.create(&env.scenario(PUSH)).await;
    env.wait_status(&id, "idle").await;
    let turn = env.turn(&id, "los").await;
    let requested = of_type(&turn, "tool.call.requested")[0];
    let call_id = requested["payload"]["call_id"].clone();
    let d = decisions(&turn, "tool_call");
    assert_eq!(d.len(), 1, "{turn:#?}");
    assert_eq!(d[0]["payload"]["outcome"], "deny");
    assert_eq!(d[0]["payload"]["subject_ref"]["call_id"], call_id);
    assert_eq!(d[0]["payload"]["enforcement"], "full");
    assert_eq!(d[0]["payload"]["matched"][0]["rule_id"], "no-push");
    let done = of_type(&turn, "tool.call.completed")[0];
    assert_eq!(done["payload"]["status"], "denied");
    // POL-006 AC1: Das Modell erhält den Grund als Tool-Fehler.
    assert!(
        done["payload"]["result"]
            .to_string()
            .contains("Push nur nach Review")
    );
    assert!(of_type(&turn, "tool.call.started").is_empty());
    // POL-025 AC2: keine Tool-Argumente im Klartext.
    assert!(!d[0].to_string().contains("git push"), "{}", d[0]);
    assert!(
        d[0]["payload"]["subject_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[tokio::test]
async fn pol_006_ac3_modified_args_are_executed_and_requested_keeps_the_original() {
    let env = Env::start().await;
    // `args` ändert nur eine Policy ab der User-Ebene (#234).
    env.user_policy(
        "dry",
        "rules:\n  - id: dry-run\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: modify\n    modify: { args: { command: \"git push --dry-run origin main\" } }\n",
    );
    let id = env.create(&env.scenario(PUSH)).await;
    env.wait_status(&id, "idle").await;
    let turn = env.turn(&id, "los").await;
    let requested = of_type(&turn, "tool.call.requested")[0];
    assert_eq!(
        requested["payload"]["args"]["command"],
        "git push origin main"
    );
    let started = of_type(&turn, "tool.call.started")[0];
    assert_eq!(
        started["payload"]["args"]["command"],
        "git push --dry-run origin main"
    );
    let d = decisions(&turn, "tool_call");
    assert_eq!(d[0]["payload"]["outcome"], "allow");
    assert!(d[0]["payload"]["modified"]["args"].is_object());
}

#[tokio::test]
async fn pol_006_ac2_notify_creates_a_notice_without_blocking() {
    let env = Env::start().await;
    env.project_policy(
        "tell",
        "rules:\n  - id: tell-push\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: notify\n    notify: { channels: [push], message: \"Shell-Aufruf\" }\n",
    );
    let id = env.create(&env.scenario(PUSH)).await;
    env.wait_status(&id, "idle").await;
    let turn = env.turn(&id, "los").await;
    assert_eq!(
        decisions(&turn, "tool_call")[0]["payload"]["outcome"],
        "allow"
    );
    // Kanal `push` wird bis Web-Push (WEB-014) zum Inbox-Eintrag (UX-001).
    assert!(
        of_type(&turn, "policy.notified")
            .iter()
            .any(|n| n["payload"]["message"] == "Shell-Aufruf"
                && n["payload"]["channels"] == json!(["push"])),
        "{turn:#?}"
    );
    assert_eq!(
        of_type(&turn, "tool.call.completed")[0]["payload"]["status"],
        "ok"
    );
}

// -------------------------------------------------------------------- POL-003 tool_result

#[tokio::test]
async fn pol_003_ac1_tool_result_deny_replaces_the_result() {
    let env = Env::start().await;
    env.project_policy(
        "results",
        "rules:\n  - id: no-leak\n    on: tool_result\n    when: 'result.text.contains(\"PASSWORT\")'\n    action: deny\n    reason: \"Ergebnis enthält ein Passwort\"\n",
    );
    let s = env.scenario("turns:\n  - emit:\n      - { tool_call: { name: Read, kind: file_read, args: { file_path: \"/tmp/x\" } } }\n      - { tool_result: \"PASSWORT=4711\" }\n");
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let turn = env.turn(&id, "lies").await;
    let d = decisions(&turn, "tool_result");
    assert_eq!(d.len(), 1, "{turn:#?}");
    assert_eq!(d[0]["payload"]["outcome"], "deny");
    let done = of_type(&turn, "tool.call.completed")[0];
    assert_eq!(done["payload"]["status"], "error");
    assert!(!done.to_string().contains("4711"), "{done}");
    assert!(done.to_string().contains("Passwort"));
}

// ------------------------------------------------------------------------------ POL-024

#[tokio::test]
async fn pol_024_ac1_unenforceable_required_rule_refuses_the_start() {
    let env = Env::start().await;
    env.project_policy(
        "edits",
        "rules:\n  - id: no-edits\n    on: tool_call\n    when: 'tool.kind == \"file_edit\"'\n    action: deny\n",
    );
    let s = env.scenario(
        "capabilities: { tool_call_gate: approval_only, approval_kinds: [shell] }\nturns: []\n",
    );
    let id = env.create(&s).await;
    let events = env.wait_for(&id, is("session.start_denied")).await;
    let denied = of_type(&events, "session.start_denied")[0];
    assert_eq!(denied["payload"]["rules"], json!(["no-edits"]));
    assert!(
        denied["payload"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("enforcement_unavailable")
    );
    assert!(of_type(&events, "session.started").is_empty());
}

#[tokio::test]
async fn pol_024_ac1_gap_on_reading_class_starts_with_a_warning() {
    let env = Env::start().await;
    env.project_policy(
        "reads",
        "rules:\n  - id: no-dotenv\n    on: tool_call\n    when: 'tool.kind == \"file_read\"'\n    action: deny\n",
    );
    let s = env.scenario("capabilities: { tool_call_gate: approval_only, approval_kinds: [shell, file_edit] }\nturns: []\n");
    let id = env.create(&s).await;
    let events = env.wait_for(&id, is("policy.enforcement_degraded")).await;
    let w = of_type(&events, "policy.enforcement_degraded")[0];
    assert_eq!(w["payload"]["rule_ids"], json!(["no-dotenv"]));
    env.wait_status(&id, "idle").await;
}

// ------------------------------------------------------------------------------ POL-025

#[tokio::test]
async fn pol_025_ac1_every_tool_call_has_exactly_one_decision() {
    let env = Env::start().await;
    let yaml = "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"ls\" } } }\n      - { tool_result: \"a\" }\n      - { tool_call: { name: Read, kind: file_read, args: { file_path: \"/tmp/a\" } }, gate: true }\n      - { on_gate: { allow: [{ tool_result: \"b\" }], deny: [{ message: \"nein\" }] } }\n      - { tool_call: { name: Edit, kind: file_edit, args: { file_path: \"/tmp/a\" } } }\n      - { tool_result: \"c\" }\n";
    // Voller Gate (Fake-Default) und nur Rückfragen (Rest beobachtet).
    for caps in [
        "",
        "capabilities: { tool_call_gate: approval_only, approval_kinds: [file_read] }\n",
    ] {
        let id = env.create(&env.scenario(&format!("{caps}{yaml}"))).await;
        env.wait_status(&id, "idle").await;
        // Die Rückfrage des Vendors (`gate: true`) legt beton dem Menschen vor.
        let turn = env.turn_approving(&id, "los").await;
        let calls: Vec<String> = of_type(&turn, "tool.call.requested")
            .iter()
            .map(|e| e["payload"]["call_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(calls.len(), 3);
        // Beobachtete Calls (ohne Gate-Anfrage) wertet der Runner nach ihrem Abschluss aus;
        // deren Entscheidung darf nach dem Turn-Ende im Log stehen. Auf sie warten statt sie
        // im Stand beim Turn-Ende zu erwarten (gesehen als Flake unter Last).
        let start = Instant::now();
        let turn = loop {
            let all = env.events(&id).await;
            let d = decisions(&all, "tool_call");
            let decided = |call: &String| {
                d.iter()
                    .any(|e| e["payload"]["subject_ref"]["call_id"] == call.as_str())
            };
            if calls.iter().all(decided) {
                break all;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "{caps}: Entscheidung fehlt: {all:#?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let d = decisions(&turn, "tool_call");
        for call in &calls {
            let n = d
                .iter()
                .filter(|e| e["payload"]["subject_ref"]["call_id"] == call.as_str())
                .count();
            assert_eq!(n, 1, "{caps}: {call}: {turn:#?}");
        }
        assert_eq!(d.len(), 3);
        if !caps.is_empty() {
            assert!(
                d.iter()
                    .any(|e| e["payload"]["enforcement"] == "observe_only")
            );
            assert!(
                d.iter()
                    .any(|e| e["payload"]["enforcement"] == "approval_only")
            );
        }
    }
}

// ----------------------------------------------------------------------- POL-001, #182

#[tokio::test]
async fn pol_001_ac2_ac3_policy_changes_apply_to_the_running_session() {
    let env = Env::start().await;
    let file = env.project_policy("git", "rules: []\n");
    let turn = PUSH.trim_start_matches("turns:\n");
    let s = env.scenario(&format!("turns:\n{turn}{turn}{turn}"));
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let first = env.turn(&id, "los").await;
    let hash1 = decisions(&first, "tool_call")[0]["payload"]["policy_set_hash"].clone();
    assert_eq!(
        of_type(&first, "tool.call.completed")[0]["payload"]["status"],
        "ok"
    );
    // Datei ändern: die nächste Auswertung nutzt die neue Version (ohne Neustart).
    std::fs::write(
        &file,
        "spec_version: 1\nname: git\nrules:\n  - id: no-push\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: deny\n",
    )
    .unwrap();
    let second = env.turn(&id, "los").await;
    assert_eq!(
        of_type(&second, "policy.set_changed").len(),
        1,
        "{second:#?}"
    );
    let d = decisions(&second, "tool_call");
    assert_ne!(d[0]["payload"]["policy_set_hash"], hash1);
    assert_eq!(d[0]["payload"]["outcome"], "deny");
    // Kaputte Änderung: alte Version bleibt, `policy.set_invalid` mit Datei und Zeile.
    std::fs::write(
        &file,
        "spec_version: 1\nname: git\nrules:\n  - id: no-push\n    on: tool_call\n    when: 'tool.kind > 5'\n    action: deny\n",
    )
    .unwrap();
    let third = env.turn(&id, "los").await;
    let invalid = of_type(&third, "policy.set_invalid");
    assert_eq!(invalid.len(), 1, "{third:#?}");
    assert_eq!(invalid[0]["payload"]["line"], 6);
    assert_eq!(
        decisions(&third, "tool_call")[0]["payload"]["outcome"],
        "deny"
    );
}

#[tokio::test]
async fn pol_001_never_valid_policy_file_refuses_the_start() {
    let env = Env::start().await;
    env.project_policy(
        "kaputt",
        "rules:\n  - id: r\n    on: tool_call\n    when: 'tool.kind > 5'\n    action: deny\n    reason: GEHEIMER-INHALT-4711\n",
    );
    let id = env.create(&env.scenario(PUSH)).await;
    let events = env.wait_for(&id, is("session.start_denied")).await;
    let reason = of_type(&events, "session.start_denied")[0]["payload"]["reasons"][0]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.contains("kaputt.yaml"), "{reason}");
    assert!(reason.contains("Zeile 6"), "{reason}");
    assert!(
        !reason.contains("GEHEIMER"),
        "keine Inhalte der Datei: {reason}"
    );
    assert!(of_type(&events, "session.started").is_empty());
}

// ------------------------------------------------------------------------------ POL-004

#[tokio::test]
async fn pol_004_ac3_session_cost_reflects_logged_cost_deltas() {
    let env = Env::start().await;
    env.project_policy(
        "budget",
        "rules:\n  - id: cap\n    on: model_request\n    when: 'session.cost_usd > 1.0'\n    action: deny\n    reason: \"Budget der Session erreicht\"\n",
    );
    let s = env.scenario("turns:\n  - emit: [{ message: \"teuer\" }, { usage: { input_tokens: 10, output_tokens: 10, cost_usd: 2.5 } }]\n  - emit: [{ message: \"nie\" }]\n");
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let first = env.turn(&id, "eins").await;
    assert_eq!(of_type(&first, "turn.completed").len(), 1);
    // Sofort danach: das cost.delta zählt schon.
    let (status, body) = env.input(&id, "zwei").await;
    assert_eq!(status, 403, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("Budget"));
}

#[tokio::test]
async fn pol_004_ac4_user_timezone_comes_from_the_setting() {
    let berlin = beton_core::timezone::UserTimeZone::parse("Europe/Berlin").unwrap();
    let env = Env::start_in(Some(berlin)).await;
    // Nur `user.timezone`, ohne Kostenfeld: der Wert kommt trotzdem vom Daemon.
    env.project_policy(
        "zeitzone",
        "rules:\n  - id: tz\n    on: model_request\n    when: 'user.timezone == \"Europe/Berlin\" && user.daily_cost_usd == 0.0'\n    action: deny\n    reason: \"Zeitzone Europe/Berlin\"\n",
    );
    let s = env.scenario("turns:\n  - emit: [{ message: \"nie\" }]\n");
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let (status, body) = env.input(&id, "eins").await;
    assert_eq!(status, 403, "{body}");
    assert!(
        body["detail"].as_str().unwrap().contains("Europe/Berlin"),
        "{body}"
    );
}

// ------------------------------------------------------------- POL-011 bis POL-013 (WP-43)

#[tokio::test]
async fn pol_011_ac1_one_card_at_the_threshold_and_deny_at_the_limit() {
    let env = Env::start().await;
    env.project_policy(
        "budget",
        "rules:\n  - id: budget\n    type: spend_cap\n    params: { scope: session, limit_usd: 5, ask_at_usd: [3] }\n",
    );
    let s = env.scenario(
        "turns:\n  - emit: [{ message: \"eins\" }, { usage: { input_tokens: 10, output_tokens: 10, cost_usd: 3.1 } }]\n  - emit: [{ message: \"zwei\" }]\n  - emit: [{ message: \"drei\" }, { usage: { input_tokens: 10, output_tokens: 10, cost_usd: 1.9 } }]\n  - emit: [{ message: \"nie\" }]\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    // 3,10 USD: genau eine Card.
    env.turn_approving(&id, "zwei").await;
    env.turn(&id, "drei").await;
    let events = env.events(&id).await;
    assert_eq!(
        of_type(&events, "approval.requested").len(),
        1,
        "{events:#?}"
    );
    // 5,00 USD: `deny` vor dem nächsten Turn, gemeldet als `budget.exhausted`.
    let (status, body) = env.input(&id, "vier").await;
    assert_eq!(status, 403, "{body}");
    assert!(
        body["detail"].as_str().unwrap().contains("Kostenlimit"),
        "{body}"
    );
    let events = env.wait_for(&id, is("budget.exhausted")).await;
    let exhausted = of_type(&events, "budget.exhausted");
    assert_eq!(exhausted.len(), 1, "{events:#?}");
    assert_eq!(exhausted[0]["payload"]["scope"], "session");
    assert_eq!(exhausted[0]["payload"]["action"], "deny");
}

#[tokio::test]
async fn pol_011_ac2_route_switches_the_model_live_after_the_limit() {
    let env = Env::start().await;
    env.project_policy(
        "budget",
        "rules:\n  - id: budget\n    type: spend_cap\n    params: { limit_usd: 1, on_exceed: route, fallback_model: fake-small }\n",
    );
    let s = env.scenario(
        "turns:\n  - emit: [{ echo_settings: true }, { usage: { input_tokens: 10, output_tokens: 10, cost_usd: 1.5 } }]\n  - emit: [{ echo_settings: true }]\n",
    );
    let id = env.create_with(&s, json!({"model": "fake-large"})).await;
    env.wait_status(&id, "idle").await;
    let first = env.turn(&id, "eins").await;
    assert!(
        format!("{first:?}").contains("model=fake-large"),
        "{first:#?}"
    );
    let second = env.turn(&id, "zwei").await;
    assert!(
        format!("{second:?}").contains("model=fake-small"),
        "{second:#?}"
    );
    let events = env.events(&id).await;
    let changed = of_type(&events, "session.settings_changed");
    assert_eq!(changed.len(), 1, "{events:#?}");
    assert_eq!(changed[0]["payload"]["model"], "fake-small");
    assert!(
        matches!(
            changed[0]["payload"]["mechanism"].as_str(),
            Some("live" | "restart")
        ),
        "{changed:#?}"
    );
    // Umrouten erschöpft kein Budget.
    assert!(of_type(&events, "budget.exhausted").is_empty());
}

#[tokio::test]
async fn pol_012_ac1_ac2_the_daily_budget_spans_all_sessions_of_the_user() {
    let env = Env::start().await;
    env.user_policy(
        "daily",
        "rules:\n  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50 }\n",
    );
    let s = env.scenario(
        "turns:\n  - emit: [{ message: \"teuer\" }, { usage: { input_tokens: 10, output_tokens: 10, cost_usd: 30.0 } }]\n  - emit: [{ message: \"nie\" }]\n",
    );
    let a = env.create(&s).await;
    let b = env.create(&s).await;
    env.wait_status(&a, "idle").await;
    env.wait_status(&b, "idle").await;
    env.turn(&a, "eins").await;
    env.turn(&b, "eins").await;
    // 60 USD heute: der nächste Turn beider Sessions wird abgelehnt.
    for id in [&a, &b] {
        let (status, body) = env.input(id, "zwei").await;
        assert_eq!(status, 403, "{body}");
    }
    // Neue Sessions lehnt schon `session_start` ab.
    let c = env.create(&s).await;
    let events = env.wait_for(&c, is("session.start_denied")).await;
    assert!(
        of_type(&events, "session.started").is_empty(),
        "{events:#?}"
    );
    let exhausted = of_type(&events, "budget.exhausted");
    assert_eq!(exhausted[0]["payload"]["scope"], "user", "{events:#?}");
}

#[tokio::test]
async fn pol_013_ac3_model_route_without_model_switch_denies_with_an_explanation() {
    let env = Env::start().await;
    env.project_policy(
        "routing",
        "rules:\n  - id: cheap\n    type: model_route\n    params: { routes: [{ when: 'request.prompt.contains(\"billig\")', model: fake-small }] }\n",
    );
    let s = env.scenario(
        "capabilities: { model_switch: none }\nturns:\n  - emit: [{ message: \"eins\" }]\n  - emit: [{ message: \"nie\" }]\n",
    );
    let id = env.create_with(&s, json!({"model": "fake-large"})).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    let (status, body) = env.input(&id, "bitte billig").await;
    assert_eq!(status, 403, "{body}");
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.contains("model"), "{body}");
    assert!(detail.contains("nicht"), "{body}");
}

#[tokio::test]
async fn pol_011_a_subscription_session_stops_at_its_list_price_equivalent() {
    let env = Env::start().await;
    env.project_policy(
        "budget",
        "rules:\n  - id: budget\n    type: spend_cap\n    params: { limit_usd: 1 }\n",
    );
    // 1 Mio. Eingabe-Tokens Sonnet 4.5 über ein Abo: kein Geld, aber 3 USD Listenpreis.
    let s = env.scenario(
        "turns:\n  - emit: [{ message: \"abo\" }, { usage: { input_tokens: 1000000, output_tokens: 0, model: claude-sonnet-4-5, subscription: true } }]\n  - emit: [{ message: \"nie\" }]\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    let (status, body) = env.input(&id, "zwei").await;
    assert_eq!(status, 403, "{body}");
    let events = env.events(&id).await;
    let cost = of_type(&events, "cost.delta");
    assert_eq!(
        cost[0]["payload"]["cost_source"], "subscription",
        "{cost:#?}"
    );
    assert!(cost[0]["payload"].get("cost_micro").is_none(), "{cost:#?}");
}

#[tokio::test]
async fn pol_011_without_a_list_price_the_budget_reports_rule_inapplicable() {
    let env = Env::start().await;
    env.project_policy(
        "budget",
        "rules:\n  - id: budget\n    type: spend_cap\n    params: { limit_usd: 1 }\n",
    );
    let s = env.scenario(
        "turns:\n  - emit: [{ message: \"abo\" }, { usage: { input_tokens: 1000000, output_tokens: 0, model: modell-ohne-preis, subscription: true } }]\n  - emit: [{ message: \"weiter\" }]\n  - emit: [{ message: \"noch\" }]\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    // Die Kosten sind unbekannt: die Regel greift nicht, sie meldet das (einmal je Session).
    env.turn(&id, "zwei").await;
    env.turn(&id, "drei").await;
    let events = env.events(&id).await;
    let issues = of_type(&events, "policy.rule_inapplicable");
    assert_eq!(issues.len(), 1, "{events:#?}");
    assert_eq!(issues[0]["payload"]["rule_ids"], json!(["budget"]));
    assert!(
        issues[0]["payload"]["reason"]
            .as_str()
            .unwrap()
            .contains("Preis"),
        "{issues:#?}"
    );
}

// ---------------------------------------------------------------------- SES-004 (#101)

#[tokio::test]
async fn ses_004_queued_input_is_checked_on_dequeue() {
    let env = Env::start().await;
    env.project_policy(
        "prompts",
        "rules:\n  - id: no-secrets\n    on: model_request\n    when: 'request.prompt.contains(\"geheim\")'\n    action: deny\n  - id: confirm\n    on: model_request\n    when: 'request.prompt.contains(\"frag\")'\n    action: ask\n",
    );
    let s = env.scenario("turns:\n  - emit: [{ message: \"langsam\", delay_ms: 1500 }]\n  - emit: [{ message: \"zwei\" }]\n  - emit: [{ message: \"drei\" }]\n");
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    assert_eq!(env.input(&id, "eins").await.0, 202);
    env.wait_status(&id, "running").await;
    // Während des Turns eingereiht; geprüft wird erst beim Abarbeiten.
    assert_eq!(env.input(&id, "geheim").await.0, 202);
    assert_eq!(env.input(&id, "bitte frag").await.0, 202);
    assert_eq!(env.input(&id, "danach").await.0, 202);
    let events = env.wait_for(&id, is("approval.requested")).await;
    // `deny` entfernt den Eintrag mit policy.decision im Verlauf.
    let d = decisions(&events, "model_request");
    assert!(
        d.iter().any(|e| e["payload"]["outcome"] == "deny"),
        "{events:#?}"
    );
    // `ask` hält die Queue an: der dritte Eintrag läuft erst nach der Freigabe.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let events = env.events(&id).await;
    assert_eq!(of_type(&events, "turn.completed").len(), 1, "{events:#?}");
    env.resolve(&id, "allow").await;
    let start = Instant::now();
    loop {
        let events = env.events(&id).await;
        if of_type(&events, "turn.completed").len() >= 3 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "{events:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ------------------------------------------------------------------ HAR-005, #233 (K3)

#[tokio::test]
async fn har_005_tool_call_without_gate_decision_ends_the_session() {
    // Der Harness meldet vollen Gate, führt aber einen Tool-Call ohne Gate-Anfrage aus (wie
    // Claude mit ausgehebeltem `PreToolUse`-Hook): Integritätsverletzung, Session endet.
    let env = Env::start().await;
    let s = env.scenario(
        "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"echo hi\" } }, skip_gate: true }\n      - { tool_result: \"hi\" }\n      - { message: \"fertig\" }\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let (status, body) = env.input(&id, "los").await;
    assert_eq!(status, 202, "{body}");
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "error" && e["payload"]["problem"]["code"] == "policy_integrity_violation"
        })
        .await;
    env.wait_status(&id, "failed").await;
    let error = events.iter().find(|e| e["type"] == "error").unwrap();
    // Keine Argumente im Klartext.
    assert!(!error.to_string().contains("echo hi"), "{error}");
}

// ------------------------------------------------------------------ POL-001, #234 (M4)

#[tokio::test]
async fn pol_001_reload_with_unenforceable_rule_ends_the_session() {
    // Nach dem Start hinzugefügte Pflichtregel, die der Harness nicht durchsetzen kann: Die
    // Session endet (wie die Prüfung beim Start, fail closed).
    let env = Env::start().await;
    let s = env.scenario(
        "capabilities: { tool_call_gate: approval_only, approval_kinds: [shell] }\nturns:\n  - emit: [{ message: \"ok\" }]\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    env.project_policy(
        "edit",
        "rules:\n  - id: no-edit\n    on: tool_call\n    when: 'tool.kind == \"file_edit\"'\n    action: deny\n",
    );
    // Die Eingabe löst den Reload aus; der Runner endet dabei (Antwort 202 oder 503).
    let _ = env.input(&id, "los").await;
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "error"
                && e["payload"]["problem"]["code"] == "policy_enforcement_unavailable"
        })
        .await;
    let error = events.iter().find(|e| e["type"] == "error").unwrap();
    assert_eq!(error["payload"]["problem"]["rules"], json!(["no-edit"]));
    env.wait_status(&id, "failed").await;
}

#[tokio::test]
async fn har_005_failed_tool_call_without_gate_decision_also_ends_the_session() {
    // Re-Review R2 (#253): Ein Kommando mit `…; exit 1` endet als Fehler (Claude `is_error`),
    // wurde aber ausgeführt. Auch das ist ohne Gate-Entscheidung eine Integritätsverletzung.
    let env = Env::start().await;
    let s = env.scenario(
        "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"rm -rf x; exit 1\" } }, skip_gate: true }\n      - { tool_error: \"exit 1\" }\n      - { message: \"fertig\" }\n",
    );
    let id = env.create(&s).await;
    env.wait_status(&id, "idle").await;
    let _ = env.input(&id, "los").await;
    env.wait_for(&id, |e| {
        e["type"] == "error" && e["payload"]["problem"]["code"] == "policy_integrity_violation"
    })
    .await;
    env.wait_status(&id, "failed").await;
}
