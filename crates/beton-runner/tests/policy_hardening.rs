//! Härtung des Policy-Gates nach dem Security-Review von #187/#199 (#231–#234): Angriffe aus
//! dem Review als Tests ohne Daemon. Jede Policy-Datei liegt in einem Temp-Verzeichnis; das
//! echte Home des Entwicklers wird nie gelesen.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_harness::{Gate, GateDecision, GateOrigin, GateRequest, HarnessAdapter, HarnessId};
use beton_policy::load::Scope;
use beton_policy::vars::StartVars;
use beton_runner::policy::{Checked, GateEvent, PolicyGate, PolicyOptions};
use tokio::sync::mpsc;

/// Gate mit User-Ebene (`<dir>/user`) und Projekt-Ebene (`<dir>/.beton/policies`).
fn gate(dir: &Path, harness: &str) -> (PolicyGate, mpsc::UnboundedReceiver<GateEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let opts = PolicyOptions {
        harness: harness.parse::<HarnessId>().unwrap(),
        workdir: dir.to_path_buf(),
        dirs: vec![
            (Scope::User, dir.join("user")),
            (Scope::Project, dir.join(".beton/policies")),
        ],
        ..PolicyOptions::default()
    };
    (PolicyGate::new(opts, tx), rx)
}

fn write_policy(dir: &Path, scope: Scope, name: &str, rules: &str) {
    let base = match scope {
        Scope::User => dir.join("user"),
        _ => dir.join(".beton/policies"),
    };
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(
        base.join(format!("{name}.yaml")),
        format!("spec_version: 1\nname: {name}\nrules:\n{rules}"),
    )
    .unwrap();
}

const YOLO: &str = "  - id: yolo\n    on: session_start\n    action: modify\n    modify: { permission_mode: yolo }\n";

fn start(mode: Option<&str>) -> StartVars {
    StartVars {
        harness: Some("claude".into()),
        permission_mode: mode.map(str::to_owned),
        ..StartVars::default()
    }
}

// ------------------------------------------------------------------- K1 (#231), HAR-027

#[tokio::test]
async fn har_027_project_policy_cannot_raise_mode_to_yolo_without_start_mode() {
    // Angriff: Das Repo legt `.beton/policies/x.yaml` mit `modify: {permission_mode: yolo}` an;
    // die Session startet ohne Permission-Mode (API ohne `permission_mode`, Import).
    let dir = tempfile::tempdir().unwrap();
    write_policy(dir.path(), Scope::Project, "x", YOLO);
    let (g, _rx) = gate(dir.path(), "claude");
    match g.check_start(start(None), None).await {
        Checked::Allow(m) => assert_eq!(m.permission_mode, None, "Lockerung auf yolo"),
        Checked::Deny { .. } => {}
        Checked::AskNeeded => panic!("unerwartet: ask"),
    }
}

#[tokio::test]
async fn har_027_user_policy_only_tightens_the_mode_also_without_start_mode() {
    // Ohne Startmodus gilt `default` als Vergleichsbasis: `yolo` und `accept_edits` lockern.
    let dir = tempfile::tempdir().unwrap();
    write_policy(dir.path(), Scope::User, "x", YOLO);
    let (g, _rx) = gate(dir.path(), "claude");
    let Checked::Allow(m) = g.check_start(start(None), None).await else {
        panic!("Start erwartet");
    };
    assert_eq!(m.permission_mode, None);

    // Verschärfen bleibt erlaubt.
    let dir = tempfile::tempdir().unwrap();
    write_policy(
        dir.path(),
        Scope::User,
        "plan",
        "  - id: plan\n    on: session_start\n    action: modify\n    modify: { permission_mode: plan }\n",
    );
    let (g, _rx) = gate(dir.path(), "claude");
    let Checked::Allow(m) = g.check_start(start(None), None).await else {
        panic!("Start erwartet");
    };
    assert_eq!(m.permission_mode.as_deref(), Some("plan"));
}

#[test]
fn har_027_policy_mode_is_rechecked_against_the_sandbox() {
    use beton_harness::{Capabilities, Mode, PermissionMode, SandboxStatus, Transport};
    use beton_runner::settings::policy_start_mode;
    let mut caps = Capabilities::minimal(Mode::Native, Transport::Native);
    caps.permission_modes = PermissionMode::ALL.to_vec();
    let none = SandboxStatus::current();
    // Lockern wird verworfen, auch ohne Startmodus.
    assert_eq!(
        policy_start_mode(None, Some("yolo"), &caps, none).unwrap(),
        None
    );
    assert_eq!(
        policy_start_mode(Some(PermissionMode::Plan), Some("default"), &caps, none).unwrap(),
        None
    );
    assert_eq!(
        policy_start_mode(Some(PermissionMode::AcceptEdits), Some("plan"), &caps, none).unwrap(),
        Some(PermissionMode::Plan)
    );
    // `yolo` bleibt ohne Sandbox unmöglich (fail closed), auch wenn er schon angefragt war.
    assert!(policy_start_mode(Some(PermissionMode::Yolo), Some("yolo"), &caps, none).is_err());
    // Unbekannte Werte verwirft beton nicht still.
    assert!(policy_start_mode(None, Some("bypass"), &caps, none).is_err());
}

