//! QA-009 AC3: Die Auswertung zufälliger CEL-Ausdrücke bleibt im Zeit- und Kostenbudget von
//! POL-002 und ist deterministisch.
//!
//! Die Ausdrücke entstehen aus einer typisierten Grammatik (Strings, Zahlen, Listen,
//! Bedingungen, Makros `map`/`filter`/`exists`/`all`, Regex und Globs), damit die meisten die
//! Typprüfung bestehen und wirklich ausgewertet werden; dazu teure Muster (verschachtelte
//! Makros über Listen aus dem Kontext). Der Fuzz-Target `fuzz/fuzz_targets/cel_eval.rs`
//! prüft dieselbe Schranke mit beliebigen Eingaben.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::time::{Duration, Instant};

use beton_policy::Phase;
use beton_policy::cel::{Clock, EvalError, Limits, Meter, Program, SystemClock, compile_expr};
use beton_policy::context::Activation;
use beton_policy::types::Ty;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use serde_json::json;

/// Obergrenze der Laufzeit einer Auswertung. Im Release-Build gilt das Zeitbudget von POL-002
/// (10 ms) unmittelbar. Im Debug-Build (Standard für `cargo test`) ist der unoptimierte Code
/// um ein Vielfaches langsamer; dort prüft der Test, dass das Budget greift: Die Auswertung
/// bricht nach 10 ms mit `Deadline` ab, spätestens nach den 32 Operationen bis zum nächsten
/// Blick auf die Uhr. Dafür reichen großzügig 40 ms Überhang.
#[cfg(not(debug_assertions))]
const BOUND: Duration = Duration::from_millis(10);
#[cfg(debug_assertions)]
const BOUND: Duration = Duration::from_millis(50);

/// Uhr, die steht: Nur das Kostenbudget begrenzt.
struct Frozen;

impl Clock for Frozen {
    fn now_us(&self) -> u64 {
        0
    }
}

fn lit() -> impl Strategy<Value = String> {
    "[a-z/*.?_ -]{0,8}".prop_map(|s| format!("{s:?}"))
}

fn string_expr() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        lit(),
        Just("tool.name".to_owned()),
        Just("tool.command".to_owned()),
        Just("tool.native_name".to_owned()),
        Just("x".to_owned()),
    ];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a} + {b})")),
            (bool_leaf(), inner.clone(), inner.clone())
                .prop_map(|(c, a, b)| format!("({c} ? {a} : {b})")),
            (list_leaf(), 0u8..6).prop_map(|(l, i)| format!("{l}[{i}]")),
        ]
    })
    .boxed()
}

fn int_expr() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        (0i64..1000).prop_map(|n| n.to_string()),
        string_leaf().prop_map(|s| format!("size({s})")),
        list_leaf().prop_map(|l| format!("size({l})")),
    ];
    leaf.prop_recursive(3, 16, 2, |inner| {
        (
            inner.clone(),
            prop::sample::select(vec!["+", "-", "*", "/", "%"]),
            inner,
        )
            .prop_map(|(a, op, b)| format!("({a} {op} {b})"))
    })
    .boxed()
}

fn string_leaf() -> BoxedStrategy<String> {
    prop_oneof![lit(), Just("tool.command".to_owned()), Just("x".to_owned())].boxed()
}

fn list_leaf() -> BoxedStrategy<String> {
    prop_oneof![
        Just("tool.paths".to_owned()),
        Just("tool.urls".to_owned()),
        (lit(), lit()).prop_map(|(a, b)| format!("[{a}, {b}]")),
    ]
    .boxed()
}

fn list_expr() -> BoxedStrategy<String> {
    list_leaf()
        .prop_recursive(3, 16, 2, |inner| {
            prop_oneof![
                (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a} + {b})")),
                (inner.clone(), string_expr()).prop_map(|(l, s)| format!("{l}.map(x, {s})")),
                (inner, string_leaf()).prop_map(|(l, s)| format!("{l}.filter(x, x.contains({s}))")),
            ]
        })
        .boxed()
}

fn bool_leaf() -> BoxedStrategy<String> {
    prop_oneof![
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("(session.cost_usd > 1.5)".to_owned()),
    ]
    .boxed()
}

const REGEXES: [&str; 5] = ["^a", "a+b", "(a|b)*c", "^/.*\\\\.env$", "(x+x+)+y"];
const GLOBS: [&str; 3] = ["/etc/**", "*.env", "/home/*/.ssh/**"];

fn bool_expr() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        bool_leaf(),
        (string_expr(), string_expr()).prop_map(|(a, b)| format!("({a} == {b})")),
        (int_expr(), int_expr()).prop_map(|(a, b)| format!("({a} < {b})")),
        (string_expr(), string_expr()).prop_map(|(a, b)| format!("{a}.contains({b})")),
        (string_expr(), string_expr()).prop_map(|(a, b)| format!("{a}.startsWith({b})")),
        (string_expr(), prop::sample::select(REGEXES.to_vec()))
            .prop_map(|(s, r)| format!("{s}.matches({r:?})")),
        (string_expr(), prop::sample::select(GLOBS.to_vec()))
            .prop_map(|(s, g)| format!("glob({s}, {g:?})")),
        string_expr().prop_map(|s| format!("under({s}, \"/home/u\")")),
        (string_expr(), list_expr()).prop_map(|(s, l)| format!("({s} in {l})")),
        (list_expr(), string_leaf()).prop_map(|(l, s)| format!("{l}.exists(x, x.startsWith({s}))")),
        (list_expr(), int_expr()).prop_map(|(l, i)| format!("{l}.all(x, size(x) < {i})")),
        Just("tool.argv.exists(c, c.exists(a, a == \"--force\"))".to_owned()),
    ];
    leaf.prop_recursive(3, 16, 2, |inner| {
        prop_oneof![
            inner.clone().prop_map(|b| format!("!{b}")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a} && {b})")),
            (inner.clone(), inner).prop_map(|(a, b)| format!("({a} || {b})")),
        ]
    })
    .boxed()
}

