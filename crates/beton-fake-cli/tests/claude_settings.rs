//! Modell-, Effort- und Permission-Mode-Wechsel des Claude-Adapters gegen die Fake-CLI über
//! eine echte Prozessgrenze (HAR-017, HAR-020, HAR-027).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_core::event::EventPayload;
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, AllowAll, HarnessAdapter, HarnessSession, HostEnv, NormalizedEvent,
    PermissionMode, SessionSpec,
};
use beton_harness_claude::ClaudeAdapter;
use tokio::sync::mpsc;

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

const ECHO: &str =
    "turns:\n  - emit: [{ echo_settings: true }]\n  - emit: [{ echo_settings: true }]\n";

fn scenario(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("szenario.yaml");
    std::fs::write(&path, yaml).unwrap();
    path
}

async fn start(
    workdir: &Path,
    scenario: &Path,
    extra: &str,
    spec: SessionSpec,
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
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let mut session = ClaudeAdapter::default()
        .start(
            SessionSpec {
                workdir: workdir.to_path_buf(),
                ..spec
            },
            ctx,
        )
        .await
        .unwrap();
    let rx = session.events().unwrap();
    (session, rx)
}

async fn until_turn_end(rx: &mut mpsc::Receiver<NormalizedEvent>) -> Vec<NormalizedEvent> {
    let mut out = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::TurnInterrupted(_)
                | EventPayload::HarnessExited(_)
        );
        out.push(e);
        if end {
            break;
        }
    }
    out
}

fn reply(events: &[NormalizedEvent]) -> String {
    events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::MessageCompleted(m) => m.content[0]["text"].as_str().map(str::to_owned),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "keine Antwort: {:?}",
                events.iter().map(|e| &e.payload).collect::<Vec<_>>()
            )
        })
}

#[tokio::test]
async fn har_017_ac1_claude_model_and_effort_apply_from_the_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let sc = scenario(dir.path(), ECHO);
    let (mut s, mut rx) = start(
        dir.path(),
        &sc,
        "",
        SessionSpec {
            model: Some("sonnet".into()),
            effort: Some("low".into()),
            ..SessionSpec::default()
        },
    )
    .await;
    s.send("eins".into()).await.unwrap();
    let first = until_turn_end(&mut rx).await;
    assert_eq!(reply(&first), "model=sonnet effort=low mode=default");
    // Live-Wechsel ohne Neustart; der Verlauf bleibt (dieselbe CLI, dieselbe Session).
    let before = s.native_session_ref();
    s.set_model(Some("opus".into()), Some("high".into()))
        .await
        .unwrap();
    s.send("zwei".into()).await.unwrap();
    let second = until_turn_end(&mut rx).await;
    assert_eq!(reply(&second), "model=opus effort=high mode=default");
    assert!(
        second
            .iter()
            .any(|e| matches!(&e.payload, EventPayload::MessageCompleted(m) if m.role == beton_core::event::MessageRole::Assistant)),
    );
    assert_eq!(s.native_session_ref(), before);
}

#[tokio::test]
async fn har_027_claude_mode_switch_is_live() {
    let dir = tempfile::tempdir().unwrap();
    let sc = scenario(dir.path(), ECHO);
    let (mut s, mut rx) = start(
        dir.path(),
        &sc,
        "",
        SessionSpec {
            permission_mode: Some(PermissionMode::Plan),
            ..SessionSpec::default()
        },
    )
    .await;
    s.send("eins".into()).await.unwrap();
    assert_eq!(
        reply(&until_turn_end(&mut rx).await),
        "model=claude-fake effort=- mode=plan"
    );
    s.set_permission_mode(PermissionMode::AcceptEdits)
        .await
        .unwrap();
    s.send("zwei".into()).await.unwrap();
    assert_eq!(
        reply(&until_turn_end(&mut rx).await),
        "model=claude-fake effort=- mode=acceptEdits"
    );
}

