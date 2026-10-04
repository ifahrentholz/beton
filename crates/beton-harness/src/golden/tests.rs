//! Tests des Golden-Frameworks mit einem minimalen Test-Adapter („echo“-Protokoll).
//! Die echten Adapter (Claude ab WP-08) nutzen dasselbe Framework.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use beton_core::event::{
    EventPayload, HarnessReady, HarnessUnmapped, MessageCompleted, MessageRole, RawJson,
    ToolCallCompleted, ToolCallRequested, ToolSource, ToolStatus, TurnCompleted,
};
use beton_core::id::TurnId;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use super::*;
use crate::adapter::{
    AuthStatus, ExitInfo, HarnessError, HarnessSession, Mode, PermissionMode, ProbeReport,
    SwitchOutcome, Transport,
};
use crate::capabilities::Capabilities;
use crate::id::HarnessId;
use crate::process::{LaunchSpec, ProcessHandle, ShutdownTimeouts};

fn case_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/golden/testdata/echo-basic")
}

/// Kopiert den Fall in ein temporäres Verzeichnis (für Tests, die Dateien ändern).
fn copy_case(skip_expected: bool) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(case_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if skip_expected && name.starts_with("expected.") {
            continue;
        }
        std::fs::copy(&path, tmp.path().join(name)).unwrap();
    }
    tmp
}

/// Test-Adapter für ein JSON-Zeilen-Protokoll ähnlich stream-json.
struct EchoAdapter;

#[async_trait]
impl HarnessAdapter for EchoAdapter {
    fn id(&self) -> HarnessId {
        "acp:echo".parse().unwrap()
    }
    fn modes(&self) -> &[Mode] {
        &[Mode::Native]
    }
    fn capabilities(&self, mode: Mode, _probe: &ProbeReport) -> Capabilities {
        let mut caps = Capabilities::minimal(mode, Transport::Native);
        caps.version_range = Some(">=1.0.0, <2.0.0".into());
        caps
    }
    async fn probe(&self, _env: &HostEnv) -> ProbeReport {
        ProbeReport {
            installed: true,
            version: Some("1.4.0".into()),
            auth_status: AuthStatus::NotApplicable,
            ..ProbeReport::default()
        }
    }
    async fn start(
        &self,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let mut process = ctx
            .launcher
            .launch(LaunchSpec {
                program: "echo-cli".into(),
                cwd: Some(spec.workdir.clone()),
                ..LaunchSpec::default()
            })
            .await?;
        let io = process.take_io().ok_or(HarnessError::Closed)?;
        let stdin: Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>> = Arc::new(Mutex::new(io.stdin));
        let (tx, rx) = mpsc::channel(256);
        let turn = Arc::new(Mutex::new(None::<TurnId>));
        let (reader_stdin, reader_turn, gate) = (stdin.clone(), turn.clone(), ctx.gate.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(io.stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let raw = RawJson::from_string(line.clone()).ok();
                let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let turn_id = *reader_turn.lock().await;
                let payload = match v["type"].as_str() {
                    Some("init") => EventPayload::HarnessReady(HarnessReady {
                        harness_session_ref: v["session"].as_str().map(str::to_owned),
                        ..HarnessReady::default()
                    }),
                    Some("text") => EventPayload::MessageCompleted(MessageCompleted {
                        message_id: "m1".into(),
                        role: MessageRole::Assistant,
                        content: vec![json!({"type": "text", "text": v["text"]})],
                        author: None,
                    }),
                    Some("ask") => {
                        let call_id = v["id"].as_str().unwrap_or_default().to_owned();
                        let _ = tx
                            .send(NormalizedEvent {
                                payload: EventPayload::ToolCallRequested(ToolCallRequested {
                                    call_id: call_id.clone(),
                                    tool: v["tool"].as_str().unwrap_or_default().into(),
                                    mcp_server: None,
                                    args: v["args"].clone(),
                                    source: ToolSource::Harness,
                                    parent_call_id: None,
                                }),
                                raw: raw.clone(),
                                turn_id,
                            })
                            .await;
                        let decision = gate
                            .decide(GateRequest {
                                turn_id,
                                call_id: call_id.clone(),
                                tool: v["tool"].as_str().unwrap_or_default().into(),
                                kind: "file_read".into(),
                                args: v["args"].clone(),
                            })
                            .await;
                        let allow = matches!(decision, GateDecision::Allow { .. });
                        let msg = json!({"type": "decision", "id": call_id, "allow": allow});
                        let _ = reader_stdin
                            .lock()
                            .await
                            .write_all(format!("{msg}\n").as_bytes())
                            .await;
                        continue;
                    }
                    Some("tool_result") => EventPayload::ToolCallCompleted(ToolCallCompleted {
                        call_id: v["id"].as_str().unwrap_or_default().into(),
                        status: ToolStatus::Ok,
                        result: Some(v["result"].clone()),
                        result_ref: None,
                        duration_ms: 0,
                    }),
                    Some("done") => EventPayload::TurnCompleted(TurnCompleted {
                        turn_id: turn_id.unwrap_or_default(),
                        stop_reason: "end_turn".into(),
                        usage_summary: json!({}),
                    }),
                    _ => EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v }),
                };
                let _ = tx
                    .send(NormalizedEvent {
                        payload,
                        raw,
                        turn_id,
                    })
                    .await;
            }
        });
        Ok(Box::new(EchoSession {
            process,
            stdin,
            turn,
            rx: Some(rx),
            workdir: spec.workdir.display().to_string(),
        }))
    }
}

