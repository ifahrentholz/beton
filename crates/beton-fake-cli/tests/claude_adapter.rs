//! Der echte Claude-Adapter gegen die Fake-CLI über eine echte Prozessgrenze (QA-002,
//! HAR-004, HAR-005, HAR-015, HAR-021, HAR-001).

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use beton_core::event::{AuthSource, EventPayload, ToolStatus};
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, AllowAll, Gate, GateDecision, GateRequest, HarnessAdapter, HarnessSession,
    HostEnv, NormalizedEvent, SessionSpec, Shutdown,
};
use beton_harness_claude::ClaudeAdapter;
use serde_json::json;
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

/// Startet den Claude-Adapter mit `BETON_CLAUDE_PATH=beton-fake-cli … <args>` (QA-002 AC1).
async fn start(
    scenario: &std::path::Path,
    extra: &str,
    gate: Arc<dyn Gate>,
    adapter: ClaudeAdapter,
) -> (Box<dyn HarnessSession>, mpsc::Receiver<NormalizedEvent>) {
    let mut env = HostEnv::default();
    env.vars.insert(
        "BETON_CLAUDE_PATH".into(),
        format!(
            "{} --protocol stream-json --scenario {} {extra}",
            fake_cli(),
            scenario.display()
        ),
    );
    let ctx = AdapterContext {
        gate,
        launcher: Arc::new(RealLauncher),
        env,
    };
    let mut session = adapter
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    let rx = session.events().unwrap();
    (session, rx)
}

/// Events bis zum Turn-Ende (oder Prozessende).
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

fn count(events: &[EventPayload], name: &str) -> usize {
    events.iter().filter(|e| e.type_name() == name).count()
}

#[tokio::test]
async fn qa_002_ac1_claude_adapter_runs_tool_approval_scenario() {
    let (mut s, mut rx) = start(
        &push_ask(),
        "",
        Arc::new(AllowAll),
        ClaudeAdapter::default(),
    )
    .await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    for t in [
        "harness.ready",
        "message.delta",
        "message.completed",
        "tool.call.requested",
        "approval.requested",
        "approval.resolved",
        "tool.call.started",
        "tool.call.completed",
        "cost.delta",
        "turn.completed",
    ] {
        assert!(
            count(&events, t) >= 1,
            "{t} fehlt: {:?}",
            events
                .iter()
                .map(EventPayload::type_name)
                .collect::<Vec<_>>()
        );
    }
    // HAR-021 AC1/AC2, HAR-015 AC5: genau ein cost.delta, Subscription ohne Betrag.
    assert_eq!(count(&events, "cost.delta"), 1);
    let cost = events
        .iter()
        .find_map(|e| match e {
            EventPayload::CostDelta(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(cost.auth_source, AuthSource::VendorCli);
    assert_eq!(cost.cost_micro, None);
    assert_eq!(cost.input_tokens, 1200);
    s.shutdown(Shutdown::default()).await.unwrap();
}

/// Gate, das Argumente ändert und Zeitpunkte notiert.
#[derive(Default)]
struct Modifying {
    calls: Mutex<Vec<Instant>>,
}

#[async_trait]
impl Gate for Modifying {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        self.calls.lock().unwrap().push(Instant::now());
        let mut args = request.args.clone();
        args["command"] = json!("git push --dry-run origin main");
        GateDecision::Allow {
            updated_args: Some(args),
        }
    }
}

#[tokio::test]
async fn har_005_ac2_modified_args_are_visible_on_started() {
    let gate = Arc::new(Modifying::default());
    let (mut s, mut rx) = start(&push_ask(), "", gate.clone(), ClaudeAdapter::default()).await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let requested = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallRequested(r) => Some(r),
            _ => None,
        })
        .unwrap();
    let started = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallStarted(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(requested.args["command"], "git push origin main");
    assert_eq!(
        started.args.as_ref().unwrap()["command"],
        "git push --dry-run origin main"
    );
}

#[tokio::test]
async fn har_005_ac3_one_approval_per_request_and_fast_bridge() {
    let mut yaml = String::from("turns:\n  - emit:\n");
    for i in 0..60 {
        yaml.push_str(&format!(
            "      - {{ tool_call: {{ name: Bash, kind: shell, args: {{ command: \"echo {i}\" }} }}, gate: true }}\n      - {{ tool_result: ok }}\n"
        ));
    }
    let (_dir, path) = scenario(&yaml);
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll), ClaudeAdapter::default()).await;
    s.send("los".into()).await.unwrap();
    let mut requested_at = Vec::new();
    let mut latencies = Vec::new();
    let mut approvals = 0;
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        match e.payload {
            EventPayload::ApprovalRequested(_) => {
                approvals += 1;
                requested_at.push(Instant::now());
            }
            EventPayload::ApprovalResolved(_) => {
                latencies.push(requested_at.last().unwrap().elapsed());
            }
            EventPayload::TurnCompleted(_) => break,
            _ => {}
        }
    }
    assert_eq!(approvals, 60, "genau ein approval.requested je Anfrage");
    latencies.sort();
    let p99 = latencies[(latencies.len() * 99 / 100).min(latencies.len() - 1)];
    assert!(p99 < Duration::from_millis(20), "p99 {p99:?}");
}

