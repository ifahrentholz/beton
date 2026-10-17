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
    launched_with(ClaudeAdapter::default(), stage).await
}

async fn launched_with(adapter: ClaudeAdapter, stage: Option<StageEnv>) -> LaunchSpec {
    let cap = Capture::default();
    let ctx = AdapterContext {
        stage,
        gate: Arc::new(AllowAll),
        launcher: Arc::new(cap.clone()),
        env: HostEnv::default(),
    };
    let _ = adapter
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
        sandbox: Default::default(),
        harness_policy: None,
        terminals: None,
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

/// Zusätzlich über `--settings` (`env`): Verifiziert gegen 2.1.296 gewinnt zwar die
/// Prozess-Umgebung gegen `env` aus User-, Projekt- und `--settings`-Einstellungen; ändert eine
/// spätere Version das, gewinnt unter den Einstellungen `--settings` (Security-Review #228).
/// Auch mit `isolated: true` (`--safe-mode` wertet `--settings` → `env` aus, verifiziert).
#[tokio::test]
async fn sbx_002_stage_vars_also_go_through_settings_env() {
    for isolated in [false, true] {
        let stage = StageEnv {
            shim_dir: PathBuf::from("/sbx/exec/bin"),
            exchange: PathBuf::from("/sbx/xchg"),
            tools: None,
            sandbox: Default::default(),
            harness_policy: None,
            terminals: None,
        };
        let spec = launched_with(
            ClaudeAdapter {
                isolated,
                ..ClaudeAdapter::default()
            },
            Some(stage),
        )
        .await;
        let settings: Vec<&String> = spec
            .args
            .iter()
            .enumerate()
            .filter(|(i, _)| *i > 0 && spec.args[i - 1] == "--settings")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(settings.len(), 1, "isolated={isolated}: {:?}", spec.args);
        let json: serde_json::Value = serde_json::from_str(settings[0]).unwrap();
        for (k, v) in &spec.env {
            if k.starts_with("CLAUDE_CODE_") {
                assert_eq!(json["env"][k], v.as_str(), "isolated={isolated}: {k}");
            }
        }
        assert!(json["env"]["CLAUDE_CODE_SHELL"].is_string(), "{json}");
    }
    // Ohne Stufenmodell keine Settings (isoliert) bzw. nur der Hook.
    let plain = launched_with(
        ClaudeAdapter {
            isolated: true,
            ..ClaudeAdapter::default()
        },
        None,
    )
    .await;
    assert!(
        !plain.args.iter().any(|a| a == "--settings"),
        "{:?}",
        plain.args
    );
}

/// #258 R5: Managed Settings haben Vorrang vor `--settings`; `disableAllHooks` bzw.
/// `allowManagedHooksOnly` schalten den `PreToolUse`-Hook von beton still ab. Der Adapter liest
/// die bekannten Pfade (Datei und `managed-settings.d/`) und meldet dann nur noch
/// `approval_only` (POL-024: Warnung bzw. Startverweigerung), ebenso bei einer nicht lesbaren
/// Datei (fail closed). Ob die echte CLI weitere Quellen kennt (MDM), prüft #260.
#[test]
fn pol_024_managed_settings_that_disable_hooks_degrade_the_tool_gate() {
    use beton_harness::{Mode, ProbeReport, ToolCallGate};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("managed-settings.json");
    let drop_in = dir.path().join("managed-settings.d");
    let adapter = ClaudeAdapter {
        managed_settings: vec![file.clone(), drop_in.clone()],
        ..ClaudeAdapter::default()
    };
    let gate = || {
        adapter
            .capabilities(Mode::Native, &ProbeReport::default())
            .tool_call_gate
    };
    assert_eq!(gate(), ToolCallGate::Full, "ohne Managed Settings");
    for (text, expected) in [
        (r#"{"model": "claude-sonnet-4-5"}"#, ToolCallGate::Full),
        (r#"{"disableAllHooks": false}"#, ToolCallGate::Full),
        (r#"{"disableAllHooks": true}"#, ToolCallGate::ApprovalOnly),
        (
            r#"{"allowManagedHooksOnly": true}"#,
            ToolCallGate::ApprovalOnly,
        ),
        ("{ kaputt", ToolCallGate::ApprovalOnly),
    ] {
        std::fs::write(&file, text).unwrap();
        assert_eq!(gate(), expected, "{text}");
    }
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(&drop_in).unwrap();
    std::fs::write(
        drop_in.join("10-hooks.json"),
        r#"{"disableAllHooks": true}"#,
    )
    .unwrap();
    assert_eq!(gate(), ToolCallGate::ApprovalOnly, "managed-settings.d");
}
