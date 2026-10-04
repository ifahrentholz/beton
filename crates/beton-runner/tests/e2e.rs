//! Runner gegen einen echten Daemon über den Tunnel-Socket (PROTO-015, RUN-002, RUN-003,
//! HAR-001 AC1). Der Runner ist das echte `beton-runner`-Binary mit dem Fake-Harness.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use beton_core::event::{EventPayload, SessionKind, SessionTrigger};
use beton_core::id::{OrgId, RunnerId, SessionId, UserId};
use beton_host::{
    LocalProvider, RunnerBoot, RunnerHandle, RunnerProvider, RunnerSpec, RunnerStatus,
};
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use serde_json::json;

struct Fixture {
    daemon: Daemon,
    store: Store,
    session: SessionRecord,
    dir: tempfile::TempDir,
    provider: LocalProvider,
}

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

async fn fixture(
    customize: impl FnOnce(beton_server::app::Runtime) -> beton_server::app::Runtime,
) -> Fixture {
    // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
    let dir = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let mut cfg = ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let daemon = start_with(cfg, store.clone(), customize).await.unwrap();
    let local = store.ensure_local().await.unwrap();
    let session = store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: dir.path().display().to_string(),
                model: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    let provider = LocalProvider::new(vec![runner_bin().into()], dir.path().join("runners"));
    Fixture {
        daemon,
        store,
        session,
        dir,
        provider,
    }
}

impl Fixture {
    fn tunnel(&self) -> PathBuf {
        self.dir.path().join("run/tunnel.sock")
    }

    fn scenario(&self, yaml: &str) -> PathBuf {
        let p = self.dir.path().join(format!("{}.yaml", RunnerId::new()));
        std::fs::write(&p, yaml).unwrap();
        p
    }

    async fn launch(&self, scenario: &Path, epoch: u64) -> RunnerHandle {
        let token = self
            .daemon
            .runtime
            .runners
            .mint_token(self.session.id)
            .unwrap();
        let spec = RunnerSpec {
            runner_id: RunnerId::new(),
            session_id: self.session.id,
            harness: "fake".into(),
            workspace: self.dir.path().to_path_buf(),
            env_allowlist: Vec::new(),
        };
        let p = self.provider.provision(&spec).await.unwrap();
        self.provider
            .start(
                &p,
                &RunnerBoot {
                    tunnel_socket: self.tunnel(),
                    token,
                    session_id: self.session.id,
                    epoch,
                    harness: "fake".into(),
                    scenario: Some(scenario.to_path_buf()),
                    model: None,
                    dev: true,
                    resume: None,
                    harnesses: Default::default(),
                    agent_ref: None,
                },
            )
            .await
            .unwrap()
    }

    async fn wait_connected(&self) {
        let start = Instant::now();
        while !self.daemon.runtime.runners.connected(self.session.id) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Runner verbindet sich nicht"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn deliver(&self, name: &str, args: serde_json::Value) -> serde_json::Value {
        self.daemon
            .runtime
            .runners
            .deliver(self.session.id, name, args, Duration::from_secs(10))
            .await
            .unwrap()
    }

    async fn events(&self) -> Vec<beton_core::event::Event> {
        self.store
            .events(OrgId::LOCAL, self.session.id, 0, 10_000)
            .await
            .unwrap()
    }

    async fn wait_for(
        &self,
        pred: impl Fn(&EventPayload) -> bool,
    ) -> Vec<beton_core::event::Event> {
        let start = Instant::now();
        loop {
            let events = self.events().await;
            if events.iter().any(|e| e.payload().is_some_and(&pred)) {
                return events;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Event kommt nicht: {:?}",
                events.iter().map(|e| e.type_name()).collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn gapless(events: &[beton_core::event::Event]) {
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    let mut ids: Vec<_> = events.iter().map(|e| e.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), events.len(), "jedes Event genau einmal");
}

const PUSH_ASK: &str = r#"
turns:
  - expect_input: "Bitte pushen"
    emit:
      - { message_delta: "Ich pushe jetzt.", chunk: 4 }
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "ok" }], deny: [{ message: "Push abgelehnt." }] } }
      - { usage: { input_tokens: 10, output_tokens: 5 } }
"#;

