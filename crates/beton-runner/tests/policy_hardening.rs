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
    let none = SandboxStatus::NONE;
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
async fn agt_014_agent_policies_are_the_agent_level_of_the_gate() {
    // `policies:` im `agent.yaml` (Inline-Regeln und Dateien per `ref`) bilden die
    // Agent-Ebene (AGT-014); sie verschärfen nur.
    let tmp = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let work = tmp.path().join("work");
    let agent = work.join(".beton/agents/streng");
    std::fs::create_dir_all(agent.join("policies")).unwrap();
    std::fs::write(
        agent.join("agent.yaml"),
        "spec_version: 1\nname: streng\nexecutor: { harness: fake }\npolicies:\n  - { id: no-shell, on: tool_call, when: 'shell.has_command(tool.argv, \"rm\")', action: deny }\n  - { ref: policies/extra.yaml }\n",
    )
    .unwrap();
    std::fs::write(
        agent.join("policies/extra.yaml"),
        "spec_version: 1\nname: extra\nrules:\n  - id: no-curl\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"curl\")'\n    action: deny\n",
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
    let setup = beton_runner::mcp::prepare(
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
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let sets = setup.agent_policies.clone();
    assert_eq!(sets.len(), 2, "{sets:?}");
    assert!(
        sets.iter()
            .all(|s| s.scope == beton_policy::load::Scope::Agent)
    );
    let (g, _rx) = gate(tmp.path(), "fake");
    g.set_inline(beton_policy::load::Scope::Agent, sets);
    for (call, cmd, rule) in [
        ("c1", "rm -rf build", "no-shell"),
        ("c2", "curl https://x.example", "no-curl"),
    ] {
        match g.decide(bash(call, cmd, GateOrigin::PreTool)).await {
            GateDecision::Deny { reason } => {
                let reason = reason.unwrap_or_default();
                assert!(reason.contains(rule), "{cmd}: {reason}");
            }
            other => panic!("{cmd}: {other:?}"),
        }
    }
    assert_eq!(
        g.decide(bash("c3", "ls", GateOrigin::PreTool)).await,
        GateDecision::Allow { updated_args: None }
    );

    // Lockernde Angaben auf der Agent-Ebene sind ein Ladefehler; der Agent startet nicht.
    std::fs::write(
        agent.join("agent.yaml"),
        "spec_version: 1\nname: streng\nexecutor: { harness: fake }\npolicies:\n  - { id: yolo, on: session_start, action: modify, modify: { permission_mode: yolo } }\n",
    )
    .unwrap();
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
        None,
        None,
        None,
    )
    .await
    .err()
    .expect("Start verweigert");
    assert!(err.contains("policies[0].modify.permission_mode"), "{err}");
}

/// Security-Review #327: `policies[].ref` bleibt im Agent-Verzeichnis. `..`, absolute Pfade
/// und Symlinks hinaus sind ein Startfehler; die Meldung enthält nichts aus der Datei.
#[cfg(unix)]
#[tokio::test]
async fn agt_014_policy_refs_cannot_leave_the_agent_dir() {
    let tmp = tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap();
    let work = tmp.path().join("work");
    let agent = work.join(".beton/agents/streng");
    std::fs::create_dir_all(agent.join("policies")).unwrap();
    // Eine gültige Policy-Datei außerhalb des Agent-Verzeichnisses (mit Marker im Grund).
    let outside = tmp.path().join("outside.yaml");
    std::fs::write(
        &outside,
        "spec_version: 1\nname: outside\nrules:\n  - id: x\n    on: tool_call\n    action: deny\n    reason: MARKER-AUSSERHALB-0815\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, agent.join("policies/link.yaml")).unwrap();
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
    for r in [
        "../../../../outside.yaml".to_owned(),
        outside.display().to_string(),
        "policies/link.yaml".to_owned(),
    ] {
        std::fs::write(
            agent.join("agent.yaml"),
            format!(
                "spec_version: 1\nname: streng\nexecutor: {{ harness: fake }}\npolicies:\n  - {{ ref: \"{r}\" }}\n"
            ),
        )
        .unwrap();
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
            None,
            None,
            None,
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("{r}: Start nicht verweigert"));
        assert!(err.contains("policies[0]"), "{r}: {err}");
        assert!(!err.contains("MARKER-AUSSERHALB"), "{r}: {err}");
    }
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

// ------------------------------------------------------- Re-Review R1 (#253), POL-001

#[tokio::test]
async fn pol_001_shell_and_mcp_variants_cannot_touch_policy_files() {
    // Angriffe aus dem Re-Review: Wörter ohne `/`, `./`, `~/` am Anfang, `$`-Expansionen,
    // Globs, `cd` ins Policy-Verzeichnis, MCP-Datei-Tools ohne Pfad-Abbildung.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".beton/policies")).unwrap();
    std::fs::create_dir_all(dir.path().join("user")).unwrap();
    let (g, _rx) = gate(dir.path(), "claude");
    for (i, command) in [
        "sed -i 's/deny/allow/' .beton/policies/x.yaml",
        "cp /tmp/p.yaml .beton/policies/x.yaml",
        "mv /tmp/p.yaml .beton/policies/x.yaml",
        "ln -sf /tmp/p.yaml .beton/policies/x.yaml",
        "echo rules: [] | tee .beton/policies/x.yaml",
        "git checkout HEAD~1 -- .beton",
        "cd .beton/policies && rm x.yaml",
        "rm $HOME/.beton/policies/defaults.yaml",
        "rm \"$BETON_HOME/policies/defaults.yaml\"",
        "rm -rf .b*",
        "rm user/defaults.yaml",
        "p=.bet; rm -rf ${p}on",
    ]
    .into_iter()
    .enumerate()
    {
        let d = g
            .decide(bash(&format!("r1_{i}"), command, GateOrigin::PreTool))
            .await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{command}: {d:?}");
    }
    for (i, command) in [
        "ls",
        "cat README.md",
        "echo $HOME",
        "git status",
        "rm -rf build",
    ]
    .into_iter()
    .enumerate()
    {
        let d = g
            .decide(bash(&format!("r1_ok_{i}"), command, GateOrigin::PreTool))
            .await;
        assert_eq!(d, GateDecision::Allow { updated_args: None }, "{command}");
    }
    let abs = dir.path().join(".beton/policies/x.yaml");
    for (i, path) in [
        abs.display().to_string(),
        ".beton/policies/x.yaml".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        let d = g
            .decide(GateRequest {
                call_id: format!("r1_mcp_{i}"),
                tool: "mcp__fs__write_file".into(),
                kind: "mcp".into(),
                mcp_server: Some("fs".into()),
                args: serde_json::json!({ "path": path, "content": "rules: []" }),
                origin: GateOrigin::PreTool,
                ..GateRequest::default()
            })
            .await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{path}: {d:?}");
    }
}

#[tokio::test]
async fn pol_001_loosening_project_change_is_not_applied_during_the_session() {
    // Ändert jemand (z. B. ein Agent über einen Weg, den das Gate nicht erkennt) eine
    // Projekt-Policy lockernd, gilt das erst in einer neuen Session.
    let dir = tempfile::tempdir().unwrap();
    write_policy(dir.path(), Scope::Project, "rm", NO_RM);
    let (g, _rx) = gate(dir.path(), "claude");
    let d = g
        .decide(bash("r1_l1", "rm -rf /x", GateOrigin::PreTool))
        .await;
    assert!(matches!(d, GateDecision::Deny { .. }));
    write_policy(dir.path(), Scope::Project, "rm", "  []\n");
    let d = g
        .decide(bash("r1_l2", "rm -rf /x", GateOrigin::PreTool))
        .await;
    assert!(
        matches!(d, GateDecision::Deny { .. }),
        "Lockerung übernommen: {d:?}"
    );
}

// ------------------------------------------------------- Re-Review R3 (#253), HAR-027

#[tokio::test]
async fn har_027_builtin_system_tools_are_put_to_the_human() {
    // Die Ausnahme ohne Rückfrage (#151) gilt nur für System-Tools von beton selbst, nicht für
    // Claude-Builtins der Klasse `system` (`ExitPlanMode`, `Agent`/`Task`, `TodoWrite`).
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
    g.set_trusted_servers(vec!["beton".into()]);
    for (i, tool) in ["ExitPlanMode", "Agent", "Task", "TodoWrite"]
        .into_iter()
        .enumerate()
    {
        let d = g
            .decide(GateRequest {
                call_id: format!("r3_{i}"),
                tool: tool.into(),
                kind: "system".into(),
                args: serde_json::json!({}),
                origin: GateOrigin::VendorAsk,
                ..GateRequest::default()
            })
            .await;
        assert!(matches!(d, GateDecision::Deny { .. }), "{tool}: {d:?}");
        let mut cards = 0;
        while let Ok(e) = rx.try_recv() {
            if matches!(
                e.payload,
                beton_core::event::EventPayload::ApprovalRequested(_)
            ) {
                cards += 1;
            }
        }
        assert_eq!(cards, 1, "{tool}: der Mensch entscheidet");
    }
}

#[test]
fn har_027_policy_forced_plan_binds_like_the_agent() {
    use beton_harness::PermissionMode::{AcceptEdits, Default, Plan};
    use beton_runner::settings::plan_locked;
    // Eine User-Policy erzwingt `plan`: Die Session verlässt `plan` nicht per ExitPlanMode.
    assert!(plan_locked(None, Some(Plan), Some(Plan)));
    // Wie bisher: der Agent bindet.
    assert!(plan_locked(Some(Plan), Some(Plan), None));
    // Vom Nutzer gewähltes `plan` ohne Agent und ohne Policy bindet nicht.
    assert!(!plan_locked(None, Some(Plan), None));
    assert!(!plan_locked(None, Some(Default), Some(AcceptEdits)));
}

#[tokio::test]
async fn pol_001_loosening_user_change_is_not_applied_during_the_session() {
    // Re-Review #2 N3 (#253): Ohne Sandbox erreicht ein Tool auch die User-Policies, z. B.
    // `find ~ -path '*pol*' -name defaults.yaml -exec sed -i s/deny/allow/ {} +`.
    let dir = tempfile::tempdir().unwrap();
    write_policy(dir.path(), Scope::User, "defaults", NO_RM);
    let (g, _rx) = gate(dir.path(), "claude");
    let d = g
        .decide(bash("n3_1", "rm -rf /x", GateOrigin::PreTool))
        .await;
    assert!(matches!(d, GateDecision::Deny { .. }));
    write_policy(dir.path(), Scope::User, "defaults", "  []\n");
    let d = g
        .decide(bash("n3_2", "rm -rf /x", GateOrigin::PreTool))
        .await;
    assert!(
        matches!(d, GateDecision::Deny { .. }),
        "Lockerung übernommen: {d:?}"
    );
}

// ------------------------------------------------------------------- #297 (Re-Review #284)

/// #297 Punkt 1 (POL-003 AC3): Ändern sich die Policies, während der Start auf eine Freigabe
/// wartet, wertet das Gate `session_start` nach der Freigabe erneut aus; eine neue `deny`-Regel
/// verweigert den Start, statt übergangen zu werden.
#[tokio::test]
async fn pol_003_session_start_is_rechecked_after_approval_when_policies_changed() {
    use beton_core::event::EventPayload;
    use beton_runner::policy::Human;
    let dir = tempfile::tempdir().unwrap();
    write_policy(
        dir.path(),
        Scope::User,
        "start",
        "  - id: confirm-start\n    on: session_start\n    action: ask\n",
    );
    let (g, mut rx) = gate(dir.path(), "claude");
    let g2 = g.clone();
    let task = tokio::spawn(async move { g2.check_start(start(None), None).await });
    let key = loop {
        let e = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("approval.requested")
            .unwrap();
        if let EventPayload::ApprovalRequested(r) = e.payload {
            break r.subject["call_id"].as_str().unwrap().to_owned();
        }
    };
    // Während der Wartezeit: neue Regel, die diesen Start ablehnt (Verschärfung).
    write_policy(
        dir.path(),
        Scope::User,
        "late",
        "  - id: no-start\n    on: session_start\n    action: deny\n    reason: Starts gesperrt\n",
    );
    assert!(g.resolve(
        &key,
        Human {
            allow: true,
            ..Human::default()
        }
    ));
    match task.await.unwrap() {
        Checked::Deny { reason, rules } => {
            assert!(reason.contains("Starts gesperrt"), "{reason}");
            assert_eq!(rules, ["no-start"]);
        }
        other => panic!("Start trotz neuer deny-Regel: {other:?}"),
    }
}

/// Kosten der Session je Abfrage: erst 3,10 USD, danach 5,20 USD.
struct RisingCost(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl beton_runner::policy::UsageSource for RisingCost {
    async fn usage(&self) -> Result<beton_runner::policy::Usage, String> {
        let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(beton_runner::policy::Usage {
            session_cost_usd: Some(if n == 0 { 3.1 } else { 5.2 }),
            session_equivalent_usd: Some(0.0),
            session_unpriced_models: Some(Vec::new()),
            ..beton_runner::policy::Usage::default()
        })
    }
}

/// #353 Punkt 5 (#297): Die Neuauswertung nach der Freigabe deckt nur denselben Grund ab.
/// Eine andere Schwelle derselben Budget-Regel (gleiche Regel-ID, anderer Teil) braucht eine
/// neue Freigabe, statt als freigegeben zu gelten.
#[tokio::test]
async fn pol_003_recheck_after_approval_needs_the_same_reason_not_only_the_same_rule() {
    use beton_core::event::EventPayload;
    use beton_runner::policy::Human;
    let dir = tempfile::tempdir().unwrap();
    write_policy(
        dir.path(),
        Scope::User,
        "budget",
        "  - id: cap\n    type: spend_cap\n    params: { scope: session, limit_usd: 10, ask_at_usd: [3, 5] }\n",
    );
    let (g, mut rx) = gate(dir.path(), "claude");
    let g = g.with_usage(std::sync::Arc::new(RisingCost(
        std::sync::atomic::AtomicUsize::new(0),
    )));
    let g2 = g.clone();
    let task = tokio::spawn(async move { g2.check_start(start(None), None).await });
    let key = loop {
        let e = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("approval.requested")
            .unwrap();
        if let EventPayload::ApprovalRequested(r) = e.payload {
            break r.subject["call_id"].as_str().unwrap().to_owned();
        }
    };
    // Während der Wartezeit ändern sich die Policies (Neuauswertung) und die Kosten steigen
    // über die nächste Schwelle derselben Regel.
    write_policy(
        dir.path(),
        Scope::User,
        "late",
        "  - id: note\n    on: tool_call\n    action: notify\n    notify: { channels: [log], message: x }\n",
    );
    assert!(g.resolve(
        &key,
        Human {
            allow: true,
            ..Human::default()
        }
    ));
    match task.await.unwrap() {
        Checked::Deny { reason, rules } => {
            assert!(
                reason.contains("5,00") || reason.contains("5.00"),
                "{reason}"
            );
            assert_eq!(rules, ["cap"]);
        }
        other => panic!("neue Schwelle als freigegeben gewertet: {other:?}"),
    }
}
