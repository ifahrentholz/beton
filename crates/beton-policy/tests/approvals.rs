//! POL-009 (Zustand & Zähler), POL-010 (Approvals & Approval-Card) und POL-014
//! (`require_approval`) in der Engine: Kombination von Timeout und `on_timeout`, Optionen der
//! Card, Grant-Fingerprints, Effects und der Regeltyp `require_approval`.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use beton_policy::Phase;
use beton_policy::approval::{DEFAULT_TIMEOUT, plan};
use beton_policy::context::Activation;
use beton_policy::doc::{DefaultAction, EffectOp, StateScope, TimeoutAction};
use beton_policy::engine::{Decision, EffectiveSet, EvalOptions, Outcome, Reason};
use beton_policy::grants::SessionGrants;
use beton_policy::load::{LoadedSet, Scope, parse_set};
use beton_policy::normalize::{RawToolCall, normalize};
use beton_policy::state::{
    APPEND_MAX, FinalOutcome, PolicyState, StateChange, VALUE_MAX_BYTES, day_key, effects_due,
    next_value,
};
use serde_json::{Value, json};
use time::UtcOffset;
use time::macros::datetime;

fn set(scope: Scope, name: &str, body: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: {name}\n{body}");
    Arc::new(
        parse_set(scope, &format!("{name}.yaml"), &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn shell_tool(command: &str) -> Value {
    let args = json!({ "command": command });
    normalize(&RawToolCall {
        harness: "claude",
        native_name: "Bash",
        acp_kind: None,
        mcp_server: None,
        args: &args,
        cwd: "/work/repo",
        home: Some("/home/u"),
        trusted_servers: None,
    })
    .unwrap()
    .to_context()
}

fn shell(command: &str) -> Activation {
    Activation::new(Phase::ToolCall, json!({ "tool": shell_tool(command) })).unwrap()
}

fn eval(sets: &[Arc<LoadedSet>], act: &Activation) -> Decision {
    EffectiveSet::new(sets.to_vec()).evaluate(act, &EvalOptions::default())
}

fn ask_reasons(d: &Decision) -> Vec<Reason> {
    match &d.outcome {
        Outcome::Ask { reasons } => reasons.clone(),
        other => panic!("ask erwartet, war {other:?}"),
    }
}

fn ask_rule(id: &str, approval: &str) -> String {
    format!(
        "  - id: {id}\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: ask\n    reason: \"{id}\"\n    approval: {approval}\n"
    )
}

// ---------------------------------------------------------------------------
// POL-010
// ---------------------------------------------------------------------------

#[test]
fn pol_010_ac1_two_ask_rules_yield_one_card_with_two_reasons() {
    let project = set(
        Scope::Project,
        "git-and-budget",
        &format!(
            "rules:\n{}",
            ask_rule("confirm-destructive-shell", "{ timeout: 30m }")
        ),
    );
    let user = set(
        Scope::User,
        "defaults",
        &format!("rules:\n{}", ask_rule("git-guard", "{ timeout: 2h }")),
    );
    let d = eval(&[project, user], &shell("git push origin feature/x"));
    let reasons = ask_reasons(&d);
    let ids: Vec<_> = reasons.iter().map(|r| r.rule.rule_id.as_str()).collect();
    // Eine Card, Gründe kanonisch (User vor Projekt).
    assert_eq!(ids, ["git-guard", "confirm-destructive-shell"]);
}

#[test]
fn pol_010_ac5_shortest_timeout_and_strictest_on_timeout() {
    let a = set(
        Scope::Project,
        "a",
        &format!(
            "rules:\n{}",
            ask_rule("rule-a", "{ timeout: 30m, on_timeout: deny }")
        ),
    );
    let b = set(
        Scope::User,
        "b",
        &format!(
            "rules:\n{}",
            ask_rule("rule-b", "{ timeout: 2h, on_timeout: abort }")
        ),
    );
    let reasons = ask_reasons(&eval(&[a, b], &shell("rm -rf dist")));
    let p = plan(&reasons, None);
    assert_eq!(p.timeout, Duration::from_secs(30 * 60));
    assert_eq!(p.on_timeout, TimeoutAction::Abort);
}

#[test]
fn pol_010_timeout_falls_back_to_agent_then_24h() {
    let s = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("no-timeout", "{ on_timeout: deny }")),
    );
    let reasons = ask_reasons(&eval(&[s], &shell("git push")));
    assert_eq!(plan(&reasons, None).timeout, DEFAULT_TIMEOUT);
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(24 * 3600));
    assert_eq!(
        plan(&reasons, Some(Duration::from_secs(600))).timeout,
        Duration::from_secs(600)
    );
}

