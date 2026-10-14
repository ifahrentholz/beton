//! POL-025: `policy.decision` aus einer Entscheidung – ohne Tool-Argumente im Klartext, mit
//! `subject_hash`, Dry-Run-Treffern und Enforcement-Level.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_core::event::PolicyOutcome;
use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event, new_decision_id, subject_hash};
use beton_policy::context::Activation;
use beton_policy::enforcement::Level;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::load::{LoadedSet, Scope, parse_set};
use serde_json::json;

fn set(body: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: guard\nrules:\n{body}");
    Arc::new(
        parse_set(Scope::Project, "guard.yaml", &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

const SECRET_ARG: &str = "rm -rf /geheimes-verzeichnis-4711";

fn shell(command: &str) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": command, "args": { "command": command } } }),
    )
    .unwrap()
}

#[test]
fn pol_025_ac2_event_has_no_tool_arguments_only_a_hash() {
    let effective = EffectiveSet::new(vec![set(
        "  - id: no-rm\n    on: tool_call\n    when: 'tool.command.contains(\"rm -rf\")'\n    action: deny\n    reason: kein rm\n",
    )]);
    let decision = effective.evaluate(&shell(SECRET_ARG), &EvalOptions::default());
    let subject = json!({ "tool": "Bash", "args": { "command": SECRET_ARG } });
    let hash = subject_hash(&subject);
    assert!(hash.starts_with("sha256:"), "{hash}");
    assert_eq!(hash, subject_hash(&subject), "deterministisch");
    let event = decision_event(
        &decision,
        DecisionMeta {
            decision_id: new_decision_id(),
            enforcement: Level::Full,
            subject_ref: json!({ "call_id": "toolu_1", "event_id": "evt_01" }),
            subject_hash: Some(hash.clone()),
            approval_id: None,
        },
    );
    assert_eq!(event.outcome, PolicyOutcome::Deny);
    assert_eq!(event.phase, "tool_call");
    assert_eq!(event.enforcement, "full");
    assert!(event.decision_id.starts_with("pd_"));
    assert_eq!(event.subject_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(event.subject_ref["call_id"], "toolu_1");
    assert_eq!(event.matched[0]["rule_id"], "no-rm");
    assert_eq!(event.matched[0]["scope"], "project");
    assert_eq!(event.matched[0]["verdict"], "deny");
    assert_eq!(event.policy_set_hash, effective.hash());
    let text = serde_json::to_string(&event).unwrap();
    assert!(!text.contains("geheimes-verzeichnis"), "{text}");
}

#[test]
fn pol_025_ac3_dry_run_hits_are_listed_but_do_not_change_the_outcome() {
    let effective = EffectiveSet::new(vec![set(
        "  - id: shadow\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: deny\n    dry_run: true\n",
    )]);
    let decision = effective.evaluate(&shell("ls"), &EvalOptions::default());
    let event = decision_event(&decision, DecisionMeta::new(Level::ApprovalOnly));
    assert_eq!(event.outcome, PolicyOutcome::Allow);
    assert_eq!(event.matched.len(), 1, "{:?}", event.matched);
    assert_eq!(event.matched[0]["rule_id"], "shadow");
    assert_eq!(event.matched[0]["dry_run"], true);
    assert_eq!(event.matched[0]["verdict"], "deny");
    assert_eq!(event.enforcement, "approval_only");
}

#[test]
fn pol_025_allow_without_match_and_errors_are_audited() {
    let effective = EffectiveSet::new(vec![set(
        "  - id: broken\n    on: tool_call\n    when: 'tool.args.n > 1'\n    action: ask\n",
    )]);
    let ok = effective.evaluate(&shell("ls"), &EvalOptions::default());
    // `tool.args.n` fehlt: Fehler in einer ask-Regel → deny (fail closed), im Event als Fehler.
    let event = decision_event(&ok, DecisionMeta::new(Level::Full));
    assert_eq!(event.outcome, PolicyOutcome::Deny);
    assert_eq!(event.errors.len(), 1);
    assert_eq!(event.errors[0]["rule_id"], "broken");
    let empty = EffectiveSet::new(vec![]).evaluate(&shell("ls"), &EvalOptions::default());
    let event = decision_event(&empty, DecisionMeta::new(Level::ObserveOnly));
    assert_eq!(event.outcome, PolicyOutcome::Allow);
    assert!(event.matched.is_empty());
    assert_eq!(event.enforcement, "observe_only");
}
