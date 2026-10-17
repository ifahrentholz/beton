//! HAR-005 (ab M2): session-spezifischer `PreToolUse`-Hook im Claude-Adapter. Jeder Tool-Call –
//! auch vom Vendor automatisch erlaubte wie `echo` – passiert das Gate; `can_use_tool` bleibt
//! für Rückfragen des Vendors zuständig. Echte Prozessgrenze: Fake-CLI (stream-json) ruft das
//! Hook-Kommando (`beton-fake-cli hook claude …`) wie Claude Code per Shell auf.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{EventPayload, ToolStatus};
use beton_harness::hook::{HookCommand, HookSetup};
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateOrigin, GateRequest, HarnessAdapter, HarnessSession,
    HostEnv, ModelGateDecision, ModelGateRequest, NormalizedEvent, ResultGateRequest, SessionSpec,
    Shutdown, ToolCallGate,
};
use beton_harness_claude::ClaudeAdapter;
use serde_json::{Value, json};
use tokio::sync::mpsc;

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

/// Gate, das Anfragen mitschreibt und je Herkunft entscheidet.
struct Recording {
    requests: Mutex<Vec<GateRequest>>,
    pre_tool: Box<dyn Fn(&GateRequest) -> GateDecision + Send + Sync>,
}

impl Recording {
    fn new(pre_tool: impl Fn(&GateRequest) -> GateDecision + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::default(),
            pre_tool: Box::new(pre_tool),
        })
    }

    fn requests(&self) -> Vec<GateRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Gate for Recording {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        self.requests.lock().unwrap().push(request.clone());
        match request.origin {
            GateOrigin::PreTool => (self.pre_tool)(&request),
            GateOrigin::VendorAsk | GateOrigin::AutoApproval => {
                GateDecision::Allow { updated_args: None }
            }
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

const ECHO: &str = r#"
turns:
  - emit:
      - { tool_call: { name: Bash, kind: shell, args: { command: "echo hi" } } }
      - { on_gate: { allow: [ { tool_result: "hi" }, { message: "ok" } ], deny: [ { message: "abgelehnt" } ] } }
"#;

const PUSH: &str = r#"
turns:
  - emit:
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [ { tool_result: "ok" } ], deny: [ { message: "abgelehnt" } ] } }
"#;

struct Run {
    _dir: tempfile::TempDir,
    session: Box<dyn HarnessSession>,
    rx: mpsc::Receiver<NormalizedEvent>,
}

async fn start(yaml: &str, gate: Arc<dyn Gate>, adapter: ClaudeAdapter, hook: bool) -> Run {
    start_in(yaml, gate, adapter, hook, None).await
}

/// Wie [`start`], mit eigenem Arbeitsverzeichnis (Projekt-Einstellungen, #233).
async fn start_in(
    yaml: &str,
    gate: Arc<dyn Gate>,
    adapter: ClaudeAdapter,
    hook: bool,
    workdir: Option<&std::path::Path>,
) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("szenario.yaml");
    std::fs::write(&scenario, yaml).unwrap();
    let run_dir = dir.path().join("run");
    let mut env = HostEnv::default();
    env.vars.insert(
        "BETON_CLAUDE_PATH".into(),
        format!(
            "{} --protocol stream-json --scenario {}",
            fake_cli(),
            scenario.display()
        ),
    );
    let ctx = AdapterContext {
        stage: None,
        gate,
        launcher: Arc::new(RealLauncher),
        env,
    };
    let spec = SessionSpec {
        workdir: workdir.map_or_else(std::env::temp_dir, std::path::Path::to_path_buf),
        hook: hook.then(|| HookSetup {
            command: HookCommand {
                program: PathBuf::from(fake_cli()),
                prefix: Vec::new(),
            },
            run_dir,
        }),
        ..SessionSpec::default()
    };
    let mut session = adapter.start(spec, ctx).await.unwrap();
    let rx = session.events().unwrap();
    Run {
        _dir: dir,
        session,
        rx,
    }
}

async fn turn(run: &mut Run, text: &str) -> Vec<EventPayload> {
    run.session.send(text.into()).await.unwrap();
    let mut out = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), run.rx.recv()).await {
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::HarnessExited(_)
        );
        out.push(e.payload);
        if end {
            break;
        }
    }
    out
}

fn completed(events: &[EventPayload]) -> Vec<(ToolStatus, Value)> {
    events
        .iter()
        .filter_map(|e| match e {
            EventPayload::ToolCallCompleted(c) => {
                Some((c.status, c.result.clone().unwrap_or_default()))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn har_005_vendor_auto_allowed_call_passes_the_gate_and_can_be_denied() {
    let gate = Recording::new(|_| GateDecision::Deny {
        reason: Some("beton: echo ist verboten".into()),
    });
    let mut run = start(ECHO, gate.clone(), ClaudeAdapter::default(), true).await;
    let events = turn(&mut run, "los").await;
    let requests = gate.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].origin, GateOrigin::PreTool);
    assert_eq!(requests[0].tool, "Bash");
    assert_eq!(requests[0].kind, "shell");
    assert_eq!(requests[0].args["command"], "echo hi");
    let done = completed(&events);
    assert_eq!(done.len(), 1, "{events:?}");
    assert_eq!(done[0].0, ToolStatus::Denied);
    assert!(
        done[0].1.to_string().contains("echo ist verboten"),
        "Begründung erreicht das Modell: {:?}",
        done[0].1
    );
    // Kein Freigabe-Event vom Adapter (das schreibt ab M2 der Runner).
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EventPayload::ApprovalRequested(_)))
    );
    run.session.shutdown(Shutdown::default()).await.unwrap();
}

