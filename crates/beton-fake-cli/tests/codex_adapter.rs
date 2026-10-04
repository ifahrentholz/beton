//! Der echte Codex-Adapter gegen die Fake-CLI (`--protocol app-server`) über eine echte
//! Prozessgrenze (QA-002, HAR-006, HAR-015, HAR-016, HAR-021).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{AuthSource, CostSource, EventPayload, ToolStatus};
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, AllowAll, AuthStatus, Gate, GateDecision, GateRequest, HarnessAdapter,
    HarnessError, HarnessSession, HostEnv, NormalizedEvent, SessionSpec, Shutdown,
};
use beton_harness_codex::{CodexAdapter, VERSION_RANGE};
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

fn push_ask() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake/push-ask.yaml")
}

fn env_for(scenario: &Path, extra: &str) -> HostEnv {
    let mut env = HostEnv::default();
    env.vars.insert(
        "BETON_CODEX_PATH".into(),
        format!(
            "{} --protocol app-server --scenario {} {extra}",
            fake_cli(),
            scenario.display()
        ),
    );
    env
}

async fn try_start(
    scenario: &Path,
    extra: &str,
    gate: Arc<dyn Gate>,
) -> Result<Box<dyn HarnessSession>, HarnessError> {
    let ctx = AdapterContext {
        gate,
        launcher: Arc::new(RealLauncher),
        env: env_for(scenario, extra),
    };
    CodexAdapter::default()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
}

async fn start(
    scenario: &Path,
    extra: &str,
    gate: Arc<dyn Gate>,
) -> (Box<dyn HarnessSession>, mpsc::Receiver<NormalizedEvent>) {
    let mut s = try_start(scenario, extra, gate).await.unwrap();
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

#[derive(Default)]
struct Recording {
    allow: bool,
    requests: Mutex<Vec<GateRequest>>,
}

#[async_trait]
impl Gate for Recording {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        self.requests.lock().unwrap().push(request);
        if self.allow {
            GateDecision::Allow { updated_args: None }
        } else {
            GateDecision::Deny {
                reason: Some("nicht erlaubt".into()),
            }
        }
    }
}

#[tokio::test]
async fn har_006_ac1_streams_answers_reasoning_and_tool_calls() {
    let (_dir, path) = scenario(
        r#"
turns:
  - emit:
      - { reasoning: "Ich prüfe das Verzeichnis." }
      - { message_delta: "Ich schaue nach.", chunk: 5 }
      - { tool_call: { name: shell, kind: shell, args: { command: "ls" } }, gate: true }
      - { tool_result: "README.md" }
      - { message: "Es gibt eine README." }
      - { usage: { input_tokens: 900, cache_read_tokens: 100, output_tokens: 40 } }
"#,
    );
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll)).await;
    let ready = rx.recv().await.unwrap();
    assert_eq!(ready.payload.type_name(), "harness.ready");
    assert!(
        s.native_session_ref().is_some(),
        "Thread-ID als native Referenz"
    );
    s.send("Was liegt hier?".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    for t in [
        "turn.started",
        "reasoning.delta",
        "reasoning.completed",
        "message.delta",
        "message.completed",
        "tool.call.requested",
        "approval.requested",
        "approval.resolved",
        "tool.call.started",
        "tool.call.output.delta",
        "tool.call.completed",
        "context.usage",
        "cost.delta",
        "turn.completed",
    ] {
        assert!(count(&events, t) >= 1, "{t} fehlt: {:?}", names(&events));
    }
    let deltas: String = events
        .iter()
        .filter_map(|e| match e {
            EventPayload::MessageDelta(d) => Some(d.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, "Ich schaue nach.");
    let completed = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(completed.status, ToolStatus::Ok);
    assert_eq!(count(&events, "message.completed"), 2);
    s.shutdown(Shutdown::default()).await.unwrap();
}

#[tokio::test]
async fn har_006_ac2_denied_git_push_is_not_executed() {
    let gate = Arc::new(Recording::default());
    let (mut s, mut rx) = start(&push_ask(), "", gate.clone()).await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let requests = gate.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "genau eine Gate-Anfrage");
    assert_eq!(requests[0].kind, "shell");
    assert_eq!(requests[0].args["command"], "git push origin main");
    let completed = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(completed.status, ToolStatus::Denied);
    assert_eq!(count(&events, "tool.call.started"), 0, "nicht ausgeführt");
    assert_eq!(count(&events, "tool.call.output.delta"), 0);
    let messages: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            EventPayload::MessageCompleted(m) => m.content[0]["text"].as_str().map(str::to_owned),
            _ => None,
        })
        .collect();
    assert!(
        messages.contains(&"Push abgelehnt.".to_owned()),
        "{messages:?}"
    );
    assert_eq!(names(&events).last(), Some(&"turn.completed"));
}

