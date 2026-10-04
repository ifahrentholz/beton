//! Golden-Transcripts des Codex-Adapters (HAR-025, HAR-006 AC1/AC2).
//! Aufgenommen mit `beton dev record-golden --harness codex [--scenario <name>]`; Fälle mit
//! `recorded_with` in `meta.yaml` stammen aus der Fake-CLI nach dem Protokoll-Schema.
//! Erwartungen neu schreiben mit `BETON_BLESS=1`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_harness::golden;
use beton_harness_codex::{CodexAdapter, TESTED_VERSIONS, capabilities};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

async fn case(name: &str) -> String {
    golden::run_case(&root().join(name), &CodexAdapter::default())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::read_to_string(root().join(name).join("expected.events.jsonl")).unwrap()
}

#[tokio::test]
async fn har_006_ac1_text_turn() {
    let events = case("text").await;
    assert!(events.contains("\"type\":\"message.delta\""));
    assert!(events.contains("\"type\":\"turn.completed\""));
}

#[tokio::test]
async fn har_006_ac1_command_tool_turn() {
    let events = case("command-tool").await;
    assert!(events.contains("\"decision\":\"allow\""));
    assert!(events.contains("\"type\":\"tool.call.started\""));
}

#[tokio::test]
async fn har_006_ac2_denied_git_push_is_not_executed() {
    let events = case("command-deny").await;
    assert!(events.contains("git push origin main"));
    assert!(events.contains("\"decision\":\"deny\""));
    assert!(events.contains("\"status\":\"denied\""));
    assert!(
        !events.contains("\"type\":\"tool.call.started\""),
        "nicht ausgeführt"
    );
    let stdin = std::fs::read_to_string(root().join("command-deny/expected.stdin.jsonl")).unwrap();
    assert!(stdin.contains("\"decision\":\"decline\""));
    // #149: Die Begründung geht vor der Ablehnung als `turn/steer` in denselben Turn.
    let steer = stdin
        .find("\"method\":\"turn/steer\"")
        .expect("turn/steer fehlt");
    assert!(steer < stdin.find("\"decision\":\"decline\"").unwrap());
    assert!(stdin.contains("Begründung: nicht erlaubt"));
}

#[tokio::test]
async fn har_006_ac3_token_usage_gives_context_usage_and_cost_delta() {
    let events = case("text").await;
    assert!(events.contains("\"type\":\"context.usage\""));
    assert!(events.contains("\"type\":\"cost.delta\""));
    assert!(events.contains("\"source\":\"subscription\""));
}

#[tokio::test]
async fn har_006_interrupt_ends_the_turn() {
    let events = case("interrupt").await;
    assert!(events.contains("\"type\":\"turn.interrupted\""));
}

#[tokio::test]
async fn har_025_ac3_error_turn() {
    let events = case("error").await;
    assert!(events.contains("\"type\":\"turn.failed\""));
}

#[tokio::test]
async fn har_006_ac1_file_edit_turn() {
    let events = case("file-edit").await;
    assert!(events.contains("\"tool\":\"fileChange\""));
}

#[tokio::test]
async fn har_006_ac1_reasoning_turn() {
    let events = case("reasoning").await;
    assert!(events.contains("\"type\":\"reasoning.completed\""));
}

#[test]
fn har_025_ac4_recorded_versions_match_catalog() {
    let problems = golden::check_versions(&capabilities(), &TESTED_VERSIONS, &root()).unwrap();
    assert!(problems.is_empty(), "{problems:#?}");
}
