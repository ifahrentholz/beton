//! Hook-Brücke (HAR-005, HAR-013): Server im Adapter, Kommando `beton hook …` als Client.
//! Fail closed: Jeder Fehler blockiert den Tool-Call (Exit-Code 2).

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use beton_harness::hook::{EXIT_BLOCK, HookCommand, HookHandler, HookReply, HookServer, client};
use serde_json::{Value, json};

struct Echo;

#[async_trait]
impl HookHandler for Echo {
    async fn handle(&self, event: &str, payload: Value) -> HookReply {
        HookReply {
            exit: 0,
            stdout: Some(json!({ "event": event, "tool": payload["tool_name"] })),
            stderr: None,
        }
    }
}

fn args(server: &HookServer, token_file: Option<&std::path::Path>) -> Vec<String> {
    vec![
        "--event".into(),
        "PreToolUse".into(),
        "--socket".into(),
        server.socket().display().to_string(),
        "--token-file".into(),
        token_file
            .unwrap_or(server.token_file())
            .display()
            .to_string(),
    ]
}

#[tokio::test]
async fn har_005_hook_roundtrip_returns_the_handler_output() {
    let dir = tempfile::tempdir().unwrap();
    let server = HookServer::start(dir.path(), Arc::new(Echo)).await.unwrap();
    let input = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#;
    let a = args(&server, None);
    let out = tokio::task::spawn_blocking(move || client(&a, input))
        .await
        .unwrap();
    assert_eq!(out.exit, 0, "{out:?}");
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v, json!({ "event": "PreToolUse", "tool": "Bash" }));
    // Socket und Token-Datei sind privat.
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(server.token_file()), 0o600);
    assert_eq!(mode(server.socket().parent().unwrap()), 0o700);
}

#[tokio::test]
async fn har_013_ac3_unreachable_runner_or_bad_token_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let server = HookServer::start(dir.path(), Arc::new(Echo)).await.unwrap();
    let input: &'static [u8] = br#"{"tool_name":"Bash"}"#;
    // Falsches Token.
    let fake_token = dir.path().join("falsch.token");
    std::fs::write(&fake_token, "00").unwrap();
    let a = args(&server, Some(&fake_token));
    let out = tokio::task::spawn_blocking(move || client(&a, input))
        .await
        .unwrap();
    assert_eq!(out.exit, EXIT_BLOCK, "{out:?}");
    // Kaputte Eingabe.
    let a = args(&server, None);
    let out = tokio::task::spawn_blocking(move || client(&a, b"kein json"))
        .await
        .unwrap();
    assert_eq!(out.exit, EXIT_BLOCK);
    // Runner weg: Server beendet, Socket entfernt.
    let a = args(&server, None);
    drop(server);
    let out = tokio::task::spawn_blocking(move || client(&a, input))
        .await
        .unwrap();
    assert_eq!(out.exit, EXIT_BLOCK);
    assert!(out.stderr.contains("beton"), "{out:?}");
    // Fehlende Argumente.
    let out = client(&[], input);
    assert_eq!(out.exit, EXIT_BLOCK);
}

#[tokio::test]
async fn har_005_command_line_is_shell_quoted_and_contains_no_token() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("mit leerzeichen");
    let server = HookServer::start(&base, Arc::new(Echo)).await.unwrap();
    let cmd = HookCommand {
        program: PathBuf::from("/opt/be ton/beton"),
        prefix: Vec::new(),
    };
    let line = server.command_line(&cmd, "claude", "PreToolUse");
    assert!(
        line.starts_with("'/opt/be ton/beton' hook claude --event PreToolUse --socket '"),
        "{line}"
    );
    let token = std::fs::read_to_string(server.token_file()).unwrap();
    assert!(!line.contains(token.trim()));
}

/// #247 (HAR-005): Kennt der Server den Prozess der CLI, nimmt er nur Verbindungen von ihm und
/// seinen Nachfahren an (Peer-Credentials: gleiche UID, PID im Prozessbaum der CLI). Ein
/// fremder Prozess derselben UID mit gültigem Token wird abgelehnt.
#[tokio::test]
async fn har_005_hook_server_accepts_only_descendants_of_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let server = HookServer::start(dir.path(), Arc::new(Echo)).await.unwrap();
    let input: &'static [u8] = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#;
    // Ein unabhängiger Prozess als „CLI“: Der Test selbst ist kein Nachfahre davon.
    let mut cli = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    server.restrict_to_descendants_of(cli.id());
    let a = args(&server, None);
    let out = tokio::task::spawn_blocking(move || client(&a, input))
        .await
        .unwrap();
    assert_eq!(out.exit, EXIT_BLOCK, "fremder Prozess mit Token: {out:?}");
    let _ = cli.kill();
    let _ = cli.wait();

    // Die CLI selbst (hier: der Testprozess) und ihre Nachfahren dürfen.
    let server = HookServer::start(dir.path(), Arc::new(Echo)).await.unwrap();
    server.restrict_to_descendants_of(std::process::id());
    let a = args(&server, None);
    let out = tokio::task::spawn_blocking(move || client(&a, input))
        .await
        .unwrap();
    assert_eq!(out.exit, 0, "{out:?}");
}

/// #247: Eltern-PIDs fremder Prozesse (Grundlage der Prüfung).
#[test]
fn har_005_ancestry_of_a_child_reaches_this_process() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    assert!(beton_sandbox::ancestry::is_descendant(
        child.id(),
        std::process::id()
    ));
    assert!(beton_sandbox::ancestry::is_descendant(
        std::process::id(),
        std::process::id()
    ));
    assert!(!beton_sandbox::ancestry::is_descendant(
        std::process::id(),
        child.id()
    ));
    let _ = child.kill();
    let _ = child.wait();
}
