//! POL-009/POL-010: Randfälle von Zustand, Grants, Effects und `require_approval` – Events,
//! Fehlerpfade und fail closed.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use beton_core::event::PolicyStateChanged;
use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::doc::{EffectOp, StateScope};
use beton_policy::engine::{EffectiveSet, EvalOptions, Outcome};
use beton_policy::grants::SessionGrants;
use beton_policy::load::{LoadedSet, Scope, parse_set};
use beton_policy::state::{PolicyState, StateChange, next_value, op_str, scope_str};
use serde_json::json;

fn set(body: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: p\n{body}");
    Arc::new(parse_set(Scope::Project, "p.yaml", &text).unwrap_or_else(|e| panic!("{e}\n{text}")))
}

fn load_err(body: &str) -> String {
    parse_set(
        Scope::Project,
        "p.yaml",
        &format!("spec_version: 1\nname: p\n{body}"),
    )
    .unwrap_err()
    .message
}

fn shell(command: &str) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": command, "args": { "command": command } } }),
    )
    .unwrap()
}

#[test]
fn pol_009_state_changed_events_roundtrip() {
    for (scope, s) in [
        (StateScope::Session, "session"),
        (StateScope::Run, "run"),
        (StateScope::UserDay, "user_day"),
        (StateScope::User, "user"),
    ] {
        assert_eq!(scope_str(scope), s);
    }
    for (op, s) in [
        (EffectOp::Set, "set"),
        (EffectOp::Increment, "increment"),
        (EffectOp::Append, "append"),
        (EffectOp::Delete, "delete"),
    ] {
        assert_eq!(op_str(op), s);
        let change = StateChange {
            scope: StateScope::Run,
            key: "k".into(),
            op,
            value: (op != EffectOp::Delete).then(|| json!(1)),
        };
        let event = change.to_event();
        assert_eq!(event.op, s);
        assert_eq!(StateChange::from_event(&event), Some(change));
    }
    let deleted = StateChange::from_event(&PolicyStateChanged {
        scope: "session".into(),
        key: "k".into(),
        op: "delete".into(),
        value: json!(null),
    })
    .unwrap();
    assert_eq!(deleted.value, None);
    // Unbekanntes wird ignoriert.
    let bad = |scope: &str, op: &str| PolicyStateChanged {
        scope: scope.into(),
        key: "k".into(),
        op: op.into(),
        value: json!(1),
    };
    assert_eq!(StateChange::from_event(&bad("team", "set")), None);
    assert_eq!(StateChange::from_event(&bad("session", "multiply")), None);
}

#[test]
fn pol_009_state_scopes_replay_and_run_reset() {
    let mut s = PolicyState::default();
    s.apply(StateScope::Run, "tries", EffectOp::Increment, None)
        .unwrap();
    s.apply(
        StateScope::Session,
        "log",
        EffectOp::Append,
        Some(json!("a")),
    )
    .unwrap();
    assert_eq!(s.snapshot(StateScope::Run)["tries"], json!(1));
    assert!(s.snapshot(StateScope::User).is_empty());
    assert!(s.get(StateScope::UserDay, "x").is_none());
    // `user`/`user_day` liegen im Store.
    let err = s
        .apply(StateScope::User, "x", EffectOp::Increment, None)
        .unwrap_err();
    assert!(err.to_string().contains("Store"), "{err}");
    // Delete im Replay entfernt den Key.
    let gone = s
        .apply(StateScope::Session, "log", EffectOp::Delete, None)
        .unwrap();
    s.restore(&gone);
    assert!(s.get(StateScope::Session, "log").is_none());
    // Änderungen an User-Scopes ignoriert das Replay.
    s.restore(&StateChange {
        scope: StateScope::User,
        key: "u".into(),
        op: EffectOp::Set,
        value: Some(json!(1)),
    });
    s.reset_run();
    assert!(s.snapshot(StateScope::Run).is_empty());
}

#[test]
fn pol_009_operation_errors_leave_the_value_alone() {
    // append auf einen Wert ohne Liste; Null zählt als leere Liste.
    assert!(next_value(Some(&json!(3)), EffectOp::Append, Some(json!(1))).is_err());
    assert!(next_value(Some(&json!({"a": 1})), EffectOp::Append, Some(json!(1))).is_err());
    assert!(next_value(Some(&json!(true)), EffectOp::Append, Some(json!(1))).is_err());
    assert_eq!(
        next_value(Some(&json!(null)), EffectOp::Append, Some(json!(1))).unwrap(),
        Some(json!([1]))
    );
    assert!(
        next_value(None, EffectOp::Append, None).is_err(),
        "append ohne Wert"
    );
    // increment: Überlauf wechselt auf Gleitkomma, Text ist ein Fehler.
    let big = next_value(Some(&json!(i64::MAX)), EffectOp::Increment, None)
        .unwrap()
        .unwrap();
    assert!(big.is_f64());
    assert!(next_value(Some(&json!(1)), EffectOp::Increment, Some(json!("x"))).is_err());
    assert!(next_value(Some(&json!([1])), EffectOp::Increment, None).is_err());
    assert!(next_value(Some(&json!(null)), EffectOp::Increment, None).is_err());
}