/// Gate, das nie antwortet.
struct Silent;

#[async_trait]
impl Gate for Silent {
    async fn decide(&self, _request: GateRequest) -> GateDecision {
        std::future::pending().await
    }
}

#[tokio::test]
async fn har_005_ac4_unanswered_gate_denies() {
    let adapter = ClaudeAdapter {
        gate_timeout: Duration::from_millis(100),
        ..ClaudeAdapter::default()
    };
    let (mut s, mut rx) = start(&push_ask(), "", Arc::new(Silent), adapter).await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let resolved = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ApprovalResolved(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(resolved.decision, beton_core::event::ApprovalDecision::Deny);
    assert_eq!(resolved.via, beton_core::event::ResolvedVia::Timeout);
    let completed = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(completed.status, ToolStatus::Denied);
    assert_eq!(count(&events, "tool.call.started"), 0);
}

#[tokio::test]
async fn har_004_ac3_interrupt_then_new_input() {
    let (_dir, path) = scenario(
        "turns:\n  - emit: [{ message_delta: \"Ich denke\" }, { hang: true }]\n  - emit: [{ message: \"Weiter geht's\" }]\n",
    );
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll), ClaudeAdapter::default()).await;
    s.send("erst".into()).await.unwrap();
    // Warten, bis der Turn läuft.
    while let Some(e) = rx.recv().await {
        if matches!(e.payload, EventPayload::MessageCompleted(_)) {
            break;
        }
    }
    s.interrupt().await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(events.last().unwrap().type_name(), "turn.interrupted");
    s.send("dann".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert_eq!(events.last().unwrap().type_name(), "turn.completed");
    assert!(count(&events, "message.completed") >= 1);
}

#[tokio::test]
async fn qa_002_ac3_crash_gives_harness_exited_with_stderr() {
    let (mut s, mut rx) = start(
        &push_ask(),
        "--crash-after 3",
        Arc::new(AllowAll),
        ClaudeAdapter::default(),
    )
    .await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let exited = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessExited(x) => Some(x),
            _ => None,
        })
        .unwrap();
    assert_eq!(exited.code, Some(1));
    assert!(
        exited.stderr_tail.contains("Absturz"),
        "{:?}",
        exited.stderr_tail
    );
}

#[tokio::test]
async fn qa_002_ac3_malformed_line_becomes_unmapped() {
    let (mut s, mut rx) = start(
        &push_ask(),
        "--malformed-line 3",
        Arc::new(AllowAll),
        ClaudeAdapter::default(),
    )
    .await;
    s.send("Bitte pushen".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    assert!(count(&events, "harness.unmapped") >= 1);
    assert_eq!(events.last().unwrap().type_name(), "turn.completed");
}

#[tokio::test]
async fn qa_002_ac3_hanging_cli_does_not_block_shutdown() {
    let (mut s, mut rx) = start(
        &push_ask(),
        "--hang-after 2",
        Arc::new(AllowAll),
        ClaudeAdapter::default(),
    )
    .await;
    s.send("Bitte pushen".into()).await.unwrap();
    let got = tokio::time::timeout(Duration::from_millis(500), async {
        let mut n = 0;
        while rx.recv().await.is_some() {
            n += 1;
        }
        n
    })
    .await;
    assert!(got.is_err(), "CLI hängt, Strom endet nicht von selbst");
    let started = Instant::now();
    s.shutdown(Shutdown::Graceful {
        timeout: Duration::from_millis(200),
    })
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(7));
}

#[tokio::test]
async fn har_015_ac3_expired_login_asks_for_vendor_login() {
    let (_dir, path) = scenario("turns:\n  - emit: [{ auth_expired: \"Sitzung abgelaufen\" }]\n");
    let (mut s, mut rx) = start(&path, "", Arc::new(AllowAll), ClaudeAdapter::default()).await;
    s.send("hallo".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let auth = events
        .iter()
        .find_map(|e| match e {
            EventPayload::HarnessAuthRequired(a) => Some(a),
            _ => None,
        })
        .unwrap();
    assert_eq!(auth.hint, "claude auth login");
    assert_eq!(events.last().unwrap().type_name(), "turn.failed");
}
