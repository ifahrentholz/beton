//! Der echte ACP-Adapter gegen den ACP-Test-Agent der Fake-CLI (`--protocol acp`) über eine
//! echte Prozessgrenze (QA-002, HAR-007, HAR-008).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_core::event::{ApprovalDecision, EventPayload, ToolStatus};
use beton_harness::process::RealLauncher;
use beton_harness::registry::AcpAgentConfig;
use beton_harness::{
    AdapterContext, AllowAll, DenyAll, ForkHistory, Gate, HarnessAdapter, HarnessSession, HostEnv,
    Mode, NormalizedEvent, ResumeSupport, SessionSpec, Shutdown,
};
use beton_harness_acp::{AcpAdapter, AcpAgent, Origin};
use tokio::sync::mpsc;

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

fn scenario(yaml: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("szenario.yaml");
    std::fs::write(&path, yaml).unwrap();
    (dir, path)
}

fn adapter() -> AcpAdapter {
    AcpAdapter::new(AcpAgent {
        slug: "test".into(),
        config: AcpAgentConfig {
            command: "acp-test-agent".into(),
            ..AcpAgentConfig::default()
        },
        origin: Origin::User,
    })
    .unwrap()
}

fn env_for(scenario: &Path) -> HostEnv {
    let mut env = HostEnv::default();
    env.vars.insert(
        "BETON_ACP_TEST_PATH".into(),
        format!(
            "{} --protocol acp --scenario {}",
            fake_cli(),
            scenario.display()
        ),
    );
    env
}

async fn start(
    scenario: &Path,
    gate: Arc<dyn Gate>,
) -> (Box<dyn HarnessSession>, mpsc::Receiver<NormalizedEvent>) {
    let ctx = AdapterContext {
        gate,
        launcher: Arc::new(RealLauncher),
        env: env_for(scenario),
    };
    let mut s = adapter()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    let rx = s.events().unwrap();
    (s, rx)
}

async fn until_turn_end(rx: &mut mpsc::Receiver<NormalizedEvent>) -> Vec<EventPayload> {
    let mut out = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::TurnInterrupted(_)
                | EventPayload::HarnessExited(_)
        );
        out.push(e.payload);
        if end {
            break;
        }
    }
    out
}

fn names(events: &[EventPayload]) -> Vec<&'static str> {
    events.iter().map(EventPayload::type_name).collect()
}

fn count(events: &[EventPayload], name: &str) -> usize {
    events.iter().filter(|e| e.type_name() == name).count()
}

const TOOL: &str = r#"
turns:
  - emit:
      - { reasoning: "Erst nachsehen." }
      - { message_delta: "Ich führe ls aus.", chunk: 4 }
      - { tool_call: { name: "ls", kind: shell, args: { command: "ls" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "README.md" }, { message: "Fertig." }], deny: [{ message: "Abgelehnt." }] } }
"#;