#[test]
fn pol_010_on_timeout_defaults_to_deny() {
    let s = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("plain", "{ timeout: 5m }")),
    );
    let reasons = ask_reasons(&eval(&[s], &shell("git push")));
    assert_eq!(plan(&reasons, None).on_timeout, TimeoutAction::Deny);
}

#[test]
fn pol_010_allow_on_timeout_only_without_veto() {
    let lenient = |scope, name: &str, id: &str, extra: &str| {
        set(
            scope,
            name,
            &format!(
                "rules:\n{}",
                ask_rule(id, &format!("{{ timeout: 5m, on_timeout: allow{extra} }}"))
            ),
        )
    };
    // Alle Regeln erlauben bei Ablauf: allow.
    let reasons = ask_reasons(&eval(
        &[lenient(Scope::Project, "p", "lenient", "")],
        &shell("npm test"),
    ));
    assert_eq!(plan(&reasons, None).on_timeout, TimeoutAction::Allow);
    // Eine Regel einer autoritativeren Ebene verbietet `allow` bei Ablauf: deny.
    let reasons = ask_reasons(&eval(
        &[
            lenient(Scope::Project, "p", "lenient", ""),
            lenient(Scope::User, "u", "strict", ", allow_on_timeout: false"),
        ],
        &shell("npm test"),
    ));
    assert_eq!(plan(&reasons, None).on_timeout, TimeoutAction::Deny);
    // Ein Verbot auf einer niedrigeren Ebene hebt das `allow` einer höheren nicht auf. (Die
    // Agent-Ebene kennt `on_timeout: allow` gar nicht, AGT-014; hier die Projekt-Ebene.)
    let reasons = ask_reasons(&eval(
        &[
            lenient(Scope::User, "u", "lenient", ""),
            lenient(Scope::Project, "a", "strict", ", allow_on_timeout: false"),
        ],
        &shell("npm test"),
    ));
    assert_eq!(plan(&reasons, None).on_timeout, TimeoutAction::Allow);
}

#[test]
fn pol_010_approve_session_only_if_all_rules_remember() {
    let remember = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("r1", "{ remember: session }")),
    );
    let reasons = ask_reasons(&eval(std::slice::from_ref(&remember), &shell("git push")));
    assert_eq!(
        plan(&reasons, None).options(),
        ["approve", "approve_session", "deny"]
    );
    let none = set(
        Scope::User,
        "u",
        &format!("rules:\n{}", ask_rule("r2", "{ remember: none }")),
    );
    let reasons = ask_reasons(&eval(&[remember, none], &shell("git push")));
    assert_eq!(plan(&reasons, None).options(), ["approve", "deny"]);
}

#[test]
fn pol_010_default_ask_is_not_rememberable() {
    let s = set(
        Scope::Project,
        "p",
        "defaults:\n  tool_call: ask\nrules: []\n",
    );
    let reasons = ask_reasons(&eval(&[s], &shell("ls")));
    assert_eq!(plan(&reasons, None).options(), ["approve", "deny"]);
}

#[test]
fn pol_010_ac2_grant_allows_same_command_only() {
    let s = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("confirm", "{ remember: session }")),
    );
    let sets = [s];
    let reasons = ask_reasons(&eval(&sets, &shell("git push origin feature/x")));
    let fp = reasons[0].fingerprint.clone().expect("Fingerprint");
    assert_eq!(
        fp,
        "shell:builtin/Bash@/work/repo:git push origin feature/x"
    );

    let mut grants = SessionGrants::default();
    let id = grants.add(&reasons[0].rule, &fp);
    let opts = EvalOptions {
        grants: Some(&grants),
        ..EvalOptions::default()
    };
    let effective = EffectiveSet::new(sets.to_vec());
    let same = effective.evaluate(&shell("git push origin feature/x"), &opts);
    assert_eq!(same.outcome, Outcome::Allow);
    assert_eq!(same.grants_used, [id]);
    let other = effective.evaluate(&shell("git push --tags origin"), &opts);
    assert!(matches!(other.outcome, Outcome::Ask { .. }), "{other:?}");
    assert!(other.grants_used.is_empty());
}

