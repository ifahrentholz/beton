//! Codex-Approval-Requests laut Policy (HAR-027, POL-006, POL-024): Jede Rückfrage geht an
//! das Gate; Rückfragen, die der Permission-Mode ohne Mensch beantwortet (`accept_edits` für
//! Datei-Änderungen im Workspace, `yolo` für alle), kommen als `AutoApproval` – die Policy
//! entscheidet trotzdem, nur eine menschliche Rückfrage entfällt.

#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{EventPayload, ToolStatus};
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateOrigin, GateRequest, HarnessAdapter, HostEnv,
    ModelGateDecision, ModelGateRequest, PermissionMode, ResultGateRequest, SessionSpec, Shutdown,
};
use beton_harness_codex::{CodexAdapter, capabilities};

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

#[derive(Default)]
struct Policy {
    requests: Mutex<Vec<GateRequest>>,
    deny: bool,
}

#[async_trait]
impl Gate for Policy {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        self.requests.lock().unwrap().push(request);
        if self.deny {
            GateDecision::Deny {
                reason: Some("Policy: nein".into()),
            }
        } else {
            GateDecision::Allow { updated_args: None }
        }
    }

    async fn model_request(&self, _r: ModelGateRequest) -> ModelGateDecision {
        ModelGateDecision::Allow {
            model: None,
            reasoning_effort: None,
        }
    }

    async fn tool_result(&self, _r: ResultGateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }
}

const EDIT: &str = r#"
turns:
  - emit:
      - { tool_call: { name: apply_patch, kind: file_edit, args: { path: notiz.txt } }, gate: true }
      - { on_gate: { allow: [ { tool_result: "ok" } ], deny: [ { message: "abgelehnt" } ] } }
"#;

async fn run(mode: Option<PermissionMode>, deny: bool) -> (Vec<GateRequest>, Vec<EventPayload>) {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("s.yaml");
    std::fs::write(&scenario, EDIT).unwrap();
    let mut env = HostEnv::default();
    env.vars.insert(
        "BETON_CODEX_PATH".into(),
        format!(
            "{} --protocol app-server --scenario {}",
            fake_cli(),
            scenario.display()
        ),
    );
    let gate = Arc::new(Policy {
        deny,
        ..Policy::default()
    });
    let ctx = AdapterContext {
        gate: gate.clone(),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let mut s = CodexAdapter::default()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                permission_mode: mode,
                ..SessionSpec::default()
            },
            ctx,
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send("los".into()).await.unwrap();
    let mut events = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_)
        );
        events.push(e.payload);
        if end {
            break;
        }
    }
    s.shutdown(Shutdown::default()).await.unwrap();
    let requests = gate.requests.lock().unwrap().clone();
    (requests, events)
}

fn status(events: &[EventPayload]) -> Option<ToolStatus> {
    events.iter().find_map(|e| match e {
        EventPayload::ToolCallCompleted(c) => Some(c.status),
        _ => None,
    })
}

#[tokio::test]
async fn har_006_ac2_vendor_ask_goes_to_the_gate_with_the_policy_kind() {
    let (requests, events) = run(None, true).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].origin, GateOrigin::VendorAsk);
    assert_eq!(requests[0].kind, "file_edit");
    assert_eq!(status(&events), Some(ToolStatus::Denied));
    // Freigabe-Events schreibt ab M2 der Runner, nicht der Adapter.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EventPayload::ApprovalRequested(_)))
    );
}

#[tokio::test]
async fn har_027_accept_edits_still_asks_the_policy_without_a_human() {
    let (requests, events) = run(Some(PermissionMode::AcceptEdits), true).await;
    assert_eq!(
        requests.len(),
        1,
        "Policy entscheidet auch bei accept_edits"
    );
    assert_eq!(requests[0].origin, GateOrigin::AutoApproval);
    assert_eq!(status(&events), Some(ToolStatus::Denied));
}

#[tokio::test]
async fn har_027_yolo_answers_every_request_by_policy() {
    assert!(
        capabilities()
            .permission_modes
            .contains(&PermissionMode::Yolo),
        "Codex deklariert yolo, weil Rückfragen jetzt laut Policy beantwortet werden"
    );
    let (requests, events) = run(Some(PermissionMode::Yolo), false).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].origin, GateOrigin::AutoApproval);
    assert_eq!(status(&events), Some(ToolStatus::Ok));
    let (_, events) = run(Some(PermissionMode::Yolo), true).await;
    assert_eq!(status(&events), Some(ToolStatus::Denied));
}