// ------------------------------------------------------------- K2 (#232), POL-005

#[tokio::test]
async fn pol_005_claude_hook_mcp_rule_on_server_and_name_denies() {
    // Angriff: `deny when tool.server == "github" && tool.name == "delete_repo"` erlaubte den
    // Aufruf unter Claude, weil `tool.name` das Präfix `mcp__github__` behielt.
    let dir = tempfile::tempdir().unwrap();
    write_policy(
        dir.path(),
        Scope::Project,
        "gh",
        "  - id: no-delete\n    on: tool_call\n    when: 'tool.server == \"github\" && tool.name == \"delete_repo\"'\n    action: deny\n",
    );
    let (g, _rx) = gate(dir.path(), "claude");
    // Genau so, wie der `PreToolUse`-Hook die Anfrage baut.
    let req = beton_harness_claude::pre_tool_request(
        &serde_json::json!({"hook_event_name": "PreToolUse", "tool_name": "mcp__github__delete_repo",
                            "tool_input": {"repo": "x"}, "tool_use_id": "toolu_1"}),
        None,
    );
    assert_eq!(req.mcp_server.as_deref(), Some("github"));
    let decision = g.decide(req).await;
    assert!(
        matches!(decision, GateDecision::Deny { .. }),
        "{decision:?}"
    );
}

// ------------------------------------------------------------- K3 (#233), HAR-005

fn bash(call: &str, command: &str, origin: GateOrigin) -> GateRequest {
    GateRequest {
        call_id: call.into(),
        tool: "Bash".into(),
        kind: "shell".into(),
        args: serde_json::json!({ "command": command }),
        origin,
        ..GateRequest::default()
    }
}

const NO_RM: &str = "  - id: no-rm\n    on: tool_call\n    when: 'tool.command.contains(\"rm -rf\")'\n    action: deny\n";

#[tokio::test]
async fn har_005_vendor_ask_with_changed_args_does_not_reuse_the_hook_decision() {
    // Angriff: Ein weiterer `PreToolUse`-Hook ändert die Argumente nach der beton-Prüfung
    // (`updatedInput`); die Rückfrage des Vendors kommt mit denselben Call-ID, aber anderen
    // Argumenten. Der gespeicherte Entscheid gilt nur für dasselbe Subjekt (`subject_hash`).
    let dir = tempfile::tempdir().unwrap();
    write_policy(dir.path(), Scope::Project, "rm", NO_RM);
    let (g, _rx) = gate(dir.path(), "claude");
    let pre = g
        .decide(bash("toolu_1", "echo hi", GateOrigin::PreTool))
        .await;
    assert_eq!(pre, GateDecision::Allow { updated_args: None });
    for origin in [GateOrigin::AutoApproval, GateOrigin::VendorAsk] {
        let ask = g.decide(bash("toolu_1", "rm -rf ~", origin)).await;
        assert!(
            matches!(ask, GateDecision::Deny { .. }),
            "{origin:?}: {ask:?}"
        );
    }
    // Dieselben Argumente: der Hook-Entscheid gilt weiter (ohne zweite Auswertung).
    let pre = g
        .decide(bash("toolu_2", "echo ok", GateOrigin::PreTool))
        .await;
    assert_eq!(pre, GateDecision::Allow { updated_args: None });
    let same = g
        .decide(bash("toolu_2", "echo ok", GateOrigin::AutoApproval))
        .await;
    assert_eq!(same, GateDecision::Allow { updated_args: None });
}

