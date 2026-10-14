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

// ------------------------------------------------------------- M4 (#234), POL-001

fn fatal(rx: &mut mpsc::UnboundedReceiver<GateEvent>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        if e.fatal
            && let beton_core::event::EventPayload::Error(err) = &e.payload
        {
            out.push(err.problem["code"].as_str().unwrap_or_default().to_owned());
        }
    }
    out
}

#[tokio::test]
async fn pol_001_reload_checks_enforcement_and_validity_like_the_start() {
    // Eine während der Session hinzugefügte Pflichtregel, die der Harness nicht durchsetzen
    // kann, bzw. eine nie gültige Datei beenden die Session (wie die Prüfung beim Start).
    let dir = tempfile::tempdir().unwrap();
    let (g, mut rx) = gate(dir.path(), "claude");
    let mut matrix = beton_policy::enforcement::Matrix::full();
    matrix.tool_call.insert(
        "file_edit".into(),
        beton_policy::enforcement::Level::ObserveOnly,
    );
    g.set_matrix(matrix);
    let _ = g.decide(bash("toolu_8", "ls", GateOrigin::PreTool)).await;
    assert!(fatal(&mut rx).is_empty());
    write_policy(
        dir.path(),
        Scope::Project,
        "edit",
        "  - id: no-edit\n    on: tool_call\n    when: 'tool.kind == \"file_edit\"'\n    action: deny\n",
    );
    let _ = g.decide(bash("toolu_9", "ls", GateOrigin::PreTool)).await;
    assert_eq!(fatal(&mut rx), ["policy_enforcement_unavailable"]);

    let dir = tempfile::tempdir().unwrap();
    let (g, mut rx) = gate(dir.path(), "claude");
    let _ = g.decide(bash("toolu_10", "ls", GateOrigin::PreTool)).await;
    std::fs::create_dir_all(dir.path().join(".beton/policies")).unwrap();
    std::fs::write(
        dir.path().join(".beton/policies/kaputt.yaml"),
        "spec_version: 1\nname: [x\n",
    )
    .unwrap();
    let _ = g.decide(bash("toolu_11", "ls", GateOrigin::PreTool)).await;
    assert_eq!(fatal(&mut rx), ["policy_invalid"]);
}

#[tokio::test]
async fn pol_001_tools_cannot_write_policy_files() {
    // Angriff: Der Agent ändert oder löscht `.beton/policies/*.yaml` bzw. die User-Policies.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".beton/policies")).unwrap();
    std::fs::create_dir_all(dir.path().join("user")).unwrap();
    let (g, _rx) = gate(dir.path(), "claude");
    let project = dir.path().join(".beton/policies/x.yaml");
    let user = dir.path().join("user/defaults.yaml");
    let write = |call: &str, tool: &str, kind: &str, path: &std::path::Path| GateRequest {
        call_id: call.into(),
        tool: tool.into(),
        kind: kind.into(),
        args: serde_json::json!({ "file_path": path, "content": "rules: []" }),
        origin: GateOrigin::PreTool,
        ..GateRequest::default()
    };
    for (i, (tool, kind, path)) in [
        ("Write", "file_write", &project),
        ("Edit", "file_edit", &user),
    ]
    .into_iter()
    .enumerate()
    {
        let d = g.decide(write(&format!("w{i}"), tool, kind, path)).await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{tool}: {d:?}");
    }
    let d = g
        .decide(bash(
            "w3",
            "rm ./.beton/policies/x.yaml",
            GateOrigin::PreTool,
        ))
        .await;
    assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
    // Lesen bleibt erlaubt.
    let d = g.decide(write("w4", "Read", "file_read", &project)).await;
    assert_eq!(d, GateDecision::Allow { updated_args: None });
}

// ------------------------------------------------------------- M6 (#234), AGT-014

