//! POL-007: Hierarchie & Kombination (lokale Ebenen) nach dem Kombinationsalgorithmus in
//! 03-policies.md. Die YAML-Policy-Tests derselben Fälle folgen mit WP-40 (QA-004).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use beton_policy::Phase;
use beton_policy::cel::{Clock, Limits};
use beton_policy::context::Activation;
use beton_policy::engine::{Capabilities, Decision, EffectiveSet, EvalOptions, Grants, Outcome};
use beton_policy::load::{LoadedSet, RuleRef, Scope, parse_set};
use proptest::prelude::*;
use serde_json::{Value, json};

fn set(scope: Scope, name: &str, body: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: {name}\n{body}");
    Arc::new(
        parse_set(scope, &format!("{name}.yaml"), &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn eval(sets: Vec<Arc<LoadedSet>>, act: &Activation) -> Decision {
    EffectiveSet::new(sets).evaluate(act, &EvalOptions::default())
}

fn tool(kind: &str, args: Value) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "X", "kind": kind, "args": args } }),
    )
    .unwrap()
}

fn navigate(host: &str) -> Activation {
    Activation::new(
        Phase::BrowserNavigate,
        json!({ "browser": { "host": host, "url": format!("https://{host}/") } }),
    )
    .unwrap()
}

fn model_request(model: &str) -> Activation {
    Activation::new(
        Phase::ModelRequest,
        json!({ "request": { "model": model, "granularity": "turn" } }),
    )
    .unwrap()
}

fn rule(id: &str, on: &str, when: &str, action: &str) -> String {
    format!("  - id: {id}\n    on: {on}\n    when: '{when}'\n    action: {action}\n    reason: \"{id}\"\n")
}

fn deny_reasons(d: &Decision) -> Vec<String> {
    match &d.outcome {
        Outcome::Deny { reasons } => reasons.iter().map(|r| r.rule.rule_id.clone()).collect(),
        other => panic!("deny erwartet, war {other:?}"),
    }
}

#[test]
fn pol_007_ac1_user_deny_beats_agent_allow() {
    let user = set(Scope::User, "u", &format!("rules:\n{}", rule("no-shell", "tool_call", "tool.kind == \"shell\"", "deny")));
    let agent = set(Scope::Agent, "a", &format!("rules:\n{}", rule("yes-shell", "tool_call", "tool.kind == \"shell\"", "allow")));
    let d = eval(vec![agent, user], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["no-shell"]);
    assert!(d.matched.iter().any(|m| m.rule.rule_id == "yes-shell"));
}

