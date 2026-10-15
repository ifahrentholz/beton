//! POL-011 `spend_cap`, POL-012 `daily_budget`, POL-013 `model_route`: Entzuckerung zu
//! CEL-Regeln, Auswertung in `session_start` und `model_request`, Listenpreis-Äquivalent bei
//! Subscriptions (ADR-0034), Klassifikation im Hot-Reload (`only_tightens`) und Explain.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use beton_core::event::PolicyOutcome;
use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::engine::{Capabilities, Decision, EffectiveSet, EvalOptions, Outcome};
use beton_policy::load::{LoadedSet, RuleKind, Scope, parse_set};
use beton_policy::policytest::{Summary, TestOptions, render, run_paths};
use beton_policy::store::only_tightens;
use serde_json::{Value, json};

fn set(scope: Scope, name: &str, rules: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: {name}\nrules:\n{rules}");
    Arc::new(
        parse_set(scope, &format!("{name}.yaml"), &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn load_err(rules: &str) -> String {
    let text = format!("spec_version: 1\nname: b\nrules:\n{rules}");
    parse_set(Scope::User, "b.yaml", &text)
        .map(|_| panic!("Ladefehler erwartet:\n{text}"))
        .unwrap_err()
        .to_string()
}

fn act(phase: Phase, mut ctx: Value) -> Activation {
    // Ohne Angabe: keine Nutzung ohne Preis (die Werte liefert sonst `policy.usage`).
    for (object, fields) in [
        ("session", &["unpriced_models"][..]),
        ("run", &["unpriced_models", "subtree_unpriced_models"][..]),
        ("user", &["daily_unpriced_models"][..]),
    ] {
        if let Some(o) = ctx.get_mut(object).and_then(Value::as_object_mut) {
            for f in fields {
                o.entry(*f).or_insert(json!([]));
            }
        }
    }
    Activation::new(phase, ctx).unwrap_or_else(|e| panic!("{e}"))
}

/// Kontext für `model_request` mit Kosten der Session.
fn request(session: Value) -> Activation {
    act(
        Phase::ModelRequest,
        json!({
            "session": session,
            "request": { "model": "claude-opus-4-5", "granularity": "turn" },
            "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
        }),
    )
}

fn eval(sets: Vec<Arc<LoadedSet>>, a: &Activation) -> Decision {
    EffectiveSet::new(sets).evaluate(a, &EvalOptions::default())
}

fn reason_ids(d: &Decision) -> Vec<String> {
    match &d.outcome {
        Outcome::Deny { reasons } | Outcome::Ask { reasons } => {
            reasons.iter().map(|r| r.rule.rule_id.clone()).collect()
        }
        Outcome::Allow => Vec::new(),
    }
}

fn reason_text(d: &Decision) -> String {
    match &d.outcome {
        Outcome::Deny { reasons } | Outcome::Ask { reasons } => reasons
            .iter()
            .map(|r| r.reason.clone())
            .collect::<Vec<_>>()
            .join("; "),
        Outcome::Allow => String::new(),
    }
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Führt eine Policy-Testdatei aus und verlangt, dass alle Fälle bestehen.
fn policytest(policy: &str, test: &str) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "b.yaml", policy);
    let file = write(dir.path(), "b.policytest.yaml", test);
    let reports = run_paths(&[file], &TestOptions::default()).unwrap();
    let summary = Summary::of(&reports);
    assert!(summary.passed > 0, "{}", render(&reports));
    assert_eq!(summary.exit_code(), 0, "{}", render(&reports));
}

const SPEND_CAP: &str = "  - id: budget\n    type: spend_cap\n    params: { scope: session, limit_usd: 5, ask_at_usd: [3] }\n";

// ------------------------------------------------------------------------- POL-011

#[test]
fn pol_011_ac1_threshold_asks_once_and_the_limit_denies_before_the_next_turn() {
    policytest(
        &format!("spec_version: 1\nname: b\nrules:\n{SPEND_CAP}"),
        r#"spec_version: 1
layers: { project: [b.yaml] }
cases:
  - name: Schwelle fragt genau einmal, das Limit lehnt ab
    steps:
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } }, expect: { outcome: allow } }
      - { cost_delta_usd: 3.1 }
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } },
          expect: { outcome: ask, rules: [budget], reason_contains: "3" }, resolve: { decision: allow } }
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } }, expect: { outcome: allow } }
      - { cost_delta_usd: 2.0 }
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } },
          expect: { outcome: deny, rules: [budget], reason_contains: "5" } }
    expect_state: { session: { "spend_cap/project/b/budget/asked": 3.0 } }
  - name: Abgelehnte Schwelle fragt beim nächsten Turn erneut
    given: { session: { cost_usd: 3.5 } }
    steps:
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } },
          expect: { outcome: ask, rules: [budget] }, resolve: { decision: deny } }
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } },
          expect: { outcome: ask, rules: [budget] } }
  - name: Genau 5,00 USD lehnt vor dem nächsten Turn ab
    phase: model_request
    given: { session: { cost_usd: 5.0 }, state: { session: { "spend_cap/project/b/budget/asked": 3.0 } } }
    input: { request: { model: claude-sonnet-4-5, granularity: turn } }
    expect: { outcome: deny, rules: [budget] }
  - name: Auch session_start prüft das Limit
    phase: session_start
    given: { session: { cost_usd: 7.0 } }
    input: { start: { trigger: resume, harness: claude } }
    expect: { outcome: deny, rules: [budget] }
"#,
    );
}

