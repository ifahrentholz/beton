//! Golden-Transcripts des Claude-Adapters (HAR-025, HAR-004 AC2, HAR-005 AC1).
//! Aufgenommen mit `beton dev record-golden --harness claude [--scenario <name>]`;
//! Erwartungen neu schreiben mit `BETON_BLESS=1`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_harness::golden;
use beton_harness_claude::{ClaudeAdapter, TESTED_VERSIONS, capabilities};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

async fn case(name: &str) {
    golden::run_case(&root().join(name), &ClaudeAdapter::default())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
}

#[tokio::test]
async fn har_004_ac2_text_turn() {
    case("text").await;
}

#[tokio::test]
async fn har_004_ac2_bash_tool_turn() {
    case("bash-tool").await;
}

#[tokio::test]
async fn har_004_ac2_file_edit_turn() {
    case("file-edit").await;
}

#[tokio::test]
async fn har_004_ac2_reasoning_turn() {
    case("reasoning").await;
}

#[tokio::test]
async fn har_004_ac2_error_max_turns() {
    case("error-max-turns").await;
}

#[tokio::test]
async fn har_005_ac1_denied_bash_is_not_executed() {
    case("bash-deny").await;
    let events = std::fs::read_to_string(root().join("bash-deny/expected.events.jsonl")).unwrap();
    assert!(events.contains("\"decision\":\"deny\""), "Ablehnung im Log");
    assert!(
        events.contains("\"status\":\"denied\""),
        "Tool-Call als abgelehnt"
    );
    assert!(
        !events.contains("\"type\":\"tool.call.started\""),
        "nicht ausgeführt"
    );
}

#[tokio::test]
async fn har_004_ac3_interrupt_ends_the_turn() {
    case("interrupt").await;
    let events = std::fs::read_to_string(root().join("interrupt/expected.events.jsonl")).unwrap();
    assert!(events.contains("\"type\":\"turn.interrupted\""));
}

#[test]
fn har_025_ac4_recorded_versions_match_catalog() {
    let problems = golden::check_versions(&capabilities(), &TESTED_VERSIONS, &root()).unwrap();
    assert!(problems.is_empty(), "{problems:#?}");
}