#[test]
fn pol_010_grant_survives_a_new_set_version() {
    let v1 = set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("confirm", "{ remember: session }")),
    );
    let reasons = ask_reasons(&eval(&[v1], &shell("git push")));
    let mut grants = SessionGrants::default();
    grants.add(&reasons[0].rule, reasons[0].fingerprint.as_deref().unwrap());
    // Gleiche Regel, anderer Inhalt des Sets (neue Version).
    let v2 = set(
        Scope::Project,
        "p",
        &format!(
            "description: neu\nrules:\n{}",
            ask_rule("confirm", "{ remember: session }")
        ),
    );
    let opts = EvalOptions {
        grants: Some(&grants),
        ..EvalOptions::default()
    };
    let d = EffectiveSet::new(vec![v2]).evaluate(&shell("git push"), &opts);
    assert_eq!(d.outcome, Outcome::Allow);
}

#[test]
fn pol_010_remember_key_defines_the_fingerprint() {
    let s = set(
        Scope::Project,
        "p",
        "rules:\n  - id: push\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: ask\n    remember_key: '\"git-push:\" + tool.cwd'\n",
    );
    let reasons = ask_reasons(&eval(&[s], &shell("git push --force")));
    assert_eq!(
        reasons[0].fingerprint.as_deref(),
        Some("git-push:/work/repo")
    );
}

#[test]
fn pol_010_fingerprint_without_command_hashes_the_args() {
    let s = set(
        Scope::Project,
        "p",
        "rules:\n  - id: mcp\n    on: tool_call\n    when: 'tool.kind == \"mcp\"'\n    action: ask\n",
    );
    let act = |args: Value| {
        Activation::new(
            Phase::ToolCall,
            json!({ "tool": { "name": "create_pull_request", "server": "github", "kind": "mcp", "args": args } }),
        )
        .unwrap()
    };
    let a = ask_reasons(&eval(
        std::slice::from_ref(&s),
        &act(json!({"b": 1, "a": 2})),
    ));
    let b = ask_reasons(&eval(
        std::slice::from_ref(&s),
        &act(json!({"a": 2, "b": 1})),
    ));
    let c = ask_reasons(&eval(&[s], &act(json!({"a": 3}))));
    let fp = a[0].fingerprint.clone().unwrap();
    assert!(
        fp.starts_with("mcp:github/create_pull_request:sha256:"),
        "{fp}"
    );
    assert_eq!(
        a[0].fingerprint, b[0].fingerprint,
        "unabhängig von der Reihenfolge"
    );
    assert_ne!(a[0].fingerprint, c[0].fingerprint);
}

#[test]
fn pol_010_no_fingerprint_outside_tool_call() {
    let s = set(
        Scope::User,
        "u",
        "rules:\n  - id: start\n    on: session_start\n    action: ask\n    approval: { remember: session }\n",
    );
    let act = Activation::new(
        Phase::SessionStart,
        json!({ "start": { "harness": "fake" } }),
    )
    .unwrap();
    let reasons = ask_reasons(&eval(&[s], &act));
    assert_eq!(reasons[0].fingerprint, None);
    assert_eq!(plan(&reasons, None).options(), ["approve", "deny"]);
}

// ---------------------------------------------------------------------------
// POL-014
// ---------------------------------------------------------------------------

fn tool_act(harness: &str, name: &str, acp_kind: Option<&str>, args: Value) -> Activation {
    let tool = normalize(&RawToolCall {
        harness,
        native_name: name,
        acp_kind,
        mcp_server: None,
        args: &args,
        cwd: "/work/repo",
        home: Some("/home/u"),
        trusted_servers: None,
    })
    .unwrap();
    Activation::new(Phase::ToolCall, json!({ "tool": tool.to_context() })).unwrap()
}

fn require(params: &str) -> Arc<LoadedSet> {
    set(
        Scope::Project,
        "p",
        &format!("rules:\n  - id: needs-human\n    type: require_approval\n    params: {params}\n"),
    )
}

#[test]
fn pol_014_ac1_kinds_ask_for_writes_of_all_harnesses() {
    let s = require("{ kinds: [file_write, file_edit] }");
    let cases = [
        tool_act(
            "claude",
            "Write",
            None,
            json!({"file_path": "/work/repo/a.txt", "content": "x"}),
        ),
        tool_act(
            "codex",
            "apply_patch",
            None,
            json!({"input": "*** Begin Patch\n*** Update File: a.txt\n@@\n-a\n+b\n*** End Patch\n"}),
        ),
        tool_act(
            "acp",
            "Edit a.txt",
            Some("edit"),
            json!({"path": "/work/repo/a.txt"}),
        ),
    ];
    for act in &cases {
        let reasons = ask_reasons(&eval(std::slice::from_ref(&s), act));
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].rule.rule_id, "needs-human");
    }
    // Lesen fragt nicht.
    let read = tool_act(
        "claude",
        "Read",
        None,
        json!({"file_path": "/work/repo/a.txt"}),
    );
    assert_eq!(eval(&[s], &read).outcome, Outcome::Allow);
}

