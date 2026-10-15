//! POL-007: Hierarchie & Kombination (lokale Ebenen) nach dem Kombinationsalgorithmus in
//! 03-policies.md. Die YAML-Policy-Tests derselben Fälle folgen mit WP-40 (QA-004).

#![allow(clippy::unwrap_used)]

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
    format!(
        "  - id: {id}\n    on: {on}\n    when: '{when}'\n    action: {action}\n    reason: \"{id}\"\n"
    )
}

fn deny_reasons(d: &Decision) -> Vec<String> {
    match &d.outcome {
        Outcome::Deny { reasons } => reasons.iter().map(|r| r.rule.rule_id.clone()).collect(),
        other => panic!("deny erwartet, war {other:?}"),
    }
}

#[test]
fn pol_007_ac1_user_deny_beats_agent_allow() {
    let user = set(
        Scope::User,
        "u",
        &format!(
            "rules:\n{}",
            rule("no-shell", "tool_call", "tool.kind == \"shell\"", "deny")
        ),
    );
    let agent = set(
        Scope::Agent,
        "a",
        &format!(
            "rules:\n{}",
            rule("yes-shell", "tool_call", "tool.kind == \"shell\"", "allow")
        ),
    );
    let d = eval(vec![agent, user], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["no-shell"]);
    assert!(d.matched.iter().any(|m| m.rule.rule_id == "yes-shell"));
}

#[test]
fn pol_007_ac2_defaults_are_lifted_only_by_the_same_scope() {
    let project = set(Scope::Project, "p", "defaults:\n  browser_navigate: deny\n");
    let agent = set(
        Scope::Agent,
        "a",
        &format!(
            "rules:\n{}",
            rule(
                "gh",
                "browser_navigate",
                "browser.host == \"github.com\"",
                "allow"
            )
        ),
    );
    let d = eval(
        vec![project.clone(), agent.clone()],
        &navigate("github.com"),
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:?}");
    assert!(
        d.defaults_applied
            .iter()
            .any(|a| a.scope == Scope::Project && !a.lifted)
    );

    let project_allow = set(
        Scope::Project,
        "p",
        &format!(
            "defaults:\n  browser_navigate: deny\nrules:\n{}",
            rule(
                "gh",
                "browser_navigate",
                "browser.host == \"github.com\"",
                "allow"
            )
        ),
    );
    let d = eval(
        vec![project_allow.clone(), agent.clone()],
        &navigate("github.com"),
    );
    assert_eq!(d.outcome, Outcome::Allow, "{d:?}");
    assert!(
        d.defaults_applied
            .iter()
            .any(|a| a.scope == Scope::Project && a.lifted)
    );
    // eine andere Domain bleibt verboten
    let d = eval(vec![project_allow, agent], &navigate("example.com"));
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_007_ac2_allowlists_of_two_scopes_intersect() {
    let project = set(
        Scope::Project,
        "p",
        &format!(
            "defaults:\n  browser_navigate: deny\nrules:\n{}{}",
            rule(
                "gh",
                "browser_navigate",
                "browser.host == \"github.com\"",
                "allow"
            ),
            rule(
                "docs",
                "browser_navigate",
                "browser.host == \"docs.rs\"",
                "allow"
            )
        ),
    );
    let agent = set(
        Scope::Agent,
        "a",
        &format!(
            "defaults:\n  browser_navigate: deny\nrules:\n{}",
            rule(
                "gh",
                "browser_navigate",
                "browser.host == \"github.com\"",
                "allow"
            )
        ),
    );
    let sets = vec![project, agent];
    assert_eq!(
        eval(sets.clone(), &navigate("github.com")).outcome,
        Outcome::Allow
    );
    assert!(matches!(
        eval(sets, &navigate("docs.rs")).outcome,
        Outcome::Deny { .. }
    ));
}

#[test]
fn pol_007_ac3_first_writer_wins_for_model() {
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: to-a\n    on: model_request\n    action: modify\n    modify: { model: model-a }\n",
    );
    let agent = set(
        Scope::Agent,
        "a",
        "rules:\n  - id: to-b\n    on: model_request\n    action: modify\n    modify: { model: model-b }\n",
    );
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
    // `args` ändern nur Ebenen ab User (#234); Team ist autoritativer als User.
    let project = set(
        Scope::Team,
        "p",
        "rules:\n  - id: p\n    on: tool_call\n    action: modify\n    modify: { args: { timeout: 10, env: { A: \"1\" } } }\n",
    );
    let agent = set(
        Scope::User,
        "a",
        "rules:\n  - id: a\n    on: tool_call\n    action: modify\n    modify: { args: { timeout: 99, env: { B: \"2\" }, dry: true } }\n",
    );
    let d = eval(
        vec![agent, project],
        &tool("shell", json!({ "command": "make" })),
    );
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
    assert_eq!(conflict.discarded_rule.scope, Scope::User);
}

