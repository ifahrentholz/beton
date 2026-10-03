//! QA-002: Die Fake-CLI spricht stream-json deterministisch und kann Fehler injizieren.

#![allow(clippy::unwrap_used)] // Testcode: Panics sind hier die Fehlermeldung.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-fake-cli")
}

fn push_ask() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake/push-ask.yaml")
}

/// Führt die CLI wie ein Adapter: Initialisierung, Nutzereingabe, Gate-Antwort.
/// Liefert stdout und Exit-Code.
fn drive(scenario: &PathBuf, extra: &[&str], allow: bool) -> (String, Option<i32>) {
    let mut child = Command::new(bin())
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
        ])
        .args(["--protocol", "stream-json", "--scenario"])
        .arg(scenario)
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let send = |stdin: &mut std::process::ChildStdin, v: Value| {
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        &mut stdin,
        json!({"type": "control_request", "request_id": "init_1", "request": {"subtype": "initialize"}}),
    );
    send(
        &mut stdin,
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "Bitte pushen"}]}}),
    );
    let mut out = String::new();
    for line in stdout.lines() {
        let line = line.unwrap();
        out.push_str(&line);
        out.push('\n');
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v["type"] == "control_request" && v["request"]["subtype"] == "can_use_tool" {
            let behavior = if allow {
                json!({"behavior": "allow", "updatedInput": v["request"]["input"]})
            } else {
                json!({"behavior": "deny", "message": "abgelehnt"})
            };
            send(
                &mut stdin,
                json!({"type": "control_response", "response": {
                "subtype": "success", "request_id": v["request_id"], "response": behavior}}),
            );
        }
        if v["type"] == "result" {
            break;
        }
    }
    drop(stdin);
    let status = child.wait().unwrap();
    (out, status.code())
}

fn lines(out: &str) -> Vec<Value> {
    out.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn qa_002_stream_json_tool_approval_flow() {
    let (out, code) = drive(&push_ask(), &[], true);
    assert_eq!(code, Some(0));
    let types: Vec<String> = lines(&out)
        .iter()
        .map(|v| {
            format!(
                "{}/{}",
                v["type"].as_str().unwrap(),
                v["subtype"]
                    .as_str()
                    .or(v["request"]["subtype"].as_str())
                    .unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        types,
        [
            "control_response/",
            "system/init",
            "assistant/",
            "assistant/",
            "control_request/can_use_tool",
            "user/",
            "result/success",
        ]
    );
    let result = lines(&out).pop().unwrap();
    assert_eq!(result["total_cost_usd"], 0.01);
    assert_eq!(result["usage"]["input_tokens"], 1200);

    let (denied, _) = drive(&push_ask(), &[], false);
    assert!(denied.contains("Permission denied"));
    assert!(denied.contains("Push abgelehnt."));
}

#[test]
fn qa_002_partial_messages_stream_deltas() {
    let (out, _) = drive(&push_ask(), &["--include-partial-messages"], true);
    let deltas = lines(&out)
        .iter()
        .filter(|v| v["type"] == "stream_event")
        .count();
    assert_eq!(deltas, 4, "16 Zeichen in Stücken von 4");
}

#[test]
fn qa_002_ac2_identical_output_over_100_runs() {
    let (first, _) = drive(&push_ask(), &[], true);
    for run in 1..100 {
        let (out, _) = drive(&push_ask(), &[], true);
        assert_eq!(out, first, "Lauf {run} weicht ab");
    }
}

#[test]
fn qa_002_ac3_crash_after_ends_with_exit_code() {
    let (out, code) = drive(&push_ask(), &["--crash-after", "3"], true);
    assert_eq!(code, Some(1));
    assert_eq!(out.lines().count(), 3);
}

#[test]
fn qa_002_ac3_malformed_line_is_invalid_json() {
    let (out, _) = drive(&push_ask(), &["--malformed-line", "3"], true);
    let bad: Vec<usize> = out
        .lines()
        .enumerate()
        .filter(|(_, l)| serde_json::from_str::<Value>(l).is_err())
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(bad, [3]);
}

#[test]
fn qa_002_ac3_hang_after_stops_responding() {
    let mut child = Command::new(bin())
        .args([
            "--protocol",
            "stream-json",
            "--hang-after",
            "1",
            "--scenario",
        ])
        .arg(push_ask())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"type": "user", "message": {"role": "user", "content": "Bitte pushen"}})
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    assert!(first.contains("\"init\""));
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(500) {
        assert!(child.try_wait().unwrap().is_none(), "Prozess sollte hängen");
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn version_is_reported_like_the_vendor_cli() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "2.1.0 (Claude Code)"
    );
}