#[test]
fn pol_014_ac2_remember_none_hides_approve_session() {
    let s = require("{ kinds: [shell], remember: none, timeout: 30m, on_timeout: abort }");
    let reasons = ask_reasons(&eval(&[s], &shell("git push")));
    let p = plan(&reasons, None);
    assert_eq!(p.options(), ["approve", "deny"]);
    assert_eq!(p.timeout, Duration::from_secs(30 * 60));
    assert_eq!(p.on_timeout, TimeoutAction::Abort);

    let s = require("{ kinds: [shell] }");
    let reasons = ask_reasons(&eval(&[s], &shell("git push")));
    assert_eq!(
        plan(&reasons, None).options(),
        ["approve", "approve_session", "deny"]
    );
}

#[test]
fn pol_014_tools_servers_and_match_are_or_combined() {
    let mcp = |server: &str, name: &str| {
        Activation::new(
            Phase::ToolCall,
            json!({ "tool": { "name": name, "native_name": format!("mcp__{server}__{name}"),
                              "server": server, "kind": "mcp", "args": {} } }),
        )
        .unwrap()
    };
    let s = require(
        "{ tools: [\"mcp:github/create_*\", \"Bash\"], servers: [linear], match: 'tool.kind == \"web_fetch\"' }",
    );
    let sets = [s];
    assert!(matches!(
        eval(&sets, &mcp("github", "create_pull_request")).outcome,
        Outcome::Ask { .. }
    ));
    assert_eq!(
        eval(&sets, &mcp("github", "list_issues")).outcome,
        Outcome::Allow
    );
    assert!(matches!(
        eval(&sets, &mcp("linear", "anything")).outcome,
        Outcome::Ask { .. }
    ));
    assert!(matches!(
        eval(&sets, &shell("ls")).outcome,
        Outcome::Ask { .. }
    ));
    let fetch = tool_act(
        "claude",
        "WebFetch",
        None,
        json!({"url": "https://example.com/"}),
    );
    assert!(matches!(eval(&sets, &fetch).outcome, Outcome::Ask { .. }));
    let read = tool_act(
        "claude",
        "Read",
        None,
        json!({"file_path": "/work/repo/a.txt"}),
    );
    assert_eq!(eval(&sets, &read).outcome, Outcome::Allow);
}

#[test]
fn pol_014_invalid_params_are_load_errors() {
    let load = |params: &str| {
        parse_set(
            Scope::Project,
            "p.yaml",
            &format!(
                "spec_version: 1\nname: p\nrules:\n  - id: r\n    type: require_approval\n    params: {params}\n"
            ),
        )
    };
    assert!(load("{}").is_err(), "ohne Kriterium");
    assert!(load("{ kinds: [teleport] }").is_err(), "unbekannte Klasse");
    assert!(
        load("{ kinds: [shell], unknown: 1 }").is_err(),
        "unbekannter Parameter"
    );
    assert!(
        load("{ match: 'tool.kind == 1' }").is_err(),
        "Typfehler im CEL"
    );
    assert!(load("{ kinds: [shell], timeout: soon }").is_err(), "Dauer");
    assert!(
        load("{ kinds: [shell], on_timeout: maybe }").is_err(),
        "on_timeout"
    );
    let err = load("{ kinds: [shell], remember: forever }").unwrap_err();
    assert!(err.message.contains("remember"), "{err}");
}

#[test]
fn pol_014_static_kinds_feed_the_enforcement_check() {
    // Die entzuckerte Regel nennt ihre Klassen (POL-024): ein Harness ohne Gate für `shell`
    // erkennt die Lücke.
    let s = require("{ kinds: [shell] }");
    let rule = &s.rules[0];
    let beton_policy::load::RuleKind::Cel { when: Some(p), .. } = &rule.kind else {
        panic!("entzuckert erwartet: {:?}", rule.kind);
    };
    assert_eq!(
        p.tool_kinds().unwrap().into_iter().collect::<Vec<_>>(),
        ["shell"]
    );
}

// ---------------------------------------------------------------------------
// POL-009
// ---------------------------------------------------------------------------

