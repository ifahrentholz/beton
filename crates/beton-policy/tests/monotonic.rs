//! QA-004 AC3: Monotonie der Kombination – eine zusätzliche `deny`-Regel macht keine
//! Entscheidung weniger streng (`allow` < `ask` < `deny`), egal auf welcher Ebene, mit
//! welcher Bedingung und neben welchen anderen Regeln und Defaults.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::engine::{Capabilities, EffectiveSet, EvalOptions, Outcome};
use beton_policy::load::{LoadedSet, Scope, parse_set};
use proptest::prelude::*;
use serde_json::json;

const SCOPES: [Scope; 5] = [
    Scope::Org,
    Scope::Team,
    Scope::User,
    Scope::Project,
    Scope::Agent,
];

/// Bedingungen über einen Shell-Call, darunter eine, die zur Laufzeit scheitert.
const CONDITIONS: [&str; 7] = [
    "true",
    "false",
    "tool.kind == \"shell\"",
    "tool.command.contains(\"push\")",
    "tool.command.startsWith(\"rm\")",
    "tool.args.missing == 1",
    "size(tool.argv) > 1",
];

const ACTIONS: [&str; 5] = ["allow", "deny", "ask", "notify", "modify"];
const DEFAULTS: [&str; 3] = ["allow", "ask", "deny"];
const COMMANDS: [&str; 4] = [
    "ls",
    "git push origin main",
    "rm -rf build",
    "cd x && git push",
];

#[derive(Debug, Clone)]
struct RuleGen {
    scope: usize,
    condition: usize,
    action: usize,
    dry_run: bool,
}

#[derive(Debug, Clone)]
struct World {
    rules: Vec<RuleGen>,
    defaults: Vec<(usize, usize)>,
    command: usize,
    args_supported: bool,
}

fn rule_gen() -> impl Strategy<Value = RuleGen> {
    (
        0..SCOPES.len(),
        0..CONDITIONS.len(),
        0..ACTIONS.len(),
        any::<bool>(),
    )
        .prop_map(|(scope, condition, action, dry)| RuleGen {
            scope,
            condition,
            action,
            dry_run: dry && action != 0,
        })
}

fn world() -> impl Strategy<Value = World> {
    (
        prop::collection::vec(rule_gen(), 0..8),
        prop::collection::vec((0..SCOPES.len(), 0..DEFAULTS.len()), 0..3),
        0..COMMANDS.len(),
        any::<bool>(),
    )
        .prop_map(|(rules, defaults, command, args_supported)| World {
            rules,
            defaults,
            command,
            args_supported,
        })
}

fn rule_yaml(i: usize, r: &RuleGen) -> String {
    // `modify.args` dürfen nur Ebenen ab User (#253); darunter wird daraus `notify`.
    let action = match ACTIONS[r.action] {
        "modify" if SCOPES[r.scope] > Scope::User => "notify",
        a => a,
    };
    let modify = if action == "modify" {
        "    modify: { args: { safe: true } }\n"
    } else {
        ""
    };
    format!(
        "  - id: r{i}\n    on: tool_call\n    when: '{}'\n    action: {action}\n{modify}    dry_run: {}\n",
        CONDITIONS[r.condition], r.dry_run
    )
}

/// Ein Set je Ebene (gleicher Name, damit Defaults und Regeln zusammen stehen).
fn sets(w: &World, extra: Option<&RuleGen>) -> Vec<Arc<LoadedSet>> {
    let mut out = Vec::new();
    for (si, scope) in SCOPES.iter().enumerate() {
        let mut rules = String::new();
        for (i, r) in w.rules.iter().enumerate().filter(|(_, r)| r.scope == si) {
            rules.push_str(&rule_yaml(i, r));
        }
        if let Some(e) = extra.filter(|e| e.scope == si) {
            rules.push_str(&rule_yaml(999, e));
        }
        let defaults: Vec<String> = w
            .defaults
            .iter()
            .filter(|(s, _)| *s == si)
            .map(|(_, d)| format!("tool_call: {}", DEFAULTS[*d]))
            .take(1)
            .collect();
        let mut text = format!("spec_version: 1\nname: s{si}\n");
        if let Some(d) = defaults.first() {
            text.push_str(&format!("defaults: {{ {d} }}\n"));
        }
        if rules.is_empty() {
            text.push_str("rules: []\n");
        } else {
            text.push_str("rules:\n");
            text.push_str(&rules);
        }
        out.push(Arc::new(
            parse_set(*scope, &format!("s{si}.yaml"), &text)
                .unwrap_or_else(|e| panic!("{e}\n{text}")),
        ));
    }
    out
}

fn strictness(o: &Outcome) -> u8 {
    match o {
        Outcome::Allow => 0,
        Outcome::Ask { .. } => 1,
        Outcome::Deny { .. } => 2,
    }
}

fn evaluate(sets: Vec<Arc<LoadedSet>>, command: &str, args_supported: bool) -> Outcome {
    let act = Activation::new(
        Phase::ToolCall,
        json!({ "tool": {
            "name": "Bash", "kind": "shell", "command": command,
            "argv": command.split("&&").map(|p| p.split_whitespace().collect::<Vec<_>>()).collect::<Vec<_>>(),
            "args": { "command": command }
        } }),
    )
    .unwrap();
    let opts = EvalOptions {
        capabilities: Capabilities {
            args: args_supported,
            ..Capabilities::default()
        },
        ..EvalOptions::default()
    };
    EffectiveSet::new(sets).evaluate(&act, &opts).outcome
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn qa_004_ac3_adding_a_deny_rule_never_loosens_a_decision(
        w in world(),
        scope in 0..SCOPES.len(),
        condition in 0..CONDITIONS.len(),
    ) {
        let command = COMMANDS[w.command];
        let before = evaluate(sets(&w, None), command, w.args_supported);
        let deny = RuleGen { scope, condition, action: 1, dry_run: false };
        let after = evaluate(sets(&w, Some(&deny)), command, w.args_supported);
        prop_assert!(
            strictness(&after) >= strictness(&before),
            "vorher {:?}, nachher {:?} ({:?}, deny auf {:?} mit `{}`)",
            before, after, w, SCOPES[scope], CONDITIONS[condition]
        );
    }

    #[test]
    fn qa_004_ac3_a_dry_run_deny_rule_never_changes_a_decision(
        w in world(),
        scope in 0..SCOPES.len(),
        condition in 0..CONDITIONS.len(),
    ) {
        let command = COMMANDS[w.command];
        let before = evaluate(sets(&w, None), command, w.args_supported);
        let shadow = RuleGen { scope, condition, action: 1, dry_run: true };
        let after = evaluate(sets(&w, Some(&shadow)), command, w.args_supported);
        prop_assert_eq!(strictness(&after), strictness(&before));
    }
}
