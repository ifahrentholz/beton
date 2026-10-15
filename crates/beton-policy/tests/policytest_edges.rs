//! POL-026, POL-027: Randfälle von Testformat, Ausführung, Explain-Darstellung und Snapshots
//! (Ladefehler mit Position, fehlerhafte Fälle, alle Erwartungsarten, alle Zustands-Scopes).

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event};
use beton_policy::context::Activation;
use beton_policy::enforcement::{Level, Matrix};
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::explain::{explain, render};
use beton_policy::grants::SessionGrants;
use beton_policy::load::{Scope, inline_set, parse_set};
use beton_policy::policytest::{
    FileReport, Summary, TestOptions, activation, discover, junit, render as render_tests,
    run_file, tool_context,
};
use beton_policy::snapshot::{self, SnapshotError};
use serde_json::{Map, Value, json};

fn write(dir: &Path, name: &str, text: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn run(path: &Path) -> FileReport {
    run_file(path, &TestOptions::default())
}

fn error_of(dir: &Path, name: &str, text: &str) -> String {
    let p = write(dir, name, text.as_bytes());
    run(&p)
        .error
        .unwrap_or_else(|| panic!("Ladefehler erwartet für {name}"))
}

fn failures(dir: &Path, name: &str, text: &str) -> Vec<String> {
    let p = write(dir, name, text.as_bytes());
    let r = run(&p);
    assert_eq!(r.error, None, "{name}");
    r.cases.iter().flat_map(|c| c.failures.clone()).collect()
}

const P: &str = "spec_version: 1
name: p
rules:
  - id: ok
    on: tool_call
    when: 'tool.kind == \"shell\"'
    action: notify
    notify: { channels: [inbox, log], message: \"hallo\" }
";

#[test]
fn pol_026_files_that_cannot_be_loaded_report_file_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(d, "p.yaml", P.as_bytes());
    assert!(error_of(d, "syntax.policytest.yaml", "cases: [\n").contains(":"));
    assert!(
        error_of(d, "v.policytest.yaml", "spec_version: 2\ncases: []\n").contains("spec_version 2")
    );
    assert!(
        error_of(
            d,
            "scope.policytest.yaml",
            "spec_version: 1\nlayers: { kunde: [p.yaml] }\ncases: []\n"
        )
        .contains("unbekannte Ebene `kunde`")
    );
    let big = vec![b'#'; (1 << 20) + 1];
    let p = write(d, "big.policytest.yaml", &big);
    assert!(run(&p).error.unwrap().contains("größer"));
    let p = write(d, "bin.policytest.yaml", &[0xff, 0xfe, 0x00]);
    assert!(run(&p).error.unwrap().contains("nicht lesbar"));
    // Absolute Pfade und Agent-Verzeichnisse als Ebene.
    let abs = d.join("p.yaml").display().to_string();
    write(d, "a/agent.yaml", b"spec_version: 1\nname: helfer\npolicies:\n  - { id: no-ls, on: tool_call, when: 'tool.command == \"ls\"', action: deny }\n");
    let ok = write(
        d,
        "abs.policytest.yaml",
        format!("spec_version: 1\nlayers: {{ user: [\"{abs}\"], agent: [a] }}\ncases:\n  - {{ name: x, phase: tool_call, input: {{ tool: {{ name: Bash, args: {{ command: ls }} }} }}, expect: {{ outcome: deny, rules: [no-ls], notifications: [ok] }} }}\n")
            .as_bytes(),
    );
    let r = run(&ok);
    assert_eq!(r.error, None);
    assert!(r.cases[0].passed, "{:?}", r.cases[0].failures);
}

#[test]
fn pol_026_agent_layers_with_errors_are_load_errors() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let case = |agent: &str| {
        format!("spec_version: 1\nlayers: {{ agent: [{agent}/agent.yaml] }}\ncases: []\n")
    };
    write(d, "noname/agent.yaml", b"spec_version: 1\npolicies: []\n");
    assert!(error_of(d, "n.policytest.yaml", &case("noname")).contains("`name` fehlt"));
    write(
        d,
        "notlist/agent.yaml",
        b"spec_version: 1\nname: x\npolicies: 3\n",
    );
    assert!(error_of(d, "l.policytest.yaml", &case("notlist")).contains("Liste"));
    write(
        d,
        "bad/agent.yaml",
        b"spec_version: 1\nname: x\npolicies:\n  - { id: A, on: tool_call, action: deny }\n",
    );
    let e = error_of(d, "b.policytest.yaml", &case("bad"));
    assert!(
        e.contains("agent.yaml:") && e.contains("policies[0].id"),
        "{e}"
    );
    write(d, "broken/agent.yaml", b"name: [\n");
    assert!(error_of(d, "y.policytest.yaml", &case("broken")).contains("agent.yaml"));
    // `~/` ohne bekanntes Home ist ein Fehler, keine stille Auswahl.
    let p = write(
        d,
        "home.policytest.yaml",
        b"spec_version: 1\nlayers: { user: [~/fehlt.yaml] }\ncases: []\n",
    );
    let opts = TestOptions {
        home: Some(d.join("leer")),
        ..TestOptions::default()
    };
    assert!(run_file(&p, &opts).error.unwrap().contains("fehlt.yaml"));
    assert!(format!("{opts:?}").contains("TestOptions"));
}