#[test]
fn pol_009_effect_values_and_notify_messages_are_evaluated() {
    let s = set(
        Scope::Project,
        "p",
        "rules:\n  - id: count-pushes\n    on: tool_call\n    when: 'shell.has_command(tool.argv, \"git\", \"push\")'\n    action: notify\n    notify: { channels: [inbox], message: 'Push Nr. {{ state.session.pushes + 1 }}' }\n    effects:\n      - { op: increment, scope: session, key: pushes }\n      - { op: set, scope: session, key: last, value: 'tool.command' }\n",
    );
    let act = Activation::new(
        Phase::ToolCall,
        json!({
            "tool": shell_tool("git push origin main"),
            "state": { "session": { "pushes": 2 } }
        }),
    )
    .unwrap();
    let d = eval(&[s], &act);
    assert_eq!(d.notifications[0].message.as_deref(), Some("Push Nr. 3"));
    let ops: Vec<_> = d.effects[0]
        .ops
        .iter()
        .map(|o| (o.scope, o.key.as_str(), o.op, o.value.clone()))
        .collect();
    assert_eq!(
        ops,
        [
            (StateScope::Session, "pushes", EffectOp::Increment, None),
            (
                StateScope::Session,
                "last",
                EffectOp::Set,
                Some(json!("git push origin main"))
            ),
        ]
    );
}

#[test]
fn pol_009_ac1_fork_takes_state_up_to_seq() {
    let mut live = PolicyState::default();
    let mut log: Vec<(u64, StateChange)> = Vec::new();
    for seq in [10, 20, 30] {
        let change = live
            .apply(StateScope::Session, "pushes", EffectOp::Increment, None)
            .unwrap();
        log.push((seq, change));
    }
    assert_eq!(live.get(StateScope::Session, "pushes"), Some(&json!(3)));
    // Replay des ganzen Logs ergibt denselben Zustand.
    let replayed = PolicyState::replay(log.iter().map(|(_, c)| c));
    assert_eq!(replayed.get(StateScope::Session, "pushes"), Some(&json!(3)));
    // Fork zwischen zweitem und drittem Push.
    let fork = PolicyState::replay(log.iter().filter(|(s, _)| *s <= 25).map(|(_, c)| c));
    assert_eq!(fork.get(StateScope::Session, "pushes"), Some(&json!(2)));
}

#[test]
fn pol_009_operations_and_limits() {
    assert_eq!(
        next_value(None, EffectOp::Increment, None).unwrap(),
        Some(json!(1))
    );
    assert_eq!(
        next_value(Some(&json!(1.5)), EffectOp::Increment, Some(json!(2))).unwrap(),
        Some(json!(3.5))
    );
    assert!(next_value(Some(&json!("x")), EffectOp::Increment, None).is_err());
    assert_eq!(
        next_value(Some(&json!(1)), EffectOp::Set, Some(json!("a"))).unwrap(),
        Some(json!("a"))
    );
    assert!(
        next_value(None, EffectOp::Set, None).is_err(),
        "set ohne Wert"
    );
    assert_eq!(
        next_value(Some(&json!(1)), EffectOp::Delete, None).unwrap(),
        None
    );
    // append: Liste, höchstens 100 Elemente (die ältesten fallen heraus).
    let mut v: Option<Value> = None;
    for i in 0..(APPEND_MAX + 5) {
        v = next_value(v.as_ref(), EffectOp::Append, Some(json!(i))).unwrap();
    }
    let list = v.unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), APPEND_MAX);
    assert_eq!(list[0], json!(5));
    // Werte höchstens 4 KiB je Key.
    let big = "x".repeat(VALUE_MAX_BYTES + 1);
    assert!(next_value(None, EffectOp::Set, Some(json!(big))).is_err());
}

#[test]
fn pol_009_ac2_effects_after_resolution() {
    let default_on = [DefaultAction::Allow, DefaultAction::Deny];
    // Freigegeben: wie allow.
    assert!(effects_due(&default_on, FinalOutcome::Allow));
    // Abgelehnt: mit Default `effects_on` ebenfalls.
    assert!(effects_due(&default_on, FinalOutcome::Deny));
    // Abgelaufen: nicht.
    assert!(!effects_due(&default_on, FinalOutcome::Expired));
    // `effects_on: [allow]`: nur bei Freigabe.
    assert!(!effects_due(&[DefaultAction::Allow], FinalOutcome::Deny));
}