#[tokio::test]
async fn har_027_repository_settings_cannot_loosen_the_mode() {
    // `.claude/settings.json` im Repository will `acceptEdits`; beton setzt den Modus
    // ausdrücklich, die Session bleibt bei `default`.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.json"),
        r#"{"permissions":{"defaultMode":"acceptEdits"}}"#,
    )
    .unwrap();
    let sc = scenario(dir.path(), ECHO);
    let (mut s, mut rx) = start(dir.path(), &sc, "", SessionSpec::default()).await;
    s.send("eins".into()).await.unwrap();
    assert_eq!(
        reply(&until_turn_end(&mut rx).await),
        "model=claude-fake effort=- mode=default"
    );
}

#[tokio::test]
async fn har_027_cli_reporting_a_looser_mode_ends_the_session() {
    // Fehlerinjektion: Die CLI meldet `bypassPermissions`, obwohl beton `default` setzt.
    let dir = tempfile::tempdir().unwrap();
    let sc = scenario(
        dir.path(),
        "turns:\n  - emit:\n      - { tool_call: { name: Bash, kind: shell, args: { command: \"rm -rf x\" } }, gate: true }\n      - { tool_result: ok }\n",
    );
    let (mut s, mut rx) = start(
        dir.path(),
        &sc,
        "--report-mode bypassPermissions",
        SessionSpec::default(),
    )
    .await;
    let _ = s.send("los".into()).await;
    let mut events = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        let exited = matches!(e.payload, EventPayload::HarnessExited(_));
        events.push(e.payload);
        if exited {
            break;
        }
    }
    let problem = events
        .iter()
        .find_map(|e| match e {
            EventPayload::Error(err) => Some(err.problem.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("kein error: {events:?}"));
    assert_eq!(problem["code"], "permission_mode_mismatch");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EventPayload::HarnessExited(_))),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EventPayload::ToolCallStarted(_))),
        "kein Tool darf laufen: {events:?}"
    );
}

#[tokio::test]
async fn har_020_ac2_claude_session_ref_is_known_before_the_first_turn() {
    let dir = tempfile::tempdir().unwrap();
    let sc = scenario(dir.path(), ECHO);
    let (mut s, mut rx) = start(dir.path(), &sc, "", SessionSpec::default()).await;
    let reference = s
        .native_session_ref()
        .expect("Referenz vor dem ersten Turn");
    s.send("eins".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    let ready = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::HarnessReady(r) => r.harness_session_ref.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(ready, reference, "die CLI übernimmt --session-id");
}

#[tokio::test]
async fn har_027_approved_exit_plan_mode_returns_to_the_previous_mode() {
    let dir = tempfile::tempdir().unwrap();
    let sc = scenario(
        dir.path(),
        "turns:\n  - emit:\n      - { tool_call: { name: ExitPlanMode, kind: other, args: { plan: \"P\" } }, gate: true }\n      - { tool_result: ok }\n  - emit: [{ echo_settings: true }]\n",
    );
    let (mut s, mut rx) = start(
        dir.path(),
        &sc,
        "",
        SessionSpec {
            permission_mode: Some(PermissionMode::Plan),
            ..SessionSpec::default()
        },
    )
    .await;
    s.send("plane".into()).await.unwrap();
    let events = until_turn_end(&mut rx).await;
    // Kein Fail-closed: `default` war der Modus vor `plan`; das Log erfährt den Wechsel.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::Error(_))),
        "{:?}",
        events.iter().map(|e| &e.payload).collect::<Vec<_>>()
    );
    assert!(events.iter().any(|e| matches!(&e.payload,
        EventPayload::SessionSettingsChanged(c) if c.permission_mode.as_deref() == Some("default"))));
    s.send("los".into()).await.unwrap();
    assert_eq!(
        reply(&until_turn_end(&mut rx).await),
        "model=claude-fake effort=- mode=default"
    );
}