#[test]
fn pol_026_malformed_cases_fail_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(d, "p.yaml", P.as_bytes());
    let head = "spec_version: 1\nlayers: { project: [p.yaml] }\ncases:\n";
    let cases = [
        (
            "  - { name: a, phase: tool_call, steps: [] }\n",
            "schließen sich aus",
        ),
        (
            "  - { name: a, steps: [], expect: { outcome: allow } }\n",
            "in den Schritten",
        ),
        ("  - { name: a }\n", "fehlt"),
        (
            "  - { name: a, phase: tool_call, input: {} }\n",
            "`expect` fehlt",
        ),
        (
            "  - { name: a, steps: [{ repeat: 1 }] }\n",
            "ohne vorherigen Schritt",
        ),
        ("  - { name: a, steps: [{}] }\n", "Schritt ohne"),
        (
            "  - { name: a, steps: [{ advance_time: bald }] }\n",
            "ungültige Dauer",
        ),
        (
            "  - { name: a, steps: [{ phase: tool_call, input: { tool: { name: Bash } }, expect: { outcome: allow } }], expect_state: { kunde: { x: 1 } } }\n",
            "unbekannter Scope",
        ),
        (
            "  - { name: a, phase: tool_call, input: { tool: { args: {} } }, expect: { outcome: allow } }\n",
            "tool.name",
        ),
        (
            "  - { name: a, phase: tool_call, input: { tool: { name: Bash, cwd: relativ } }, expect: { outcome: allow } }\n",
            "nicht absolut",
        ),
        (
            "  - { name: a, phase: tool_call, input: { tool: { name: Bash } }, expect: { outcome: allow }, resolve: { decision: allow } }\n",
            "keine Freigabe offen",
        ),
    ];
    for (i, (case, needle)) in cases.iter().enumerate() {
        let f = failures(
            d,
            &format!("c{i}.policytest.yaml"),
            &format!("{head}{case}"),
        );
        assert!(
            f.iter().any(|m| m.contains(needle)),
            "Fall {i}: `{needle}` fehlt in {f:?}"
        );
    }
    let now = write(
        d,
        "now.policytest.yaml",
        format!("{head}  - {{ name: a, phase: tool_call, input: {{ tool: {{ name: Bash }} }}, expect: {{ outcome: allow }} }}\n")
            .replace("cases:", "fixtures: { now: gestern }\ncases:")
            .as_bytes(),
    );
    let r = run(&now);
    assert!(
        r.cases[0].failures[0].contains("RFC-3339"),
        "{:?}",
        r.cases[0].failures
    );
}

