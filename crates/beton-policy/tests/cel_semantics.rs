//! POL-002: CEL-Semantik der eigenen Auswertung (Operatoren, Konvertierungen, Zeitwerte,
//! Fehlerfälle) und POL-004-Kontext (Typkonvertierung, Phasen).

#![allow(clippy::unwrap_used)]

use beton_policy::Phase;
use beton_policy::cel::{EvalError, Meter, compile_bool, compile_expr};
use beton_policy::context::Activation;
use beton_policy::types::{DECLS, Ty};
use beton_policy::value::{Key, Value};
use serde_json::json;

fn ctx() -> Activation {
    Activation::new(
        Phase::SessionStart,
        json!({
            "now": "2026-01-01T12:00:00Z",
            "session": {
                "cost_usd": 2,
                "turns": 3,
                "started_at": "2026-01-01T11:00:00Z",
                "unpriced_models": ["a", "b"],
                "is_async": false,
                "branch": null
            },
            "start": { "model": "opus", "sandbox": { "enabled": true }, "network": { "proxy_enabled": false } },
            "state": { "session": { "pushes": 2, "names": ["x"], "big": 18446744073709551615u64, "f": 1.5, "m": { "k": 1 } } },
        }),
    )
    .unwrap()
}

fn value(src: &str) -> Result<Value, EvalError> {
    let p =
        compile_expr(src, Phase::SessionStart, Ty::Dyn).unwrap_or_else(|e| panic!("{src}: {e}"));
    p.eval(&ctx(), &mut Meter::unlimited())
}

#[test]
fn pol_002_true_expressions() {
    for src in [
        // Arithmetik
        "1 + 2 == 3 && 7 - 2 == 5 && 3 * 4 == 12 && 7 / 2 == 3 && 7 % 4 == 3",
        "2u + 3u == 5u && 5u - 2u == 3u && 2u * 3u == 6u && 7u / 2u == 3u && 7u % 4u == 3u",
        "1.5 + 1.5 == 3.0 && 2.5 - 0.5 == 2.0 && 2.0 * 1.5 == 3.0 && 3.0 / 2.0 == 1.5",
        "-(3) == -3 && -(1.5) == -1.5 && -duration(\"1m\") < duration(\"0s\")",
        "\"ab\" + \"cd\" == \"abcd\" && b\"a\" + b\"b\" == b\"ab\" && [1] + [2] == [1, 2]",
        // Vergleiche
        "\"a\" < \"b\" && b\"a\" < b\"b\" && false < true && 1u < 2 && 2 > 1u && 1.5 > 1u && 1u < 1.5",
        "1 <= 1 && 2 >= 1 && 1.0 <= 1 && 1 != 2 && \"a\" != \"b\" && null == null",
        "[1, 2] == [1, 2] && {\"a\": 1} == {\"a\": 1} && b\"x\" == b\"x\"",
        // Zeit
        "now - session.started_at == duration(\"1h\")",
        "now - duration(\"1h\") == session.started_at && duration(\"1h\") + now > now",
        "duration(\"30m\") + duration(\"30m\") == duration(\"1h\") && duration(\"2h\") - duration(\"1h\") == duration(\"1h\")",
        "timestamp(\"2026-01-02T00:00:00Z\") > now && int(now) == 1767268800",
        "duration(\"1.5h\") == duration(\"90m\") && duration(\"250ms\") < duration(\"1s\") && duration(\"0\") == duration(\"0s\")",
        "duration(\"1us\") < duration(\"1ms\") && duration(\"10ns\") < duration(\"1us\") && duration(\"-5m\") < duration(\"0s\")",
        // Konvertierungen
        "int(\"42\") == 42 && int(2.9) == 2 && int(3u) == 3 && uint(3) == 3u && uint(3u) == 3u",
        "double(\"1.5\") == 1.5 && double(2u) == 2.0 && double(1.5) == 1.5 && string(true) == \"true\"",
        "string(1.5) == \"1.5\" && string(2u) == \"2\" && string(\"x\") == \"x\"",
        // Listen, Maps, in, Index
        "session.unpriced_models[0] == \"a\" && session.unpriced_models[1u] == \"b\" && \"a\" in session.unpriced_models",
        "\"pushes\" in state.session && !(\"x\" in state.session) && 1 in [1.0]",
        "state.session.m[\"k\"] == 1 && {1: \"a\"}[1] == \"a\" && {true: 1}[true] == 1 && {2u: 1}[2u] == 1",
        "size(\"äb\") == 2 && size(b\"ab\") == 2 && size({\"a\": 1}) == 1 && session.unpriced_models.size() == 2",
        "state.session.names.exists(n, n == \"x\") && state.session.m.all(k, k == \"k\")",
        "[1, 2, 3].map(x, x * 2) == [2, 4, 6] && [1, 2, 3].filter(x, x > 1).size() == 2",
        "[1, 2, 3].map(x, x > 1, x) == [2, 3]",
        "state.session.big > 0u && state.session.f == 1.5 && state.session.pushes + 1 == 3",
        // Objekte, null und has()
        "session.branch == null && !has(session.branch) && !has(session.title) && has(session.turns)",
        "start.sandbox.enabled && !start.network.proxy_enabled && has(start.sandbox.enabled)",
        "!has(session.parent_id) && session.cost_usd == 2.0 && !session.is_async",
        "(true ? 1 : 2.0) == 1 && (false ? \"a\" : \"b\") == \"b\"",
        "bytes_human(1073741824) == \"1.0 GiB\" && bytes_human(10u) == \"10 B\"",
        "glob(\"a\", start.model + \"\") == false || true",
        "matches(\"abc\", start.model) == false && glob(\"opus\", start.model)",
        "url.host(\"http://[::1]:8080/x\") == \"[::1]\" && url.path(\"https://a.example\") == \"/\"",
        "shell.has_command([[\"sudo\", \"git\", \"push\"]], \"git\", \"push\")",
        "under(\"/\", \"/\") && under(\"/a/./b\", \"/a\")",
        "domain_matches(\"x\", \"*\") && !domain_matches(\"\", \"*\")",
    ] {
        assert_eq!(value(src), Ok(Value::Bool(true)), "{src}");
    }
}

