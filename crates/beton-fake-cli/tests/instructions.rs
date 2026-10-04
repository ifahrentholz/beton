//! Agent-Instructions kommen bei jedem Harness-Typ genau einmal an (AGT-005 AC1, HAR-004
//! AC5): die echten Adapter gegen die Fake-CLI über eine echte Prozessgrenze. Die Fake-CLI
//! zeichnet mit `--record` auf, was beim Modell als Kontext ankäme (System-Prompt-Anhang,
//! Developer-Instructions, Nutzer-Nachrichten). Der Direkt-API-Harness hat seinen Test beim
//! Mock-Server (`beton-harness-direct/tests/direct.rs`).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_core::event::EventPayload;
use beton_harness::process::RealLauncher;
use beton_harness::registry::AcpAgentConfig;
use beton_harness::{
    AdapterContext, AllowAll, HarnessAdapter, HarnessSession, HostEnv, NormalizedEvent,
    SessionSpec, Shutdown,
};
use beton_harness_acp::{AcpAdapter, AcpAgent, Origin};
use beton_harness_claude::ClaudeAdapter;
use beton_harness_codex::CodexAdapter;
use serde_json::Value;
use tokio::sync::mpsc;

const MARKER: &str = "REGEL-42";
const INSTRUCTIONS: &str = "REGEL-42: Arbeite nur im Worktree.\n\n--- Projektdatei AGENTS.md ---\nTests zuerst.\n--- Ende AGENTS.md ---";
const TWO_TURNS: &str =
    "turns:\n  - emit: [{ message: \"eins\" }]\n  - emit: [{ message: \"zwei\" }]\n";

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

struct Fx {
    _dir: tempfile::TempDir,
    scenario: PathBuf,
    record: PathBuf,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("szenario.yaml");
    std::fs::write(&scenario, TWO_TURNS).unwrap();
    let record = dir.path().join("kontext.jsonl");
    Fx {
        _dir: dir,
        scenario,
        record,
    }
}

fn command(protocol: &str, fx: &Fx) -> String {
    format!(
        "{} --protocol {protocol} --scenario {} --record {}",
        fake_cli(),
        fx.scenario.display(),
        fx.record.display()
    )
}

fn ctx(var: &str, value: String) -> AdapterContext {
    let mut env = HostEnv::default();
    env.vars.insert(var.into(), value);
    AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(RealLauncher),
        env,
    }
}

fn spec() -> SessionSpec {
    SessionSpec {
        workdir: std::env::temp_dir(),
        instructions: Some(INSTRUCTIONS.into()),
        ..SessionSpec::default()
    }
}

async fn until_turn_end(rx: &mut mpsc::Receiver<NormalizedEvent>) {
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        if matches!(
            e.payload,
            EventPayload::TurnCompleted(_)
                | EventPayload::TurnFailed(_)
                | EventPayload::HarnessExited(_)
        ) {
            return;
        }
    }
    panic!("Turn-Ende fehlt");
}

/// Zwei Turns spielen und die aufgezeichneten Kontext-Einträge liefern.
async fn two_turns(mut session: Box<dyn HarnessSession>, record: &Path) -> Vec<(String, String)> {
    let mut rx = session.events().unwrap();
    for input in ["erste Frage", "zweite Frage"] {
        session.send(input.into()).await.unwrap();
        until_turn_end(&mut rx).await;
    }
    session.shutdown(Shutdown::Kill).await.unwrap();
    std::fs::read_to_string(record)
        .unwrap()
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            (
                v["source"].as_str().unwrap().to_owned(),
                v["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Die Instructions stehen genau einmal im Kontext, und zwar in `source`.
fn exactly_once(context: &[(String, String)], source: &str) {
    let hits: Vec<&(String, String)> = context
        .iter()
        .filter(|(_, text)| text.contains(MARKER))
        .collect();
    assert_eq!(hits.len(), 1, "{context:#?}");
    assert_eq!(hits[0].0, source, "{context:#?}");
    assert_eq!(hits[0].1.matches(MARKER).count(), 1, "{context:#?}");
    assert!(hits[0].1.contains("Tests zuerst."), "{context:#?}");
    // Beide Nutzer-Nachrichten kamen an.
    let users = context.iter().filter(|(s, _)| s == "user").count();
    assert_eq!(users, 2, "{context:#?}");
}

#[tokio::test]
async fn agt_005_ac1_claude_gets_the_instructions_once_via_append_system_prompt_file() {
    let fx = fx();
    let session = ClaudeAdapter::default()
        .start(
            spec(),
            ctx("BETON_CLAUDE_PATH", command("stream-json", &fx)),
        )
        .await
        .unwrap();
    let context = two_turns(session, &fx.record).await;
    // HAR-004 AC5: Die CLI hat den Inhalt aus `--append-system-prompt-file` gelesen.
    exactly_once(&context, "append_system_prompt");
    assert_eq!(context[0].1, INSTRUCTIONS);
}

#[tokio::test]
async fn agt_005_ac1_codex_gets_the_instructions_once_as_developer_instructions() {
    let fx = fx();
    let session = CodexAdapter::default()
        .start(spec(), ctx("BETON_CODEX_PATH", command("app-server", &fx)))
        .await
        .unwrap();
    let context = two_turns(session, &fx.record).await;
    exactly_once(&context, "developer_instructions");
}

#[tokio::test]
async fn agt_005_ac1_acp_gets_the_instructions_once_before_the_first_message() {
    let fx = fx();
    let adapter = AcpAdapter::new(AcpAgent {
        slug: "test".into(),
        config: AcpAgentConfig {
            command: "acp-test-agent".into(),
            ..AcpAgentConfig::default()
        },
        origin: Origin::User,
    })
    .unwrap();
    let session = adapter
        .start(spec(), ctx("BETON_ACP_TEST_PATH", command("acp", &fx)))
        .await
        .unwrap();
    let context = two_turns(session, &fx.record).await;
    exactly_once(&context, "user");
    // Präfix der ersten Nachricht; die zweite bleibt unverändert.
    assert!(context[0].1.ends_with("erste Frage"), "{context:#?}");
    assert_eq!(context[1].1, "zweite Frage");
}

#[tokio::test]
async fn agt_005_without_instructions_nothing_extra_arrives() {
    let fx = fx();
    let session = ClaudeAdapter::default()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx("BETON_CLAUDE_PATH", command("stream-json", &fx)),
        )
        .await
        .unwrap();
    let context = two_turns(session, &fx.record).await;
    assert!(context.iter().all(|(s, _)| s == "user"), "{context:#?}");
}
