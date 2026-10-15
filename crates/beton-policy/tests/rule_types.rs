//! Eingebaute Regeltypen aus WP-42 (POL-015 bis POL-018): Entzuckerung zu CEL-Regeln und ihre
//! Wirkung in der Engine. Die mitgelieferten YAML-Policy-Tests je Regeltyp liegen unter
//! `policies/tests/rule-types/` (POL-026 AC4).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::doc::{Action, DefaultAction};
use beton_policy::engine::{Decision, EffectiveSet, EvalOptions, Outcome};
use beton_policy::load::{LoadedSet, RuleKind, Scope, parse_set};
use beton_policy::policytest::activation;
use beton_policy::store::only_tightens;
use serde_json::{Map, Value, json};

fn set(scope: Scope, name: &str, rules: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: {name}\nrules:\n{rules}");
    Arc::new(
        parse_set(scope, &format!("{name}.yaml"), &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn load_err(rules: &str) -> String {
    let text = format!("spec_version: 1\nname: t\nrules:\n{rules}");
    match parse_set(Scope::Project, "t.yaml", &text) {
        Ok(_) => panic!("Ladefehler erwartet:\n{text}"),
        Err(e) => e.to_string(),
    }
}

/// Normalisierter Tool-Call wie im Runner (POL-005), Harness `claude`.
fn call(name: &str, args: Value) -> Activation {
    call_with(name, args, json!({}))
}

fn call_with(name: &str, args: Value, fixtures: Value) -> Activation {
    let mut input = Map::new();
    input.insert("tool".into(), json!({ "name": name, "args": args }));
    activation(
        Phase::ToolCall,
        &fixtures,
        &input,
        Some("claude"),
        Some("/home/u"),
    )
    .unwrap()
}

fn eval(sets: Vec<Arc<LoadedSet>>, act: &Activation) -> Decision {
    EffectiveSet::new(sets).evaluate(act, &EvalOptions::default())
}

fn outcome(d: &Decision) -> &'static str {
    match d.outcome {
        Outcome::Allow => "allow",
        Outcome::Deny { .. } => "deny",
        Outcome::Ask { .. } => "ask",
    }
}

fn reasons(d: &Decision) -> Vec<(String, String)> {
    match &d.outcome {
        Outcome::Allow => Vec::new(),
        Outcome::Deny { reasons } | Outcome::Ask { reasons } => reasons
            .iter()
            .map(|r| (r.rule.rule_id.clone(), r.reason.clone()))
            .collect(),
    }
}

// ---------------------------------------------------------------------------------------
// POL-015 tool_allowlist
// ---------------------------------------------------------------------------------------

const READ_ONLY: &str = "  - id: read-only\n    type: tool_allowlist\n    params: { allow_kinds: [system, file_read, search], deny_reason: \"Nur lesende Tools\" }\n";

#[test]
fn pol_015_ac1_allow_kinds_deny_bash_and_edit_but_allow_read_and_session_spawn() {
    let s = set(Scope::Project, "p", READ_ONLY);
    for (name, args) in [
        ("Bash", json!({ "command": "ls" })),
        (
            "Edit",
            json!({ "file_path": "/repo/a", "old_string": "a", "new_string": "b" }),
        ),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), "deny", "{name}: {d:?}");
        let r = reasons(&d);
        assert_eq!(r[0].0, "defaults.tool_call", "{r:?}");
        assert!(r[0].1.contains("Nur lesende Tools"), "{r:?}");
    }
    for (name, args) in [
        ("Read", json!({ "file_path": "/repo/a" })),
        ("mcp__beton__session_spawn", json!({ "agent": "x" })),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), "allow", "{name}: {d:?}");
        assert!(
            d.matched.iter().any(|m| m.rule.rule_id == "read-only"),
            "{name}: {:?}",
            d.matched
        );
    }
}

#[test]
fn pol_015_ac2_allowlists_on_project_and_agent_intersect() {
    let project = set(
        Scope::Project,
        "p",
        "  - id: p-list\n    type: tool_allowlist\n    params: { allow_kinds: [file_read, shell] }\n",
    );
    let agent = set(
        Scope::Agent,
        "a",
        "  - id: a-list\n    type: tool_allowlist\n    params: { allow_kinds: [file_read, file_edit] }\n",
    );
    let sets = vec![project, agent];
    let read = eval(
        sets.clone(),
        &call("Read", json!({ "file_path": "/repo/a" })),
    );
    assert_eq!(outcome(&read), "allow", "{read:?}");
    let bash = eval(sets.clone(), &call("Bash", json!({ "command": "ls" })));
    assert_eq!(outcome(&bash), "deny", "{bash:?}");
    assert_eq!(reasons(&bash)[0].0, "defaults.tool_call");
    let edit = eval(
        sets,
        &call(
            "Edit",
            json!({ "file_path": "/repo/a", "old_string": "a", "new_string": "b" }),
        ),
    );
    assert_eq!(outcome(&edit), "deny", "{edit:?}");
}

