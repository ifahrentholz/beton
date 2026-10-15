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

fn act(phase: Phase, ctx: Value) -> Activation {
    Activation::new(phase, ctx).unwrap_or_else(|e| panic!("{e}"))
}

/// Kontext für `model_request` mit Kosten der Session.
fn request(session: Value) -> Activation {
    act(
        Phase::ModelRequest,
        json!({
            "session": session,
            "request": { "model": "claude-opus-4-1", "granularity": "turn" },
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
      - { phase: model_request, input: { request: { model: m, granularity: turn } }, expect: { outcome: allow } }
      - { cost_delta_usd: 3.1 }
      - { phase: model_request, input: { request: { model: m, granularity: turn } },
          expect: { outcome: ask, rules: [budget], reason_contains: "3" }, resolve: { decision: allow } }
      - { phase: model_request, input: { request: { model: m, granularity: turn } }, expect: { outcome: allow } }
      - { cost_delta_usd: 1.9 }
      - { phase: model_request, input: { request: { model: m, granularity: turn } },
          expect: { outcome: deny, rules: [budget], reason_contains: "5" } }
    expect_state: { session: { "spend_cap/project/b/budget/asked": 3.0 } }
  - name: Abgelehnte Schwelle fragt beim nächsten Turn erneut
    given: { session: { cost_usd: 3.5 } }
    steps:
      - { phase: model_request, input: { request: { model: m, granularity: turn } },
          expect: { outcome: ask, rules: [budget] }, resolve: { decision: deny } }
      - { phase: model_request, input: { request: { model: m, granularity: turn } },
          expect: { outcome: ask, rules: [budget] } }
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
    let d = eval(vec![s], &request(json!({ "cost_usd": 21.0, "equivalent_usd": 0.0 })));
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
                "request": { "model": "m", "granularity": "turn" },
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
                "request": { "model": "m", "granularity": "turn" },
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
            rules.iter().any(|r| r.phases == [phase] && r.action == "deny"),
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
                "request": { "model": "m", "granularity": "turn" },
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
    ] {
        let e = load_err(&rule(params));
        assert!(e.contains(needle), "{params}: {e}");
    }
    // `type` schließt eigene Bedingungen und Effects weiter aus.
    assert!(load_err("  - id: x\n    type: spend_cap\n    on: model_request\n    params: { limit_usd: 1 }\n").contains("on"));
}

// ------------------------------------------------------------------------- POL-012

const DAILY: &str = "  - id: daily\n    type: daily_budget\n    params: { limit_usd: 50, ask_at_usd: [40] }\n";

fn user_request(daily: f64, equivalent: f64) -> Activation {
    act(
        Phase::ModelRequest,
        json!({
            "user": { "daily_cost_usd": daily, "daily_equivalent_usd": equivalent, "timezone": "Europe/Berlin" },
            "request": { "model": "m", "granularity": "turn" },
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
                "start": { "trigger": "interactive", "harness": "claude", "model": "claude-opus-4-1" },
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
}

#[test]
fn pol_012_team_scope_and_own_timezone_follow_with_m4() {
    let rule = |params: &str| format!("  - id: x\n    type: daily_budget\n    params: {params}\n");
    assert!(load_err(&rule("{ scope: team, limit_usd: 5 }")).contains("M4"));
    assert!(load_err(&rule("{ limit_usd: 5, timezone: Europe/Berlin }")).contains("timezone"));
    assert!(load_err(&rule("{ limit_usd: 5, fallback_effort: low, on_exceed: route, fallback_model: x }")).contains("fallback_effort"));
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
        &start_with(Some("claude-opus-4-1"), false, 0.0),
    );
    assert_eq!(reason_ids(&d), ["routing"]);
    assert!(matches!(d.outcome, Outcome::Deny { .. }));
    let text = reason_text(&d);
    assert!(text.contains("claude-sonnet-*"), "{text}");
    assert_eq!(
        eval(vec![s.clone()], &start_with(Some("claude-sonnet-4-5"), false, 0.0)).outcome,
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
                "request": { "model": "claude-opus-4-1", "granularity": "turn" } }),
    );
    assert!(matches!(eval(vec![s], &turn).outcome, Outcome::Deny { .. }));
}

#[test]
fn pol_013_ac2_an_async_session_uses_the_routed_model_and_effort() {
    let s = set(Scope::User, "u", ROUTING);
    let d = eval(vec![s], &start_with(Some("claude-opus-4-1"), true, 0.0));
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
        json!({ "request": { "model": "claude-opus-4-1", "granularity": "turn" } }),
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
        json!({ "request": { "model": "claude-opus-4-1", "granularity": "turn" } }),
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
        ("{ routes: [{ when: 'session.cost_usd > \"5\"', model: x }] }", "routes[0].when"),
        ("{ routes: [{ when: 'session.is_async' }] }", "routes[0]"),
        ("{ routes: [{ model: x, temperature: 1 }] }", "temperature"),
        ("{ routes: [{ model: x }, { model: y }] }", "routes[1]"),
        ("{ routes: [{ model: x, reasoning_effort: max }] }", "reasoning_effort"),
        ("{ allowed_models: [x], on_unsupported: never }", "on_unsupported"),
        ("{ allowed_models: [x], fallback_model: y }", "fallback_model"),
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
        with("  - id: allow-list\n    type: model_route\n    params: { allowed_models: [\"claude-*\"] }\n"),
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
    let new = set(Scope::Project, "p", &SPEND_CAP.replace("limit_usd: 5", "limit_usd: 50"));
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
                "request": { "model": "m", "granularity": "turn" },
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
