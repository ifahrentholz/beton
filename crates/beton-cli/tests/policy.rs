//! `beton policy test|eval|explain|show` (POL-026, POL-027): Exit-Codes, JUnit, lokale
//! Auswertung und – mit Daemon, Runner und Fake-Harness – Explain einer echten Entscheidung
//! samt Set-Snapshot, den der Runner über den Tunnel ablegt.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Serve, run, stderr, stdout};
use serde_json::Value;

const GIT: &str = "spec_version: 1
name: git
rules:
  - id: no-force-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\") && shell.has_flag(tool.argv, \"--force\")'
    action: deny
    reason: Force-Push ist verboten
  - id: hello
    on: session_start
    action: notify
    notify: { channels: [log], message: Start }
";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Env {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(env.project.path().join(".git")).unwrap();
        write(&env.project.path().join(".beton/policies/git.yaml"), GIT);
        env
    }

    fn beton(&self) -> Command {
        let mut c = common::beton(self.home.path());
        c.current_dir(self.project.path())
            .env("HOME", self.home.path());
        c
    }
}

#[test]
fn pol_026_ac1_exit_code_0_on_success_1_on_failure_with_expected_and_actual() {
    let env = Env::new();
    let tests = env.project.path().join(".beton/policies/tests");
    write(
        &tests.join("ok.policytest.yaml"),
        "spec_version: 1\nlayers: { project: [../git.yaml] }\ncases:\n  - { name: Force-Push wird geblockt, phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push --force\" } } }, expect: { outcome: deny, rules: [no-force-push] } }\n",
    );
    // Ohne Pfad: `.beton/policies/tests/` des Projekts.
    let ok = run(env.beton().args(["policy", "test"]));
    assert!(ok.status.success(), "{}{}", stdout(&ok), stderr(&ok));
    assert!(
        stdout(&ok).contains("✓ Force-Push wird geblockt"),
        "{}",
        stdout(&ok)
    );
    assert!(stdout(&ok).contains(".beton/policies/tests/ok.policytest.yaml"));

    write(
        &tests.join("bad.policytest.yaml"),
        "spec_version: 1\nlayers: { project: [../git.yaml] }\ncases:\n  - { name: falsch erwartet, phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push --force\" } } }, expect: { outcome: allow } }\n",
    );
    let failed = run(env
        .beton()
        .args(["policy", "test", ".beton/policies/tests/"]));
    assert_eq!(failed.status.code(), Some(1));
    let out = stdout(&failed);
    for needle in [
        "✗ falsch erwartet",
        "erwartet: allow",
        "tatsächlich: deny durch no-force-push",
        "Explain-Trace",
        "→ true",
    ] {
        assert!(out.contains(needle), "`{needle}` fehlt:\n{out}");
    }

    let json = run(env.beton().args([
        "--json",
        "policy",
        "test",
        ".beton/policies/tests/bad.policytest.yaml",
    ]));
    assert_eq!(json.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(v[0]["cases"][0]["actual"]["outcome"], "deny");

    let none = run(env.beton().args(["policy", "test", "gibt-es-nicht"]));
    assert_eq!(none.status.code(), Some(2), "{}", stderr(&none));
}

#[test]
fn pol_026_ac2_junit_file_is_written() {
    let env = Env::new();
    let test = env.project.path().join("t.policytest.yaml");
    write(
        &test,
        "spec_version: 1\nlayers: { project: [.beton/policies/git.yaml] }\nharness: codex\ncases:\n  - { name: a & b, phase: tool_call, input: { tool: { name: exec_command, args: { command: \"git push -f\" } } }, expect: { outcome: allow, enforcement: approval_only } }\n",
    );
    let out = run(env.beton().args([
        "policy",
        "test",
        "t.policytest.yaml",
        "--junit",
        "out/junit.xml",
    ]));
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let xml = std::fs::read_to_string(env.project.path().join("out/junit.xml")).unwrap();
    assert!(xml.starts_with("<?xml"), "{xml}");
    assert!(xml.contains("<testcase name=\"a &amp; b\""), "{xml}");
    assert!(xml.contains("tests=\"1\" failures=\"0\""), "{xml}");
}

#[test]
fn pol_027_eval_and_show_work_locally_without_daemon() {
    let env = Env::new();
    write(
        &env.project.path().join("req.json"),
        r#"{"tool": {"name": "Bash", "args": {"command": "git push --force origin"}}}"#,
    );
    let out = run(env.beton().args([
        "policy",
        "eval",
        "--phase",
        "tool_call",
        "--input",
        "req.json",
    ]));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Ergebnis: deny"), "{text}");
    assert!(
        text.contains("shell.has_flag(tool.argv, \"--force\") → true"),
        "{text}"
    );
    write(
        &env.project.path().join("codex.json"),
        r#"{"tool": {"name": "exec_command", "args": {"command": "git push --force origin"}}}"#,
    );
    let json = run(env.beton().args([
        "--json",
        "policy",
        "eval",
        "--phase",
        "tool_call",
        "--input",
        "codex.json",
        "--harness",
        "codex",
    ]));
    let v: Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(v["outcome"], "deny");
    assert_eq!(v["enforcement"], "approval_only", "{v}");

    let bad = run(env
        .beton()
        .args(["policy", "eval", "--phase", "bogus", "--input", "req.json"]));
    assert_eq!(bad.status.code(), Some(2));

    let show = run(env.beton().args(["policy", "show", "--effective"]));
    assert!(show.status.success(), "{}", stderr(&show));
    let text = stdout(&show);
    assert!(text.contains("project  git"), "{text}");
    assert!(text.contains("no-force-push (tool_call) deny"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pol_027_ac1_ac2_explain_a_real_decision_with_the_runner_snapshot() {
    let serve = Serve::start();
    write(&serve.work.path().join(".beton/policies/git.yaml"), GIT);
    let id = serve.create_idle_session().await;
    let client = serve.client();
    // Der Runner hat beim Start ausgewertet (`session_start`) und den Snapshot abgelegt.
    let deadline = Instant::now() + Duration::from_secs(30);
    let seq = loop {
        let events = common::events(&client, &id).await;
        if let Some(e) = events.iter().find(|e| e["type"] == "policy.decision") {
            break e["seq"].as_u64().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "kein policy.decision: {events:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let out = run(common::beton(serve.home()).args([
        "--json",
        "policy",
        "explain",
        "--session",
        &id,
        "--seq",
        &seq.to_string(),
    ]));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["explanation"]["snapshot"], "verified", "{v}");
    let verdict = &v["explanation"]["verdicts"][0];
    assert_eq!(verdict["rule_id"], "hello");
    assert_eq!(verdict["scope"], "project");
    assert!(
        verdict["origin"]
            .as_str()
            .unwrap()
            .ends_with(".beton/policies/git.yaml")
    );

    let text = run(common::beton(serve.home()).args([
        "policy",
        "explain",
        "--session",
        &id,
        "--seq",
        &seq.to_string(),
    ]));
    assert!(
        stdout(&text).contains("Pass 2 – Verdikte"),
        "{}",
        stdout(&text)
    );

    // Auswertung im Kontext der Session, ohne neue Events.
    let before = common::events(&client, &id).await.len();
    write(
        &serve.work.path().join("req.json"),
        r#"{"tool": {"name": "Bash", "args": {"command": "git push --force"}}}"#,
    );
    let req = serve.work.path().join("req.json");
    let eval = run(common::beton(serve.home()).args([
        "--json",
        "policy",
        "eval",
        "--phase",
        "tool_call",
        "--input",
        req.to_str().unwrap(),
        "--session",
        &id,
    ]));
    assert!(eval.status.success(), "{}", stderr(&eval));
    let v: Value = serde_json::from_str(&stdout(&eval)).unwrap();
    assert_eq!(v["outcome"], "deny");
    let show = run(common::beton(serve.home()).args(["policy", "show", "--session", &id]));
    assert!(stdout(&show).contains("no-force-push"), "{}", stdout(&show));
    assert_eq!(common::events(&client, &id).await.len(), before);
}