#[test]
fn pol_002_runtime_errors() {
    for src in [
        "1 / 0 == 0",
        "1 % 0 == 0",
        "1u / 0u == 0u",
        "1u % 0u == 0u",
        "-9223372036854775807 - 2 < 0",
        "9223372036854775807 * 2 > 0",
        "18446744073709551615u + 1u > 0u",
        "0u - 1u > 0u",
        "2u * 18446744073709551615u > 0u",
        "-(-9223372036854775807 - 1) > 0",
        "int(\"x\") == 1",
        "int(18446744073709551615u) == 1",
        "int(1e300) == 1",
        "uint(-1) == 1u",
        "double(\"x\") == 1.0",
        "string(null) == \"\"",
        "timestamp(start.model) == now",
        "duration(start.model) == duration(\"1s\")",
        "[1][-1] == 1",
        "{\"a\": 1}[\"b\"] == 1",
        "{\"a\": 1, \"a\": 2}.a == 1",
        "matches(\"x\", start.model + \"(\")",
        "bytes_human(-1) == \"\"",
        "under(start.model, \"/\")",
        "url.path(\"nope\") == \"\"",
        "url.host(\"mailto:x@example.com\") == \"\"",
        "session.branch.startsWith(\"x\")",
        "state.session.names.missing",
        "state.session.pushes.x == 1",
    ] {
        let program = compile_expr(src, Phase::SessionStart, Ty::Dyn)
            .unwrap_or_else(|e| panic!("{src}: {e}"));
        let result = program.eval(&ctx(), &mut Meter::unlimited());
        assert!(
            matches!(result, Err(EvalError::Error(_))),
            "{src}: {result:?}"
        );
    }
}

