//! Das Policy-Gate des Runners ohne Daemon: Fehlerinjektion (POL-024 AC3), Latenz ohne
//! menschliche Interaktion (HAR-005 AC3) und die Matrix aller Katalog-Harnesses (POL-024 AC2).

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_core::event::EventPayload;
use beton_harness::{Gate, GateDecision, GateOrigin, GateRequest, HarnessId, Mode, ProbeReport};
use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::enforcement::Level;
use beton_policy::engine::{Decision, EffectiveSet, EvalOptions};
use beton_policy::load::Scope;
use beton_runner::policy::{Evaluator, GateEvent, PolicyGate, PolicyOptions};
use serde_json::json;
use tokio::sync::mpsc;

fn options(dir: &Path) -> PolicyOptions {
    PolicyOptions {
        harness: "fake".parse::<HarnessId>().unwrap(),
        workdir: dir.to_path_buf(),
        dirs: vec![(Scope::Project, dir.join(".beton/policies"))],
        ..PolicyOptions::default()
    }
}

fn bash(call: &str, command: &str) -> GateRequest {
    GateRequest {
        call_id: call.into(),
        tool: "Bash".into(),
        kind: "shell".into(),
        args: json!({ "command": command }),
        origin: GateOrigin::PreTool,
        ..GateRequest::default()
    }
}

fn drain(rx: &mut mpsc::UnboundedReceiver<GateEvent>) -> Vec<EventPayload> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e.payload);
    }
    out
}

struct Failing;

impl Evaluator for Failing {
    fn evaluate(
        &self,
        _set: &EffectiveSet,
        _ctx: &Activation,
        _opts: &EvalOptions,
    ) -> Result<Decision, String> {
        Err("Engine kaputt (Fehlerinjektion)".into())
    }
}

struct Slow;

impl Evaluator for Slow {
    fn evaluate(
        &self,
        set: &EffectiveSet,
        ctx: &Activation,
        opts: &EvalOptions,
    ) -> Result<Decision, String> {
        std::thread::sleep(Duration::from_secs(2));
        Ok(set.evaluate(ctx, opts))
    }
}

#[tokio::test]
async fn pol_024_ac3_engine_error_or_timeout_denies() {
    let dir = tempfile::tempdir().unwrap();
    for (evaluator, name) in [
        (Arc::new(Failing) as Arc<dyn Evaluator>, "Fehler"),
        (Arc::new(Slow) as Arc<dyn Evaluator>, "Timeout"),
    ] {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gate = PolicyGate::new(options(dir.path()), tx).with_evaluator(evaluator);
        let started = Instant::now();
        let decision = gate.decide(bash("c1", "ls")).await;
        assert!(
            matches!(decision, GateDecision::Deny { .. }),
            "{name}: {decision:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{name}: wartet nicht ewig"
        );
        let events = drain(&mut rx);
        let d = events
            .iter()
            .find_map(|e| match e {
                EventPayload::PolicyDecision(d) => Some(d),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name}: policy.decision fehlt"));
        assert_eq!(d.outcome, beton_core::event::PolicyOutcome::Deny);
        assert!(!d.errors.is_empty(), "{name}: Fehler im Audit");
    }
}

#[tokio::test]
async fn har_005_ac3_one_decision_per_request_below_20ms_p99() {
    let dir = tempfile::tempdir().unwrap();
    let policies = dir.path().join(".beton/policies");
    std::fs::create_dir_all(&policies).unwrap();
    let mut rules = String::from("spec_version: 1\nname: viele\nrules:\n");
    for i in 0..50 {
        rules.push_str(&format!(
            "  - id: r{i}\n    on: tool_call\n    when: 'tool.kind == \"shell\" && tool.command.contains(\"verboten-{i}\")'\n    action: deny\n"
        ));
    }
    std::fs::write(policies.join("viele.yaml"), rules).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let gate = PolicyGate::new(options(dir.path()), tx);
    let mut latencies = Vec::new();
    for i in 0..300 {
        let command = if i % 3 == 0 { "verboten-7" } else { "ls" };
        let started = Instant::now();
        let d = gate.decide(bash(&format!("c{i}"), command)).await;
        latencies.push(started.elapsed());
        assert_eq!(matches!(d, GateDecision::Deny { .. }), i % 3 == 0);
    }
    let events = drain(&mut rx);
    let decisions = events
        .iter()
        .filter(|e| matches!(e, EventPayload::PolicyDecision(_)))
        .count();
    assert_eq!(decisions, 300, "genau ein policy.decision je Anfrage");
    latencies.sort();
    let p99 = latencies[latencies.len() * 99 / 100];
    assert!(p99 < Duration::from_millis(20), "p99 {p99:?}");
}

#[tokio::test]
async fn pol_025_vendor_ask_after_the_hook_reuses_the_decision() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let gate = PolicyGate::new(options(dir.path()), tx);
    // Erst der Hook (voll), dann die Rückfrage des Vendors zum selben Call: im Modus
    // `yolo` ohne Menschen, nur ein policy.decision.
    gate.set_permission_mode(beton_harness::PermissionMode::Yolo);
    assert!(matches!(
        gate.decide(bash("c1", "ls")).await,
        GateDecision::Allow { .. }
    ));
    let ask = GateRequest {
        origin: GateOrigin::VendorAsk,
        ..bash("c1", "ls")
    };
    assert!(matches!(gate.decide(ask).await, GateDecision::Allow { .. }));
    let events = drain(&mut rx);
    let n = events
        .iter()
        .filter(|e| matches!(e, EventPayload::PolicyDecision(_)))
        .count();
    assert_eq!(n, 1, "{events:?}");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EventPayload::ApprovalRequested(_)))
    );
}