struct EchoSession {
    process: Box<dyn ProcessHandle>,
    stdin: Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>,
    turn: Arc<Mutex<Option<TurnId>>>,
    rx: Option<mpsc::Receiver<NormalizedEvent>>,
    workdir: String,
}

#[async_trait]
impl HarnessSession for EchoSession {
    async fn send(&mut self, input: UserInput) -> Result<TurnId, HarnessError> {
        let id = TurnId::from_ulid(ulid::Ulid(1));
        *self.turn.lock().await = Some(id);
        let msg = json!({"type": "user", "text": input.text, "cwd": self.workdir});
        self.stdin
            .lock()
            .await
            .write_all(format!("{msg}\n").as_bytes())
            .await?;
        Ok(id)
    }
    async fn steer(&mut self, _: UserInput) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn interrupt(&mut self) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn set_model(
        &mut self,
        _: String,
        _: Option<String>,
    ) -> Result<SwitchOutcome, HarnessError> {
        Ok(SwitchOutcome::Live)
    }
    async fn set_permission_mode(&mut self, _: PermissionMode) -> Result<(), HarnessError> {
        Ok(())
    }
    async fn compact(&mut self) -> Result<(), HarnessError> {
        Ok(())
    }
    fn events(&mut self) -> Option<mpsc::Receiver<NormalizedEvent>> {
        self.rx.take()
    }
    fn native_session_ref(&self) -> Option<String> {
        None
    }
    async fn shutdown(mut self: Box<Self>, _how: Shutdown) -> Result<ExitInfo, HarnessError> {
        Ok(self.process.terminate(ShutdownTimeouts::default()).await?)
    }
}

#[tokio::test]
async fn har_025_ac1_golden_case_replays_offline() {
    run_case(&case_dir(), &EchoAdapter).await.unwrap();
    let events = std::fs::read_to_string(case_dir().join("expected.events.jsonl")).unwrap();
    assert!(events.contains("<workdir>/README.md"), "Pfade normalisiert");
    assert!(events.contains("<id:"), "IDs normalisiert");
    let stdin = std::fs::read_to_string(case_dir().join("expected.stdin.jsonl")).unwrap();
    assert!(stdin.contains("\"allow\":true"), "Gate-Antwort an die CLI");
}

