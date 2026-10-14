//! Kein Einmal-Modus für Codex (SES-010, HAR-027, #146): `codex exec` lässt sich nicht ohne
//! Tools starten, daher lehnt der Adapter Einmal-Aufrufe ab, ohne die CLI zu starten
//! (fail closed); `titles.generator: auto` fällt dann auf die Heuristik zurück.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use beton_harness::golden::{RawLine, ReplayLauncher};
use beton_harness::{
    AdapterContext, AllowAll, HarnessAdapter, HarnessError, HostEnv, OneShotRequest,
};
use beton_harness_codex::CodexAdapter;

/// Antwort im Format von `codex exec --json` (codex-cli 0.153.2): Käme der Aufruf zustande,
/// gäbe es einen Titel – der Test zeigt, dass er gar nicht erst startet.
const OK: [&str; 3] = [
    r#"{"type":"thread.started","thread_id":"01a1076a-0000-7000-8000-000000000000"}"#,
    r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Rate-Limiter für Login-Route"}}"#,
    r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":1000,"output_tokens":12}}"#,
];

fn launcher() -> ReplayLauncher {
    ReplayLauncher::new(
        OK.iter()
            .map(|l| RawLine {
                ms: 0,
                after_stdin: 0,
                out: (*l).to_owned(),
            })
            .collect(),
        0,
    )
}

#[tokio::test]
async fn ses_010_codex_one_shot_fails_closed_without_starting_the_cli() {
    let l = launcher();
    let ctx = AdapterContext {
        gate: Arc::new(AllowAll),
        launcher: Arc::new(l.clone()),
        env: HostEnv::default(),
    };
    let request = OneShotRequest {
        instructions: "Erzeuge einen kurzen Titel.".into(),
        prompt: "Die Login-Route braucht einen Rate-Limiter.".into(),
        workdir: std::env::temp_dir(),
        model: None,
        timeout: Duration::from_secs(10),
        scenario: None,
    };
    let err = CodexAdapter::default()
        .one_shot(&request, &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, HarnessError::OneShotUnsupported), "{err:?}");
    assert_eq!(err.code(), "capability_unsupported");
    // Kein Prozess: weder `codex exec` noch ein anderer Aufruf, auch kein stdin.
    assert!(l.launches().is_empty(), "{:?}", l.launches());
    assert!(l.stdin_lines().is_empty());
}