#[cfg(unix)]
#[tokio::test]
async fn agt_014_agent_with_policies_does_not_start_until_they_are_enforced() {
    // `policies:` im `agent.yaml` wird validiert, aber noch nicht geladen (WP-49). Statt die
    // Regeln still zu ignorieren, startet ein solcher Agent nicht (fail closed).
    let tmp = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let work = tmp.path().join("work");
    let agent = work.join(".beton/agents/streng");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(
        agent.join("agent.yaml"),
        "spec_version: 1\nname: streng\nexecutor: { harness: fake }\npolicies:\n  - { id: no-shell, on: tool_call, when: 'tool.kind == \"shell\"', action: deny }\n",
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
    let fake: HarnessId = "fake".parse().unwrap();
    let caps = beton_harness::Capabilities::minimal(
        beton_harness::Mode::Native,
        beton_harness::Transport::Native,
    );
    let err = beton_runner::mcp::prepare(
        beton_core::id::SessionId::new(),
        &fake,
        &caps,
        &work,
        beton_runner::mcp::AgentSource::Ref("streng"),
        &beton_harness::registry::HarnessLayers::default(),
        &paths,
        None,
        None,
        None,
    )
    .await
    .err()
    .expect("Start verweigert");
    assert!(err.contains("policies") && err.contains("WP-49"), "{err}");
}

// ------------------------------------------------------------- Minor (#234)

#[tokio::test]
async fn pol_025_ac2_vendor_raw_line_is_not_attached_to_the_decision() {
    // Die Original-Zeile einer Rückfrage enthält die Argumente im Klartext.
    let dir = tempfile::tempdir().unwrap();
    let (g, mut rx) = gate(dir.path(), "claude");
    let mut req = bash("toolu_20", "echo geheim-4711", GateOrigin::AutoApproval);
    req.raw = Some(beton_core::event::RawJson::from_value(&serde_json::json!({
        "request": { "input": { "command": "echo geheim-4711" } }
    })));
    let _ = g.decide(req).await;
    while let Ok(e) = rx.try_recv() {
        if let beton_core::event::EventPayload::PolicyDecision(_) = e.payload {
            assert!(e.raw.is_none(), "raw am policy.decision");
        }
    }
}

#[tokio::test]
async fn har_005_empty_call_id_is_denied() {
    // Leere Call-IDs teilten sich gespeicherte Entscheide.
    let dir = tempfile::tempdir().unwrap();
    let (g, _rx) = gate(dir.path(), "claude");
    for origin in [GateOrigin::PreTool, GateOrigin::AutoApproval] {
        let d = g.decide(bash("", "echo hi", origin)).await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{origin:?}: {d:?}");
    }
}

#[tokio::test]
async fn pol_010_human_changed_args_are_checked_again() {
    // Ein Mensch gibt `git push` frei, ändert dabei aber die Argumente in etwas, das eine
    // `deny`-Regel trifft: Die geänderten Argumente prüft die Policy erneut.
    let dir = tempfile::tempdir().unwrap();
    write_policy(
        dir.path(),
        Scope::Project,
        "push",
        &format!(
            "  - id: confirm-push\n    on: tool_call\n    when: 'tool.command.contains(\"git push\")'\n    action: ask\n{NO_RM}"
        ),
    );
    let (g, mut rx) = gate(dir.path(), "claude");
    let g2 = g.clone();
    let task = tokio::spawn(async move {
        g2.decide(bash("toolu_21", "git push", GateOrigin::PreTool))
            .await
    });
    loop {
        let e = rx.recv().await.unwrap();
        if matches!(
            e.payload,
            beton_core::event::EventPayload::ApprovalRequested(_)
        ) {
            break;
        }
    }
    assert!(g.resolve(
        "toolu_21",
        beton_runner::policy::Human {
            allow: true,
            reason: None,
            updated_args: Some(serde_json::json!({ "command": "rm -rf ~" })),
            ..beton_runner::policy::Human::default()
        }
    ));
    let d = task.await.unwrap();
    assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
}

#[tokio::test]
async fn pol_003_tool_result_sees_adapter_kind_args_and_cwd() {
    // `tool_result` hatte weder die Klasse des Adapters noch `args` und `cwd`.
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().display().to_string();
    write_policy(
        dir.path(),
        Scope::Project,
        "result",
        &format!(
            "  - id: hide\n    on: tool_result\n    when: 'tool.kind == \"shell\" && tool.args.command == \"cat x\" && tool.cwd == \"{cwd}\"'\n    action: deny\n"
        ),
    );
    let (g, _rx) = gate(dir.path(), "fake");
    let mut call = bash("toolu_22", "cat x", GateOrigin::PreTool);
    call.tool = "Shell".into();
    let d = g
        .tool_result(beton_harness::ResultGateRequest {
            call,
            ok: true,
            exit_code: Some(0),
            error: None,
            text: "inhalt".into(),
            bytes: 6,
            duration_ms: 1,
            truncated: false,
        })
        .await;
    assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
}