#[test]
fn pol_002_dyn_values_are_checked_at_runtime() {
    // `state.session.*` ist dyn: Typfehler fallen erst zur Laufzeit auf.
    for src in [
        "state.session.pushes + \"x\" == 1",
        "state.session.pushes < \"x\"",
        "!state.session.pushes",
        "state.session.pushes ? 1 : 2",
        "-state.session.names == 1",
        "state.session.pushes[0] == 1",
        "state.session.pushes in state.session.pushes",
        "size(state.session.pushes) == 1",
        "state.session.pushes.exists(x, x)",
        "state.session.pushes.startsWith(\"x\")",
        "double(state.session.names) == 1.0",
        "int(state.session.names) == 1",
        "bytes_human(state.session.names) == \"\"",
        "state.session.names - state.session.names == 1",
        "state.session.names * 2 == 1",
        "shell.has_command(state.session.names, \"x\")",
        "shell.has_flag([state.session.names, [state.session.pushes]], \"x\")",
        "state.session.pushes && true",
        "{state.session.names: 1}.size() == 1",
        "state.session.m.all(k, state.session.pushes)",
    ] {
        let program = compile_expr(src, Phase::SessionStart, Ty::Dyn)
            .unwrap_or_else(|e| panic!("{src}: {e}"));
        let result = program.eval(&ctx(), &mut Meter::unlimited());
        assert!(result.is_err(), "{src}: {result:?}");
    }
}

#[test]
fn pol_002_type_errors_at_load() {
    for src in [
        "\"a\" - \"b\"",
        "1 * 1.0",
        "1 % 1.0",
        "-\"a\"",
        "\"a\"[0]",
        "1 in \"a\"",
        "size(1)",
        "matches(1, \"a\")",
        "\"a\".startsWith(1)",
        "duration(1)",
        "int(true)",
        "glob(1, \"a\")",
        "bytes_human(\"a\")",
        "shell.has_command(\"a\", \"b\")",
        "shell.has_flag([[\"a\"]], 1)",
        "url.host(1)",
        "unknown_fn(1)",
        "\"a\".nope()",
        "1 == \"a\"",
        "\"a\" < 1",
        "true && 1",
        "!1",
        "1 ? 2 : 3",
        "session.unpriced_models.exists(x, 1)",
        "1.exists(x, true)",
        "session.cost_usd.x",
        "{[1]: 2}",
        "duration(\"bald\")",
        "timestamp(\"gestern\")",
        "size(1, 2)",
        "glob(\"a\", \"[\") || glob(\"a\")",
        "Struct{a: 1}",
        "[?1]",
        "session.unpriced_models.map(k, v, x, y)",
    ] {
        assert!(
            compile_expr(src, Phase::SessionStart, Ty::Dyn).is_err(),
            "{src} sollte nicht laden"
        );
    }
}

#[test]
fn pol_002_values_roundtrip_to_json() {
    let v = value("{\"a\": [1, 2u, 1.5, true, null, \"s\", b\"x\"], \"t\": now, \"d\": duration(\"1m\"), 1: 2}").unwrap();
    assert_eq!(
        v.to_json(),
        json!({ "a": [1, 2, 1.5, true, null, "s", [120]], "t": "2026-01-01T12:00:00Z", "d": 60.0, "1": 2 })
    );
    assert_eq!(v.type_name(), "map");
    assert_eq!(Value::from_json(&json!([1, 2])).to_string(), "[1,2]");
    assert_eq!(Value::from(true).to_string(), "true");
    assert_eq!(Value::Null.to_string(), "null");
    assert_eq!(Value::from(7).type_name(), "int");
    assert_eq!(
        Value::from_json(&json!(18446744073709551615u64)),
        Value::Uint(u64::MAX)
    );
    assert_eq!(Key::Uint(1).to_value(), Value::Uint(1));
    assert_eq!(Key::from_value(&Value::Double(1.0)), None);
    for (v, name) in [
        (Value::Bytes(b"x".as_slice().into()), "bytes"),
        (value("now").unwrap(), "timestamp"),
        (value("duration(\"1s\")").unwrap(), "duration"),
        (Value::list(vec![]), "list"),
        (Value::Double(f64::NAN), "double"),
    ] {
        assert_eq!(v.type_name(), name);
    }
    assert_eq!(Value::Double(f64::NAN).to_json(), json!(null));
    assert_ne!(Value::from("a"), Value::from(1));
}