#[test]
fn pol_009_ac3_user_day_resets_at_midnight_in_the_users_timezone() {
    // UTC (Default bis zur Einstellung `user.timezone`, #195).
    assert_eq!(
        day_key(datetime!(2026-01-01 23:59:59 UTC), UtcOffset::UTC),
        "2026-01-01"
    );
    assert_eq!(
        day_key(datetime!(2026-01-02 00:00:00 UTC), UtcOffset::UTC),
        "2026-01-02"
    );
    // UTC+2: der Tag beginnt um 22:00Z.
    let plus2 = UtcOffset::from_hms(2, 0, 0).unwrap();
    assert_eq!(
        day_key(datetime!(2026-01-01 21:59:59 UTC), plus2),
        "2026-01-01"
    );
    assert_eq!(
        day_key(datetime!(2026-01-01 22:00:00 UTC), plus2),
        "2026-01-02"
    );
}

// ---------------------------------------------------------------------------
// Security-Review B1: Fingerprint bindet Tool, Server und Arbeitsverzeichnis
// ---------------------------------------------------------------------------

fn mcp_act(server: &str, name: &str, args: Value) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": name, "native_name": format!("mcp__{server}__{name}"),
                          "server": server, "kind": "mcp", "args": args } }),
    )
    .unwrap()
}

fn granted_after(sets: &[Arc<LoadedSet>], first: &Activation) -> SessionGrants {
    let reasons = ask_reasons(&eval(sets, first));
    let mut grants = SessionGrants::default();
    grants.add(&reasons[0].rule, reasons[0].fingerprint.as_deref().unwrap());
    grants
}

fn outcome_with(sets: &[Arc<LoadedSet>], grants: &SessionGrants, act: &Activation) -> Outcome {
    let opts = EvalOptions {
        grants: Some(grants),
        ..EvalOptions::default()
    };
    EffectiveSet::new(sets.to_vec())
        .evaluate(act, &opts)
        .outcome
}

#[test]
fn pol_010_ac2_grant_does_not_cover_another_tool_with_the_same_args() {
    let sets = [require("{ servers: [github, gitlab] }")];
    let args = json!({"owner": "ifahrentholz", "repo": "shop"});
    let grants = granted_after(&sets, &mcp_act("github", "get_repository", args.clone()));
    // Derselbe Aufruf: erlaubt.
    assert_eq!(
        outcome_with(
            &sets,
            &grants,
            &mcp_act("github", "get_repository", args.clone())
        ),
        Outcome::Allow
    );
    // Anderes Tool mit gleichen Argumenten: neue Card.
    assert!(matches!(
        outcome_with(
            &sets,
            &grants,
            &mcp_act("github", "delete_repository", args.clone())
        ),
        Outcome::Ask { .. }
    ));
    // Anderer Server, gleiches Tool und gleiche Argumente: neue Card.
    assert!(matches!(
        outcome_with(&sets, &grants, &mcp_act("gitlab", "get_repository", args)),
        Outcome::Ask { .. }
    ));
}

#[test]
fn pol_010_ac2_grant_for_empty_args_is_bound_to_the_tool() {
    let sets = [set(
        Scope::Project,
        "p",
        "rules:\n  - id: other\n    on: tool_call\n    action: ask\n",
    )];
    let act = |name: &str| {
        Activation::new(
            Phase::ToolCall,
            json!({ "tool": { "name": name, "server": "builtin", "kind": "other", "args": {} } }),
        )
        .unwrap()
    };
    let grants = granted_after(&sets, &act("Harmless"));
    assert_eq!(
        outcome_with(&sets, &grants, &act("Harmless")),
        Outcome::Allow
    );
    assert!(matches!(
        outcome_with(&sets, &grants, &act("Dangerous")),
        Outcome::Ask { .. }
    ));
}

#[test]
fn pol_010_ac2_shell_grant_is_bound_to_the_working_directory() {
    let sets = [set(
        Scope::Project,
        "p",
        &format!("rules:\n{}", ask_rule("confirm", "{ remember: session }")),
    )];
    let in_dir = |cwd: &str| {
        let mut tool = shell_tool("rm -rf build");
        tool["cwd"] = json!(cwd);
        Activation::new(Phase::ToolCall, json!({ "tool": tool })).unwrap()
    };
    let grants = granted_after(&sets, &in_dir("/work/repo"));
    assert_eq!(
        outcome_with(&sets, &grants, &in_dir("/work/repo")),
        Outcome::Allow
    );
    assert!(matches!(
        outcome_with(&sets, &grants, &in_dir("/work/other")),
        Outcome::Ask { .. }
    ));
}