#[tokio::test]
async fn har_007_ac1_prompt_streaming_and_permission_roundtrip() {
    let (_dir, path) = scenario(TOOL);
    let (mut s, mut rx) = start(&path, Arc::new(AllowAll)).await;
    assert_eq!(
        rx.recv().await.unwrap().payload.type_name(),
        "harness.ready"
    );
    s.send("Was liegt hier?".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(
        names(&events),
        [
            "turn.started",
            "reasoning.delta",
            "reasoning.completed",
            "message.delta",
            "message.delta",
            "message.delta",
            "message.delta",
            "message.delta",
            "message.completed",
            "tool.call.requested",
            "approval.requested",
            "approval.resolved",
            "tool.call.started",
            "tool.call.completed",
            "message.delta",
            "message.completed",
            "turn.completed",
        ]
    );
    let EventPayload::ToolCallRequested(call) = &events[9] else {
        panic!()
    };
    assert_eq!(call.args["command"], "ls");
    let done = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(done.status, ToolStatus::Ok);
    s.shutdown(Shutdown::default()).await.unwrap();
}

#[tokio::test]
async fn har_007_ac1_denied_permission_is_not_executed() {
    let (_dir, path) = scenario(TOOL);
    let (mut s, mut rx) = start(&path, Arc::new(DenyAll)).await;
    s.send("Was liegt hier?".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let resolved = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ApprovalResolved(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(resolved.decision, ApprovalDecision::Deny);
    assert_eq!(count(&events, "tool.call.started"), 0);
    let done = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(done.status, ToolStatus::Denied);
    assert_eq!(names(&events).last(), Some(&"turn.completed"));
}

#[tokio::test]
async fn har_007_ac1_cancel_interrupts_the_turn() {
    let (_dir, path) = scenario(
        "turns:\n  - emit: [{ message_delta: \"Ich denke\", chunk: 3 }, { hang: true }]\n  - emit: [{ message: \"Weiter.\" }]\n",
    );
    let (mut s, mut rx) = start(&path, Arc::new(AllowAll)).await;
    s.send("erst".into()).await.unwrap();
    while let Some(e) = rx.recv().await {
        if matches!(e.payload, EventPayload::MessageDelta(_)) {
            break;
        }
    }
    s.interrupt().await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(names(&events).last(), Some(&"turn.interrupted"));
    s.send("dann".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(names(&events).last(), Some(&"turn.completed"));
}

#[tokio::test]
async fn har_007_ac1_cancel_during_permission_request_answers_cancelled() {
    let (_dir, path) = scenario(TOOL);
    struct Never;
    #[async_trait::async_trait]
    impl Gate for Never {
        async fn decide(&self, _r: beton_harness::GateRequest) -> beton_harness::GateDecision {
            std::future::pending().await
        }
    }
    let (mut s, mut rx) = start(&path, Arc::new(Never)).await;
    s.send("los".into()).await.unwrap();
    while let Some(e) = rx.recv().await {
        if matches!(e.payload, EventPayload::ApprovalRequested(_)) {
            break;
        }
    }
    s.interrupt().await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(count(&events, "tool.call.started"), 0);
    assert_eq!(names(&events).last(), Some(&"turn.interrupted"));
}

#[tokio::test]
async fn har_007_ac3_without_session_load_resume_is_cold_and_fork_uses_preamble() {
    let adapter = adapter();
    let (_dir, cold) = scenario("capabilities: { resume: none }\nturns: []\n");
    let probe = adapter.probe(&env_for(&cold)).await;
    assert!(probe.installed, "{probe:?}");
    assert_eq!(probe.version.as_deref(), Some("0.1.0"), "aus agentInfo");
    let caps = adapter.capabilities(Mode::Native, &probe);
    assert_eq!(caps.resume, ResumeSupport::Cold);
    assert_eq!(caps.fork_history, ForkHistory::Preamble);

    let (_dir2, warm) = scenario("turns: []\n");
    let probe = adapter.probe(&env_for(&warm)).await;
    assert_eq!(
        adapter.capabilities(Mode::Native, &probe).resume,
        ResumeSupport::Warm
    );
}

#[tokio::test]
async fn har_007_session_load_continues_the_session() {
    let (_dir, path) = scenario("turns:\n  - emit: [{ message: \"Hallo.\" }]\n");
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env: env_for(&path),
    };
    let s = adapter()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                resume: Some("sess_alt".into()),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    assert_eq!(s.native_session_ref().as_deref(), Some("sess_alt"));
}

#[tokio::test]
async fn har_007_ac4_agent_exit_during_turn_gives_exited_and_failed() {
    let (_dir, path) =
        scenario("turns:\n  - emit: [{ message_delta: \"Gleich\", chunk: 2 }, { crash: 4 }]\n");
    let (mut s, mut rx) = start(&path, Arc::new(AllowAll)).await;
    s.send("los".into()).await.unwrap();
    let mut events = until_turn_end(&mut rx).await;
    events.extend(until_turn_end(&mut rx).await);
    assert_eq!(count(&events, "turn.failed"), 1, "{:?}", names(&events));
    let exited = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessExited(x) => Some(x),
            _ => None,
        })
        .unwrap();
    assert_eq!(exited.code, Some(4));
    let failed_at = names(&events)
        .iter()
        .position(|n| *n == "turn.failed")
        .unwrap();
    let exited_at = names(&events)
        .iter()
        .position(|n| *n == "harness.exited")
        .unwrap();
    assert!(failed_at < exited_at);
}

#[tokio::test]
async fn har_007_auth_error_asks_for_agent_login() {
    let (_dir, path) = scenario("turns:\n  - emit: [{ auth_expired: \"bitte anmelden\" }]\n");
    let (mut s, mut rx) = start(&path, Arc::new(AllowAll)).await;
    s.send("hallo".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let auth = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessAuthRequired(a) => Some(a),
            _ => None,
        })
        .unwrap();
    assert_eq!(auth.harness, "acp:test");
    assert!(auth.hint.contains("acp-test-agent"), "{}", auth.hint);
    assert_eq!(names(&events).last(), Some(&"turn.failed"));
}