#[test]
fn pol_007_ac2_defaults_are_lifted_only_by_the_same_scope() {
    let project = set(Scope::Project, "p", "defaults:\n  browser_navigate: deny\n");
    let agent = set(Scope::Agent, "a", &format!("rules:\n{}", rule("gh", "browser_navigate", "browser.host == \"github.com\"", "allow")));
    let d = eval(vec![project.clone(), agent.clone()], &navigate("github.com"));
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:?}");
    assert!(d.defaults_applied.iter().any(|a| a.scope == Scope::Project && !a.lifted));

    let project_allow = set(
        Scope::Project,
        "p",
        &format!("defaults:\n  browser_navigate: deny\nrules:\n{}", rule("gh", "browser_navigate", "browser.host == \"github.com\"", "allow")),
    );
    let d = eval(vec![project_allow.clone(), agent.clone()], &navigate("github.com"));
    assert_eq!(d.outcome, Outcome::Allow, "{d:?}");
    assert!(d.defaults_applied.iter().any(|a| a.scope == Scope::Project && a.lifted));
    // eine andere Domain bleibt verboten
    let d = eval(vec![project_allow, agent], &navigate("example.com"));
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_007_ac2_allowlists_of_two_scopes_intersect() {
    let project = set(
        Scope::Project,
        "p",
        &format!("defaults:\n  browser_navigate: deny\nrules:\n{}{}",
            rule("gh", "browser_navigate", "browser.host == \"github.com\"", "allow"),
            rule("docs", "browser_navigate", "browser.host == \"docs.rs\"", "allow")),
    );
    let agent = set(
        Scope::Agent,
        "a",
        &format!("defaults:\n  browser_navigate: deny\nrules:\n{}", rule("gh", "browser_navigate", "browser.host == \"github.com\"", "allow")),
    );
    let sets = vec![project, agent];
    assert_eq!(eval(sets.clone(), &navigate("github.com")).outcome, Outcome::Allow);
    assert!(matches!(eval(sets, &navigate("docs.rs")).outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_007_ac3_first_writer_wins_for_model() {
    let user = set(Scope::User, "u", "rules:\n  - id: to-a\n    on: model_request\n    action: modify\n    modify: { model: model-a }\n");
    let agent = set(Scope::Agent, "a", "rules:\n  - id: to-b\n    on: model_request\n    action: modify\n    modify: { model: model-b }\n");
    let d = eval(vec![agent, user], &model_request("opus"));
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.modified.model.as_deref(), Some("model-a"));
    let [conflict] = &d.conflicts[..] else {
        panic!("{:?}", d.conflicts);
    };
    assert_eq!(conflict.field, "model");
    assert_eq!(conflict.discarded, json!("model-b"));
    assert_eq!(conflict.discarded_rule.rule_id, "to-b");
    assert_eq!(conflict.kept_rule.rule_id, "to-a");
}

#[test]
fn pol_007_ac4_args_patches_by_pointer() {
    let project = set(Scope::Project, "p", "rules:\n  - id: p\n    on: tool_call\n    action: modify\n    modify: { args: { timeout: 10, env: { A: \"1\" } } }\n");
    let agent = set(Scope::Agent, "a", "rules:\n  - id: a\n    on: tool_call\n    action: modify\n    modify: { args: { timeout: 99, env: { B: \"2\" }, dry: true } }\n");
    let d = eval(vec![agent, project], &tool("shell", json!({ "command": "make" })));
    assert_eq!(
        d.modified.args,
        Some(json!({ "timeout": 10, "env": { "A": "1", "B": "2" }, "dry": true }))
    );
    let [conflict] = &d.conflicts[..] else {
        panic!("{:?}", d.conflicts);
    };
    assert_eq!(conflict.field, "/args/timeout");
    assert_eq!(conflict.kept, json!(10));
    assert_eq!(conflict.discarded, json!(99));
    assert_eq!(conflict.discarded_rule.scope, Scope::Agent);
}

#[test]
fn pol_007_ac4_overlapping_pointers_conflict() {
    let user = set(Scope::User, "u", "rules:\n  - id: u\n    on: tool_call\n    action: modify\n    modify: { args: { env: null } }\n");
    let agent = set(Scope::Agent, "a", "rules:\n  - id: a\n    on: tool_call\n    action: modify\n    modify: { args: { env: { B: \"2\" } } }\n");
    let d = eval(vec![user, agent], &tool("shell", json!({ "env": { "X": "y" } })));
    assert_eq!(d.modified.args, Some(json!({ "env": null })));
    assert_eq!(d.conflicts.len(), 1);
    assert_eq!(d.conflicts[0].field, "/args/env/B");
}

/// Ein Satz von Sets über alle lokalen Ebenen mit Deny, Ask, Allow, Defaults und Modify.
fn mixed_sets() -> Vec<Arc<LoadedSet>> {
    vec![
        set(Scope::User, "u-a", &format!("rules:\n{}{}", rule("u1", "tool_call", "tool.kind == \"shell\" && tool.args.command.startsWith(\"rm\")", "deny"), rule("u2", "tool_call", "tool.kind == \"file_write\"", "ask"))),
        set(Scope::User, "u-b", "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { safe: true } }\n"),
        set(Scope::Project, "p-a", &format!("defaults:\n  tool_call: ask\nrules:\n{}{}", rule("p1", "tool_call", "tool.kind == \"file_read\"", "allow"), rule("p2", "tool_call", "tool.kind == \"shell\"", "ask"))),
        set(Scope::Project, "p-b", "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { safe: false, note: p } }\n"),
        set(Scope::Agent, "a-a", &format!("rules:\n{}{}", rule("a1", "tool_call", "true", "allow"), rule("a2", "tool_call", "tool.kind == \"search\"", "deny"))),
        set(Scope::Agent, "a-b", "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { note: a } }\n  - id: n\n    on: tool_call\n    action: notify\n"),
    ]
}

fn canonical(d: &Decision) -> Value {
    let mut v = serde_json::to_value(d).unwrap();
    v.as_object_mut().unwrap().remove("eval_us");
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn pol_007_ac5_result_is_independent_of_load_order(
        order in Just((0..6).collect::<Vec<usize>>()).prop_shuffle(),
        kind in prop::sample::select(vec!["shell", "file_write", "file_read", "search", "other"]),
        command in prop::sample::select(vec!["rm -rf x", "ls"]),
    ) {
        let sets = mixed_sets();
        let permuted: Vec<_> = order.iter().map(|&i| sets[i].clone()).collect();
        let act = tool(kind, json!({ "command": command }));
        let a = eval(sets, &act);
        let b = eval(permuted, &act);
        prop_assert_eq!(canonical(&a), canonical(&b));
        prop_assert_eq!(EffectiveSet::new(mixed_sets()).hash(), EffectiveSet::new(mixed_sets().into_iter().rev().collect()).hash());
    }
}

#[test]
fn pol_007_mixed_outcomes() {
    let sets = mixed_sets();
    // rm: deny (User) schlägt alles; Gründe in kanonischer Reihenfolge
    let d = eval(sets.clone(), &tool("shell", json!({ "command": "rm -rf x" })));
    assert_eq!(deny_reasons(&d), ["u1"]);
    assert!(!d.notifications.is_empty(), "notify wird auch bei deny ausgeführt");
    // Shell ohne rm: ask (Projekt) – genau eine Card mit dem Grund
    let d = eval(sets.clone(), &tool("shell", json!({ "command": "ls" })));
    let Outcome::Ask { reasons } = &d.outcome else { panic!("{d:?}") };
    let ids: Vec<_> = reasons.iter().map(|r| r.rule.rule_id.as_str()).collect();
    assert_eq!(ids, ["p2", "defaults.tool_call"]);
    // file_read: Projekt-Default ask ist durch p1 aufgehoben
    let d = eval(sets.clone(), &tool("file_read", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.modified.args, Some(json!({ "safe": true, "note": "p" })));
    // other: Projekt-Default ask greift (Agent-allow hebt ihn nicht auf)
    let d = eval(sets, &tool("other", json!({})));
    assert!(matches!(d.outcome, Outcome::Ask { .. }), "{d:?}");
}

#[test]
fn pol_007_errors_fail_closed() {
    // Fehler in einer deny-Regel → deny
    let s = set(Scope::User, "u", &format!("rules:\n{}", rule("e", "tool_call", "tool.args.missing == 1", "deny")));
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["e"]);
    assert_eq!(d.errors.len(), 1);
    // Fehler in einer allow-Regel hebt den Default nicht auf
    let s = set(Scope::User, "u", &format!("defaults:\n  tool_call: deny\nrules:\n{}", rule("e", "tool_call", "tool.args.missing == 1", "allow")));
    let d = eval(vec![s], &tool("shell", json!({})));
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    // Fehler in einer notify-Regel wird nur protokolliert
    let s = set(Scope::User, "u", &format!("rules:\n{}", rule("e", "tool_call", "tool.args.missing == 1", "notify")));
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.errors.len(), 1);
    // Fehler in einer modify-Regel → deny (Modifikation nicht sicher anwendbar)
    let s = set(Scope::User, "u", "rules:\n  - id: e\n    on: tool_call\n    when: tool.args.missing == 1\n    action: modify\n    modify: { args: { a: 1 } }\n");
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["e"]);
}