#[test]
fn pol_026_every_kind_of_expectation_can_fail() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(
        d,
        "p.yaml",
        b"spec_version: 1
name: p
rules:
  - { id: tell, on: tool_call, action: notify, notify: { channels: [log], message: \"hallo\" } }
  - { id: patch, on: tool_call, action: modify, modify: { args: { safe: true } } }
  - { id: shadow, on: tool_call, action: deny, dry_run: true }
",
    );
    let f = failures(
        d,
        "e.policytest.yaml",
        "spec_version: 1
layers: { user: [p.yaml] }
cases:
  - name: alles falsch
    phase: tool_call
    input: { tool: { name: Bash, args: { command: ls } } }
    expect:
      outcome: allow
      would: [anders]
      modified: { args: { safe: false } }
      notifications: [fehlt, { rule: tell, message: tschuess }]
      conflicts: [{ field: model }]
      enforcement: observe_only
",
    );
    for needle in [
        "would:",
        "modified:",
        "notifications: {\"rule\":\"fehlt\"}",
        "tschuess",
        "conflicts:",
        "enforcement: erwartet observe_only",
    ] {
        assert!(
            f.iter().any(|m| m.contains(needle)),
            "`{needle}` fehlt in {f:?}"
        );
    }
}

#[test]
fn pol_026_state_of_all_scopes_counters_and_explicit_tool_fields() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(
        d,
        "p.yaml",
        b"spec_version: 1
name: p
rules:
  - id: count
    on: tool_call
    action: allow
    effects:
      - { op: increment, scope: user_day, key: calls }
      - { op: append, scope: user, key: seen, value: 'tool.command' }
      - { op: set, scope: run, key: last, value: 'tool.command' }
  - id: budget
    on: tool_call
    when: 'run.cost_usd > 2.0'
    action: deny
  - id: repeated
    on: tool_call
    when: 'tool.repeat_count == 7 && tool.call_index == 9 && session.tool_calls == 41'
    action: deny
  - id: broken-effect
    on: tool_call
    when: 'tool.command == \"x\"'
    action: allow
    effects:
      - { op: increment, scope: session, key: text }
",
    );
    let p = write(
        d,
        "s.policytest.yaml",
        b"spec_version: 1
layers: { project: [p.yaml] }
fixtures:
  run: { cost_usd: 1.5 }
  state: { user_day: { calls: 1 }, user: { seen: [] }, run: { last: none } }
cases:
  - name: Zustand in allen Scopes
    steps:
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: ls } } }, expect: { outcome: allow } }
      - { cost_delta_usd: 1.0 }
      - { phase: tool_call, input: { tool: { name: Bash, args: { command: pwd } } }, expect: { outcome: deny, rules: [budget] } }
    expect_state: { user_day: { calls: 3 }, user: { seen: [ls, pwd] }, run: { last: pwd } }
  - name: Ausdrueckliche Felder gelten vorrangig
    phase: tool_call
    given: { session: { tool_calls: 41 } }
    input: { tool: { name: Bash, args: { command: ls }, repeat_count: 7, call_index: 9 } }
    expect: { outcome: deny, rules: [repeated] }
  - name: Fehlerhafter Effect
    steps:
      - { phase: tool_call, given: { state: { session: { text: abc } } }, input: { tool: { name: Bash, args: { command: x } } }, expect: { outcome: allow } }
",
    );
    let r = run(&p);
    assert_eq!(r.error, None);
    assert!(
        r.cases[0].passed,
        "{:?}\n{}",
        r.cases[0].failures, r.cases[0].trace
    );
    assert!(
        r.cases[1].passed,
        "{:?}\n{}",
        r.cases[1].failures, r.cases[1].trace
    );
    assert!(!r.cases[2].passed);
    assert!(
        r.cases[2].failures[0].contains("Effect broken-effect"),
        "{:?}",
        r.cases[2].failures
    );
    let text = render_tests(std::slice::from_ref(&r));
    assert!(text.contains("Effect broken-effect"), "{text}");
}

#[test]
fn pol_026_matrix_errors_and_case_harness() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(d, "p.yaml", P.as_bytes());
    let matrix = |h: &str| -> Result<Matrix, String> {
        match h {
            "kaputt" => Err("unbekannt".into()),
            "codex" => {
                let mut m = Matrix::full();
                m.tool_call.insert("shell".into(), Level::ApprovalOnly);
                Ok(m)
            }
            _ => Ok(Matrix::full()),
        }
    };
    let opts = TestOptions {
        matrix: Some(&matrix),
        ..TestOptions::default()
    };
    let file = write(
        d,
        "m.policytest.yaml",
        b"spec_version: 1\nlayers: { project: [p.yaml] }\nharness: kaputt\ncases: []\n",
    );
    assert!(
        run_file(&file, &opts)
            .error
            .unwrap()
            .contains("Harness `kaputt`")
    );
    let case = write(
        d,
        "c.policytest.yaml",
        b"spec_version: 1
layers: { project: [p.yaml] }
cases:
  - { name: codex, harness: codex, phase: tool_call, input: { tool: { name: exec_command, args: { command: ls } } }, expect: { outcome: allow, enforcement: approval_only } }
  - { name: kaputt, harness: kaputt, phase: tool_call, input: { tool: { name: Bash } }, expect: { outcome: allow } }
",
    );
    let r = run_file(&case, &opts);
    assert!(r.cases[0].passed, "{:?}", r.cases[0].failures);
    assert!(r.cases[1].failures[0].contains("Harness `kaputt`"));
}