/// Teure Muster: verschachtelte Makros über die Listen des Kontexts.
fn expensive() -> BoxedStrategy<String> {
    prop_oneof![
        Just("tool.paths.map(a, tool.paths.map(b, tool.paths.map(c, a + b + c))).size() > 0".to_owned()),
        Just("tool.paths.all(a, tool.paths.all(b, tool.paths.exists(c, (a + b + c).matches(\"(x+x+)+y\"))))".to_owned()),
        Just("tool.argv.map(c, c.map(a, tool.command + a + tool.command)).exists(c, c.exists(a, a.contains(\"zz\")))".to_owned()),
        Just("[tool.paths, tool.paths, tool.paths, tool.paths].all(l, l.all(p, l.all(q, glob(p + q, \"/etc/**\"))))".to_owned()),
    ]
    .boxed()
}

fn expr() -> BoxedStrategy<String> {
    prop_oneof![
        6 => bool_expr(),
        2 => string_expr(),
        2 => int_expr(),
        1 => list_expr(),
        2 => expensive(),
    ]
    .boxed()
}

fn context() -> impl Strategy<Value = serde_json::Value> {
    let s = "[a-z/._ -]{0,48}";
    (
        s,
        s,
        prop::collection::vec(s, 0..40),
        prop::collection::vec("https://[a-z]{1,8}\\.example/[a-z]{0,8}", 0..8),
        prop::collection::vec(prop::collection::vec(s, 0..8), 0..8),
        0.0f64..10.0,
    )
        .prop_map(|(name, command, paths, urls, argv, cost)| {
            json!({
                "tool": {
                    "name": name, "native_name": name, "command": command, "kind": "shell",
                    "paths": paths, "urls": urls, "argv": argv, "args": { "command": command },
                },
                "session": { "cost_usd": cost },
            })
        })
}

fn timed(
    program: &Program,
    act: &Activation,
    limits: &Limits,
) -> (Result<String, EvalError>, Duration) {
    let clock = SystemClock::default();
    let started = Instant::now();
    let mut meter = Meter::new(limits, &clock);
    let result = program.eval(act, &mut meter);
    let took = started.elapsed();
    if result.is_ok() {
        assert!(
            meter.used() <= limits.max_cost,
            "Kosten-Limit überschritten"
        );
    }
    (result.map(|v| format!("{v:?}")), took)
}

#[test]
fn qa_009_ac3_random_cel_stays_within_time_and_cost_budget() {
    let limits = Limits::default();
    let mut runner = TestRunner::new(Config {
        cases: 256,
        failure_persistence: None,
        ..Config::default()
    });
    let compiled = Cell::new(0u32);
    let cut_by_cost = Cell::new(0u32);
    runner
        .run(&(expr(), context()), |(src, ctx)| {
            let act = Activation::new(Phase::ToolCall, ctx).unwrap();
            let Ok(program) = compile_expr(&src, Phase::ToolCall, Ty::Dyn) else {
                return Ok(());
            };
            compiled.set(compiled.get() + 1);
            // Zeitbudget mit echter Uhr; bei Überschreitung zweimal wiederholen und das
            // Minimum werten (ein ausgelasteter Rechner ist kein Befund).
            let (real, mut took) = timed(&program, &act, &limits);
            for _ in 0..2 {
                if took <= BOUND {
                    break;
                }
                took = took.min(timed(&program, &act, &limits).1);
            }
            prop_assert!(took <= BOUND, "{src}: {took:?} > {BOUND:?}");

            // Kosten-Limit und Determinismus ohne Zeitbudget.
            let run = || {
                let mut meter = Meter::new(&limits, &Frozen);
                let r = program.eval(&act, &mut meter).map(|v| format!("{v:?}"));
                (r, meter.used())
            };
            let (a, cost_a) = run();
            let (b, cost_b) = run();
            prop_assert_eq!(&a, &b, "{} nicht deterministisch", src);
            prop_assert_eq!(cost_a, cost_b);
            if a == Err(EvalError::CostExceeded) {
                cut_by_cost.set(cut_by_cost.get() + 1);
            }
            if real != Err(EvalError::Deadline) {
                prop_assert_eq!(&real, &a, "{} hängt von der Uhr ab", src);
            }
            Ok(())
        })
        .unwrap();
    // Die Grammatik soll überwiegend gültige Ausdrücke liefern, und die teuren Muster sollen
    // das Kostenbudget tatsächlich erreichen.
    let (compiled, cut_by_cost) = (compiled.get(), cut_by_cost.get());
    assert!(
        compiled >= 128,
        "nur {compiled} von 256 Ausdrücken übersetzt"
    );
    assert!(
        cut_by_cost > 0,
        "kein Ausdruck hat das Kostenbudget erreicht"
    );
}