#[test]
fn pol_007_absorbed_error_is_no_error() {
    let s = set(Scope::User, "u", &format!("rules:\n{}", rule("e", "tool_call", "tool.kind == \"x\" && tool.args.missing == 1", "deny")));
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert!(d.errors.is_empty());
}

#[test]
fn pol_007_pass_two_sees_the_modified_subject() {
    let user = set(Scope::User, "u", "rules:\n  - id: route\n    on: model_request\n    action: modify\n    modify: { model: cheap }\n");
    let project = set(Scope::Project, "p", &format!("rules:\n{}", rule("no-cheap", "model_request", "request.model == \"cheap\"", "deny")));
    let d = eval(vec![user, project], &model_request("opus"));
    assert_eq!(deny_reasons(&d), ["no-cheap"]);
}

#[test]
fn pol_007_args_patch_is_visible_in_pass_two() {
    let user = set(Scope::User, "u", "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { command: \"rm -rf /\" } }\n");
    let project = set(Scope::Project, "p", &format!("rules:\n{}", rule("no-rm", "tool_call", "tool.args.command.startsWith(\"rm\")", "deny")));
    let d = eval(vec![user, project], &tool("shell", json!({ "command": "ls" })));
    assert_eq!(deny_reasons(&d), ["no-rm"]);
}

