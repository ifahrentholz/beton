//! Harness-Contract-Suite (QA-016 AC2): Claude-, Codex- und ACP-Adapter sowie der
//! Fake-Harness bestehen dieselbe Suite, jeweils über die Fake-CLI ihres Protokolls.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use beton_harness::contract::{self, AdapterSubject, Outcome};
use beton_harness::fake::FakeAdapter;
use beton_harness::process::RealLauncher;
use beton_harness::registry::AcpAgentConfig;
use beton_harness::{HarnessAdapter, HostEnv};
use beton_harness_acp::{AcpAdapter, AcpAgent, Origin};
use beton_harness_claude::ClaudeAdapter;
use beton_harness_codex::CodexAdapter;

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

fn subject(
    adapter: Arc<dyn HarnessAdapter>,
    var: &'static str,
    protocol: &'static str,
) -> AdapterSubject {
    AdapterSubject {
        adapter,
        env: Arc::new(move |scenario: &Path| {
            let mut env = HostEnv::default();
            env.vars.insert(
                var.into(),
                format!(
                    "{} --protocol {protocol} --scenario {}",
                    fake_cli(),
                    scenario.display()
                ),
            );
            env
        }),
        launcher: Arc::new(RealLauncher),
        workdir: std::env::temp_dir(),
    }
}

async fn passes(subject: AdapterSubject) -> contract::ContractReport {
    let dir = tempfile::tempdir().unwrap();
    let report = contract::run(&subject, dir.path()).await;
    assert!(report.is_ok(), "{}", report.summary());
    report
}

#[tokio::test]
async fn qa_016_ac2_claude_adapter_passes_the_suite() {
    let report = passes(subject(
        Arc::new(ClaudeAdapter::default()),
        "BETON_CLAUDE_PATH",
        "stream-json",
    ))
    .await;
    for check in [
        "streaming",
        "interrupt",
        "approval_deny",
        "resume",
        "model_switch",
    ] {
        assert_eq!(report.outcome(check), Some(&Outcome::Passed), "{check}");
    }
}

#[tokio::test]
async fn qa_016_ac2_codex_adapter_passes_the_suite() {
    let report = passes(subject(
        Arc::new(CodexAdapter::default()),
        "BETON_CODEX_PATH",
        "app-server",
    ))
    .await;
    for check in [
        "streaming",
        "usage_reporting",
        "interrupt",
        "approval_allow",
        "approval_deny",
        "resume",
        "model_switch",
    ] {
        assert_eq!(report.outcome(check), Some(&Outcome::Passed), "{check}");
    }
}

#[tokio::test]
async fn qa_016_ac2_acp_adapter_passes_the_suite() {
    let adapter = AcpAdapter::new(AcpAgent {
        slug: "test".into(),
        config: AcpAgentConfig {
            command: "acp-test-agent".into(),
            ..AcpAgentConfig::default()
        },
        origin: Origin::User,
    })
    .unwrap();
    let report = passes(subject(Arc::new(adapter), "BETON_ACP_TEST_PATH", "acp")).await;
    for check in [
        "streaming",
        "interrupt",
        "approval_allow",
        "approval_deny",
        "resume",
    ] {
        assert_eq!(report.outcome(check), Some(&Outcome::Passed), "{check}");
    }
    // Nicht deklariert und korrekt abgelehnt.
    assert!(matches!(
        report.outcome("model_switch"),
        Some(Outcome::Skipped(_))
    ));
    assert!(matches!(
        report.outcome("usage_reporting"),
        Some(Outcome::Skipped(_))
    ));
}

#[tokio::test]
async fn qa_016_fake_harness_passes_the_suite() {
    passes(subject(
        Arc::new(FakeAdapter),
        "BETON_FAKE_PATH",
        "stream-json",
    ))
    .await;
}