#[test]
fn pol_007_ac4_overlapping_pointers_conflict() {
    let user = set(
        Scope::Org,
        "u",
        "rules:\n  - id: u\n    on: tool_call\n    action: modify\n    modify: { args: { env: null } }\n",
    );
    let agent = set(
        Scope::User,
        "a",
        "rules:\n  - id: a\n    on: tool_call\n    action: modify\n    modify: { args: { env: { B: \"2\" } } }\n",
    );
    let d = eval(
        vec![user, agent],
        &tool("shell", json!({ "env": { "X": "y" } })),
    );
    assert_eq!(d.modified.args, Some(json!({ "env": null })));
    assert_eq!(d.conflicts.len(), 1);
    assert_eq!(d.conflicts[0].field, "/args/env/B");
}

/// Ein Satz von Sets über alle lokalen Ebenen mit Deny, Ask, Allow, Defaults und Modify.
fn mixed_sets() -> Vec<Arc<LoadedSet>> {
    vec![
        set(
            Scope::User,
            "u-a",
            &format!(
                "rules:\n{}{}",
                rule(
                    "u1",
                    "tool_call",
                    "tool.kind == \"shell\" && tool.args.command.startsWith(\"rm\")",
                    "deny"
                ),
                rule("u2", "tool_call", "tool.kind == \"file_write\"", "ask")
            ),
        ),
        set(
            Scope::User,
            "u-b",
            "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { safe: true } }\n",
        ),
        set(
            Scope::Project,
            "p-a",
            &format!(
                "defaults:\n  tool_call: ask\nrules:\n{}{}",
                rule("p1", "tool_call", "tool.kind == \"file_read\"", "allow"),
                rule("p2", "tool_call", "tool.kind == \"shell\"", "ask")
            ),
        ),
        set(
            Scope::Team,
            "p-b",
            "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { safe: false, note: p } }\n",
        ),
        set(
            Scope::Agent,
            "a-a",
            &format!(
                "rules:\n{}{}",
                rule("a1", "tool_call", "true", "allow"),
                rule("a2", "tool_call", "tool.kind == \"search\"", "deny")
            ),
        ),
        set(
            Scope::Org,
            "a-b",
            "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { note: a } }\n  - id: n\n    on: tool_call\n    action: notify\n",
        ),
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
        let h1 = EffectiveSet::new(mixed_sets()).hash().to_owned();
        let h2 = EffectiveSet::new(mixed_sets().into_iter().rev().collect()).hash().to_owned();
        prop_assert_eq!(h1, h2);
    }
}