#[test]
fn pol_004_context_conversion_follows_declaration() {
    let ok = Activation::new(
        Phase::ToolResult,
        json!({ "tool": { "argv": [["a"]] }, "result": { "ok": true, "exit_code": 0, "bytes": 3 } }),
    );
    assert!(ok.is_ok());
    for (phase, ctx) in [
        (Phase::ToolCall, json!([])),
        (Phase::ToolCall, json!({ "result": {} })),
        (Phase::ToolCall, json!({ "tool": { "nope": 1 } })),
        (Phase::ToolCall, json!({ "tool": { "name": 1 } })),
        (Phase::ToolCall, json!({ "tool": { "argv": [[1]] } })),
        (Phase::ToolCall, json!({ "tool": { "args": [] } })),
        (Phase::ToolCall, json!({ "tool": 5 })),
        (Phase::ToolCall, json!({ "session": { "turns": 1.5 } })),
        (Phase::ToolCall, json!({ "session": { "is_async": "ja" } })),
        (
            Phase::ToolCall,
            json!({ "session": { "started_at": "gestern" } }),
        ),
        (Phase::ToolCall, json!({ "session": { "labels": "a" } })),
        (Phase::ToolCall, json!({ "state": { "session": [] } })),
    ] {
        assert!(Activation::new(phase, ctx.clone()).is_err(), "{ctx}");
    }
    let mut act =
        Activation::new(Phase::ModelRequest, json!({ "request": { "model": "a" } })).unwrap();
    act.set_field("request", "model", &json!("b")).unwrap();
    assert_eq!(act.field("request", "model"), Some(Value::from("b")));
    assert!(act.set_field("request", "nope", &json!("b")).is_err());
    assert!(act.set_field("tool", "args", &json!({})).is_err());
    assert!(act.set_field("now", "x", &json!(1)).is_err());
    assert_eq!(act.var("session"), Value::Null);
    assert_eq!(act.phase(), Phase::ModelRequest);
    assert!(format!("{act:?}").contains("request"));
}

#[test]
fn pol_004_declarations_cover_every_phase_variable() {
    for phase in Phase::ALL {
        assert!(DECLS.var("session", phase).is_ok());
        assert!(!phase.as_str().is_empty());
    }
    assert_eq!(
        DECLS.var("result", Phase::ToolCall).unwrap_err(),
        Some(&[Phase::ToolResult][..])
    );
    assert_eq!(
        DECLS.var("gibt_es_nicht", Phase::ToolCall).unwrap_err(),
        None
    );
    assert_eq!(DECLS.field("tool", "kind"), Some(&Ty::String));
    let shown: Vec<String> = [
        Ty::Bytes,
        Ty::Null,
        Ty::Uint,
        Ty::Duration,
        Ty::Timestamp,
        Ty::map(Ty::String, Ty::Dyn),
        Ty::list(Ty::Int),
        Ty::Object("tool"),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        shown,
        [
            "bytes",
            "null",
            "uint",
            "duration",
            "timestamp",
            "map(string, dyn)",
            "list(int)",
            "tool"
        ]
    );
    assert_eq!(
        Ty::list(Ty::Int).join(&Ty::list(Ty::Null)),
        Ty::list(Ty::Int)
    );
    assert_eq!(
        Ty::map(Ty::String, Ty::Int).join(&Ty::map(Ty::String, Ty::Int)),
        Ty::map(Ty::String, Ty::Int)
    );
    assert_eq!(Ty::Int.join(&Ty::String), Ty::Dyn);
    assert!(Ty::map(Ty::String, Ty::Int).assignable_to(&Ty::map(Ty::String, Ty::Dyn)));
}

#[test]
fn pol_002_program_accessors_and_bool_check() {
    let p = compile_bool("true", Phase::ToolCall).unwrap();
    assert_eq!(p.source(), "true");
    assert_eq!(p.ty(), &Ty::Bool);
    let p = compile_expr("state.session", Phase::ToolCall, Ty::Dyn).unwrap();
    let act = Activation::new(Phase::ToolCall, json!({})).unwrap();
    assert!(p.eval_bool(&act, &mut Meter::unlimited()).is_err());
    assert!(!EvalError::Error("x".into()).is_fatal());
    assert!(EvalError::Deadline.is_fatal());
    let mut meter = Meter::unlimited();
    assert!(meter.begin_expr().is_ok());
    assert_eq!(meter.used(), 0);
}
