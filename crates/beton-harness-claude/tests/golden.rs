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

// ------------------------------------------------------------- HAR-005: PreToolUse-Hook

/// Hook-Eingaben der echten CLI (2.1.296) einer Aufnahme.
fn hook_payloads(name: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(root().join(name).join("hooks.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn har_005_pretool_hook_deny_blocks_a_vendor_auto_allowed_call() {
    case("pretool-hook-deny").await;
    let events =
        std::fs::read_to_string(root().join("pretool-hook-deny/expected.events.jsonl")).unwrap();
    // Die CLI meldet die Ablehnung des Hooks als Tool-Fehler mit der Begründung.
    assert!(
        events.contains("PreToolUse:Bash hook error: nicht erlaubt"),
        "{events}"
    );
    assert!(!events.contains("\"type\":\"tool.call.started\""));
    let stdin =
        std::fs::read_to_string(root().join("pretool-hook-deny/expected.stdin.jsonl")).unwrap();
    assert!(
        !stdin.contains("\"behavior\""),
        "keine Rückfrage des Vendors"
    );
}

#[tokio::test]
async fn har_005_pretool_hook_allow_and_vendor_ask() {
    case("pretool-hook-allow").await;
    case("pretool-hook-vendor-ask").await;
    // `echo` erlaubt die CLI ohne Rückfrage: nur der Hook hat es geprüft.
    let allow =
        std::fs::read_to_string(root().join("pretool-hook-allow/expected.stdin.jsonl")).unwrap();
    assert!(!allow.contains("\"behavior\""), "{allow}");
    // `mkdir` fragt die CLI nach dem Hook zusätzlich per `can_use_tool`.
    let ask = std::fs::read_to_string(root().join("pretool-hook-vendor-ask/expected.stdin.jsonl"))
        .unwrap();
    assert!(ask.contains("\"behavior\":\"allow\""), "{ask}");
}

#[test]
fn har_005_recorded_hook_payloads_map_to_gate_requests() {
    for name in [
        "pretool-hook-deny",
        "pretool-hook-allow",
        "pretool-hook-vendor-ask",
    ] {
        let raw = std::fs::read_to_string(root().join(name).join("raw.jsonl")).unwrap();
        let payloads = hook_payloads(name);
        assert!(!payloads.is_empty(), "{name}");
        for payload in payloads {
            assert_eq!(payload["hook_event_name"], "PreToolUse");
            let req = beton_harness_claude::pre_tool_request(&payload, None);
            assert_eq!(req.tool, "Bash", "{name}");
            assert_eq!(req.kind, "shell", "{name}");
            assert_eq!(req.origin, beton_harness::GateOrigin::PreTool);
            assert!(req.args["command"].is_string(), "{name}");
            // Dieselbe Call-ID wie der `tool_use` im Strom der CLI.
            assert!(raw.contains(&req.call_id), "{name}: {}", req.call_id);
        }
    }
    // Antworten an die CLI: Ablehnung mit Begründung, Erlauben ohne Entscheidung.
    let deny = beton_harness_claude::pre_tool_reply(Some(beton_harness::GateDecision::Deny {
        reason: Some("nicht erlaubt".into()),
    }));
    assert_eq!(deny.exit, 0);
    let out = deny.stdout.unwrap();
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        out["hookSpecificOutput"]["permissionDecisionReason"],
        "nicht erlaubt"
    );
    let pass = beton_harness_claude::pre_tool_reply(Some(beton_harness::GateDecision::Allow {
        updated_args: None,
    }));
    assert!(pass.stdout.is_none());
    let none = beton_harness_claude::pre_tool_reply(None);
    assert_eq!(
        none.stdout.unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}

/// USE-004 gegen die echte CLI (2.1.296, Subscription): alle Abo-Fenster aus dem
/// `rate_limit_event`, Tokens je Modell ohne Geldbetrag.
#[tokio::test]
async fn use_004_ac1_ac2_subscription_usage_from_the_real_cli() {
    case("subscription-usage").await;
    let events =
        std::fs::read_to_string(root().join("subscription-usage/expected.events.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = events
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let windows: Vec<&str> = lines
        .iter()
        .filter(|e| e["type"] == "usage.subscription")
        .map(|e| e["payload"]["window"].as_str().unwrap())
        .collect();
    assert_eq!(windows, ["5h", "7d"]);
    let cost = lines.iter().find(|e| e["type"] == "cost.delta").unwrap();
    assert_eq!(cost["payload"]["source"], "subscription");
    assert_eq!(cost["payload"]["auth_source"], "vendor_cli");
    assert!(cost["payload"].get("cost_micro").is_none(), "{cost}");
    assert!(
        !lines.iter().any(|e| e["type"] == "harness.unmapped"),
        "{events}"
    );
}