#[tokio::test]
async fn proto_015_runner_streams_turn_with_approval() {
    let f = fixture(|r| r).await;
    let h = f.launch(&f.scenario(PUSH_ASK), 1).await;
    f.wait_connected().await;
    let ack = f
        .deliver("input.submit", json!({"text": "Bitte pushen"}))
        .await;
    assert!(ack["turn_id"].as_str().unwrap().starts_with("trn_"));
    // Freigabe über das Gate des Runners.
    let start = Instant::now();
    loop {
        match f
            .daemon
            .runtime
            .runners
            .deliver(
                f.session.id,
                "approval.resolve",
                json!({"call_id": "call_1_1", "decision": "allow"}),
                Duration::from_secs(5),
            )
            .await
        {
            Ok(_) => break,
            Err(_) if start.elapsed() < Duration::from_secs(5) => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    let events = f
        .wait_for(|p| matches!(p, EventPayload::TurnCompleted(_)))
        .await;
    gapless(&events);
    let types: Vec<&str> = events.iter().map(|e| e.type_name()).collect();
    for t in [
        "session.created",
        "runner.status",
        "harness.ready",
        "turn.started",
        "message.completed",
        "tool.call.completed",
        "cost.delta",
        "turn.completed",
    ] {
        assert!(types.contains(&t), "{t} fehlt: {types:?}");
    }
    assert!(!types.contains(&"message.delta"), "Deltas sind transient");
    f.provider
        .terminate(&h, beton_host::TerminateMode::Force)
        .await
        .unwrap();
}

#[tokio::test]
async fn proto_015_ac1_events_survive_tunnel_outage_exactly_once_in_order() {
    let f = fixture(|r| r).await;
    let mut yaml = String::from("turns:\n  - emit:\n");
    for i in 0..40 {
        yaml.push_str(&format!(
            "      - {{ message: \"Zeile {i}\", delay_ms: 50 }}\n"
        ));
    }
    let h = f.launch(&f.scenario(&yaml), 1).await;
    f.wait_connected().await;
    f.deliver("input.submit", json!({"text": "los"})).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    // Tunnel trennen und eine Weile ablehnen (Spec: 30 s; hier verkürzt).
    f.daemon
        .runtime
        .runners
        .disconnect_for(f.session.id, Duration::from_secs(2));
    let events = f
        .wait_for(|p| matches!(p, EventPayload::TurnCompleted(_)))
        .await;
    gapless(&events);
    let lines: Vec<String> = events
        .iter()
        .filter_map(|e| match e.payload() {
            Some(EventPayload::MessageCompleted(m)) => {
                m.content[0]["text"].as_str().map(str::to_owned)
            }
            _ => None,
        })
        .collect();
    let expected: Vec<String> = (0..40).map(|i| format!("Zeile {i}")).collect();
    assert_eq!(lines, expected, "alle genau einmal, in Runner-Reihenfolge");
    f.provider
        .terminate(&h, beton_host::TerminateMode::Force)
        .await
        .unwrap();
}

#[tokio::test]
async fn proto_015_ac2_stale_epoch_stops_writing() {
    let f = fixture(|r| r).await;
    let h = f.launch(&f.scenario("turns: []"), 7).await;
    let start = Instant::now();
    loop {
        if let RunnerStatus::Exited { code, stderr_tail } = f.provider.status(&h).await.unwrap() {
            assert_ne!(code, Some(0));
            assert!(stderr_tail.contains("stale_epoch"), "{stderr_tail}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        f.events().await.len(),
        1,
        "nur session.created, nichts vom Runner"
    );
}

async fn assert_no_listening_port() {
    let f = fixture(|r| r).await;
    let h = f.launch(&f.scenario("turns: []"), 1).await;
    f.wait_connected().await;
    let pid = h.pid.unwrap().to_string();
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN"])
        .output();
    match out {
        Ok(out) => assert!(
            String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "Runner lauscht: {}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Err(_) => eprintln!("lsof fehlt; Prüfung übersprungen"),
    }
    f.provider
        .terminate(&h, beton_host::TerminateMode::Force)
        .await
        .unwrap();
}

#[tokio::test]
async fn proto_015_ac3_host_side_runner_has_no_listener() {
    assert_no_listening_port().await;
}

#[tokio::test]
async fn run_002_ac3_runner_opens_no_tcp_listen_port() {
    assert_no_listening_port().await;
}

#[tokio::test]
async fn run_003_ac3_har_001_ac1_harness_crash_fails_runner_and_session() {
    let f = fixture(|r| r).await;
    let h = f
        .launch(
            &f.scenario("turns:\n  - emit: [{ message: Hallo }, { crash: 137 }]\n"),
            1,
        )
        .await;
    f.wait_connected().await;
    f.deliver("input.submit", json!({"text": "los"})).await;
    let events = f
        .wait_for(|p| matches!(p, EventPayload::SessionStatus(s) if s.status == beton_core::event::SessionStatus::Failed))
        .await;
    let exited = events
        .iter()
        .find_map(|e| match e.payload() {
            Some(EventPayload::HarnessExited(x)) => Some(x.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(exited.code, Some(137));
    let failed = events
        .iter()
        .find_map(|e| match e.payload() {
            Some(EventPayload::RunnerStatus(s)) if s.to == "failed" => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    assert!(failed.reason.unwrap().contains("137"));
    assert_eq!(
        f.store
            .session(OrgId::LOCAL, f.session.id)
            .await
            .unwrap()
            .status,
        beton_core::event::SessionStatus::Failed
    );
    let _ = f
        .provider
        .terminate(&h, beton_host::TerminateMode::Force)
        .await;
}

#[tokio::test]
async fn run_003_ac2_idle_runner_is_stopped_and_session_marked_stopped() {
    let f = fixture(|mut r| {
        r.tunnel.idle_timeout = Duration::from_millis(500);
        r.tunnel.idle_check = Duration::from_millis(100);
        r
    })
    .await;
    let h = f.launch(&f.scenario("turns: []"), 1).await;
    f.wait_connected().await;
    let events = f
        .wait_for(|p| matches!(p, EventPayload::SessionStatus(s) if s.status == beton_core::event::SessionStatus::Stopped))
        .await;
    assert!(events.iter().any(
        |e| matches!(e.payload(), Some(EventPayload::RunnerStatus(s)) if s.to == "terminated")
    ));
    let start = Instant::now();
    while !matches!(
        f.provider.status(&h).await.unwrap(),
        RunnerStatus::Exited { .. }
    ) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Runner beendet sich nicht"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn runner_token_only_opens_its_own_session() {
    let f = fixture(|r| r).await;
    let other = SessionId::new();
    let token = f.daemon.runtime.runners.mint_token(other).unwrap();
    let spec = RunnerSpec {
        runner_id: RunnerId::new(),
        session_id: f.session.id,
        harness: "fake".into(),
        workspace: f.dir.path().to_path_buf(),
        env_allowlist: Vec::new(),
    };
    let p = f.provider.provision(&spec).await.unwrap();
    let h = f
        .provider
        .start(
            &p,
            &RunnerBoot {
                tunnel_socket: f.tunnel(),
                token,
                session_id: f.session.id,
                epoch: 1,
                harness: "fake".into(),
                scenario: Some(f.scenario("turns: []")),
                model: None,
                dev: true,
                resume: None,
                harnesses: Default::default(),
                agent_ref: None,
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !f.daemon.runtime.runners.connected(f.session.id),
        "fremdes Token bindet nicht"
    );
    f.provider
        .terminate(&h, beton_host::TerminateMode::Force)
        .await
        .unwrap();
}

#[test]
fn run_002_ac2_runner_exits_when_daemon_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("s.yaml");
    std::fs::write(&scenario, "turns: []").unwrap();
    // Ein "Daemon" (sh), der den Runner startet; der Runner kennt dessen PID.
    let script = format!(
        "printf 'bt_run_x\\n' | env BETON_RUNNER_PARENT_PID=$$ {bin} & echo $!; wait",
        bin = runner_bin()
    );
    let mut daemon = std::process::Command::new("sh")
        .args(["-c", &script])
        .env("BETON_TUNNEL_SOCKET", dir.path().join("fehlt.sock"))
        .env("BETON_SESSION_ID", SessionId::new().to_string())
        .env("BETON_RUNNER_ID", RunnerId::new().to_string())
        .env("BETON_EPOCH", "1")
        .env("BETON_HARNESS", "fake")
        .env("BETON_SCENARIO", &scenario)
        .env("BETON_DEV", "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(daemon.stdout.as_mut().unwrap()),
        &mut line,
    )
    .unwrap();
    let runner_pid = line.trim().to_owned();
    std::thread::sleep(Duration::from_millis(300));
    daemon.kill().unwrap();
    let _ = daemon.wait();
    let start = Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &runner_pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "Runner lebt nach 5 s noch"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
