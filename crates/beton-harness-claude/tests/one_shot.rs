//! Einmal-Modus `claude -p` für Session-Titel (SES-010, ADR-0034): Aufruf, bereinigte
//! Umgebung und Auswertung, gegen eine aufgezeichnete Antwort von Claude Code 2.1.285.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use beton_core::event::{AuthSource, CostSource};
use beton_harness::golden::{RawLine, ReplayLauncher};
use beton_harness::{AdapterContext, AllowAll, HarnessAdapter, HostEnv, OneShotRequest};
use beton_harness_claude::{ClaudeAdapter, SUBSCRIPTION_ENV_REMOVE};

/// Antwort von `claude -p --output-format json --model haiku` (2.1.285, gekürzt).
const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":4410,"duration_api_ms":4281,"num_turns":1,"result":"Rate-Limiter für Login-Route","stop_reason":"end_turn","session_id":"cdc5c966-11eb-498d-b5af-5f9c84c50e37","total_cost_usd":0.024157,"usage":{"input_tokens":9,"cache_creation_input_tokens":10774,"cache_read_input_tokens":0,"output_tokens":520},"modelUsage":{"claude-haiku-4-5-20251001":{"inputTokens":9,"outputTokens":520,"cacheReadInputTokens":0,"cacheCreationInputTokens":10774,"costUSD":0.024157,"contextWindow":200000}},"permission_denials":[]}"#;

fn request() -> OneShotRequest {
    OneShotRequest {
        instructions: "Erzeuge einen kurzen Titel.".into(),
        prompt: "Die Login-Route braucht einen Rate-Limiter.".into(),
        workdir: std::env::temp_dir(),
        model: None,
        timeout: Duration::from_secs(10),
        scenario: None,
    }
}

fn replay(line: &str) -> ReplayLauncher {
    ReplayLauncher::new(
        vec![RawLine {
            ms: 0,
            after_stdin: 0,
            out: line.to_owned(),
        }],
        0,
    )
}

fn ctx(launcher: ReplayLauncher) -> AdapterContext {
    AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(launcher),
        env: HostEnv::default(),
    }
}

#[tokio::test]
async fn ses_010_ac4_title_via_claude_print_mode_with_subscription_env() {
    let launcher = replay(RESULT);
    let adapter = ClaudeAdapter::default();
    assert_eq!(
        adapter.auth,
        AuthSource::VendorCli,
        "Default ist die Subscription"
    );
    let reply = adapter
        .one_shot(&request(), &ctx(launcher.clone()))
        .await
        .unwrap();
    assert_eq!(reply.text, "Rate-Limiter für Login-Route");
    assert_eq!(reply.model, "claude-haiku-4-5-20251001");

    // Genau ein Aufruf: nicht-interaktiver Modus der CLI, kleinstes Modell, keine Tools.
    let launches = launcher.launches();
    assert_eq!(launches.len(), 1);
    let spec = &launches[0];
    assert_eq!(spec.program.to_str(), Some("claude"));
    let args = spec.args.join(" ");
    for want in [
        "-p",
        "--output-format json",
        "--no-session-persistence",
        "--model haiku",
        "--system-prompt Erzeuge einen kurzen Titel.",
    ] {
        assert!(args.contains(want), "{want} fehlt in {args}");
    }
    let tools = spec.args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(spec.args[tools + 1], "", "keine Tools");
    // Der Inhalt geht über stdin, nicht über argv (Prozessliste).
    assert!(!args.contains("Login-Route"));
    assert_eq!(
        launcher.stdin_lines(),
        vec!["Die Login-Route braucht einen Rate-Limiter.".to_owned()]
    );
    // Bereinigte Umgebung: keine API-Keys in den CLI-Prozess (HAR-015, ADR-0034).
    for key in SUBSCRIPTION_ENV_REMOVE {
        assert!(spec.env_remove.iter().any(|k| k == key), "{key}");
    }
    assert!(spec.env.is_empty());
    assert!(!spec.clear_env, "die Anmeldung der CLI bleibt erreichbar");
}

#[tokio::test]
async fn ses_010_ac3_one_shot_reports_usage_as_subscription() {
    let reply = ClaudeAdapter::default()
        .one_shot(&request(), &ctx(replay(RESULT)))
        .await
        .unwrap();
    let cost = reply.cost.unwrap();
    assert_eq!(cost.harness, "claude");
    assert_eq!(cost.source, CostSource::Subscription);
    assert_eq!(cost.auth_source, AuthSource::VendorCli);
    assert_eq!(cost.cost_micro, None, "Subscription: keine Ausgabe");
    assert_eq!(cost.output_tokens, 520);
    assert_eq!(cost.cache_write_tokens, 10774);
}

#[tokio::test]
async fn ses_010_one_shot_errors_are_reported() {
    let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
    let err = ClaudeAdapter::default()
        .one_shot(&request(), &ctx(replay(failed)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "protocol_error");
    let err = ClaudeAdapter::default()
        .one_shot(&request(), &ctx(replay("kein json")))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "protocol_error");
}

#[tokio::test]
async fn ses_010_api_key_auth_keeps_the_environment() {
    let launcher = replay(RESULT);
    let adapter = ClaudeAdapter {
        auth: AuthSource::ApiKey,
        ..ClaudeAdapter::default()
    };
    let reply = adapter
        .one_shot(&request(), &ctx(launcher.clone()))
        .await
        .unwrap();
    assert!(launcher.launches()[0].env_remove.is_empty());
    let cost = reply.cost.unwrap();
    assert_eq!(cost.source, CostSource::Reported);
    assert_eq!(cost.cost_micro, Some(24157));
}

#[test]
fn har_027_one_shot_never_runs_in_a_looser_mode_or_with_tools() {
    // Wie die Session (HAR-027): Modus ausdrücklich, sonst gälte `permissions.defaultMode`
    // aus `.claude/settings.json` des Repositorys. `dontAsk` lehnt alles ab, was nicht vorab
    // erlaubt ist; ohne eingebaute Tools und ohne MCP-Server des Nutzers bzw. Projekts gibt
    // es nichts auszuführen.
    for isolated in [false, true] {
        let args = beton_harness_claude::one_shot_args(&request(), isolated);
        let after = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
        };
        assert_eq!(after("--permission-mode"), Some("dontAsk"), "{args:?}");
        assert_eq!(after("--tools"), Some(""));
        assert!(args.iter().any(|a| a == "--strict-mcp-config"), "{args:?}");
        assert!(
            !args
                .iter()
                .any(|a| a.contains("bypass") || a == "--mcp-config")
        );
    }
}
