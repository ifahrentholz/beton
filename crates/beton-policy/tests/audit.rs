//! POL-025: `policy.decision` aus einer Entscheidung – ohne Tool-Argumente im Klartext, mit
//! `subject_hash`, Dry-Run-Treffern und Enforcement-Level.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_core::event::PolicyOutcome;
use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event, new_decision_id, subject_hash};
use beton_policy::cel::Clock;
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

/// Stehende Uhr: Die Audit-Tests prüfen nicht das Zeitbudget (POL-002); unter fremder Last auf
/// dem CI-Runner darf es sie nicht verfälschen (#273, #314). Die Engine bleibt unverändert.
struct FrozenClock;

impl Clock for FrozenClock {
    fn now_us(&self) -> u64 {
        0
    }
}

fn frozen() -> EvalOptions<'static> {
    EvalOptions {
        clock: Some(&FrozenClock),
        ..EvalOptions::default()
    }
}

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
    let decision = effective.evaluate(&shell(SECRET_ARG), &frozen());
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
    let decision = effective.evaluate(&shell("ls"), &frozen());
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
    let ok = effective.evaluate(&shell("ls"), &frozen());
    // `tool.args.n` fehlt: Fehler in einer ask-Regel → deny (fail closed), im Event als Fehler.
    let event = decision_event(&ok, DecisionMeta::new(Level::Full));
    assert_eq!(event.outcome, PolicyOutcome::Deny);
    assert_eq!(event.errors.len(), 1);
    assert_eq!(event.errors[0]["rule_id"], "broken");
    let empty = EffectiveSet::new(vec![]).evaluate(&shell("ls"), &frozen());
    let event = decision_event(&empty, DecisionMeta::new(Level::ObserveOnly));
    assert_eq!(event.outcome, PolicyOutcome::Allow);
    assert!(event.matched.is_empty());
    assert_eq!(event.enforcement, "observe_only");
}

/// Minor aus #234 (POL-025 AC2): Fehlertexte in `policy.decision.errors` nennen keine Werte aus
/// den Tool-Argumenten; `subject_hash` ist unabhängig von der Reihenfolge der Schlüssel.
#[test]
fn pol_025_ac2_errors_carry_no_argument_values() {
    let effective = EffectiveSet::new(vec![set(
        "  - id: as-int\n    on: tool_call\n    when: 'int(tool.args.command) > 1'\n    action: deny\n  - id: as-url\n    on: tool_call\n    when: 'url.host(tool.args.command) == \"x\"'\n    action: deny\n  - id: as-key\n    on: tool_call\n    when: 'tool.args[tool.command] == 1'\n    action: deny\n",
    )]);
    let decision = effective.evaluate(&shell(SECRET_ARG), &frozen());
    let event = decision_event(&decision, DecisionMeta::new(Level::Full));
    assert_eq!(event.errors.len(), 3, "{:?}", event.errors);
    let text = serde_json::to_string(&event).unwrap();
    assert!(!text.contains("geheimes-verzeichnis"), "{text}");
    assert_eq!(
        subject_hash(&json!({ "tool": "Bash", "args": { "b": 1, "a": { "y": 2, "x": 1 } } })),
        subject_hash(&json!({ "args": { "a": { "x": 1, "y": 2 }, "b": 1 }, "tool": "Bash" }))
    );
}

/// Re-Review R6 (#253): Auch `matches()` mit Muster aus den Argumenten, `duration()` und
/// `under()` mit relativen Pfaden nennen in Fehlern keine Werte.
#[test]
fn pol_025_ac2_regex_duration_and_path_errors_carry_no_values() {
    let effective = EffectiveSet::new(vec![set(
        "  - id: as-regex\n    on: tool_call\n    when: 'tool.command.matches(tool.args.command + \"(\")'\n    action: deny\n  - id: as-duration\n    on: tool_call\n    when: 'duration(tool.args.command) > duration(\"1s\")'\n    action: deny\n  - id: as-path\n    on: tool_call\n    when: 'under(tool.args.command, \"/repo\")'\n    action: deny\n",
    )]);
    let decision = effective.evaluate(&shell(SECRET_ARG), &frozen());
    let event = decision_event(&decision, DecisionMeta::new(Level::Full));
    assert_eq!(event.errors.len(), 3, "{:?}", event.errors);
    let text = serde_json::to_string(&event).unwrap();
    assert!(!text.contains("geheimes-verzeichnis"), "{text}");
}
