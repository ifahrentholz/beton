//! Einmal-Modus `codex exec` für Session-Titel (SES-010, ADR-0034).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use beton_core::event::{AuthSource, CostSource};
use beton_harness::golden::{RawLine, ReplayLauncher};
use beton_harness::{AdapterContext, AllowAll, HarnessAdapter, HostEnv, OneShotRequest};
use beton_harness_codex::{CodexAdapter, SUBSCRIPTION_ENV_REMOVE};

/// Erfolgsfall im Format von `codex exec --json` (Ereignisse laut codex-cli 0.153.2).
const OK: [&str; 4] = [
    r#"{"type":"thread.started","thread_id":"01a1076a-0000-7000-8000-000000000000"}"#,
    r#"{"type":"turn.started"}"#,
    r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Rate-Limiter für Login-Route"}}"#,
    r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":1000,"output_tokens":12}}"#,
];

/// Aufgezeichneter Fehlerfall (codex-cli 0.153.2, Modell vom Konto nicht unterstützt).
const FAILED: [&str; 4] = [
    r#"{"type":"thread.started","thread_id":"01a1076a-2e29-7ae0-a05c-c58e39f7fa32"}"#,
    r#"{"type":"turn.started"}"#,
    r#"{"type":"error","message":"The model is not supported when using Codex with a ChatGPT account."}"#,
    r#"{"type":"turn.failed","error":{"message":"The model is not supported when using Codex with a ChatGPT account."}}"#,
];

fn launcher(lines: &[&str]) -> ReplayLauncher {
    ReplayLauncher::new(
        lines
            .iter()
            .map(|l| RawLine {
                ms: 0,
                after_stdin: 0,
                out: (*l).to_owned(),
            })
            .collect(),
        0,
    )
}

fn ctx(l: &ReplayLauncher) -> AdapterContext {
    AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(l.clone()),
        env: HostEnv::default(),
    }
}

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

#[tokio::test]
async fn ses_010_title_via_codex_exec_with_subscription_env() {
    let l = launcher(&OK);
    let reply = CodexAdapter::default()
        .one_shot(&request(), &ctx(&l))
        .await
        .unwrap();
    assert_eq!(reply.text, "Rate-Limiter für Login-Route");
    let spec = &l.launches()[0];
    let args = spec.args.join(" ");
    assert!(
        args.starts_with("exec --json --ephemeral --skip-git-repo-check --sandbox read-only"),
        "{args}"
    );
    assert_eq!(
        spec.args.last().map(String::as_str),
        Some("-"),
        "Inhalt über stdin"
    );
    assert!(!args.contains("Login-Route"));
    let stdin = l.stdin_lines().join("\n");
    assert!(stdin.contains("Erzeuge einen kurzen Titel."), "{stdin}");
    assert!(stdin.contains("Die Login-Route braucht"), "{stdin}");
    for key in SUBSCRIPTION_ENV_REMOVE {
        assert!(spec.env_remove.iter().any(|k| k == key), "{key}");
    }
    let cost = reply.cost.unwrap();
    assert_eq!(cost.source, CostSource::Subscription);
    assert_eq!(cost.auth_source, AuthSource::VendorCli);
    assert_eq!(
        (
            cost.input_tokens,
            cost.cache_read_tokens,
            cost.output_tokens
        ),
        (200, 1000, 12)
    );
}

#[tokio::test]
async fn ses_010_codex_exec_failure_is_an_error() {
    let l = launcher(&FAILED);
    let err = CodexAdapter::default()
        .one_shot(&request(), &ctx(&l))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "protocol_error");
}