#[test]
fn pol_010_grants_are_deduplicated_and_listed() {
    let s = set("rules:\n  - id: c\n    on: tool_call\n    action: ask\n");
    let d = EffectiveSet::new(vec![s]).evaluate(&shell("ls"), &EvalOptions::default());
    let Outcome::Ask { reasons } = d.outcome else {
        panic!("{d:?}")
    };
    let mut g = SessionGrants::default();
    assert!(g.is_empty());
    let id = g.add(&reasons[0].rule, "shell:ls");
    g.add_with_id(&reasons[0].rule, "shell:ls", "grt_other");
    assert_eq!(g.all().len(), 1);
    assert_eq!(g.all()[0].id, id);
    assert!(!g.is_empty());
}

#[test]
fn pol_010_remember_key_without_text_is_not_rememberable() {
    // Ergebnis kein Text bzw. Fehler: kein Fingerprint, Fehler protokolliert.
    let s = set(
        "rules:\n  - id: k\n    on: tool_call\n    action: ask\n    remember_key: 'tool.args.missing'\n",
    );
    let d = EffectiveSet::new(vec![s]).evaluate(&shell("ls"), &EvalOptions::default());
    let Outcome::Ask { reasons } = &d.outcome else {
        panic!("{d:?}")
    };
    assert_eq!(reasons[0].fingerprint, None);
    assert!(
        d.errors.iter().any(|e| e.error.contains("remember_key")),
        "{d:?}"
    );
}

#[test]
fn pol_009_broken_templates_and_effect_values_do_not_change_the_decision() {
    let s = set(
        "rules:\n  - id: n\n    on: tool_call\n    action: notify\n    notify: { message: 'Wert {{ tool.args.missing }} fertig' }\n    effects:\n      - { op: set, scope: session, key: v, value: 'tool.args.missing' }\n      - { op: increment, scope: session, key: n }\n",
    );
    let d = EffectiveSet::new(vec![s]).evaluate(&shell("ls"), &EvalOptions::default());
    assert_eq!(d.outcome, Outcome::Allow);
    assert_eq!(d.notifications[0].message.as_deref(), Some("Wert ? fertig"));
    // Der fehlerhafte Effect fehlt, der andere bleibt.
    let keys: Vec<_> = d.effects[0].ops.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["n"]);
    assert!(d.errors.len() >= 2, "{:?}", d.errors);
}

#[test]
fn pol_009_effects_need_values_and_keys() {
    assert!(load_err("rules:\n  - id: e\n    on: tool_call\n    action: allow\n    effects:\n      - { op: set, scope: session, key: x }\n").contains("value"));
    assert!(load_err("rules:\n  - id: e\n    on: tool_call\n    action: allow\n    effects:\n      - { op: append, scope: run, key: x }\n").contains("value"));
    assert!(load_err("rules:\n  - id: e\n    on: tool_call\n    action: allow\n    effects:\n      - { op: increment, scope: run, key: \"\" }\n").contains("leer"));
    assert!(load_err("rules:\n  - id: t\n    on: tool_call\n    action: ask\n    approval: { timeout: -5m }\n").contains("positiv"));
}

#[test]
fn pol_014_param_types_are_checked() {
    let rule = |params: &str| {
        format!("rules:\n  - id: r\n    type: require_approval\n    params: {params}\n")
    };
    assert!(load_err(&rule("{ kinds: shell }")).contains("Liste"));
    assert!(load_err(&rule("{ tools: [1] }")).contains("Text"));
    assert!(load_err(&rule("{ match: [1] }")).contains("Text"));
    assert!(load_err(&rule("{ kinds: [shell], timeout: 0s }")).contains("positiv"));
    // Ohne `params` fehlt jedes Kriterium.
    assert!(load_err("rules:\n  - id: r\n    type: require_approval\n").contains("mindestens"));
    // Eigene Begründung bleibt, on_timeout allow ist erlaubt.
    let s = set(
        "rules:\n  - id: r\n    type: require_approval\n    reason: \"Bitte prüfen\"\n    params: { servers: [github], on_timeout: allow, remember: session }\n",
    );
    let act = Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "x", "server": "github", "kind": "mcp", "args": {} } }),
    )
    .unwrap();
    let d = EffectiveSet::new(vec![s]).evaluate(&act, &EvalOptions::default());
    let Outcome::Ask { reasons } = d.outcome else {
        panic!("{d:?}")
    };
    assert_eq!(reasons[0].reason, "Bitte prüfen");
}
