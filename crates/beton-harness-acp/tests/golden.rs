//! Golden-Transcripts des ACP-Adapters gegen den deterministischen ACP-Test-Agent der
//! Testsuite (HAR-007 AC1, HAR-025 AC3). Neu erzeugen:
//! `BETON_RECORD_FAKE_GOLDEN=1 cargo test -p beton-fake-cli --test golden_from_fake -- --ignored`;
//! Erwartungen neu schreiben mit `BETON_BLESS=1`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_harness::golden;
use beton_harness::registry::AcpAgentConfig;
use beton_harness_acp::{AcpAdapter, AcpAgent, Origin};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn adapter() -> AcpAdapter {
    AcpAdapter::new(AcpAgent {
        slug: "test".into(),
        config: AcpAgentConfig {
            command: "acp-test-agent".into(),
            ..AcpAgentConfig::default()
        },
        origin: Origin::User,
    })
    .unwrap()
}

async fn case(name: &str) -> String {
    golden::run_case(&root().join(name), &adapter())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::read_to_string(root().join(name).join("expected.events.jsonl")).unwrap()
}

#[tokio::test]
async fn har_007_ac1_text_turn_matches_golden() {
    let events = case("text").await;
    assert!(events.contains("\"type\":\"reasoning.completed\""));
    assert!(events.contains("\"type\":\"message.delta\""));
}

#[tokio::test]
async fn har_007_ac1_tool_call_with_permission_matches_golden() {
    let events = case("tool-call").await;
    assert!(events.contains("\"decision\":\"allow\""));
    assert!(events.contains("\"type\":\"tool.call.started\""));
}

#[tokio::test]
async fn har_007_ac1_permission_deny_matches_golden() {
    let events = case("permission-deny").await;
    assert!(events.contains("\"decision\":\"deny\""));
    assert!(events.contains("\"status\":\"denied\""));
    assert!(!events.contains("\"type\":\"tool.call.started\""));
}

#[tokio::test]
async fn har_007_ac1_cancel_matches_golden() {
    let events = case("interrupt").await;
    assert!(events.contains("\"type\":\"turn.interrupted\""));
    let stdin = std::fs::read_to_string(root().join("interrupt/expected.stdin.jsonl")).unwrap();
    assert!(stdin.contains("session/cancel"));
}

#[tokio::test]
async fn har_025_ac3_error_matches_golden() {
    let events = case("error").await;
    assert!(events.contains("\"type\":\"turn.failed\""));
}