#[test]
fn pol_007_dry_run_is_reported_but_not_combined() {
    let s = set(Scope::User, "u", &format!("mode: dry_run\nrules:\n{}", rule("would", "tool_call", "true", "deny")));
    let r = set(Scope::Project, "p", "rules:\n  - id: rd\n    on: tool_call\n    action: ask\n    dry_run: true\n");
    let d = eval(vec![s, r], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    let would: Vec<_> = d.would.iter().map(|m| (m.rule.rule_id.as_str(), m.verdict.as_str(), m.dry_run)).collect();
    assert_eq!(would, [("would", "deny", true), ("rd", "ask", true)]);
}

struct GrantFor(&'static str);

impl Grants for GrantFor {
    fn grant(&self, rule: &RuleRef, _ctx: &Activation) -> Option<String> {
        (rule.rule_id == self.0).then(|| "grant-1".to_owned())
    }
}

#[test]
fn pol_007_grants_turn_ask_into_allow() {
    let s = set(Scope::Project, "p", &format!("rules:\n{}", rule("confirm", "tool_call", "true", "ask")));
    let grants = GrantFor("confirm");
    let opts = EvalOptions { grants: Some(&grants), ..EvalOptions::default() };
    let d = EffectiveSet::new(vec![s]).evaluate(&tool("shell", json!({})), &opts);
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.grants_used, ["grant-1"]);
}

#[test]
fn pol_007_unsupported_modification() {
    let body = |on_unsupported: &str| {
        format!("rules:\n  - id: route\n    on: model_request\n    action: modify\n    modify: {{ model: cheap }}\n    on_unsupported: {on_unsupported}\n")
    };
    let caps = Capabilities { model: false, ..Capabilities::default() };
    let opts = EvalOptions { capabilities: caps, ..EvalOptions::default() };
    let run = |s: &str| EffectiveSet::new(vec![set(Scope::User, "u", &body(s))]).evaluate(&model_request("opus"), &opts);
    assert_eq!(deny_reasons(&run("deny")), ["route"]);
    assert!(matches!(run("ask").outcome, Outcome::Ask { .. }));
    let skipped = run("skip");
    assert_eq!(skipped.outcome, Outcome::Allow);
    assert_eq!(skipped.modified.model, None);
    assert!(!skipped.warnings.is_empty());
}

#[test]
fn pol_007_builtin_rule_types_fail_closed_until_implemented() {
    let s = set(Scope::User, "u", "rules:\n  - id: budget\n    type: spend_cap\n    params: { scope: session, limit_usd: 25 }\n");
    let d = eval(vec![s.clone()], &model_request("opus"));
    assert_eq!(deny_reasons(&d), ["budget"]);
    // in anderen Phasen wirkt der Regeltyp nicht
    assert_eq!(eval(vec![s], &tool("shell", json!({}))).outcome, Outcome::Allow);
}

struct SlowClock(AtomicU64);

impl Clock for SlowClock {
    fn now_us(&self) -> u64 {
        self.0.fetch_add(4_000, Ordering::SeqCst)
    }
}

#[test]
fn pol_007_time_budget_exceeded_denies() {
    let rules: String = (0..10).map(|i| rule(&format!("r{i}"), "tool_call", "tool.kind == \"nie\"", "deny")).collect();
    let s = set(Scope::User, "u", &format!("rules:\n{rules}"));
    let clock = SlowClock(AtomicU64::new(0));
    let opts = EvalOptions { clock: Some(&clock), limits: Limits::default(), ..EvalOptions::default() };
    let d = EffectiveSet::new(vec![s]).evaluate(&tool("shell", json!({})), &opts);
    let Outcome::Deny { reasons } = &d.outcome else { panic!("{d:?}") };
    assert!(reasons[0].reason.contains("Zeitbudget"), "{reasons:?}");
}

#[test]
fn pol_007_no_sets_allows() {
    let d = eval(vec![], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert!(d.policy_set_hash.starts_with("sha256:"));
}
