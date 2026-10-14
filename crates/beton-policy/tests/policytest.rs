//! POL-026: deklarative Policy-Tests in YAML – Ebenen, Fixtures, Fälle mit erwarteter
//! Entscheidung, mehrschrittige Fälle mit Zustand und simulierter Uhr, JUnit-Ausgabe.
//! QA-004 AC1: ein fehlschlagender Fall zeigt erwartete und tatsächliche Entscheidung samt
//! auslösender Regel.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_policy::enforcement::{Level, Matrix};
use beton_policy::policytest::{
    FileReport, Summary, TestOptions, discover, junit, render, run_file, run_paths,
};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, text).unwrap();
    path
}

fn run(path: &Path) -> FileReport {
    run_file(path, &TestOptions::default())
}

const GIT_POLICY: &str = "spec_version: 1
name: git
rules:
  - id: no-force-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\") && shell.has_flag(tool.argv, \"--force\")'
    action: deny
    reason: Force-Push ist verboten
  - id: confirm-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\")'
    action: ask
    reason: Push bestätigen
";

#[test]
fn pol_026_ac1_passing_cases_succeed_and_tool_calls_are_normalized() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "git.yaml", GIT_POLICY);
    let test = write(
        dir.path(),
        "tests/git.policytest.yaml",
        "spec_version: 1
layers:
  project: [../git.yaml]
harness: codex
fixtures:
  session: { harness: codex, cost_usd: 0.5, branch: feature/x }
cases:
  - name: Force-Push wird geblockt
    phase: tool_call
    input: { tool: { name: exec_command, kind: shell, args: { command: \"git push --force origin main\" } } }
    expect: { outcome: deny, rules: [no-force-push], reason_contains: force }
  - name: Push fragt nach
    phase: tool_call
    input: { tool: { name: exec_command, args: { command: \"cd repo && git push origin feature/x\" } } }
    expect: { outcome: ask, rules: [confirm-push] }
  - name: ls ist erlaubt
    phase: tool_call
    input: { tool: { name: exec_command, args: { command: ls } } }
    expect: { outcome: allow }
",
    );
    let report = run(&test);
    assert_eq!(report.error, None);
    assert_eq!(report.cases.len(), 3);
    for c in &report.cases {
        assert!(c.passed, "{}: {:?}\n{}", c.name, c.failures, c.trace);
    }
    let summary = Summary::of(&[report]);
    assert_eq!(summary.exit_code(), 0);
    assert_eq!((summary.passed, summary.failed), (3, 0));
}

#[test]
fn pol_026_ac1_qa_004_ac1_failing_case_shows_expected_actual_rule_and_trace() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "git.yaml", GIT_POLICY);
    let test = write(
        dir.path(),
        "git.policytest.yaml",
        "spec_version: 1
layers: { project: [git.yaml] }
cases:
  - name: Push ist erlaubt (falsch erwartet)
    phase: tool_call
    input: { tool: { name: Bash, args: { command: \"git push --force\" } } }
    expect: { outcome: allow }
  - name: Grund passt nicht
    phase: tool_call
    input: { tool: { name: Bash, args: { command: \"git push --force\" } } }
    expect: { outcome: deny, rules: [ask-everything], reason_contains: Schleife }
",
    );
    let report = run(&test);
    assert_eq!(report.error, None);
    let first = &report.cases[0];
    assert!(!first.passed);
    assert_eq!(first.expected["outcome"], "allow");
    assert_eq!(first.actual["outcome"], "deny");
    assert!(first.line >= 4, "Zeile des Falls: {}", first.line);
    let second = &report.cases[1];
    assert!(!second.passed);
    assert_eq!(second.failures.len(), 2, "{:?}", second.failures);
    let text = render(std::slice::from_ref(&report));
    for needle in [
        "Push ist erlaubt (falsch erwartet)",
        "erwartet: allow",
        "tatsächlich: deny",
        "no-force-push",
        "shell.has_flag(tool.argv, \"--force\") → true",
        "confirm-push",
        "ask-everything",
        "Schleife",
    ] {
        assert!(text.contains(needle), "`{needle}` fehlt in\n{text}");
    }
    assert_eq!(Summary::of(&[report]).exit_code(), 1);
}

#[test]
fn pol_026_ac2_junit_output_is_valid_xml() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "git.yaml", GIT_POLICY);
    let ok = write(
        dir.path(),
        "a.policytest.yaml",
        "spec_version: 1
layers: { project: [git.yaml] }
cases:
  - name: \"ls <erlaubt> & \\\"gut\\\"\"
    phase: tool_call
    input: { tool: { name: Bash, args: { command: ls } } }
    expect: { outcome: allow }
  - name: falsch
    phase: tool_call
    input: { tool: { name: Bash, args: { command: \"git push\" } } }
    expect: { outcome: deny }
",
    );
    let broken = write(
        dir.path(),
        "b.policytest.yaml",
        "spec_version: 1\ncases: 3\n",
    );
    let reports = vec![run(&ok), run(&broken)];
    let xml = junit(&reports);
    let doc = roxmltree::Document::parse(&xml)
        .unwrap_or_else(|e| panic!("kein gültiges XML: {e}\n{xml}"));
    assert!(xml.starts_with("<?xml"), "{xml}");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "testsuites", "{xml}");
    assert_eq!(root.attribute("tests"), Some("3"));
    assert_eq!(root.attribute("failures"), Some("1"));
    assert_eq!(root.attribute("errors"), Some("1"));
    let count = |n: &str| doc.descendants().filter(|d| d.has_tag_name(n)).count();
    assert_eq!(count("testsuite"), 2, "{xml}");
    assert_eq!(count("testcase"), 3, "{xml}");
    assert_eq!(count("failure"), 1, "{xml}");
    assert_eq!(count("error"), 1, "{xml}");
    let names: Vec<&str> = doc
        .descendants()
        .filter(|d| d.has_tag_name("testcase"))
        .filter_map(|d| d.attribute("name"))
        .collect();
    assert!(names.contains(&"ls <erlaubt> & \"gut\""), "{names:?}");
    assert!(
        xml.contains("ls &lt;erlaubt&gt; &amp; &quot;gut&quot;"),
        "{xml}"
    );
}

#[test]
fn pol_026_ac3_steps_carry_effects_counters_clock_and_cost() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "push.yaml",
        "spec_version: 1
name: push
rules:
  - id: count-pushes
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\")'
    action: notify
    notify: { channels: [inbox], message: \"Push Nr. {{ has(state.session.pushes) ? state.session.pushes + 1 : 1 }}\" }
    effects:
      - { op: increment, scope: session, key: pushes }
  - id: third-push-asks
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\") && has(state.session.pushes) && state.session.pushes >= 2'
    action: ask
    reason: Dritter Push
    approval: { remember: session }
  - id: night
    on: tool_call
    when: 'now > timestamp(\"2026-01-01T12:30:00Z\")'
    action: deny
    reason: Feierabend
  - id: expensive
    on: tool_call
    when: 'session.cost_usd > 1.0'
    action: deny
    reason: zu teuer
  - id: loop
    on: tool_call
    when: 'tool.repeat_count >= 3'
    action: ask
    reason: Schleife
  - id: calls
    on: tool_call
    when: 'session.tool_calls > 9'
    action: deny
",
    );
    let test = write(
        dir.path(),
        "push.policytest.yaml",
        "spec_version: 1
layers: { user: [push.yaml] }
fixtures:
  session: { cost_usd: 0.5 }
cases:
  - name: Zähler, Freigabe und Grant über mehrere Schritte
    steps:
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push\" } } },
          expect: { outcome: allow, notifications: [{ rule: count-pushes, message: \"Push Nr. 1\" }] } }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push origin x\" } } }, expect: { outcome: allow } }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push origin y\" } } },
          expect: { outcome: ask, rules: [third-push-asks] }, resolve: { decision: allow, remember: session } }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push origin y\" } } }, expect: { outcome: allow } }
    expect_state: { session: { pushes: 4 } }
  - name: Abgelehnte Freigabe wendet Effects mit Default effects_on an
    steps:
      - { phase: tool_call, given: { state: { session: { pushes: 2 } } }, input: { tool: { name: Bash, args: { command: \"git push\" } } },
          expect: { outcome: ask }, resolve: { decision: deny } }
    expect_state: { session: { pushes: 3 } }
  - name: Uhr und Kosten
    steps:
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: allow } }
      - { advance_time: 31m }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls -l } } }, expect: { outcome: deny, rules: [night] } }
  - name: Kosten-Delta
    steps:
      - { cost_delta_usd: 0.4 }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: allow } }
      - { cost_delta_usd: 0.2 }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: deny, rules: [expensive] } }
  - name: Wiederholung zählt identische Calls
    steps:
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"npm test\" } } }, expect: { outcome: allow } }
      - { repeat: 1 }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"npm test\" } } },
          expect: { outcome: ask, rules: [loop] }, resolve: { decision: allow, remember: none } }
      - { repeat: 6, expect: { outcome: ask } }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: \"npm test\" } } }, expect: { outcome: deny, rules: [calls] } }
    expect_state: { session: { } }
",
    );
    let report = run(&test);
    assert_eq!(report.error, None);
    for c in &report.cases {
        assert!(c.passed, "{}: {:?}\n{}", c.name, c.failures, c.trace);
    }
    assert_eq!(report.cases.len(), 5);
}

#[test]
fn pol_026_ac3_wrong_state_expectation_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "p.yaml",
        "spec_version: 1
name: p
rules:
  - { id: count, on: tool_call, action: allow, effects: [{ op: increment, scope: session, key: n }] }
",
    );
    let test = write(
        dir.path(),
        "p.policytest.yaml",
        "spec_version: 1
layers: { project: [p.yaml] }
cases:
  - name: zählt
    steps:
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: allow } }
    expect_state: { session: { n: 2 } }
