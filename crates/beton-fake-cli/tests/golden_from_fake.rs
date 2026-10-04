//! Golden-Fälle aus der Fake-CLI (HAR-025 AC3, HAR-007 AC1).
//!
//! - ACP: Der deterministische ACP-Test-Agent ist Teil der Testsuite (HAR-007 AC1); seine
//!   Aufnahmen sind die Golden-Fälle von `beton-harness-acp`.
//! - Codex: Fälle, die mit der echten CLI nicht aufgenommen wurden, entstehen aus der
//!   Fake-CLI nach dem exportierten Protokoll-Schema (`recorded_with` in `meta.yaml`).
//!
//! Neu erzeugen (schreibt in die Golden-Ordner der Adapter-Crates):
//! `BETON_RECORD_FAKE_GOLDEN=1 cargo test -p beton-fake-cli --test golden_from_fake -- --ignored`

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_harness::golden::{self, Meta, RecordingLauncher, Script, ScriptDecision, ScriptInput};
use beton_harness::process::RealLauncher;
use beton_harness::registry::AcpAgentConfig;
use beton_harness::{HarnessAdapter, HostEnv};
use beton_harness_acp::{AcpAdapter, AcpAgent, Origin};
use beton_harness_codex::CodexAdapter;

fn fake_cli() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Der ACP-Test-Agent als Harness `acp:test` (auch in den Golden-Tests des ACP-Crates).
pub fn acp_test_adapter() -> AcpAdapter {
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

struct Case {
    name: &'static str,
    description: &'static str,
    scenario: &'static str,
    inputs: Vec<ScriptInput>,
    gate: Vec<ScriptDecision>,
}

fn send(text: &str) -> ScriptInput {
    ScriptInput {
        send: Some(text.into()),
        interrupt_after_events: None,
    }
}

const TOOL: &str = r#"
turns:
  - emit:
      - { message_delta: "Ich lege den Ordner an.", chunk: 8 }
      - { tool_call: { name: "mkdir beton-ordner", kind: shell, args: { command: "mkdir beton-ordner" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "" }, { message: "erledigt." }], deny: [{ message: "abgelehnt." }] } }
"#;

fn acp_cases() -> Vec<Case> {
    use ScriptDecision::{Allow, Deny};
    vec![
        Case {
            name: "text",
            description: "Reiner Text-Turn mit Überlegung",
            scenario: "turns:\n  - emit:\n      - { reasoning: \"Kurz überlegen.\" }\n      - { message_delta: \"Beton\", chunk: 2 }\n",
            inputs: vec![send("Antworte nur mit dem Wort: Beton")],
            gate: vec![],
        },
        Case {
            name: "tool-call",
            description: "Tool-Call mit Permission-Request, erlaubt",
            scenario: TOOL,
            inputs: vec![send("Lege den Ordner beton-ordner an.")],
            gate: vec![Allow],
        },
        Case {
            name: "permission-deny",
            description: "Permission-Request abgelehnt",
            scenario: TOOL,
            inputs: vec![send("Lege den Ordner beton-ordner an.")],
            gate: vec![Deny],
        },
        Case {
            name: "interrupt",
            description: "Cancel während der Antwort",
            scenario: "turns:\n  - emit:\n      - { message_delta: \"1 2 3 4 5 6 7 8 9\", chunk: 2 }\n      - { hang: true }\n",
            inputs: vec![ScriptInput {
                send: Some("Zähle bis 300.".into()),
                interrupt_after_events: Some(3),
            }],
            gate: vec![],
        },
        Case {
            name: "error",
            description: "Fehlerantwort auf session/prompt",
            scenario: "turns:\n  - emit:\n      - { error: \"Interner Fehler des Agents\" }\n",
            inputs: vec![send("Antworte nur mit: ok")],
            gate: vec![],
        },
    ]
}

