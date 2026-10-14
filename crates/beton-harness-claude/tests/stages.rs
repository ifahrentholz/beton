//! Zweistufige Sandbox (SBX-002): Das Bash-Tool von Claude Code läuft über
//! `CLAUDE_CODE_SHELL=<bt-exec-zsh|bt-exec-bash>` in Stufe 2, die cwd-Datei und
//! Hintergrund-Ausgaben liegen im Austausch mit Stufe 2 (`CLAUDE_CODE_TMPDIR`). Verifiziert
//! gegen claude 2.1.296; nicht `CLAUDE_CODE_SHELL_PREFIX`, der auch Hooks und MCP-Server
//! umhüllt.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beton_harness::process::{LaunchSpec, ProcessHandle, ProcessLauncher};
use beton_harness::{AdapterContext, AllowAll, HarnessAdapter, HostEnv, SessionSpec, StageEnv};
use beton_harness_claude::ClaudeAdapter;

#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<LaunchSpec>>>);

#[async_trait]
impl ProcessLauncher for Capture {
    async fn launch(&self, spec: LaunchSpec) -> std::io::Result<Box<dyn ProcessHandle>> {
        self.0.lock().unwrap().push(spec);
        Err(std::io::Error::other("nur Aufzeichnung"))
    }
}

async fn launched(stage: Option<StageEnv>) -> LaunchSpec {
    let cap = Capture::default();
    let ctx = AdapterContext {
        stage,
        gate: Arc::new(AllowAll),
        launcher: Arc::new(cap.clone()),
        env: HostEnv::default(),
    };
    let _ = ClaudeAdapter::default()
        .start(
            SessionSpec {
                workdir: std::env::temp_dir(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await;
    cap.0.lock().unwrap().pop().unwrap()
}

#[tokio::test]
async fn sbx_002_claude_bash_tool_goes_through_bt_exec_in_stage_two() {
    let stage = StageEnv {
        shim_dir: PathBuf::from("/sbx/exec/bin"),
        exchange: PathBuf::from("/sbx/tools"),
        tools: None,
    };
    let spec = launched(Some(stage)).await;
    let get = |k: &str| {
        spec.env
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
    };
    let shell = get("CLAUDE_CODE_SHELL").expect("CLAUDE_CODE_SHELL");
    assert!(
        shell == "/sbx/exec/bin/bt-exec-zsh" || shell == "/sbx/exec/bin/bt-exec-bash",
        "{shell}"
    );
    assert_eq!(get("CLAUDE_CODE_TMPDIR").as_deref(), Some("/sbx/tools"));
    // Nie der Präfix: Er würde Hooks und MCP-Relays in Stufe 2 schicken.
    assert!(get("CLAUDE_CODE_SHELL_PREFIX").is_none());
    // Ohne Stufenmodell bleibt alles beim Alten.
    let plain = launched(None).await;
    assert!(
        plain
            .env
            .iter()
            .all(|(k, _)| k != "CLAUDE_CODE_SHELL" && k != "CLAUDE_CODE_TMPDIR"),
        "{:?}",
        plain.env
    );
}