#[test]
fn pol_026_reports_without_cases_and_with_control_characters() {
    let reports = vec![FileReport {
        file: "x\u{1}.policytest.yaml".into(),
        error: Some("kaputt <a>".into()),
        cases: Vec::new(),
        duration_us: 5,
    }];
    let xml = junit(&reports);
    assert!(roxmltree::Document::parse(&xml).is_ok(), "{xml}");
    assert!(xml.contains("x?.policytest.yaml"));
    let text = render_tests(&reports);
    assert!(text.contains("Datei nicht ladbar: kaputt <a>"));
    assert_eq!(Summary::of(&reports).exit_code(), 1);
    assert!(discover(&[PathBuf::from("/gibt/es/nicht")]).is_err());
}

#[test]
fn pol_027_eval_context_from_fixtures_and_input() {
    let fixtures = json!({ "session": { "cost_usd": 3.0 }, "state": { "session": { "n": 1 } } });
    let mut input = Map::new();
    input.insert(
        "tool".into(),
        json!({ "name": "mcp__github__create_pull_request", "args": { "title": "x" } }),
    );
    let act = activation(Phase::ToolCall, &fixtures, &input, None, Some("/home/u")).unwrap();
    assert!(
        matches!(act.field("tool", "server"), Some(beton_policy::value::Value::String(s)) if &*s == "github")
    );
    let mut request = Map::new();
    request.insert(
        "request".into(),
        json!({ "model": "m", "granularity": "turn" }),
    );
    let act = activation(
        Phase::ModelRequest,
        &fixtures,
        &request,
        Some("codex"),
        None,
    )
    .unwrap();
    assert!(act.field("request", "model").is_some());
    assert!(activation(Phase::ModelRequest, &fixtures, &input, None, None).is_err());
    // ACP: Klasse aus `acp_kind`, explizite Felder vorrangig.
    let mut acp = Map::new();
    acp.insert("name".into(), json!("Datei lesen"));
    acp.insert("acp_kind".into(), json!("read"));
    acp.insert("args".into(), json!({ "path": "/repo/a" }));
    acp.insert("parse_uncertain".into(), json!(true));
    let (ctx, source) = tool_context(&acp, "acp:gemini", "/repo", None).unwrap();
    assert_eq!(ctx["kind"], "file_read");
    assert_eq!(ctx["parse_uncertain"], true);
    assert_eq!(source.acp_kind.as_deref(), Some("read"));
    let mut srv = Map::new();
    srv.insert("name".into(), json!("get"));
    srv.insert("server".into(), json!("github"));
    let (ctx, _) = tool_context(&srv, "codex", "/repo", None).unwrap();
    assert_eq!(ctx["server"], "github");
}

const SET: &str = "spec_version: 1
name: all
mode: dry_run
defaults: { tool_call: ask }
rules:
  - { id: shadow, on: tool_call, when: 'tool.kind == \"shell\" || tool.args.n > 1', action: deny }
";

const ENFORCED: &str = "spec_version: 1
name: live
defaults: { tool_call: deny }
rules:
  - { id: model-a, on: tool_call, action: modify, modify: { args: { a: 1 } } }
  - { id: broken, on: tool_call, when: 'tool.args.n > 1', action: deny }
  - { id: ask-me, on: tool_call, when: 'tool.kind == \"shell\"', action: ask }
  - { id: open, on: tool_call, when: 'tool.command == \"lift\"', action: allow }
";