fn codex_cases() -> Vec<Case> {
    use ScriptDecision::Allow;
    vec![
        Case {
            name: "file-edit",
            description: "Turn mit Datei-Änderung (fileChange)",
            scenario: r#"
turns:
  - emit:
      - { tool_call: { name: apply_patch, kind: file_edit, args: { path: "notiz.txt", diff: "-Das ist alt.\n+Das ist neu.\n" } }, gate: true }
      - { tool_result: "ok" }
      - { message_delta: "fertig.", chunk: 4 }
      - { usage: { input_tokens: 2100, cache_read_tokens: 1800, output_tokens: 60 } }
"#,
            inputs: vec![send("Ersetze in notiz.txt das Wort alt durch neu.")],
            gate: vec![Allow],
        },
        Case {
            name: "reasoning",
            description: "Turn mit Reasoning-Zusammenfassung",
            scenario: r#"
turns:
  - emit:
      - { reasoning: "17 mal 23 ist 391." }
      - { message_delta: "391", chunk: 2 }
      - { usage: { input_tokens: 900, output_tokens: 30 } }
"#,
            inputs: vec![send("Was ist 17 mal 23? Antworte nur mit der Zahl.")],
            gate: vec![],
        },
    ]
}

async fn record(
    adapter: &dyn HarnessAdapter,
    var: &str,
    protocol: &str,
    harness: &str,
    version: &str,
    root: &Path,
    case: &Case,
) {
    let work = tempfile::tempdir().unwrap();
    let scenario = work.path().join("szenario.yaml");
    std::fs::write(&scenario, case.scenario).unwrap();
    let workdir = work.path().join("workdir");
    std::fs::create_dir(&workdir).unwrap();
    let mut env = HostEnv::default();
    env.vars.insert(
        var.into(),
        format!(
            "{} --protocol {protocol} --scenario {}",
            fake_cli(),
            scenario.display()
        ),
    );
    let script = Script {
        model: None,
        inputs: case.inputs.clone(),
        gate: case.gate.clone(),
    };
    let launcher = RecordingLauncher::new(Arc::new(RealLauncher));
    golden::drive(
        adapter,
        &script,
        Arc::new(launcher.clone()),
        env,
        workdir.clone(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    let raw: Vec<golden::RawLine> = launcher
        .raw_lines()
        .into_iter()
        .map(|mut l| {
            for from in [workdir.canonicalize().unwrap(), workdir.clone()] {
                l.out = l
                    .out
                    .replace(&from.display().to_string(), golden::GOLDEN_WORKDIR);
            }
            l
        })
        .collect();
    let meta = Meta {
        harness: harness.into(),
        cli_version: version.into(),
        recorded_at: beton_core::time::Timestamp::now().to_string(),
        scenario: case.description.into(),
        platform: "beton-fake-cli".into(),
        exit_code: 0,
        recorded_with: Some(format!("beton-fake-cli --protocol {protocol}")),
    };
    let dir = root.join(case.name);
    let _ = std::fs::remove_dir_all(&dir);
    golden::write_recording(&dir, &meta, &script, &raw, &[]).unwrap();
    golden::check_case(&dir, adapter, true).await.unwrap();
}

#[tokio::test]
#[ignore = "erzeugt Golden-Fälle; nur mit BETON_RECORD_FAKE_GOLDEN=1"]
async fn record_golden_cases_from_fake_cli() {
    if std::env::var("BETON_RECORD_FAKE_GOLDEN").as_deref() != Ok("1") {
        return;
    }
    let acp = acp_test_adapter();
    let root = crates().join("beton-harness-acp/tests/golden");
    for case in acp_cases() {
        record(
            &acp,
            "BETON_ACP_TEST_PATH",
            "acp",
            "acp:test",
            "0.1.0",
            &root,
            &case,
        )
        .await;
    }
    let codex = CodexAdapter::default();
    let root = crates().join("beton-harness-codex/tests/golden");
    for case in codex_cases() {
        record(
            &codex,
            "BETON_CODEX_PATH",
            "app-server",
            "codex",
            beton_harness_codex::TESTED_VERSIONS[0],
            &root,
            &case,
        )
        .await;
    }
}