",
    );
    let report = run(&test);
    assert!(!report.cases[0].passed);
    assert!(
        report.cases[0]
            .failures
            .iter()
            .any(|f| f.contains("state.session.n")),
        "{:?}",
        report.cases[0].failures
    );
}

#[test]
fn pol_026_layers_simulate_the_hierarchy_with_agent_policies() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "user.yaml",
        "spec_version: 1\nname: user\nrules:\n  - { id: no-curl, on: tool_call, when: 'tool.command.contains(\"curl\")', action: deny }\n",
    );
    write(
        dir.path(),
        "project.yaml",
        "spec_version: 1\nname: project\ndefaults: { browser_navigate: deny }\nrules:\n  - { id: docs, on: browser_navigate, when: 'browser.host == \"docs.rs\"', action: allow }\n",
    );
    write(
        dir.path(),
        "agents/helper/agent.yaml",
        "spec_version: 1
name: helper
executor: { harness: claude }
policies:
  - { id: allow-all, on: tool_call, action: allow }
  - { id: github, on: browser_navigate, when: 'browser.host == \"github.com\"', action: allow }
",
    );
    let test = write(
        dir.path(),
        "h.policytest.yaml",
        "spec_version: 1
layers:
  user: [user.yaml]
  project: [project.yaml]
  agent: [agents/helper/agent.yaml]
cases:
  - name: User-deny schlägt Agent-allow
    phase: tool_call
    input: { tool: { name: Bash, args: { command: \"curl x\" } } }
    expect: { outcome: deny, rules: [no-curl] }
  - name: Agent-allow allein reicht nicht gegen Projekt-Default
    phase: browser_navigate
    input: { browser: { host: github.com, url: \"https://github.com/\" } }
    expect: { outcome: deny, rules: [defaults.browser_navigate] }
  - name: Projekt-allow hebt den Default auf
    phase: browser_navigate
    input: { browser: { host: docs.rs, url: \"https://docs.rs/\" } }
    expect: { outcome: allow }
",
    );
    let report = run(&test);
    assert_eq!(report.error, None);
    for c in &report.cases {
        assert!(c.passed, "{}: {:?}\n{}", c.name, c.failures, c.trace);
    }
}

#[test]
fn pol_026_harness_sets_enforcement_and_modify_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "m.yaml",
        "spec_version: 1
name: m
rules:
  - id: patch
    on: tool_call
    when: 'tool.kind == \"shell\"'
    action: modify
    modify: { args: { timeout: 5 } }
  - id: route
    on: model_request
    when: 'session.cost_usd > 20.0'
    action: modify
    modify: { model: claude-sonnet-4-5 }
",
    );
    let test = write(
        dir.path(),
        "m.policytest.yaml",
        "spec_version: 1
layers: { project: [m.yaml] }
harness: codex
cases:
  - name: Codex kann args nicht ändern
    phase: tool_call
    input: { tool: { name: exec_command, args: { command: ls } } }
    expect: { outcome: deny, rules: [patch], enforcement: approval_only }
  - name: Modell wird umgeroutet
    phase: model_request
    given: { session: { cost_usd: 25.0 } }
    input: { request: { model: claude-opus-4-1, granularity: turn } }
    expect: { outcome: allow, modified: { model: claude-sonnet-4-5 }, enforcement: full }
",
    );
    let matrix = |harness: &str| -> Result<Matrix, String> {
        assert_eq!(harness, "codex");
        let mut m = Matrix::full();
        m.tool_call.insert("shell".into(), Level::ApprovalOnly);
        if let Some(c) = m.modify.get_mut(&beton_policy::Phase::ToolCall) {
            c.args = false;
        }
        Ok(m)
    };
    let report = run_file(
        &test,
        &TestOptions {
            matrix: Some(&matrix),
            ..TestOptions::default()
        },
    );
    assert_eq!(report.error, None);
    for c in &report.cases {
        assert!(c.passed, "{}: {:?}\n{}", c.name, c.failures, c.trace);
    }
}

#[test]
fn pol_026_modified_conflicts_and_dry_run_expectations() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "u.yaml",
        "spec_version: 1\nname: u\nrules:\n  - { id: model-a, on: model_request, action: modify, modify: { model: a } }\n",
    );
    write(
        dir.path(),
        "a.yaml",
        "spec_version: 1\nname: a\nrules:\n  - { id: model-b, on: model_request, action: modify, modify: { model: b } }\n  - { id: shadow, on: model_request, action: deny, dry_run: true }\n",
    );
    let test = write(
        dir.path(),
        "c.policytest.yaml",
        "spec_version: 1
layers: { user: [u.yaml], agent: [a.yaml] }
cases:
  - name: Erster Schreiber gewinnt
    phase: model_request
    input: { request: { model: x } }
    expect:
      outcome: allow
      modified: { model: a }
      conflicts: [{ field: model, kept: a, discarded: b }]
      would: [shadow]
  - name: Keine Konflikte erwartet, aber vorhanden
    phase: model_request
    input: { request: { model: x } }
    expect: { outcome: allow, conflicts: [] }
",
    );
    let report = run(&test);
    assert!(report.cases[0].passed, "{:?}", report.cases[0].failures);
    assert!(!report.cases[1].passed);
}

#[test]
fn pol_026_invalid_test_files_are_errors_with_position() {
    let dir = tempfile::tempdir().unwrap();
    let unknown = write(
        dir.path(),
        "u.policytest.yaml",
        "spec_version: 1\ncases:\n  - name: x\n    phase: tool_call\n    input: {}\n    expect: { outcome: allow, bogus: 1 }\n",
    );
    let report = run(&unknown);
    let err = report.error.clone().unwrap();
    assert!(err.contains("bogus") && err.contains(":6:"), "{err}");
    assert_eq!(Summary::of(&[report]).exit_code(), 1);

    let missing_layer = write(
        dir.path(),
        "m.policytest.yaml",
        "spec_version: 1\nlayers: { project: [fehlt.yaml] }\ncases: []\n",
    );
    let err = run(&missing_layer).error.unwrap();
    assert!(err.contains("fehlt.yaml"), "{err}");

    let bad_policy = write(
        dir.path(),
        "bad.yaml",
        "spec_version: 1\nname: bad\nrules:\n  - { id: x, on: tool_call, when: 'session.cost_usd > \"5\"', action: deny }\n",
    );
    let _ = bad_policy;
    let err = run(&write(
        dir.path(),
        "b.policytest.yaml",
        "spec_version: 1\nlayers: { project: [bad.yaml] }\ncases: []\n",
    ))
    .error
    .unwrap();
    assert!(err.contains("bad.yaml:4"), "{err}");

    let wrong_phase_var = write(
        dir.path(),
        "w.policytest.yaml",
        "spec_version: 1\ncases:\n  - name: x\n    phase: model_request\n    input: { tool: { name: Bash } }\n    expect: { outcome: allow }\n",
    );
    let report = run(&wrong_phase_var);
    assert_eq!(report.error, None);
    assert!(!report.cases[0].passed);
    assert!(
        report.cases[0].failures[0].contains("tool"),
        "{:?}",
        report.cases[0].failures
    );
}

#[test]
fn pol_026_ac1_discover_and_run_paths() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "git.yaml", GIT_POLICY);
    write(
        dir.path(),
        "tests/a.policytest.yaml",
        "spec_version: 1\nlayers: { project: [../git.yaml] }\ncases:\n  - { name: a, phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: allow } }\n",
    );
    write(
        dir.path(),
        "tests/sub/b.policytest.yaml",
        "spec_version: 1\nlayers: { project: [../../git.yaml] }\ncases:\n  - { name: b, phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push -f\" } } }, expect: { outcome: ask } }\n",
    );
    write(dir.path(), "tests/notes.yaml", "kein Test\n");
    let found = discover(&[dir.path().join("tests")]).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].ends_with("a.policytest.yaml"));
    assert!(discover(&[dir.path().join("fehlt")]).is_err());
    let reports = run_paths(&[dir.path().join("tests")], &TestOptions::default()).unwrap();
    let summary = Summary::of(&reports);
    assert_eq!((summary.passed, summary.failed, summary.errors), (2, 0, 0));
    assert_eq!(summary.exit_code(), 0);
}

#[test]
fn pol_026_home_is_expanded_in_layers() {
    let home = tempfile::tempdir().unwrap();
    write(home.path(), ".beton/policies/defaults.yaml", GIT_POLICY);
    let dir = tempfile::tempdir().unwrap();
    let test = write(
        dir.path(),
        "d.policytest.yaml",
        "spec_version: 1\nlayers: { user: [~/.beton/policies/defaults.yaml] }\ncases:\n  - { name: d, phase: tool_call, input: { tool: { name: Bash, args: { command: \"git push --force\" } } }, expect: { outcome: deny } }\n",
    );
    let report = run_file(
        &test,
        &TestOptions {
            home: Some(home.path().to_path_buf()),
            ..TestOptions::default()
        },
    );
    assert_eq!(report.error, None);
    assert!(report.cases[0].passed, "{:?}", report.cases[0].failures);
}