#[test]
fn pol_027_explain_renders_every_part_of_a_decision() {
    let agent = inline_set(
        Scope::Agent,
        "maestra",
        "agents/maestra/agent.yaml",
        &[json!({ "id": "maestra-no-merge", "type": "git_guard", "params": { "merge": "deny" } })],
    )
    .unwrap();
    // `modify.args` nur ab der User-Ebene (#253): der zweite Schreiber steht auf Team-Ebene.
    let team = inline_set(
        Scope::Team,
        "team",
        "team",
        &[json!({ "id": "patch-b", "on": "tool_call", "action": "modify", "modify": { "args": { "a": 2 } } })],
    )
    .unwrap();
    let set = EffectiveSet::new(vec![
        Arc::new(parse_set(Scope::User, "shadow.yaml", SET).unwrap()),
        Arc::new(parse_set(Scope::User, "live.yaml", ENFORCED).unwrap()),
        Arc::new(agent),
        Arc::new(team),
    ]);
    let act = Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": "lift", "args": { "command": "lift" } } }),
    )
    .unwrap();
    let mut grants = SessionGrants::default();
    let rule = set
        .sets()
        .iter()
        .find(|s| s.name == "live")
        .unwrap()
        .rule_ref("ask-me");
    grants.add_with_id(&rule, "shell:builtin/Bash@:lift", "grt_1");
    let decision = set.evaluate(
        &act,
        &EvalOptions {
            grants: Some(&grants),
            ..EvalOptions::default()
        },
    );
    let mut event = decision_event(&decision, DecisionMeta::new(Level::Full));
    event.decision_id = "pd_test".into();
    let ex = explain(&event, Some(&set));
    let text = render(&ex);
    for needle in [
        "pd_test",
        "[dry_run]",
        "would_deny",
        "angewendet:",
        "Konflikt /args/a",
        "Defaults:",
        "aufgehoben durch allow derselben Ebene",
        "Fehler (fail closed",
        "Grants: grt_1",
        "git_guard",
        // Entzuckerter Regeltyp mit Typ und CEL-Bedingung (POL-017, WP-42).
        "maestra-no-merge/merge (tool_call) deny [git_guard]  when: tool.kind == \"shell\"",
        "tool.args.n > 1 → Fehler",
    ] {
        assert!(text.contains(needle), "`{needle}` fehlt in\n{text}");
    }
}

#[test]
fn pol_027_snapshot_rejects_contents_that_do_not_load() {
    let good = parse_set(Scope::Project, "p.yaml", P).unwrap();
    let set = EffectiveSet::new(vec![Arc::new(good)]);
    let manifest = set.manifest();
    let mut sources: HashMap<String, Vec<u8>> = set.sources().into_iter().collect();
    // Inhalt, der zur Version passt, aber kein UTF-8 bzw. kein gültiges Set ist.
    let version_of = |b: &[u8]| {
        use sha2::Digest as _;
        format!("sha256:{}", hex::encode(sha2::Sha256::digest(b)))
    };
    for bad in [
        vec![0xffu8, 0xfe],
        b"spec_version: 1\nname: p\nrules: 3\n".to_vec(),
    ] {
        let mut m = manifest.clone();
        m.sets[0].version = version_of(&bad);
        sources.insert(m.sets[0].version.clone(), bad);
        let r = snapshot::rebuild(&m, |v| sources.get(v).cloned());
        assert!(matches!(r, Err(SnapshotError::Invalid { .. })), "{r:?}");
    }
    // Inline-Inhalt, der kein JSON-Array ist.
    let mut m = manifest.clone();
    m.sets[0].format = beton_policy::load::SourceFormat::Inline;
    let bad = b"{}".to_vec();
    m.sets[0].version = version_of(&bad);
    sources.insert(m.sets[0].version.clone(), bad);
    assert!(matches!(
        snapshot::rebuild(&m, |v| sources.get(v).cloned()),
        Err(SnapshotError::Invalid { .. })
    ));
    // Anderer Name als im Manifest.
    let mut m = manifest.clone();
    m.sets[0].name = "anders".into();
    let r = snapshot::rebuild(&m, |v| sources.get(v).cloned());
    assert!(matches!(r, Err(SnapshotError::Corrupt { .. })), "{r:?}");
    for e in [
        SnapshotError::Missing {
            set: "s".into(),
            version: "v".into(),
        },
        SnapshotError::HashMismatch {
            expected: "a".into(),
            actual: "b".into(),
        },
    ] {
        assert!(!e.to_string().is_empty());
    }
    let _: Value = serde_json::to_value(&manifest).unwrap();
}