#[tokio::test]
async fn har_007_wrong_protocol_version_is_incompatible() {
    let (_dir, path) = scenario("faults: { bad_handshake: true }\nturns: []\n");
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env: env_for(&path),
    };
    let err = adapter()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), "harness_incompatible", "{err}");
}

/// Kindprozess für HAR-008 AC4: startet das Preset `gemini` mit einem Skript als `gemini`,
/// das seine Umgebung ausgibt und dann den ACP-Test-Agent ausführt.
#[cfg(unix)]
#[tokio::test]
#[ignore = "wird von har_008_ac4_gemini_preset_starts_without_api_key gestartet"]
async fn gemini_env_child() {
    use std::os::unix::fs::PermissionsExt;

    use beton_harness::registry::{HarnessLayers, Registry, RegistryOptions};
    use beton_harness::{HarnessId, UserInput};

    let Ok(out) = std::env::var("BETON_TEST_ENV_OUT") else {
        return;
    };
    let dir = PathBuf::from(&out).parent().unwrap().to_path_buf();
    let scenario = dir.join("szenario.yaml");
    std::fs::write(
        &scenario,
        "turns:\n  - emit: [{ message: \"Hallo von Gemini.\" }]\n",
    )
    .unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("gemini");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nenv > '{out}'\nexec '{}' --protocol acp --scenario '{}' \"$@\"\n",
            fake_cli(),
            scenario.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut registry = Registry::new(RegistryOptions::default());
    assert!(beton_harness_acp::register(&mut registry, &HarnessLayers::default()).is_empty());
    let env = HostEnv {
        path: Some(bin.into_os_string()),
        ..HostEnv::default()
    };
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let id: HarnessId = "acp:gemini".parse().unwrap();
    let mut s = registry
        .start(
            &id,
            SessionSpec {
                workdir: dir.clone(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("Hallo")).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(
        names(&events).last(),
        Some(&"turn.completed"),
        "{:?}",
        names(&events)
    );
    s.shutdown(Shutdown::default()).await.unwrap();
}

#[cfg(unix)]
#[test]
fn har_008_ac4_gemini_preset_starts_without_api_key() {
    for keys in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("env.txt");
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["gemini_env_child", "--exact", "--ignored", "--nocapture"])
            .env("BETON_TEST_ENV_OUT", &out)
            .env("BETON_TEST_MARKER", "geerbt");
        // Leere Umgebung: kein Key gesetzt; zweiter Lauf: Keys gesetzt, dürfen nicht ankommen.
        for (k, _) in std::env::vars() {
            if k.ends_with("_API_KEY") {
                cmd.env_remove(k);
            }
        }
        if keys {
            cmd.env("GEMINI_API_KEY", "test-gemini-darf-nicht-ankommen")
                .env("OPENAI_API_KEY", "test-openai-darf-nicht-ankommen");
        }
        let status = cmd.status().unwrap();
        assert!(
            status.success(),
            "Start ohne API-Key (Keys gesetzt: {keys})"
        );
        let env = std::fs::read_to_string(&out).unwrap();
        assert!(
            env.contains("BETON_TEST_MARKER=geerbt"),
            "Umgebung wird sonst geerbt"
        );
        assert!(!env.contains("_API_KEY="), "{env}");
    }
}