#[test]
fn pol_024_ac2_matrix_of_all_catalog_harnesses_matches_the_spec_table() {
    let registry = beton_runner::builtin_registry(&Default::default(), true);
    let level = |id: &str, phase: Phase, kind: Option<&str>| {
        let id: HarnessId = id.parse().unwrap();
        let adapter = registry.get(&id).unwrap_or_else(|| panic!("{id} fehlt"));
        adapter
            .capabilities(Mode::Native, &ProbeReport::default())
            .enforcement()
            .level(phase, kind)
    };
    // 03-policies.md, POL-024 (Tabelle): je Harness und Phase.
    for kind in beton_policy::enforcement::KINDS {
        assert_eq!(
            level("claude", Phase::ToolCall, Some(kind)),
            Level::Full,
            "claude {kind}"
        );
        assert_eq!(level("fake", Phase::ToolCall, Some(kind)), Level::Full);
    }
    assert_eq!(
        level("codex", Phase::ToolCall, Some("shell")),
        Level::ApprovalOnly
    );
    assert_eq!(
        level("codex", Phase::ToolCall, Some("file_edit")),
        Level::ApprovalOnly
    );
    assert_eq!(
        level("codex", Phase::ToolCall, Some("file_read")),
        Level::ObserveOnly
    );
    assert_eq!(level("codex", Phase::ToolCall, Some("system")), Level::Full);
    assert_eq!(
        level("acp:gemini", Phase::ToolCall, Some("shell")),
        Level::ApprovalOnly
    );
    assert_eq!(
        level("acp:gemini", Phase::ToolCall, Some("file_read")),
        Level::ObserveOnly
    );
    for h in ["claude", "codex", "acp:gemini", "fake"] {
        assert_eq!(level(h, Phase::SessionStart, None), Level::Full, "{h}");
        assert_eq!(level(h, Phase::ModelRequest, None), Level::Full, "{h}");
    }
    assert_eq!(
        level("claude", Phase::ToolResult, Some("shell")),
        Level::ObserveOnly
    );
    assert_eq!(
        level("codex", Phase::ToolResult, Some("shell")),
        Level::ObserveOnly
    );
    assert_eq!(level("fake", Phase::ToolResult, Some("shell")), Level::Full);
    // Isoliert ohne Hook: nur Rückfragen.
    let isolated = beton_harness_claude::ClaudeAdapter {
        isolated: true,
        ..Default::default()
    };
    let m = beton_harness::HarnessAdapter::capabilities(
        &isolated,
        Mode::Native,
        &ProbeReport::default(),
    )
    .enforcement();
    assert_eq!(m.level(Phase::ToolCall, Some("shell")), Level::ApprovalOnly);
    assert_eq!(
        m.level(Phase::ToolCall, Some("file_read")),
        Level::ObserveOnly
    );
}