#[test]
fn pol_011_ac1_skipped_thresholds_ask_only_for_the_highest() {
    let s = set(
        Scope::User,
        "u",
        "  - id: budget\n    type: spend_cap\n    params: { limit_usd: 25, ask_at_usd: [20, 10] }\n",
    );
    let d = eval(
        vec![s],
        &request(json!({ "cost_usd": 21.0, "equivalent_usd": 0.0 })),
    );
    assert!(matches!(d.outcome, Outcome::Ask { ref reasons } if reasons.len() == 1));
    assert!(reason_text(&d).contains("20"), "{}", reason_text(&d));
    // Die Freigabe merkt sich die Schwelle 20 (Effects erst nach der Auflösung).
    assert_eq!(d.effects.len(), 1);
    assert_eq!(d.effects[0].ops[0].key, "spend_cap/user/u/budget/asked");
    assert_eq!(d.effects[0].ops[0].value, Some(json!(20.0)));
}

#[test]
fn pol_011_ac2_route_switches_the_model_after_the_limit() {
    let s = set(
        Scope::User,
        "u",
        "  - id: budget\n    type: spend_cap\n    params: { limit_usd: 5, on_exceed: route, fallback_model: claude-haiku-4-5, fallback_effort: low }\n",
    );
    let under = eval(
        vec![s.clone()],
        &request(json!({ "cost_usd": 4.0, "equivalent_usd": 0.0 })),
    );
    assert_eq!(under.outcome, Outcome::Allow);
    assert_eq!(under.modified.model, None);
    let over = eval(
        vec![s.clone()],
        &request(json!({ "cost_usd": 5.0, "equivalent_usd": 0.0 })),
    );
    assert_eq!(over.outcome, Outcome::Allow);
    assert_eq!(over.modified.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(over.modified.reasoning_effort.as_deref(), Some("low"));
    // Ohne Modellwechsel im Harness greift `on_unsupported` (Default `deny`).
    let opts = EvalOptions {
        capabilities: Capabilities {
            model: false,
            reasoning_effort: false,
            ..Capabilities::default()
        },
        ..EvalOptions::default()
    };
    let d = EffectiveSet::new(vec![s]).evaluate(
        &request(json!({ "cost_usd": 5.0, "equivalent_usd": 0.0 })),
        &opts,
    );
    assert_eq!(reason_ids(&d), ["budget"]);
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_011_ac3_subtree_counts_the_costs_of_child_sessions() {
    let s = set(
        Scope::User,
        "u",
        "  - id: tree\n    type: spend_cap\n    params: { scope: subtree, limit_usd: 5 }\n",
    );
    // Parent 1 USD, zwei Childs mit 2 und 2,5 USD: der Teilbaum hat 5,5 USD.
    let ctx = |subtree: f64| {
        act(
            Phase::ModelRequest,
            json!({
                "session": { "cost_usd": 1.0, "equivalent_usd": 0.0 },
                "run": { "cost_usd": 1.0, "equivalent_usd": 0.0,
                         "subtree_cost_usd": subtree, "subtree_equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
            }),
        )
    };
    let d = eval(vec![s.clone()], &ctx(1.0 + 2.0 + 2.5));
    assert_eq!(reason_ids(&d), ["tree"]);
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    assert_eq!(eval(vec![s], &ctx(1.0)).outcome, Outcome::Allow);
}

#[test]
fn pol_011_run_scope_reads_the_run_costs() {
    let s = set(
        Scope::User,
        "u",
        "  - id: run-cap\n    type: spend_cap\n    params: { scope: run, limit_usd: 2, ask_at_usd: [1] }\n",
    );
    let d = eval(
        vec![s],
        &act(
            Phase::ModelRequest,
            json!({
                "run": { "cost_usd": 1.5, "equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        ),
    );
    assert_eq!(reason_ids(&d), ["run-cap"]);
    assert_eq!(d.effects[0].ops[0].key, "spend_cap/user/u/run-cap/asked");
    assert_eq!(
        d.effects[0].ops[0].scope,
        beton_policy::doc::StateScope::Run
    );
}

#[test]
fn pol_011_ac4_explain_shows_the_desugared_cel_rules() {
    let s = set(Scope::Project, "b", SPEND_CAP);
    let effective = EffectiveSet::new(vec![s.clone()]);
    let d = effective.evaluate(
        &request(json!({ "cost_usd": 6.0, "equivalent_usd": 0.0 })),
        &EvalOptions::default(),
    );
    let event = beton_policy::audit::decision_event(
        &d,
        beton_policy::audit::DecisionMeta {
            decision_id: "pd_test".into(),
            enforcement: beton_policy::enforcement::Level::Full,
            subject_ref: json!({}),
            subject_hash: None,
            approval_id: None,
        },
    );
    assert_eq!(event.outcome, PolicyOutcome::Deny);
    let ex = beton_policy::explain::explain(&event, Some(&effective));
    let text = beton_policy::explain::render(&ex);
    // Alle Teile des Regeltyps mit Phase und entzuckerter Bedingung.
    let rules = &ex.sets[0].rules;
    assert!(rules.len() >= 4, "{rules:#?}");
    assert!(rules.iter().all(|r| r.id == "budget"
        && r.rule_type.as_deref() == Some("spend_cap")
        && r.when.is_some()));
    for phase in ["session_start", "model_request"] {
        assert!(
            rules
                .iter()
                .any(|r| r.phases == [phase] && r.action == "deny"),
            "{rules:#?}"
        );
    }
    assert!(
        text.contains("session.cost_usd + session.equivalent_usd >= 5.0"),
        "{text}"
    );
    assert!(text.contains("spend_cap/project/b/budget/asked"), "{text}");
}

#[test]
fn pol_027_ac1_explain_names_the_part_of_a_rule_type_that_matched() {
    // Mehrere Teile tragen dieselbe Regel-ID: Explain zeigt die Bedingung des gegriffenen
    // Teils (Phase, Verdikt), nicht die des ersten Teils.
    let s = set(Scope::Project, "b", SPEND_CAP);
    let effective = EffectiveSet::new(vec![s]);
    let d = effective.evaluate(
        &request(json!({ "cost_usd": 6.0, "equivalent_usd": 0.0 })),
        &EvalOptions::default(),
    );
    let event = beton_policy::audit::decision_event(
        &d,
        beton_policy::audit::DecisionMeta::new(beton_policy::enforcement::Level::Full),
    );
    let ex = beton_policy::explain::explain(&event, Some(&effective));
    let v = &ex.verdicts[0];
    assert_eq!(v.verdict, "deny");
    assert_eq!(
        v.when.as_deref(),
        Some("session.cost_usd + session.equivalent_usd >= 5.0"),
        "{ex:#?}"
    );
}

#[test]
fn pol_011_subscription_sessions_count_the_list_price_equivalent() {
    // ADR-0034: Subscription-Nutzung hat kein Geld, zählt fürs Budget aber mit dem
    // Listenpreis-Äquivalent laut Katalog.
    let s = set(Scope::User, "u", SPEND_CAP);
    let d = eval(
        vec![s.clone()],
        &request(json!({ "cost_usd": 0.0, "equivalent_usd": 5.2 })),
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:#?}");
    let mixed = eval(
        vec![s],
        &request(json!({ "cost_usd": 2.0, "equivalent_usd": 1.5 })),
    );
    assert!(matches!(mixed.outcome, Outcome::Ask { .. }), "{mixed:#?}");
}

#[test]
fn pol_011_missing_costs_fail_closed() {
    // Ohne Verbrauchswerte (Server nicht erreichbar) ist die Bedingung ein Fehler: `deny`.
    let s = set(Scope::User, "u", SPEND_CAP);
    let d = eval(vec![s], &request(json!({ "cost_usd": 1.0 })));
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:#?}");
    assert!(!d.errors.is_empty());
}

#[test]
fn pol_011_without_state_threshold_rules_fail_closed() {
    let s = set(Scope::User, "u", SPEND_CAP);
    let d = eval(
        vec![s],
        &act(
            Phase::ModelRequest,
            json!({
                "session": { "cost_usd": 3.5, "equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
            }),
        ),
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:#?}");
}

#[test]
fn pol_011_params_are_checked_on_load() {
    let rule = |params: &str| format!("  - id: x\n    type: spend_cap\n    params: {params}\n");
    for (params, needle) in [
        ("{ scope: session }", "limit_usd"),
        ("{ limit_usd: 0 }", "limit_usd"),
        ("{ limit_usd: -1 }", "limit_usd"),
        ("{ limit_usd: fünf }", "limit_usd"),
        ("{ limit_usd: 5, ask_at_usd: [5] }", "ask_at_usd"),
        ("{ limit_usd: 5, ask_at_usd: [2, 2] }", "ask_at_usd"),
        ("{ limit_usd: 5, ask_at_usd: 3 }", "ask_at_usd"),
        ("{ limit_usd: 5, scope: user }", "scope"),
        ("{ limit_usd: 5, on_exceed: stop }", "on_exceed"),
        ("{ limit_usd: 5, on_exceed: route }", "fallback_model"),
        ("{ limit_usd: 5, fallback_model: x }", "fallback_model"),
        (
            "{ limit_usd: 5, on_exceed: route, fallback_model: x, fallback_effort: max }",
            "fallback_effort",
        ),
        ("{ limit_usd: 5, kinds: [shell] }", "kinds"),
        ("{ limit_usd: 5, scope: 3 }", "scope"),
        ("{ limit_usd: 5, ask_at_usd: [drei] }", "ask_at_usd[0]"),
        (
            "{ limit_usd: 5, on_exceed: route, fallback_model: \"\" }",
            "fallback_model",
        ),
    ] {
        let e = load_err(&rule(params));
        assert!(e.contains(needle), "{params}: {e}");
    }
    // `type` schließt eigene Bedingungen und Effects weiter aus.
    assert!(
        load_err(
            "  - id: x\n    type: spend_cap\n    on: model_request\n    params: { limit_usd: 1 }\n"
        )
        .contains("on")
    );
}

// ------------------------------------------------------------------------- POL-012

const DAILY: &str =
    "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, ask_at_usd: [40] }\n";

fn user_request(daily: f64, equivalent: f64) -> Activation {
    act(
        Phase::ModelRequest,
        json!({
            "user": { "daily_cost_usd": daily, "daily_equivalent_usd": equivalent, "timezone": "Europe/Berlin" },
            "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
            "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
        }),
    )
}

#[test]
fn pol_012_ac1_the_daily_budget_counts_all_sessions_of_the_user() {
    // Zwei parallele Sessions mit je 30 USD: `user.daily_cost_usd` ist 60.
    let s = set(Scope::User, "u", DAILY);
    let d = eval(vec![s.clone()], &user_request(30.0 + 30.0, 0.0));
    assert_eq!(reason_ids(&d), ["daily"]);
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    // Subscription-Nutzung zählt mit dem Listenpreis-Äquivalent.
    let d = eval(vec![s.clone()], &user_request(20.0, 31.0));
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    // Die Schwelle merkt sich der Tageszustand (`user_day`).
    let d = eval(vec![s], &user_request(41.0, 0.0));
    assert!(matches!(d.outcome, Outcome::Ask { .. }));
    assert_eq!(
        d.effects[0].ops[0].scope,
        beton_policy::doc::StateScope::UserDay
    );
    assert_eq!(d.effects[0].ops[0].key, "daily_budget/user/u/daily/asked");
}

#[test]
fn pol_012_ac2_new_sessions_are_denied_or_routed_at_session_start() {
    let start = |daily: f64| {
        act(
            Phase::SessionStart,
            json!({
                "user": { "daily_cost_usd": daily, "daily_equivalent_usd": 0.0 },
                "start": { "trigger": "interactive", "harness": "claude", "model": "claude-opus-4-5" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        )
    };
    let deny = set(Scope::User, "u", DAILY);
    assert!(matches!(
        eval(vec![deny.clone()], &start(55.0)).outcome,
        Outcome::Deny { .. }
    ));
    assert_eq!(eval(vec![deny], &start(10.0)).outcome, Outcome::Allow);
    let route = set(
        Scope::User,
        "u",
        "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, on_exceed: route, fallback_model: claude-haiku-4-5 }\n",
    );
    let d = eval(vec![route], &start(55.0));
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.modified.model.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn pol_012_on_exceed_ask_asks_at_every_turn() {
    let s = set(
        Scope::User,
        "u",
        "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, on_exceed: ask }\n",
    );
    let d = eval(vec![s], &user_request(51.0, 0.0));
    assert!(matches!(d.outcome, Outcome::Ask { .. }), "{d:#?}");
    // Ohne Merken: keine Effects, also beim nächsten Turn wieder.
    assert!(d.effects.is_empty());
    // Ein `ask` ab dem Limit ist ein erreichtes Budget (`budget.exhausted`, action ask).
    let s = set(
        Scope::User,
        "u",
        "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, on_exceed: ask }\n",
    );
    let effective = EffectiveSet::new(vec![s]);
    let d = effective.evaluate(&user_request(51.0, 0.0), &EvalOptions::default());
    let found = beton_policy::budget::exhausted(&d, &effective);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, beton_core::event::BudgetAction::Ask);
    assert_eq!(found[0].scope.as_str(), "user");
}

#[test]
fn pol_011_dry_run_budgets_are_neither_counted_nor_exhausted() {
    let s = set(
        Scope::User,
        "u",
        "  - id: shadow\n    type: spend_cap\n    dry_run: true\n    params: { scope: run, limit_usd: 1 }\n",
    );
    let effective = EffectiveSet::new(vec![s]);
    assert!(beton_policy::budget::budget_rules(&effective).is_empty());
    let d = effective.evaluate(
        &act(
            Phase::ModelRequest,
            json!({
                "run": { "cost_usd": 2.0, "equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
            }),
        ),
        &EvalOptions::default(),
    );
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.would.len(), 1, "{d:#?}");
    assert!(beton_policy::budget::exhausted(&d, &effective).is_empty());
}

#[test]
fn pol_012_team_scope_and_own_timezone_follow_with_m4() {
    let rule = |params: &str| format!("  - id: x\n    type: daily_budget\n    params: {params}\n");
    assert!(load_err(&rule("{ scope: team, limit_usd: 5 }")).contains("M4"));
    assert!(load_err(&rule("{ scope: org, limit_usd: 5 }")).contains("scope"));
    assert!(load_err(&rule("{ limit_usd: 5, timezone: Europe/Berlin }")).contains("timezone"));
    assert!(
        load_err(&rule(
            "{ limit_usd: 5, fallback_effort: low, on_exceed: route, fallback_model: x }"
        ))
        .contains("fallback_effort")
    );
}

// ------------------------------------------------------------------------- POL-013

const ROUTING: &str = r#"  - id: routing
    type: model_route
    params:
      allowed_models: ["claude-sonnet-*", "claude-haiku-*", "gpt-5*"]
      routes:
        - { when: "session.is_async", model: claude-sonnet-4-5, reasoning_effort: medium }
        - { when: "user.daily_cost_usd > 30.0", model: claude-haiku-4-5 }
"#;

fn start_with(model: Option<&str>, is_async: bool, daily: f64) -> Activation {
    let mut start = json!({ "trigger": "interactive", "harness": "claude" });
    if let Some(m) = model {
        start["model"] = json!(m);
    }
    act(
        Phase::SessionStart,
        json!({
            "session": { "is_async": is_async },
            "user": { "daily_cost_usd": daily },
            "start": start,
        }),
    )
}

#[test]
fn pol_013_ac1_a_start_with_a_model_outside_the_allowlist_is_denied_with_a_reason() {
    let s = set(Scope::User, "u", ROUTING);
    let d = eval(
        vec![s.clone()],
        &start_with(Some("claude-opus-4-5"), false, 0.0),
    );
    assert_eq!(reason_ids(&d), ["routing"]);
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    let text = reason_text(&d);
    assert!(text.contains("claude-sonnet-*"), "{text}");
    assert_eq!(
        eval(
            vec![s.clone()],
            &start_with(Some("claude-sonnet-4-5"), false, 0.0)
        )
        .outcome,
        Outcome::Allow
    );
    // Ein unbekanntes Modell (Default des Harness) ist nicht nachweisbar erlaubt.
    assert!(matches!(
        eval(vec![s.clone()], &start_with(None, false, 0.0)).outcome,
        Outcome::Deny { .. }
    ));
    // Auch jeder Turn prüft das Modell.
    let turn = act(
        Phase::ModelRequest,
        json!({ "session": { "is_async": false }, "user": { "daily_cost_usd": 0.0 },
                "request": { "model": "claude-opus-4-5", "granularity": "turn" } }),
    );
    assert!(matches!(eval(vec![s], &turn).outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_013_ac2_an_async_session_uses_the_routed_model_and_effort() {
    let s = set(Scope::User, "u", ROUTING);
    let d = eval(vec![s], &start_with(Some("claude-opus-4-5"), true, 0.0));
    // Pass 2 sieht das umgeroutete Modell: die Allowlist lässt es zu.
    assert_eq!(d.outcome, Outcome::Allow, "{d:#?}");
    assert_eq!(d.modified.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(d.modified.reasoning_effort.as_deref(), Some("medium"));
}

#[test]
fn pol_013_first_matching_route_wins_without_conflicts() {
    let s = set(Scope::User, "u", ROUTING);
    // Beide Routen passen: die erste gewinnt, die zweite gilt nicht als Konflikt.
    let d = eval(vec![s.clone()], &start_with(Some("x"), true, 31.0));
    assert_eq!(d.modified.model.as_deref(), Some("claude-sonnet-4-5"));
    assert!(d.conflicts.is_empty(), "{:?}", d.conflicts);
    let d = eval(vec![s], &start_with(Some("x"), false, 31.0));
    assert_eq!(d.modified.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(d.modified.reasoning_effort, None);
}

#[test]
fn pol_013_ac3_without_model_switch_on_unsupported_denies_with_an_explanation() {
    let s = set(
        Scope::User,
        "u",
        "  - id: cheap\n    type: model_route\n    params: { routes: [{ model: claude-haiku-4-5 }] }\n",
    );
    let opts = EvalOptions {
        capabilities: Capabilities {
            model: false,
            ..Capabilities::default()
        },
        ..EvalOptions::default()
    };
    let turn = act(
        Phase::ModelRequest,
        json!({ "request": { "model": "claude-opus-4-5", "granularity": "turn" } }),
    );
    let d = EffectiveSet::new(vec![s]).evaluate(&turn, &opts);
    assert_eq!(reason_ids(&d), ["cheap"]);
    let text = reason_text(&d);
    assert!(text.contains("model"), "{text}");
    assert!(text.contains("nicht"), "{text}");
    // `on_unsupported: skip` verwirft die Modifikation mit Warnung.
    let skip = set(
        Scope::User,
        "u",
        "  - id: cheap\n    type: model_route\n    params: { on_unsupported: skip, routes: [{ model: claude-haiku-4-5 }] }\n",
    );
    let d = EffectiveSet::new(vec![skip]).evaluate(&turn, &opts);
    assert_eq!(d.outcome, Outcome::Allow);
    assert!(!d.warnings.is_empty());
}

#[test]
fn pol_013_routes_may_use_phase_specific_variables() {
    // `request.*` gibt es nur in `model_request`: die Route gilt dann nur dort.
    let s = set(
        Scope::Project,
        "p",
        "  - id: no-opus\n    type: model_route\n    params: { routes: [{ when: 'request.model.startsWith(\"claude-opus\")', model: claude-sonnet-4-5 }] }\n",
    );
    let phases: Vec<Phase> = s
        .rules
        .iter()
        .map(|r| match &r.kind {
            RuleKind::Cel { phase, .. } => *phase,
            RuleKind::Builtin(_) => panic!("entzuckert erwartet"),
        })
        .collect();
    assert_eq!(phases, [Phase::ModelRequest]);
    let turn = act(
        Phase::ModelRequest,
        json!({ "request": { "model": "claude-opus-4-5", "granularity": "turn" } }),
    );
    assert_eq!(
        eval(vec![s], &turn).modified.model.as_deref(),
        Some("claude-sonnet-4-5")
    );
}

#[test]
fn pol_013_params_are_checked_on_load() {
    let rule = |params: &str| format!("  - id: x\n    type: model_route\n    params: {params}\n");
    for (params, needle) in [
        ("{}", "allowed_models"),
        ("{ allowed_models: [] }", "allowed_models"),
        ("{ allowed_models: claude }", "allowed_models"),
        (
            "{ routes: [{ when: 'session.cost_usd > \"5\"', model: x }] }",
            "routes[0].when",
        ),
        ("{ routes: [{ when: 'session.is_async' }] }", "routes[0]"),
        ("{ routes: [{ model: x, temperature: 1 }] }", "temperature"),
        ("{ routes: [{ model: x }, { model: y }] }", "routes[1]"),
        (
            "{ routes: [{ model: x, reasoning_effort: max }] }",
            "reasoning_effort",
        ),
        (
            "{ allowed_models: [x], on_unsupported: never }",
            "on_unsupported",
        ),
        (
            "{ allowed_models: [x], fallback_model: y }",
            "fallback_model",
        ),
        ("{ allowed_models: [1] }", "allowed_models[0]"),
        ("{ routes: claude }", "routes"),
        ("{ routes: [x] }", "routes[0]"),
        ("{ routes: [{ model: 1 }] }", "routes[0].model"),
        (
            "{ allowed_models: [x], on_unsupported: 1 }",
            "on_unsupported",
        ),
    ] {
        let e = load_err(&rule(params));
        assert!(e.contains(needle), "{params}: {e}");
    }
}

// --------------------------------------------------- Hot-Reload (POL-001, #253), POL-009

#[test]
fn pol_001_new_budget_rules_tighten_but_routing_does_not() {
    let base = "  - id: keep\n    on: tool_call\n    action: deny\n";
    let with = |added: &str| {
        let old = set(Scope::Project, "p", base);
        let new = set(Scope::Project, "p", &format!("{base}{added}"));
        only_tightens(Some(&old), &new)
    };
    // `deny`/`ask` samt eigener Schwellen-Zustände verschärfen.
    assert_eq!(with(SPEND_CAP), Ok(()));
    assert_eq!(with(DAILY), Ok(()));
    assert_eq!(
        with("  - id: c\n    type: spend_cap\n    params: { limit_usd: 5, on_exceed: ask }\n"),
        Ok(())
    );
    assert_eq!(
        with(
            "  - id: allow-list\n    type: model_route\n    params: { allowed_models: [\"claude-*\"] }\n"
        ),
        Ok(())
    );
    // Umrouten ist `modify`: erst in einer neuen Session.
    for added in [
        "  - id: r\n    type: model_route\n    params: { routes: [{ model: claude-haiku-4-5 }] }\n",
        "  - id: c\n    type: spend_cap\n    params: { limit_usd: 5, on_exceed: route, fallback_model: claude-haiku-4-5 }\n",
    ] {
        assert!(with(added).is_err(), "{added}");
    }
    // Ein geändertes Limit ist eine Änderung einer bestehenden Regel.
    let old = set(Scope::Project, "p", SPEND_CAP);
    let new = set(
        Scope::Project,
        "p",
        &SPEND_CAP.replace("limit_usd: 5", "limit_usd: 50"),
    );
    assert!(only_tightens(Some(&old), &new).is_err());
    // Unverändert bleibt unverändert.
    assert_eq!(only_tightens(Some(&old), &old.clone()), Ok(()));
}

#[test]
fn pol_009_handwritten_effects_cannot_write_keys_of_rule_types() {
    let e = load_err(
        "  - id: sneaky\n    on: tool_call\n    action: notify\n    effects: [{ op: set, scope: session, key: \"spend_cap/user/u/budget/asked\", value: \"99.0\" }]\n",
    );
    assert!(e.contains("spend_cap/"), "{e}");
    let e = load_err(
        "  - id: sneaky\n    on: tool_call\n    action: notify\n    effects: [{ op: delete, scope: user_day, key: \"daily_budget/user/u/daily/asked\" }]\n",
    );
    assert!(e.contains("reserviert"), "{e}");
}

// ------------------------------------------------------------------ API-006 AC4

#[test]
fn api_006_ac4_the_session_budget_is_a_user_level_spend_cap() {
    use beton_policy::budget::{
        BudgetScope, SESSION_BUDGET_RULE, SESSION_BUDGET_SET, budget_rules, session_budget,
    };
    let set = Arc::new(session_budget(0.5).unwrap());
    assert_eq!(set.scope, Scope::User);
    assert_eq!(set.name, SESSION_BUDGET_SET);
    assert!(set.rules.iter().all(|r| r.id == SESSION_BUDGET_RULE
        && r.doc.rule_type == Some(beton_policy::doc::RuleType::SpendCap)));
    // Der Lauf samt Sub-Agents zählt (Security-Review #284, M2): `scope: subtree`.
    let effective = EffectiveSet::new(vec![set.clone()]);
    assert_eq!(budget_rules(&effective)[0].scope, BudgetScope::Subtree);
    let turn = |session: f64, tree: f64| {
        act(
            Phase::ModelRequest,
            json!({
                "session": { "cost_usd": session, "equivalent_usd": 0.0 },
                "run": { "cost_usd": session, "equivalent_usd": 0.0,
                         "subtree_cost_usd": tree, "subtree_equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        )
    };
    let d = eval(vec![set.clone()], &turn(0.1, 0.6));
    assert_eq!(reason_ids(&d), [SESSION_BUDGET_RULE]);
    assert!(
        reason_text(&d).contains("--max-cost"),
        "{}",
        reason_text(&d)
    );
    assert_eq!(eval(vec![set], &turn(0.1, 0.4)).outcome, Outcome::Allow);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(session_budget(bad).is_err(), "{bad}");
    }
}

// ------------------------------------------------------------ Meldungen des Runners

#[test]
fn pol_011_budget_exhaustion_is_recognized_from_the_decision() {
    use beton_policy::budget::{BudgetScope, exhausted};
    let s = set(
        Scope::User,
        "u",
        &format!("{SPEND_CAP}{DAILY}  - id: plain\n    on: model_request\n    action: deny\n"),
    );
    let effective = EffectiveSet::new(vec![s]);
    let d = effective.evaluate(
        &act(
            Phase::ModelRequest,
            json!({
                "session": { "cost_usd": 6.0, "equivalent_usd": 0.0 },
                "user": { "daily_cost_usd": 41.0, "daily_equivalent_usd": 0.0 },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        ),
        &EvalOptions::default(),
    );
    // Nur das erreichte Limit zählt, nicht die Schwelle von `daily` und nicht `plain`.
    let found = exhausted(&d, &effective);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].rule.rule_id, "budget");
    assert_eq!(found[0].scope, BudgetScope::Session);
    assert_eq!(found[0].action, beton_core::event::BudgetAction::Deny);
}

#[test]
fn pol_011_budget_rules_name_their_cost_scope() {
    use beton_policy::budget::{BudgetScope, budget_rules};
    let s = set(
        Scope::User,
        "u",
        &format!(
            "{SPEND_CAP}{DAILY}  - id: tree\n    type: spend_cap\n    params: {{ scope: subtree, limit_usd: 9 }}\n{ROUTING}"
        ),
    );
    let rules = budget_rules(&EffectiveSet::new(vec![s]));
    let ids: Vec<(&str, BudgetScope)> = rules
        .iter()
        .map(|r| (r.rule.rule_id.as_str(), r.scope))
        .collect();
    assert_eq!(
        ids,
        [
            ("budget", BudgetScope::Session),
            ("daily", BudgetScope::User),
            ("tree", BudgetScope::Subtree),
        ]
    );
}

// ------------------------------------- Security-Review #284: Modelle ohne Preis (M1, M2)

#[test]
fn pol_002_priced_knows_the_list_prices_of_the_catalog() {
    let s = set(
        Scope::User,
        "u",
        "  - id: p\n    on: model_request\n    when: 'priced(request.model)'\n    action: deny\n",
    );
    let turn = |model: &str| {
        act(
            Phase::ModelRequest,
            json!({ "request": { "model": model, "granularity": "turn" } }),
        )
    };
    for (model, priced) in [
        ("claude-sonnet-4-5", true),
        ("claude-sonnet-4-5-20250929", true),
        ("gpt-5.3-codex", true),
        ("modell-ohne-preis", false),
        ("fake-large", false),
    ] {
        let d = eval(vec![s.clone()], &turn(model));
        assert_eq!(matches!(d.outcome, Outcome::Deny { .. }), priced, "{model}");
    }
}

fn unpriced_turn(model: &str, session: Value) -> Activation {
    let mut session = session;
    for f in ["cost_usd", "equivalent_usd"] {
        if session.get(f).is_none() {
            session[f] = json!(0.0);
        }
    }
    act(
        Phase::ModelRequest,
        json!({
            "session": session,
            "request": { "model": model, "granularity": "turn" },
            "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
        }),
    )
}

#[test]
fn pol_011_on_unpriced_asks_once_per_session_and_model() {
    policytest(
        &format!("spec_version: 1\nname: b\nrules:\n{SPEND_CAP}"),
        r#"spec_version: 1
layers: { project: [b.yaml] }
cases:
  - name: Ein Modell ohne Katalogpreis fragt einmal, ein zweites wieder
    steps:
      - { phase: model_request, input: { request: { model: modell-ohne-preis, granularity: turn } },
          expect: { outcome: ask, rules: [budget], reason_contains: Katalogpreis }, resolve: { decision: allow } }
      - { phase: model_request, input: { request: { model: modell-ohne-preis, granularity: turn } },
          expect: { outcome: allow } }
      - { phase: model_request, input: { request: { model: noch-eins-ohne-preis, granularity: turn } },
          expect: { outcome: ask, rules: [budget] }, resolve: { decision: allow } }
      - { phase: model_request, input: { request: { model: claude-sonnet-4-5, granularity: turn } },
          expect: { outcome: allow } }
  - name: Schon genutzte Modelle ohne Preis zählen auch bei einem Modell mit Preis
    phase: model_request
    given: { session: { unpriced_models: [modell-ohne-preis] } }
    input: { request: { model: claude-sonnet-4-5, granularity: turn } }
    expect: { outcome: ask, rules: [budget], reason_contains: Katalogpreis }
  - name: Ein abgelehntes Modell fragt beim nächsten Turn erneut
    steps:
      - { phase: model_request, input: { request: { model: modell-ohne-preis, granularity: turn } },
          expect: { outcome: ask }, resolve: { decision: deny } }
      - { phase: model_request, input: { request: { model: modell-ohne-preis, granularity: turn } },
          expect: { outcome: ask } }
"#,
    );
}

#[test]
fn pol_011_on_unpriced_deny_and_ignore() {
    let rule = |mode: &str| {
        set(
            Scope::User,
            "u",
            &format!(
                "  - id: budget\n    type: spend_cap\n    params: {{ limit_usd: 5, on_unpriced: {mode} }}\n"
            ),
        )
    };
    let d = eval(
        vec![rule("deny")],
        &unpriced_turn("modell-ohne-preis", json!({})),
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:#?}");
    assert!(reason_text(&d).contains("Katalogpreis"));
    let d = eval(
        vec![rule("ignore")],
        &unpriced_turn("modell-ohne-preis", json!({})),
    );
    assert_eq!(d.outcome, Outcome::Allow);
    // `ask` (Default) merkt sich das Modell bei der Freigabe.
    let d = eval(
        vec![rule("ask")],
        &unpriced_turn("modell-ohne-preis", json!({})),
    );
    assert!(matches!(d.outcome, Outcome::Ask { .. }));
    assert!(
        d.effects[0].ops[0].key.ends_with("/unpriced"),
        "{:?}",
        d.effects
    );
    assert_eq!(
        d.effects[0].ops[0].value,
        Some(json!(["modell-ohne-preis"]))
    );
    assert!(
        load_err(
            "  - id: x\n    type: spend_cap\n    params: { limit_usd: 5, on_unpriced: never }\n"
        )
        .contains("on_unpriced")
    );
    // Auch `daily_budget` kennt den Parameter (Bereich: genutzte Modelle des Tages).
    let daily = set(
        Scope::User,
        "u",
        "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50 }\n",
    );
    let d = eval(
        vec![daily],
        &act(
            Phase::ModelRequest,
            json!({
                "user": { "daily_cost_usd": 0.0, "daily_equivalent_usd": 0.0,
                          "daily_unpriced_models": ["modell-ohne-preis"] },
                "request": { "model": "claude-sonnet-4-5", "granularity": "turn" },
                "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
            }),
        ),
    );
    assert!(matches!(d.outcome, Outcome::Ask { .. }), "{d:#?}");
}

#[test]
fn pol_013_routes_below_the_user_level_to_unpriced_models_are_denied_while_a_budget_is_active() {
    let budget = set(Scope::User, "u", SPEND_CAP);
    let route = |scope: Scope, model: &str| {
        set(
            scope,
            "r",
            &format!(
                "  - id: cheap\n    type: model_route\n    params: {{ routes: [{{ model: {model} }}] }}\n"
            ),
        )
    };
    let turn = unpriced_turn("claude-sonnet-4-5", json!({}));
    // Projekt-Route auf ein Modell ohne Preis: abgelehnt, solange ein Budget gilt.
    let d = eval(
        vec![budget.clone(), route(Scope::Project, "modell-ohne-preis")],
        &turn,
    );
    assert_eq!(reason_ids(&d), ["cheap"], "{d:#?}");
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    assert!(
        reason_text(&d).contains("Katalogpreis"),
        "{}",
        reason_text(&d)
    );
    // Ebenso auf Agent-Ebene.
    let d = eval(
        vec![budget.clone(), route(Scope::Agent, "modell-ohne-preis")],
        &turn,
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    // Eine Route der User-Ebene fragt wie jedes Modell ohne Preis (`on_unpriced: ask`).
    let d = eval(
        vec![budget.clone(), route(Scope::User, "modell-ohne-preis")],
        &turn,
    );
    assert!(matches!(d.outcome, Outcome::Ask { .. }), "{d:#?}");
    // Mit Katalogpreis bzw. ohne Budget bleibt die Projekt-Route erlaubt.
    let d = eval(
        vec![budget, route(Scope::Project, "claude-haiku-4-5")],
        &turn,
    );
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.modified.model.as_deref(), Some("claude-haiku-4-5"));
    let d = eval(vec![route(Scope::Project, "modell-ohne-preis")], &turn);
    assert_eq!(d.outcome, Outcome::Allow);
    // Auch ein `on_exceed: route` eines Repos auf ein Modell ohne Preis gilt als Umgehung.
    let repo = set(
        Scope::Project,
        "p",
        "  - id: repo-cap\n    type: spend_cap\n    params: { limit_usd: 0.01, on_exceed: route, fallback_model: modell-ohne-preis }\n",
    );
    let d = eval(
        vec![repo],
        &unpriced_turn("claude-sonnet-4-5", json!({ "cost_usd": 1.0 })),
    );
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:#?}");
}

#[test]
fn pol_011_an_unpriced_start_model_of_a_repo_agent_or_spawn_is_denied() {
    let budget = set(Scope::User, "u", SPEND_CAP);
    let start = |extra: Value| {
        let mut ctx = json!({
            "session": { "cost_usd": 0.0, "equivalent_usd": 0.0 },
            "start": { "trigger": "interactive", "harness": "claude", "model": "modell-ohne-preis" },
            "state": { "session": {}, "run": {}, "user_day": {}, "user": {} },
        });
        if let (Some(c), Value::Object(e)) = (ctx.as_object_mut(), extra) {
            for (k, v) in e {
                if let (Some(Value::Object(dst)), Value::Object(src)) = (c.get_mut(&k), &v) {
                    dst.extend(src.clone());
                } else {
                    c.insert(k, v);
                }
            }
        }
        act(Phase::SessionStart, ctx)
    };
    for extra in [
        json!({ "agent": { "name": "repo", "source": "project" } }),
        json!({ "agent": { "name": "x", "source": "path" } }),
        json!({ "agent": { "name": "kind", "source": "subagent" } }),
        json!({ "start": { "trigger": "spawn" } }),
    ] {
        let d = eval(vec![budget.clone()], &start(extra.clone()));
        assert!(matches!(d.outcome, Outcome::Deny { .. }), "{extra}: {d:#?}");
        assert!(reason_text(&d).contains("Katalogpreis"), "{extra}");
    }
    // Vom User gewählt (eigener Agent, Built-in oder ohne Agent): Rückfrage.
    for extra in [
        json!({ "agent": { "name": "mine", "source": "user" } }),
        json!({ "agent": { "name": "maestra", "source": "builtin" } }),
        json!({}),
    ] {
        let d = eval(vec![budget.clone()], &start(extra.clone()));
        assert!(matches!(d.outcome, Outcome::Ask { .. }), "{extra}: {d:#?}");
    }
}

#[test]
fn pol_012_threshold_keys_below_the_user_level_carry_the_project_identity() {
    let text = "spec_version: 1\nname: daily\nrules:\n  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, ask_at_usd: [40] }\n";
    let a =
        Arc::new(parse_set(Scope::Project, "/repo-a/.beton/policies/daily.yaml", text).unwrap());
    let b =
        Arc::new(parse_set(Scope::Project, "/repo-b/.beton/policies/daily.yaml", text).unwrap());
    let key = |s: &Arc<LoadedSet>| {
        let d = eval(vec![s.clone()], &user_request(41.0, 0.0));
        d.effects[0].ops[0].key.clone()
    };
    let (ka, kb) = (key(&a), key(&b));
    assert!(ka.starts_with("daily_budget/project/"), "{ka}");
    assert_ne!(ka, kb, "Schwellen eines Repos gelten nicht für ein anderes");
    // Die User-Ebene bleibt repo-übergreifend (ein Tagesbudget für alle Sessions).
    let u = set(
        Scope::User,
        "daily",
        "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, ask_at_usd: [40] }\n",
    );
    assert_eq!(key(&u), "daily_budget/user/daily/daily/asked");
    // Hot-Reload erkennt die eigenen Keys weiter als eigene.
    let base = "  - id: keep\n    on: tool_call\n    action: deny\n";
    let old = Arc::new(
        parse_set(
            Scope::Project,
            "/repo-a/.beton/policies/daily.yaml",
            &format!("spec_version: 1\nname: daily\nrules:\n{base}"),
        )
        .unwrap(),
    );
    let new = Arc::new(
        parse_set(
            Scope::Project,
            "/repo-a/.beton/policies/daily.yaml",
            &format!("{text}{base}"),
        )
        .unwrap(),
    );
    assert_eq!(only_tightens(Some(&old), &new), Ok(()));
}
