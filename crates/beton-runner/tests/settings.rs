//! Modell-, Effort- und Permission-Mode-Wechsel sowie Warm-Resume nach Runner-Neustart mit
//! echtem Daemon und Runner (HAR-017, HAR-020, HAR-027, SES-006/SES-007 #110). Claude Code ist
//! die Fake-CLI (QA-002), ihr Verlauf liegt mit `--persist` in einem eigenen
//! `CLAUDE_CONFIG_DIR`; sonst spielt der Fake-Harness.

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

/// Testumgebung: eigenes Home, Arbeitsverzeichnis `work`, Daemon im Entwicklermodus.
struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    #[allow(dead_code)]
    store: Store,
    token: String,
    addr: SocketAddr,
}

impl Env {
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    async fn start(vars: impl FnOnce(&Path) -> Vec<(&'static str, String)>) -> Self {
        // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
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
        for (k, v) in vars(dir.path()) {
            inherit.insert(k.to_owned(), v);
        }
        let runners = dir.path().join("runners");
        // Der Import (SES-008) sucht in denselben Vendor-Verzeichnissen wie der Runner.
        let mut vendor_env = beton_harness::HostEnv::default();
        for (k, v) in &inherit {
            if beton_harness::VENDOR_DIR_VARS.contains(&k.as_str()) {
                vendor_env.vars.insert(k.clone(), v.clone());
            }
        }
        let daemon = start_with(cfg, store.clone(), move |mut r| {
            r.sessions.vendor_env = vendor_env;
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

    async fn create(&self, body: Value) -> String {
        let (status, body) = self.http("POST", "/v1/sessions", Some(body)).await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn fork(&self, id: &str, body: Value) -> Value {
        let (status, body) = self
            .http("POST", &format!("/v1/sessions/{id}/fork"), Some(body))
            .await;
        assert_eq!(status, 201, "{body}");
        body
    }

    async fn patch(&self, id: &str, body: Value) -> (u16, Value) {
        self.http("PATCH", &format!("/v1/sessions/{id}"), Some(body))
            .await
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
                "Status {want} kommt nicht (ist {})",
                self.status(id).await
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Beendet den Runner der Session hart (SIGKILL), wie bei einem Absturz.
    async fn kill_runner(&self, id: &str) {
        let session: beton_core::id::SessionId = id.parse().unwrap();
        let pid = self
            .daemon
            .runtime
            .sessions
            .launched
            .pid(session)
            .await
            .expect("Runner-PID");
        let status = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let start = Instant::now();
        while self.daemon.runtime.runners.connected(session) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Runner lebt noch"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
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

    /// Sendet eine Eingabe und wartet auf das Ende des Turns; liefert die Texte des Agents.
    async fn turn(&self, id: &str, text: &str) -> Vec<String> {
        let before = self.events(id).await.len();
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({"text": text})),
            )
            .await;
        assert_eq!(status, 202, "{body}");
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            let new = &events[before.min(events.len())..];
            if new.iter().any(|e| {
                matches!(
                    e["type"].as_str(),
                    Some("turn.completed" | "turn.failed" | "turn.interrupted")
                )
            }) {
                return new
                    .iter()
                    .filter(|e| {
                        e["type"] == "message.completed" && e["payload"]["role"] == "assistant"
                    })
                    .map(|e| {
                        e["payload"]["content"][0]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .collect();
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

fn one<'a>(events: &'a [Value], t: &str) -> &'a Value {
    let found = of_type(events, t);
    assert_eq!(
        found.len(),
        1,
        "{t}: {}",
        serde_json::to_string_pretty(events).unwrap()
    );
    found[0]
}

fn claude_cmd(scenario: &Path, persist: bool) -> String {
    format!(
        "{} --protocol stream-json --scenario {}{}",
        fake_cli().display(),
        scenario.display(),
        if persist { " --persist" } else { "" }
    )
}

/// Fake-Harness-Session mit Szenario.
async fn fake_session(env: &Env, yaml: &str, extra: Value) -> String {
    let scenario = write(
        &env.path(&format!("{}.yaml", beton_core::id::RunnerId::new())),
        yaml,
    );
    let mut body = json!({
        "target": "fake",
        "cwd": env.work(),
        "harness_opts": {"scenario": scenario},
    });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let id = env.create(body).await;
    env.wait_status(&id, "idle").await;
    id
}

/// Claude-Session über die Fake-CLI mit eigenem Verlauf (`--persist`).
async fn claude_env(yaml: &str) -> Env {
    let yaml = yaml.to_owned();
    Env::start(move |dir| {
        let scenario = write(&dir.join("claude.yaml"), &yaml);
        vec![
            ("BETON_CLAUDE_PATH", claude_cmd(&scenario, true)),
            (
                "CLAUDE_CONFIG_DIR",
                dir.join("claude-config").display().to_string(),
            ),
        ]
    })
    .await
}

fn settings_changed(events: &[Value]) -> Vec<&Value> {
    of_type(events, "session.settings_changed")
}

const ECHO: &str = "turns:\n  - emit: [{ echo_history: true }, { echo_settings: true }]\n  - emit: [{ echo_history: true }, { echo_settings: true }]\n  - emit: [{ echo_history: true }, { echo_settings: true }]\n";

#[tokio::test]
async fn har_017_ac1_claude_model_switch_applies_to_next_turn_with_history() {
    let env = claude_env(ECHO).await;
    let id = env
        .create(json!({"target": "claude", "cwd": env.work(), "model": "sonnet"}))
        .await;
    env.wait_status(&id, "idle").await;
    let first = env.turn(&id, "eins").await;
    assert_eq!(first[1], "model=sonnet effort=- mode=default");
    let (status, body) = env.patch(&id, json!({"model": "opus"})).await;
    assert_eq!(status, 200, "{body}");
    let events = env
        .wait_for(&id, |e| e["type"] == "session.settings_changed")
        .await;
    let changed = settings_changed(&events);
    assert_eq!(changed[0]["payload"]["model"], "opus");
    assert_eq!(changed[0]["payload"]["mechanism"], "live");
    let second = env.turn(&id, "zwei").await;
    // Verlauf erhalten (dieselbe native Session), Modell gewechselt.
    assert_eq!(second[0], "eins");
    assert_eq!(second[1], "model=opus effort=- mode=default");
    let events = env.events(&id).await;
    let costs = of_type(&events, "cost.delta");
    assert_eq!(costs.last().unwrap()["payload"]["model"], "opus");
    // Der Wechsel wirkt ab dem nächsten Turn: `effective_from_turn` ist dessen ID.
    let started = of_type(&events, "turn.started");
    assert_eq!(
        changed[0]["payload"]["effective_from_turn"],
        started.last().unwrap()["payload"]["turn_id"]
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_017_ac2_restart_switch_resumes_with_previous_context() {
    let env = Env::start(|_| Vec::new()).await;
    let id = fake_session(
        &env,
        &format!("capabilities: {{ model_switch: restart }}\n{ECHO}"),
        json!({}),
    )
    .await;
    assert_eq!(env.turn(&id, "eins").await[0], "(kein Verlauf)");
    let (status, body) = env.patch(&id, json!({"model": "fake-large"})).await;
    assert_eq!(status, 200, "{body}");
    let events = env
        .wait_for(&id, |e| e["type"] == "session.settings_changed")
        .await;
    let changed = settings_changed(&events);
    assert_eq!(changed[0]["payload"]["mechanism"], "restart");
    assert_eq!(changed[0]["payload"]["model"], "fake-large");
    // Neu gestartet: zweites `harness.ready` mit derselben nativen Referenz.
    let start = Instant::now();
    let mut events = events;
    while of_type(&events, "harness.ready").len() < 2 && start.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        events = env.events(&id).await;
    }
    let ready = of_type(&events, "harness.ready");
    assert_eq!(ready.len(), 2, "{events:#?}");
    assert_eq!(
        ready[0]["payload"]["harness_session_ref"],
        ready[1]["payload"]["harness_session_ref"]
    );
    let second = env.turn(&id, "zwei").await;
    assert_eq!(second[0], "eins", "der nächste Turn kennt den Kontext");
    assert_eq!(second[1], "model=fake-large effort=- mode=default");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_017_ac3_unsupported_effort_is_mapped_and_reported() {
    let env = Env::start(|_| Vec::new()).await;
    // Fake: Stufen low, medium, high.
    let id = fake_session(&env, ECHO, json!({"effort": "xhigh"})).await;
    let events = env.events(&id).await;
    let start = settings_changed(&events);
    assert_eq!(start[0]["payload"]["effort"], "high");
    assert_eq!(start[0]["payload"]["requested_effort"], "xhigh");
    assert_eq!(
        env.turn(&id, "eins").await[1],
        "model=fake-model effort=high mode=default"
    );
    let (status, body) = env.patch(&id, json!({"effort": "low"})).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = env.patch(&id, json!({"effort": "xhigh"})).await;
    assert_eq!(status, 200, "{body}");
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "session.settings_changed"
                && e["payload"]["requested_effort"] == "xhigh"
                && e["payload"]["mechanism"] == "live"
        })
        .await;
    let last = *settings_changed(&events).last().unwrap();
    assert_eq!(last["payload"]["effort"], "high");
    // Unbekannte Stufe: validation_failed; Harness ohne Stufen: capability_unsupported.
    let (status, body) = env.patch(&id, json!({"effort": "max"})).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (400, Some("validation_failed"))
    );
    let none = fake_session(&env, "capabilities: { efforts: [] }\nturns: []", json!({})).await;
    let (status, body) = env.patch(&none, json!({"effort": "high"})).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (409, Some("capability_unsupported"))
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_017_ac4_switch_during_turn_waits_for_the_turn_end() {
    let env = Env::start(|_| Vec::new()).await;
    let id = fake_session(
        &env,
        "turns:\n  - emit: [{ message_delta: \"langsam und gründlich\", chunk: 2, chunk_delay_ms: 150 }]\n  - emit: [{ echo_settings: true }]\n",
        json!({}),
    )
    .await;
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "eins"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    env.wait_for(&id, |e| e["type"] == "turn.started").await;
    let (status, body) = env
        .patch(
            &id,
            json!({"model": "fake-large", "permission_mode": "plan"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        settings_changed(&env.events(&id).await).is_empty(),
        "noch nicht gewechselt, der Turn läuft"
    );
    let events = env
        .wait_for(&id, |e| e["type"] == "session.settings_changed")
        .await;
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    let completed = types.iter().position(|t| *t == "turn.completed").unwrap();
    let changed = types
        .iter()
        .position(|t| *t == "session.settings_changed")
        .unwrap();
    assert!(
        changed > completed,
        "Wechsel erst nach dem Turn-Ende: {types:?}"
    );
    let reply = env.turn(&id, "zwei").await;
    assert_eq!(reply[0], "model=fake-large effort=- mode=plan");
    let events = env.events(&id).await;
    let started = of_type(&events, "turn.started");
    assert_eq!(
        settings_changed(&events)[0]["payload"]["effective_from_turn"],
        started[1]["payload"]["turn_id"],
        "effective_from_turn = nächster Turn"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_027_ac1_yolo_without_sandbox_is_refused_everywhere() {
    let env = Env::start(|_| Vec::new()).await;
    for target in ["claude", "fake"] {
        let (status, body) = env
            .http(
                "POST",
                "/v1/sessions",
                Some(json!({"target": target, "cwd": env.work(), "permission_mode": "yolo"})),
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(body["code"], "sandbox_required");
    }
    // Keine Session angelegt (kein unsandboxed Start).
    let (_, list) = env.http("GET", "/v1/sessions?filter=all", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0, "{list}");
    // Wechsel auf yolo und Fork mit yolo ebenso.
    let id = fake_session(&env, ECHO, json!({})).await;
    let (status, body) = env.patch(&id, json!({"permission_mode": "yolo"})).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (409, Some("sandbox_required"))
    );
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/fork"),
            Some(json!({"permission_mode": "yolo", "workspace": "shared"})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (409, Some("sandbox_required"))
    );
    assert!(settings_changed(&env.events(&id).await).is_empty());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_027_ac3_mode_change_writes_settings_changed() {
    let env = Env::start(|_| Vec::new()).await;
    let id = fake_session(&env, ECHO, json!({"permission_mode": "accept_edits"})).await;
    assert_eq!(
        env.turn(&id, "eins").await[1],
        "model=fake-model effort=- mode=accept_edits"
    );
    let (status, body) = env.patch(&id, json!({"permission_mode": "plan"})).await;
    assert_eq!(status, 200, "{body}");
    let events = env
        .wait_for(&id, |e| e["type"] == "session.settings_changed")
        .await;
    let changed = settings_changed(&events);
    assert_eq!(changed[0]["payload"]["permission_mode"], "plan");
    assert_eq!(changed[0]["payload"]["mechanism"], "live");
    assert_eq!(
        env.turn(&id, "zwei").await[1],
        "model=fake-model effort=- mode=plan"
    );
    // Was der Harness nicht abbilden kann, wird abgelehnt statt ignoriert.
    let limited = fake_session(
        &env,
        "capabilities: { permission_modes: [default] }\nturns: []",
        json!({}),
    )
    .await;
    let (status, body) = env
        .patch(&limited, json!({"permission_mode": "plan"}))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (409, Some("capability_unsupported"))
    );
    let (status, body) = env
        .patch(&limited, json!({"permission_mode": "bypassPermissions"}))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (400, Some("validation_failed"))
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_020_ac1_killed_runner_resumes_idle_claude_session_with_context() {
    let env = claude_env(ECHO).await;
    let id = env
        .create(json!({"target": "claude", "cwd": env.work()}))
        .await;
    env.wait_status(&id, "idle").await;
    env.turn(&id, "eins").await;
    env.kill_runner(&id).await;
    // Die nächste Nachricht startet einen neuen Runner, der die native Session fortsetzt.
    let reply = env.turn(&id, "zwei").await;
    assert_eq!(reply[0], "eins", "Kontext erhalten");
    let events = env.events(&id).await;
    assert_eq!(one(&events, "session.resumed")["payload"]["mode"], "native");
    let started = of_type(&events, "session.started");
    assert_eq!(started.len(), 2);
    assert_eq!(
        started[0]["payload"]["harness_session_ref"],
        started[1]["payload"]["harness_session_ref"]
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_020_ac2_native_ref_is_logged_before_the_first_turn() {
    let env = claude_env(ECHO).await;
    let id = env
        .create(json!({"target": "claude", "cwd": env.work()}))
        .await;
    env.wait_status(&id, "idle").await;
    let before = env.events(&id).await;
    assert!(of_type(&before, "turn.started").is_empty());
    let reference = one(&before, "session.started")["payload"]["harness_session_ref"]
        .as_str()
        .expect("Referenz vor dem ersten Turn")
        .to_owned();
    env.turn(&id, "eins").await;
    let after = env.events(&id).await;
    assert_eq!(
        one(&after, "harness.ready")["payload"]["harness_session_ref"],
        reference.as_str()
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_020_ac3_active_turn_and_open_approval_are_closed_after_restart() {
    let env = Env::start(|_| Vec::new()).await;
    let id = fake_session(
        &env,
        "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"git push\" } }, gate: true }\n      - { tool_result: ok }\n  - emit: [{ message: weiter }]\n",
        json!({}),
    )
    .await;
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "push"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    env.wait_for(&id, |e| e["type"] == "approval.requested")
        .await;
    env.kill_runner(&id).await;
    // Die nächste Eingabe hängt nicht in der Queue des toten Turns: Der Server schließt ihn ab
    // und startet einen neuen Runner.
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "nochmal"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let events = env
        .wait_for(&id, |e| {
            e["type"] == "turn.started"
                && e["payload"]["turn_id"] != Value::Null
                && e["seq"].as_u64() > Some(0)
        })
        .await;
    let events = {
        let start = Instant::now();
        let mut events = events;
        while of_type(&events, "turn.started").len() < 2 {
            assert!(start.elapsed() < Duration::from_secs(30), "{events:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
            events = env.events(&id).await;
        }
        events
    };
    let approval = one(&events, "approval.resolved");
    assert_eq!(approval["payload"]["decision"], "abort");
    assert_eq!(approval["payload"]["via"], "system");
    let failed = one(&events, "turn.failed");
    assert_eq!(failed["payload"]["problem"]["code"], "runner_restarted");
    let started = of_type(&events, "turn.started");
    assert_eq!(
        failed["payload"]["turn_id"],
        started[0]["payload"]["turn_id"]
    );
    // Reihenfolge: Freigabe verworfen, Turn gescheitert, dann der neue Turn.
    let pos = |t: &str| events.iter().position(|e| e["type"] == t).unwrap();
    assert!(pos("approval.resolved") < pos("turn.failed"));
    assert!(started[1]["seq"].as_u64() > failed["seq"].as_u64());
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_006_110_fork_takes_effort_and_permission_mode() {
    let env = Env::start(|_| Vec::new()).await;
    let id = fake_session(&env, ECHO, json!({})).await;
    env.turn(&id, "eins").await;
    let forked = env
        .fork(
            &id,
            json!({"effort": "xhigh", "permission_mode": "plan", "workspace": "shared"}),
        )
        .await;
    let fork = forked["session"]["id"].as_str().unwrap().to_owned();
    env.wait_status(&fork, "idle").await;
    let events = env.events(&fork).await;
    let created = one(&events, "session.created");
    assert_eq!(created["payload"]["effort"], "xhigh");
    assert_eq!(created["payload"]["permission_mode"], "plan");
    let reply = env.turn(&fork, "zwei").await;
    assert_eq!(
        reply.last().unwrap(),
        "model=fake-model effort=high mode=plan"
    );
    // Gegen den Ziel-Harness geprüft.
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{id}/fork"),
            Some(json!({"effort": "ultra", "workspace": "shared"})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (400, Some("validation_failed"))
    );
    env.daemon.shutdown().await;
}

/// Agent im Projekt mit `executor` (AGT-004): Effort und Permission-Mode kommen aus ihm.
fn agent(env: &Env, name: &str, executor: &str) {
    write(
        &env.work().join(format!(".beton/agents/{name}/agent.yaml")),
        &format!(
            "spec_version: 1\nname: {name}\nexecutor: {executor}\ninstructions: {{ text: \"Arbeite sorgfältig.\", project_files: none }}\n"
        ),
    );
}

#[tokio::test]
async fn har_027_agent_executor_sets_effort_and_mode_but_never_yolo() {
    let env = Env::start(|_| Vec::new()).await;
    let scenario = write(&env.path("echo.yaml"), ECHO);
    agent(
        &env,
        "careful",
        "{ harness: fake, reasoning_effort: xhigh, permission_mode: plan }",
    );
    let id = env
        .create(
            json!({"agent": "careful", "cwd": env.work(), "harness_opts": {"scenario": scenario}}),
        )
        .await;
    env.wait_status(&id, "idle").await;
    let created = one(&env.events(&id).await, "session.created").clone();
    assert_eq!(created["payload"]["permission_mode"], "plan");
    assert_eq!(created["payload"]["effort"], "xhigh");
    assert_eq!(
        env.turn(&id, "eins").await.last().unwrap(),
        "model=fake-model effort=high mode=plan"
    );
    // Die Anfrage überschreibt den Agent.
    let id = env
        .create(json!({"agent": "careful", "cwd": env.work(), "permission_mode": "default", "harness_opts": {"scenario": scenario}}))
        .await;
    env.wait_status(&id, "idle").await;
    assert_eq!(
        env.turn(&id, "eins").await.last().unwrap(),
        "model=fake-model effort=high mode=default"
    );
    // Ein Agent (z. B. aus dem Repository) kann YOLO nicht erzwingen (fail closed).
    agent(&env, "reckless", "{ harness: fake, permission_mode: yolo }");
    let (status, body) = env
        .http(
            "POST",
            "/v1/sessions",
            Some(json!({"agent": "reckless", "cwd": env.work(), "harness_opts": {"scenario": scenario}})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (409, Some("sandbox_required")),
        "{body}"
    );
    env.daemon.shutdown().await;
}