/// Namensmuster gelten nur für eingebaute Tools; MCP-Tools nennt man `mcp:<server>/<tool>`
/// bzw. über `allow_servers` (sonst erlaubte `allow: [Read]` auch ein MCP-Tool `Read`).
#[test]
fn pol_015_name_patterns_cover_builtins_and_mcp_patterns_cover_servers() {
    let s = set(
        Scope::User,
        "u",
        "  - id: list\n    type: tool_allowlist\n    params: { allow: [\"Read\", \"Gr*\", \"mcp:github/*\"], allow_servers: [beton] }\n",
    );
    for (name, args, want) in [
        ("Read", json!({ "file_path": "/repo/a" }), "allow"),
        ("Grep", json!({ "pattern": "x", "path": "/repo" }), "allow"),
        ("mcp__github__get_issue", json!({ "n": 1 }), "allow"),
        ("mcp__beton__session_spawn", json!({}), "allow"),
        ("mcp__evil__Read", json!({ "file_path": "/repo/a" }), "deny"),
        ("mcp__gitlab__get_issue", json!({}), "deny"),
        (
            "Write",
            json!({ "file_path": "/repo/a", "content": "x" }),
            "deny",
        ),
    ] {
        let d = eval(vec![s.clone()], &call(name, args));
        assert_eq!(outcome(&d), want, "{name}: {d:?}");
    }
}

#[test]
fn pol_015_desugars_to_layer_default_deny_plus_allow_rule() {
    let s = set(Scope::Project, "p", READ_ONLY);
    assert_eq!(s.defaults.get(&Phase::ToolCall), Some(&DefaultAction::Deny));
    assert_eq!(s.rules.len(), 1);
    let rule = &s.rules[0];
    assert_eq!(rule.id, "read-only");
    match &rule.kind {
        RuleKind::Cel {
            phase,
            when: Some(when),
            action,
        } => {
            assert_eq!(*phase, Phase::ToolCall);
            assert_eq!(*action, Action::Allow);
            assert!(when.source().contains("tool.kind in"), "{}", when.source());
        }
        other => panic!("CEL-Regel erwartet: {other:?}"),
    }
}

#[test]
fn pol_015_dry_run_allowlist_only_reports() {
    let s = set(
        Scope::Project,
        "p",
        "  - id: shadow-list\n    type: tool_allowlist\n    dry_run: true\n    params: { allow_kinds: [file_read] }\n",
    );
    assert!(!s.defaults.contains_key(&Phase::ToolCall));
    let d = eval(vec![s.clone()], &call("Bash", json!({ "command": "ls" })));
    assert_eq!(outcome(&d), "allow", "{d:?}");
    assert!(
        d.would.iter().any(|m| m.rule.rule_id == "shadow-list"),
        "{:?}",
        d.would
    );
    let d = eval(vec![s], &call("Read", json!({ "file_path": "/repo/a" })));
    assert!(d.would.is_empty(), "{:?}", d.would);
}

#[test]
fn pol_015_params_are_checked_when_loading() {
    for (rules, needle) in [
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow_kind: [shell] }\n",
            "allow_kind",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow_kinds: [shel] }\n",
            "shel",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { allow: Read }\n",
            "Liste",
        ),
        (
            "  - id: x\n    type: tool_allowlist\n    params: { deny_reason: [x] }\n",
            "Text",
        ),
    ] {
        let e = load_err(rules);
        assert!(e.contains(needle), "{e}");
    }
}

/// Hot-Reload (POL-001, #253): Eine neue Allowlist bringt `allow`-Regeln mit, die Defaults
/// anderer Sets derselben Ebene aufheben können; sie gilt deshalb erst in einer neuen Session.
#[test]
fn pol_001_new_tool_allowlist_counts_as_loosening() {
    let base = set(
        Scope::Project,
        "p",
        "  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: deny\n",
    );
    let with_list = set(
        Scope::Project,
        "p",
        &format!(
            "  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: deny\n{READ_ONLY}"
        ),
    );
    assert!(only_tightens(Some(&base), &with_list).is_err());
    assert!(only_tightens(Some(&with_list), &with_list).is_ok());
    // Entfernen lockert ebenfalls.
    assert!(only_tightens(Some(&with_list), &base).is_err());
}