#[tokio::test]
async fn har_025_ac1_mismatch_shows_readable_event_diff() {
    let tmp = copy_case(false);
    let path = tmp.path().join("expected.events.jsonl");
    let changed = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Ich lese die Datei.", "Ich lese etwas anderes.");
    std::fs::write(&path, changed).unwrap();
    let err = check_case(tmp.path(), &EchoAdapter, false)
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("--- expected.events.jsonl (erwartet)"),
        "{text}"
    );
    assert!(
        text.contains(" - ") && text.contains("Ich lese etwas anderes."),
        "{text}"
    );
    assert!(
        text.contains(" + ") && text.contains("Ich lese die Datei."),
        "{text}"
    );
    assert!(text.contains(BLESS_ENV));
}

#[tokio::test]
async fn qa_003_ac1_changed_expectation_needs_explicit_bless() {
    let tmp = copy_case(true);
    let err = check_case(tmp.path(), &EchoAdapter, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("fehlt"), "{err}");
    check_case(tmp.path(), &EchoAdapter, true).await.unwrap();
    check_case(tmp.path(), &EchoAdapter, false).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("expected.events.jsonl")).unwrap(),
        std::fs::read_to_string(case_dir().join("expected.events.jsonl")).unwrap()
    );
}

#[test]
fn normalizer_replaces_ids_timestamps_and_workdir() {
    let mut n = Normalizer::new("/w");
    let line = r#"{"a":"9f1c2d3e-1111-2222-3333-444455556666","b":"ses_01JB8Y2D0M3K4J5H6G7F8E9D0C","c":"9f1c2d3e-1111-2222-3333-444455556666","t":"2026-10-03T12:00:00.123Z","p":"/w/x.rs"}"#;
    assert_eq!(
        n.line(line),
        r#"{"a":"<id:1>","b":"<id:2>","c":"<id:1>","p":"<workdir>/x.rs","t":"<ts>"}"#
    );
}

#[test]
fn har_025_ac2_recording_with_leftover_key_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = Meta {
        harness: "claude".into(),
        cli_version: "2.1.0".into(),
        ..Meta::default()
    };
    let leaked = RawLine {
        ms: 1,
        after_stdin: 0,
        out: format!(
            r#"{{"env":"ANTHROPIC_API_KEY=sk-ant-oat01-{}"}}"#,
            "q".repeat(12)
        ),
    };
    let dir = tmp.path().join("fall");
    let err = write_recording(&dir, &meta, &Script::default(), &[leaked], &[]).unwrap_err();
    assert!(err.to_string().contains("Secrets"), "{err}");
    assert!(!dir.exists(), "nichts geschrieben");

    let known = format!("sk-ant-oat01-{}", "q".repeat(12));
    let ok = RawLine {
        ms: 1,
        after_stdin: 0,
        out: format!(r#"{{"env":"{known}","cred":"bt_cred_github_1"}}"#),
    };
    write_recording(
        &dir,
        &meta,
        &Script::default(),
        &[ok],
        std::slice::from_ref(&known),
    )
    .unwrap();
    let raw = std::fs::read_to_string(dir.join("raw.jsonl")).unwrap();
    assert!(!raw.contains(&known));
    assert!(raw.contains("<secret:1>") && raw.contains("<bt_cred>"));
}

#[test]
fn har_025_ac4_recorded_versions_match_catalog() {
    let root = case_dir().parent().unwrap().to_path_buf();
    let mut caps = Capabilities::minimal(Mode::Native, Transport::Native);
    caps.version_range = Some(">=1.0.0, <2.0.0".into());
    assert!(check_versions(&caps, &["1.4.0"], &root).unwrap().is_empty());

    caps.version_range = Some(">=2.0.0".into());
    let problems = check_versions(&caps, &["1.4.0", "1.5.0"], &root).unwrap();
    assert!(
        problems.iter().any(|p| p.contains("liegt nicht in")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("keine Aufnahme für die unterstützte Version 1.5.0")),
        "{problems:?}"
    );
}