#[tokio::test]
async fn har_005_completed_call_without_hook_decision_is_an_integrity_violation() {
    // Bei Matrix `full` muss jeder Tool-Call vorher das Gate passiert haben. Fehlt die
    // Hook-Anfrage, wurde der Hook ausgehebelt: Integritätsverletzung statt `observe_only`.
    let dir = tempfile::tempdir().unwrap();
    let (g, _rx) = gate(dir.path(), "claude");
    let req = bash("toolu_3", "echo hi", GateOrigin::PreTool);
    assert!(g.integrity_violation(&req).is_some());
    let _ = g.decide(req.clone()).await;
    assert!(g.integrity_violation(&req).is_none(), "geprüft");
    // Ohne Hook (isoliert, nur Rückfragen) ist ein Call ohne Anfrage erwartbar: beobachtet.
    let isolated = beton_harness_claude::ClaudeAdapter {
        isolated: true,
        ..beton_harness_claude::ClaudeAdapter::default()
    };
    g.set_matrix(
        isolated
            .capabilities(
                beton_harness::Mode::Native,
                &beton_harness::ProbeReport::default(),
            )
            .enforcement(),
    );
    assert!(
        g.integrity_violation(&bash("toolu_4", "echo hi", GateOrigin::PreTool))
            .is_none()
    );
}

/// Folge von K3 (#233): Mit `--setting-sources user` liest Claude Code `CLAUDE.md` und
/// `.claude/skills` des Projekts nicht mehr selbst (gegen 2.1.296 geprüft). beton liefert beides:
/// `CLAUDE.md` über die Instructions (AGT-005, auch ohne Agent), Projekt-Skills über das
/// Session-Skill-Verzeichnis (AGT-008).
#[cfg(unix)]
#[tokio::test]
async fn agt_005_agt_008_claude_gets_project_claude_md_and_skills_from_beton() {
    let tmp = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let work = tmp.path().join("work");
    let skill = work.join(".claude/skills/projtest");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(work.join("CLAUDE.md"), "Regeln aus CLAUDE.md").unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: projtest\ndescription: Testskill\n---\nTu nichts.\n",
    )
    .unwrap();
    let run_dir = tmp.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let paths = beton_runner::mcp::Paths {
        beton_home: tmp.path().join("home/.beton"),
        home: Some(tmp.path().join("home")),
        run_dir,
        relay: std::path::PathBuf::from("/bin/false"),
    };
    let claude: HarnessId = "claude".parse().unwrap();
    let caps = beton_harness_claude::ClaudeAdapter::default().capabilities(
        beton_harness::Mode::Native,
        &beton_harness::ProbeReport::default(),
    );
    let setup = beton_runner::mcp::prepare(
        beton_core::id::SessionId::new(),
        &claude,
        &caps,
        &work,
        beton_runner::mcp::AgentSource::None,
        &beton_harness::registry::HarnessLayers::default(),
        &paths,
        None,
        None,
    )
    .await
    .unwrap();
    let text = setup.instructions.expect("Instructions mit CLAUDE.md");
    assert!(text.contains("Regeln aus CLAUDE.md"), "{text}");
    let skills = setup.injection.skills_dir.expect("Session-Skills");
    assert!(
        skills.join("skills/projtest/SKILL.md").is_file(),
        "{}",
        skills.display()
    );
}

// ------------------------------------------------------------- M1 (#234), POL-005

#[tokio::test]
async fn pol_005_foreign_beton_server_is_not_a_system_tool() {
    // Angriff: Ein MCP-Server namens `beton` aus der Vendor-Konfiguration (beton hat selbst
    // keinen eingebunden) galt als System-Tool und lief ohne menschliche Rückfrage (#151).
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let g = PolicyGate::new(
        PolicyOptions {
            harness: "claude".parse::<HarnessId>().unwrap(),
            workdir: dir.path().to_path_buf(),
            approval_timeout: std::time::Duration::from_millis(200),
            ..PolicyOptions::default()
        },
        tx,
    );
    let ask = |call: &str| GateRequest {
        call_id: call.into(),
        tool: "mcp__beton__session_list".into(),
        kind: "system".into(),
        mcp_server: Some("beton".into()),
        args: serde_json::json!({}),
        origin: GateOrigin::VendorAsk,
        ..GateRequest::default()
    };
    let decision = g.decide(ask("toolu_6")).await;
    assert!(
        matches!(decision, GateDecision::Deny { .. }),
        "{decision:?}"
    );
    let mut cards = 0;
    while let Ok(e) = rx.try_recv() {
        if let beton_core::event::EventPayload::ApprovalRequested(_) = e.payload {
            cards += 1;
        }
        if let beton_core::event::EventPayload::PolicyDecision(d) = e.payload {
            assert_eq!(d.outcome, beton_core::event::PolicyOutcome::Allow);
        }
    }
    assert_eq!(cards, 1, "fremder Server: der Mensch entscheidet");
    // Von beton eingebunden: System-Tool ohne Rückfrage.
    g.set_trusted_servers(vec!["beton".into()]);
    let decision = g.decide(ask("toolu_7")).await;
    assert_eq!(decision, GateDecision::Allow { updated_args: None });
}