#[tokio::test]
async fn har_005_hook_and_vendor_ask_reach_the_gate_once_each() {
    let gate = Recording::new(|_| GateDecision::Allow { updated_args: None });
    let mut run = start(PUSH, gate.clone(), ClaudeAdapter::default(), true).await;
    let events = turn(&mut run, "los").await;
    let origins: Vec<GateOrigin> = gate.requests().iter().map(|r| r.origin).collect();
    assert_eq!(origins, vec![GateOrigin::PreTool, GateOrigin::VendorAsk]);
    let ids: Vec<String> = gate.requests().iter().map(|r| r.call_id.clone()).collect();
    assert_eq!(ids[0], ids[1], "dieselbe Call-ID");
    assert!(matches!(completed(&events)[0].0, ToolStatus::Ok));
    run.session.shutdown(Shutdown::default()).await.unwrap();
}

#[tokio::test]
async fn pol_006_ac3_modified_args_from_the_hook_are_executed() {
    let gate = Recording::new(|_| GateDecision::Allow {
        updated_args: Some(json!({ "command": "git push --dry-run origin main" })),
    });
    let mut run = start(PUSH, gate.clone(), ClaudeAdapter::default(), true).await;
    let events = turn(&mut run, "los").await;
    let requests = gate.requests();
    // Die Rückfrage des Vendors sieht die geänderten Argumente.
    assert_eq!(
        requests[1].args["command"],
        "git push --dry-run origin main"
    );
    // `tool.call.requested` behält die Originalargumente.
    let requested = events
        .iter()
        .find_map(|e| match e {
            EventPayload::ToolCallRequested(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(requested.args["command"], "git push origin main");
    assert_eq!(requested.kind.as_deref(), Some("shell"));
    run.session.shutdown(Shutdown::default()).await.unwrap();
}

#[tokio::test]
async fn har_005_without_hook_or_isolated_only_vendor_asks_reach_the_gate() {
    for (adapter, hook) in [
        (ClaudeAdapter::default(), false),
        (
            ClaudeAdapter {
                isolated: true,
                ..ClaudeAdapter::default()
            },
            true,
        ),
    ] {
        let gate = Recording::new(|_| GateDecision::Deny { reason: None });
        let mut run = start(ECHO, gate.clone(), adapter, hook).await;
        let caps = run.session.capabilities().unwrap();
        assert_eq!(caps.tool_call_gate, ToolCallGate::ApprovalOnly);
        let events = turn(&mut run, "los").await;
        assert!(gate.requests().is_empty(), "kein Hook, keine Rückfrage");
        assert!(matches!(completed(&events)[0].0, ToolStatus::Ok));
        run.session.shutdown(Shutdown::default()).await.unwrap();
    }
}

#[tokio::test]
async fn har_005_hook_session_declares_full_gate() {
    let gate = Recording::new(|_| GateDecision::Allow { updated_args: None });
    let run = start(ECHO, gate, ClaudeAdapter::default(), true).await;
    let caps = run.session.capabilities().unwrap();
    assert_eq!(caps.tool_call_gate, ToolCallGate::Full);
    run.session.shutdown(Shutdown::default()).await.unwrap();
}

/// K3 (#233): `disableAllHooks: true` in `.claude/settings.json` bzw. `settings.local.json` des
/// Repositorys schaltete den beton-Hook ab (gegen 2.1.296 mit der echten CLI geprüft; die
/// Fake-CLI bildet das nach). beton lädt keine Projekt-Einstellungen; der Hook bleibt.
#[tokio::test]
async fn har_005_project_settings_cannot_disable_the_hook() {
    for file in ["settings.json", "settings.local.json"] {
        let work = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(work.path().join(".claude")).unwrap();
        std::fs::write(
            work.path().join(".claude").join(file),
            r#"{"disableAllHooks": true}"#,
        )
        .unwrap();
        let gate = Recording::new(|_| GateDecision::Deny {
            reason: Some("beton: echo ist verboten".into()),
        });
        let mut run = start_in(
            ECHO,
            gate.clone(),
            ClaudeAdapter::default(),
            true,
            Some(work.path()),
        )
        .await;
        let events = turn(&mut run, "los").await;
        assert_eq!(gate.requests().len(), 1, "{file}: Hook abgeschaltet");
        assert_eq!(completed(&events)[0].0, ToolStatus::Denied, "{file}");
        run.session.shutdown(Shutdown::default()).await.unwrap();
    }
}