#[test]
fn pol_007_mixed_outcomes() {
    let sets = mixed_sets();
    // rm: deny (User) schlägt alles; Gründe in kanonischer Reihenfolge
    let d = eval(
        sets.clone(),
        &tool("shell", json!({ "command": "rm -rf x" })),
    );
    assert_eq!(deny_reasons(&d), ["u1"]);
    assert!(
        !d.notifications.is_empty(),
        "notify wird auch bei deny ausgeführt"
    );
    // Shell ohne rm: ask (Projekt) – genau eine Card mit dem Grund
    let d = eval(sets.clone(), &tool("shell", json!({ "command": "ls" })));
    let Outcome::Ask { reasons } = &d.outcome else {
        panic!("{d:?}")
    };
    let ids: Vec<_> = reasons.iter().map(|r| r.rule.rule_id.as_str()).collect();
    assert_eq!(ids, ["p2", "defaults.tool_call"]);
    // file_read: Projekt-Default ask ist durch p1 aufgehoben
    let d = eval(sets.clone(), &tool("file_read", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    // `args`-Patches ab User (#234): Org `note: a` vor Team `note: p`, Team `safe: false` vor
    // User `safe: true`.
    assert_eq!(d.modified.args, Some(json!({ "safe": false, "note": "a" })));
    // other: Projekt-Default ask greift (Agent-allow hebt ihn nicht auf)
    let d = eval(sets, &tool("other", json!({})));
    assert!(matches!(d.outcome, Outcome::Ask { .. }), "{d:?}");
}

#[test]
fn pol_007_errors_fail_closed() {
    // Fehler in einer deny-Regel → deny
    let s = set(
        Scope::User,
        "u",
        &format!(
            "rules:\n{}",
            rule("e", "tool_call", "tool.args.missing == 1", "deny")
        ),
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["e"]);
    assert_eq!(d.errors.len(), 1);
    // Fehler in einer allow-Regel hebt den Default nicht auf
    let s = set(
        Scope::User,
        "u",
        &format!(
            "defaults:\n  tool_call: deny\nrules:\n{}",
            rule("e", "tool_call", "tool.args.missing == 1", "allow")
        ),
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    // Fehler in einer notify-Regel wird nur protokolliert
    let s = set(
        Scope::User,
        "u",
        &format!(
            "rules:\n{}",
            rule("e", "tool_call", "tool.args.missing == 1", "notify")
        ),
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.errors.len(), 1);
    // Fehler in einer modify-Regel → deny (Modifikation nicht sicher anwendbar)
    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: e\n    on: tool_call\n    when: tool.args.missing == 1\n    action: modify\n    modify: { args: { a: 1 } }\n",
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(deny_reasons(&d), ["e"]);
}

#[test]
fn pol_007_absorbed_error_is_no_error() {
    let s = set(
        Scope::User,
        "u",
        &format!(
            "rules:\n{}",
            rule(
                "e",
                "tool_call",
                "tool.kind == \"x\" && tool.args.missing == 1",
                "deny"
            )
        ),
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert!(d.errors.is_empty());
}

#[test]
fn pol_007_pass_two_sees_the_modified_subject() {
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: route\n    on: model_request\n    action: modify\n    modify: { model: cheap }\n",
    );
    let project = set(
        Scope::Project,
        "p",
        &format!(
            "rules:\n{}",
            rule(
                "no-cheap",
                "model_request",
                "request.model == \"cheap\"",
                "deny"
            )
        ),
    );
    let d = eval(vec![user, project], &model_request("opus"));
    assert_eq!(deny_reasons(&d), ["no-cheap"]);
}

#[test]
fn pol_007_args_patch_is_visible_in_pass_two() {
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { command: \"rm -rf /\" } }\n",
    );
    let project = set(
        Scope::Project,
        "p",
        &format!(
            "rules:\n{}",
            rule(
                "no-rm",
                "tool_call",
                "tool.args.command.startsWith(\"rm\")",
                "deny"
            )
        ),
    );
    let d = eval(
        vec![user, project],
        &tool("shell", json!({ "command": "ls" })),
    );
    assert_eq!(deny_reasons(&d), ["no-rm"]);
}

#[test]
fn pol_007_dry_run_is_reported_but_not_combined() {
    let s = set(
        Scope::User,
        "u",
        &format!(
            "mode: dry_run\nrules:\n{}",
            rule("would", "tool_call", "true", "deny")
        ),
    );
    let r = set(
        Scope::Project,
        "p",
        "rules:\n  - id: rd\n    on: tool_call\n    action: ask\n    dry_run: true\n",
    );
    let d = eval(vec![s, r], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    let would: Vec<_> = d
        .would
        .iter()
        .map(|m| (m.rule.rule_id.as_str(), m.verdict.as_str(), m.dry_run))
        .collect();
    assert_eq!(would, [("would", "deny", true), ("rd", "ask", true)]);
}

struct GrantFor(&'static str);

impl Grants for GrantFor {
    fn grant(&self, rule: &RuleRef, _fingerprint: &str) -> Option<String> {
        (rule.rule_id == self.0).then(|| "grant-1".to_owned())
    }
}

#[test]
fn pol_007_grants_turn_ask_into_allow() {
    let s = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", rule("confirm", "tool_call", "true", "ask")),
    );
    let grants = GrantFor("confirm");
    let opts = EvalOptions {
        grants: Some(&grants),
        ..EvalOptions::default()
    };
    let d = EffectiveSet::new(vec![s]).evaluate(&tool("shell", json!({})), &opts);
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.grants_used, ["grant-1"]);
}

#[test]
fn pol_007_unsupported_modification() {
    let body = |on_unsupported: &str| {
        format!(
            "rules:\n  - id: route\n    on: model_request\n    action: modify\n    modify: {{ model: cheap }}\n    on_unsupported: {on_unsupported}\n"
        )
    };
    let caps = Capabilities {
        model: false,
        ..Capabilities::default()
    };
    let opts = EvalOptions {
        capabilities: caps,
        ..EvalOptions::default()
    };
    let run = |s: &str| {
        EffectiveSet::new(vec![set(Scope::User, "u", &body(s))])
            .evaluate(&model_request("opus"), &opts)
    };
    assert_eq!(deny_reasons(&run("deny")), ["route"]);
    assert!(matches!(run("ask").outcome, Outcome::Ask { .. }));
    let skipped = run("skip");
    assert_eq!(skipped.outcome, Outcome::Allow);
    assert_eq!(skipped.modified.model, None);
    assert!(!skipped.warnings.is_empty());
}

#[test]
fn pol_007_builtin_rule_types_fail_closed_until_implemented() {
    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: budget\n    type: spend_cap\n    params: { scope: session, limit_usd: 25 }\n",
    );
    let d = eval(vec![s.clone()], &model_request("opus"));
    assert_eq!(deny_reasons(&d), ["budget"]);
    // in anderen Phasen wirkt der Regeltyp nicht
    assert_eq!(
        eval(vec![s], &tool("shell", json!({}))).outcome,
        Outcome::Allow
    );
}

struct SlowClock(AtomicU64);

impl Clock for SlowClock {
    fn now_us(&self) -> u64 {
        self.0.fetch_add(4_000, Ordering::SeqCst)
    }
}

#[test]
fn pol_007_time_budget_exceeded_denies() {
    let rules: String = (0..10)
        .map(|i| {
            rule(
                &format!("r{i}"),
                "tool_call",
                "tool.kind == \"nie\"",
                "deny",
            )
        })
        .collect();
    let s = set(Scope::User, "u", &format!("rules:\n{rules}"));
    let clock = SlowClock(AtomicU64::new(0));
    let opts = EvalOptions {
        clock: Some(&clock),
        limits: Limits::default(),
        ..EvalOptions::default()
    };
    let d = EffectiveSet::new(vec![s]).evaluate(&tool("shell", json!({})), &opts);
    let Outcome::Deny { reasons } = &d.outcome else {
        panic!("{d:?}")
    };
    assert!(reasons[0].reason.contains("Zeitbudget"), "{reasons:?}");
}

#[test]
fn pol_007_no_sets_allows() {
    let d = eval(vec![], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    assert!(d.policy_set_hash.starts_with("sha256:"));
}

#[test]
fn pol_007_effects_and_notifications_are_reported() {
    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: count\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: notify\n    notify: { channels: [push, log], message: \"Push {{ 1 + 1 }}\" }\n    effects:\n      - { op: increment, scope: session, key: pushes }\n  - id: only-deny\n    on: tool_call\n    action: allow\n    effects_on: [deny]\n    effects:\n      - { op: set, scope: run, key: k, value: \"tool.kind\" }\n  - id: confirm\n    on: tool_call\n    when: tool.kind == \"file_write\"\n    action: ask\n    approval: { timeout: 30m, on_timeout: deny, remember: session }\n    effects:\n      - { op: append, scope: user, key: writes, value: \"tool.name\" }\n",
    );
    let d = eval(vec![s.clone()], &tool("shell", json!({})));
    assert_eq!(d.outcome, Outcome::Allow);
    let effect_rules: Vec<_> = d.effects.iter().map(|e| e.rule.rule_id.as_str()).collect();
    assert_eq!(effect_rules, ["count"]);
    assert_eq!(d.notifications.len(), 1);
    // `{{ … }}` ist CEL und wird ausgewertet (POL-002, POL-009).
    assert_eq!(d.notifications[0].message.as_deref(), Some("Push 2"));
    // ask: Effects vorgemerkt bis zur Auflösung; Approval-Einstellungen am Grund
    let d = eval(vec![s], &tool("file_write", json!({})));
    let Outcome::Ask { reasons } = &d.outcome else {
        panic!("{d:?}")
    };
    assert!(reasons[0].approval.is_some());
    assert!(d.effects.iter().any(|e| e.rule.rule_id == "confirm"));
    assert!(serde_json::to_value(&d).unwrap()["outcome"] == "ask");
}

#[test]
fn pol_007_redact_and_session_start_modifications() {
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: r1\n    on: model_request\n    action: modify\n    modify: { redact: [\"sk-[a-z]+\"], reasoning_effort: low }\n",
    );
    let project = set(
        Scope::Project,
        "p",
        "rules:\n  - id: r2\n    on: model_request\n    action: modify\n    modify: { redact: [\"sk-[a-z]+\", \"ghp_\\\\w+\"], reasoning_effort: low }\n",
    );
    let d = eval(vec![user, project], &model_request("opus"));
    assert_eq!(d.modified.redact, ["sk-[a-z]+", "ghp_\\w+"]);
    assert_eq!(d.modified.reasoning_effort.as_deref(), Some("low"));
    assert!(d.conflicts.is_empty(), "gleicher Wert ist kein Konflikt");
    assert!(!d.modified.is_empty());

    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: plan\n    on: session_start\n    action: modify\n    modify: { permission_mode: plan, model: m }\n",
    );
    let act = Activation::new(Phase::SessionStart, json!({ "start": { "model": "x" } })).unwrap();
    let d = eval(vec![s], &act);
    assert_eq!(d.modified.permission_mode.as_deref(), Some("plan"));
    assert_eq!(d.modified.model.as_deref(), Some("m"));
}

#[test]
fn pol_007_dry_run_modify_and_errors() {
    let s = set(
        Scope::User,
        "u",
        "mode: dry_run\ndefaults:\n  tool_call: deny\nrules:\n  - id: m\n    on: tool_call\n    action: modify\n    modify: { args: { a: 1 } }\n  - id: e\n    on: tool_call\n    when: tool.args.missing == 1\n    action: deny\n  - id: em\n    on: tool_call\n    when: tool.args.missing == 1\n    action: modify\n    modify: { args: { b: 1 } }\n",
    );
    let d = eval(vec![s], &tool("shell", json!({})));
    assert_eq!(
        d.outcome,
        Outcome::Allow,
        "Dry-Run-Sets entscheiden nicht, auch nicht per Default"
    );
    assert_eq!(d.modified.args, None);
    assert_eq!(d.would.len(), 1);
    assert_eq!(d.errors.len(), 2);
}

/// #182: Nach einem `args`-Patch in Pass 1 sieht Pass 2 auch die abgeleiteten Felder des
/// modifizierten Aufrufs (`tool.command`, `tool.argv`, `tool.paths`), nicht die alten.
#[test]
fn pol_005_args_patch_recomputes_derived_fields_for_pass_two() {
    use beton_policy::normalize::{ToolSource, normalize_source};
    let source = Arc::new(ToolSource {
        harness: "claude".into(),
        native_name: "Bash".into(),
        acp_kind: None,
        mcp_server: None,
        cwd: "/work/repo".into(),
        home: None,
        trusted_servers: None,
    });
    let original = json!({ "command": "rm -rf /tmp/x" });
    let normalized = normalize_source(&source, &original).unwrap();
    let act = Activation::new(Phase::ToolCall, json!({ "tool": normalized.to_context() }))
        .unwrap()
        .with_tool_source(source);
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: safe\n    on: tool_call\n    action: modify\n    modify: { args: { command: \"echo ok > out.txt\" } }\n",
    );
    let project = set(
        Scope::Project,
        "p",
        &format!(
            "rules:\n{}{}{}",
            rule(
                "no-rm",
                "tool_call",
                "shell.has_command(tool.argv, \"rm\")",
                "deny"
            ),
            rule(
                "no-rm-text",
                "tool_call",
                "tool.command.contains(\"rm -rf\")",
                "deny"
            ),
            rule(
                "out-path",
                "tool_call",
                "\"/work/repo/out.txt\" in tool.paths",
                "ask"
            ),
        ),
    );
    let d = eval(vec![user, project], &act);
    match &d.outcome {
        Outcome::Ask { reasons } => {
            assert_eq!(reasons.len(), 1);
            assert_eq!(reasons[0].rule.rule_id, "out-path");
        }
        other => panic!("ask erwartet (Pass 2 auf dem neuen Kommando), war {other:?}"),
    }
}

/// #182, fail closed: Lässt sich der modifizierte Aufruf nicht neu normalisieren, lehnt die
/// Engine ab, statt mit veralteten abgeleiteten Feldern weiterzuprüfen.
#[test]
fn pol_005_failed_renormalization_after_args_patch_denies() {
    use beton_policy::normalize::ToolSource;
    let source = Arc::new(ToolSource {
        harness: "claude".into(),
        native_name: "Bash".into(),
        acp_kind: None,
        mcp_server: None,
        cwd: "relativ/kein/pfad".into(),
        home: None,
        trusted_servers: None,
    });
    let act = tool("shell", json!({ "command": "ls" })).with_tool_source(source);
    let user = set(
        Scope::User,
        "u",
        "rules:\n  - id: patch\n    on: tool_call\n    action: modify\n    modify: { args: { command: \"echo\" } }\n",
    );
    let d = eval(vec![user], &act);
    assert_eq!(deny_reasons(&d), ["engine"]);
    assert!(
        d.errors
            .iter()
            .any(|e| e.error.contains("Modifikation nicht anwendbar")),
        "{:?}",
        d.errors
    );
}

#[test]
fn pol_004_effective_set_lists_programs_and_phases() {
    let s = set(
        Scope::Project,
        "p",
        &format!(
            "defaults:\n  browser_navigate: deny\nrules:\n{}{}",
            rule("a", "tool_call", "tool.kind == \"shell\"", "deny"),
            rule("b", "model_request", "session.cost_usd > 1.0", "deny"),
        ),
    );
    let effective = EffectiveSet::new(vec![s]);
    assert!(effective.has_phase(Phase::ToolCall));
    assert!(effective.has_phase(Phase::ModelRequest));
    assert!(effective.has_phase(Phase::BrowserNavigate), "Default deny");
    assert!(!effective.has_phase(Phase::ToolResult));
    assert!(!effective.has_phase(Phase::SessionStart));
    let fields: Vec<_> = effective
        .programs(Phase::ModelRequest)
        .flat_map(|p| p.fields())
        .collect();
    assert!(
        fields.contains(&"session.cost_usd".to_owned()),
        "{fields:?}"
    );
    assert_eq!(effective.programs(Phase::ToolResult).count(), 0);
}

#[test]
fn pol_006_no_capabilities_means_every_modification_is_unsupported() {
    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: route\n    on: model_request\n    action: modify\n    modify: { model: cheap }\n    on_unsupported: deny\n",
    );
    let opts = EvalOptions {
        capabilities: Capabilities::none(),
        ..EvalOptions::default()
    };
    let d = EffectiveSet::new(vec![s]).evaluate(&model_request("opus"), &opts);
    assert_eq!(deny_reasons(&d), ["route"]);
    use beton_policy::engine::Verdict;
    let names: Vec<_> = [
        Verdict::Deny,
        Verdict::Ask,
        Verdict::Modify,
        Verdict::Allow,
        Verdict::Notify,
        Verdict::Error,
    ]
    .iter()
    .map(|v| v.as_str())
    .collect();
    assert_eq!(names, ["deny", "ask", "modify", "allow", "notify", "error"]);
}