#[tokio::test]
async fn har_006_ac3_token_usage_gives_context_usage_and_cost_delta() {
    let (mut s, mut rx) = start(&push_ask(), "", Arc::new(AllowAll)).await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let usage = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ContextUsage(u) => Some(u),
            _ => None,
        })
        .unwrap();
    assert_eq!((usage.used_tokens, usage.window_tokens), (1280, 272_000));
    assert_eq!(
        count(&events, "cost.delta"),
        1,
        "genau ein cost.delta je Turn"
    );
    let cost = events
        .iter()
        .find_map(|e| match e {
            EventPayload::CostDelta(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!((cost.input_tokens, cost.output_tokens), (1200, 80));
    // HAR-015 AC5: ohne Auth-Angabe und ohne API-Key über den Login der CLI.
    assert_eq!(cost.auth_source, AuthSource::VendorCli);
    assert_eq!(cost.source, CostSource::Subscription);
    assert_eq!(
        cost.cost_micro, None,
        "Kosten erst mit dem Preis-Katalog (USE-002)"
    );
}

#[tokio::test]
async fn har_006_ac4_handshake_schema_mismatch_is_incompatible() {
    let (_dir, path) = scenario("faults: { bad_handshake: true }\nturns: []\n");
    match try_start(&path, "", Arc::new(AllowAll)).await {
        Err(HarnessError::Incompatible { detected, expected }) => {
            assert_eq!(detected, "unbekannt");
            assert_eq!(expected, VERSION_RANGE);
        }
        Err(e) => panic!("erwartet harness.incompatible, erhalten {e}"),
        Ok(_) => panic!("Start hätte scheitern müssen"),
    }
}

#[tokio::test]
async fn har_006_ac4_protocol_version_outside_range_is_incompatible() {
    let (_dir, path) = scenario("version: \"0.99.0\"\nturns: []\n");
    match try_start(&path, "", Arc::new(AllowAll)).await {
        Err(e @ HarnessError::Incompatible { .. }) => {
            assert_eq!(e.code(), "harness_incompatible");
            let HarnessError::Incompatible { detected, expected } = e else {
                unreachable!()
            };
            assert_eq!(
                (detected.as_str(), expected.as_str()),
                ("0.99.0", VERSION_RANGE)
            );
        }
        Err(e) => panic!("erwartet harness.incompatible, erhalten {e}"),
        Ok(_) => panic!("Start hätte scheitern müssen"),
    }
}

#[tokio::test]
async fn har_006_interrupt_ends_the_turn_and_accepts_new_input() {
    let (_dir, path) = scenario(
        "turns:\n  - emit: [{ message_delta: \"Ich denke\", chunk: 3 }, { hang: true }]\n  - emit: [{ message: \"Weiter geht's\" }]\n",
    );
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll)).await;
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
async fn har_006_resume_continues_the_thread() {
    let (mut s, mut rx) = start(&push_ask(), "", Arc::new(AllowAll)).await;
    let thread = s.native_session_ref().unwrap();
    s.send("Bitte pushen".into()).await.unwrap();
    until_turn_end(&mut rx).await;
    s.shutdown(Shutdown::default()).await.unwrap();
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env: env_for(&push_ask(), ""),
    };
    let resumed = CodexAdapter::default()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                resume: Some(thread.clone()),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    assert_eq!(resumed.native_session_ref(), Some(thread));
}

#[tokio::test]
async fn qa_002_ac3_crash_gives_harness_exited_and_failed_turn() {
    let (_dir, path) = scenario("turns:\n  - emit: [{ message: \"kurz\" }, { crash: 3 }]\n");
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll)).await;
    s.send("los".into()).await.unwrap();
    let mut events = until_turn_end(&mut rx).await;
    events.extend(until_turn_end(&mut rx).await);
    assert!(count(&events, "turn.failed") == 1, "{:?}", names(&events));
    let exited = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessExited(x) => Some(x),
            _ => None,
        })
        .unwrap();
    assert_eq!(exited.code, Some(3));
    assert!(exited.stderr_tail.contains("Absturz"));
}

#[tokio::test]
async fn qa_002_ac3_malformed_line_becomes_unmapped() {
    // Zeile 1 ist die initialize-Antwort; Zeile 6 liegt im ersten Turn.
    let (mut s, mut rx) = start(&push_ask(), "--malformed-line 6", Arc::new(AllowAll)).await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert!(
        count(&events, "harness.unmapped") >= 1,
        "{:?}",
        names(&events)
    );
    assert_eq!(names(&events).last(), Some(&"turn.completed"));
}

#[tokio::test]
async fn har_015_expired_login_asks_for_codex_login() {
    let (_dir, path) = scenario("turns:\n  - emit: [{ auth_expired: \"Sitzung abgelaufen\" }]\n");
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll)).await;
    s.send("hallo".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let auth = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessAuthRequired(a) => Some(a),
            _ => None,
        })
        .unwrap();
    assert_eq!(auth.hint, "codex login");
    assert_eq!(names(&events).last(), Some(&"turn.failed"));
}

#[tokio::test]
async fn har_016_login_status_comes_from_the_cli() {
    let mut env = HostEnv::default();
    env.vars
        .insert("BETON_CODEX_PATH".into(), fake_cli().into());
    assert_eq!(
        CodexAdapter::default().auth_status(&env).await,
        AuthStatus::LoggedIn
    );
}

#[tokio::test]
async fn ses_004_ac3_codex_steer_is_processed_in_the_running_turn() {
    let (_d, path) = scenario(
        "turns:\n  - emit:\n      - { message: \"Ich fange an.\" }\n      - { await_steer: \"Bitte auch die README\" }\n      - { message: \"README ergänze ich mit.\" }\n",
    );
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll)).await;
    assert!(beton_harness_codex::capabilities().steering);
    s.send("Los".into()).await.unwrap();
    s.steer("Bitte auch die README".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(count(&events, "turn.started"), 1, "{:?}", names(&events));
    assert_eq!(count(&events, "turn.interrupted"), 0);
    assert_eq!(names(&events).last(), Some(&"turn.completed"));
    // Die Antwort nach dem Steer gehört zum selben Turn.
    assert!(events.iter().any(|e| matches!(e,
        EventPayload::MessageCompleted(m) if m.content[0]["text"] == "README ergänze ich mit.")));
    // Ohne laufenden Turn lehnt der Adapter ab.
    assert!(s.steer("zu spät".into()).await.is_err());
    s.shutdown(Shutdown::Kill).await.unwrap();
}
