//! QA-009: CEL-Parse + Auswertung mit zufälligem Kontext (POL-002, POL-004).
//!
//! Gefuzzt: `cel::compile_expr` (Parser von cel-rust, Typprüfung von beton), `Program::eval`,
//! `Program::trace`, `Program::tool_kinds`, `Program::fields` und `Activation::new` mit
//! einem Kontext, der aus den Deklarationen (`types::DECLS`) der gewählten Phase entsteht.
//!
//! Eingabe: `<ausdruck>` oder `<ausdruck>\0<kontext-bytes>`; die Kontext-Bytes wählen Phase
//! und Werte (`arbitrary`).
//!
//! Invarianten:
//! - QA-009 AC3: Eine Auswertung dauert nie länger als 10 ms (das Zeitbudget von POL-002);
//!   bei Überschreitung wird zweimal wiederholt und das Minimum gezählt, damit ein
//!   ausgelasteter Runner keinen Fehlalarm auslöst. Ein Hänger ist ein libFuzzer-Timeout.
//! - Das Kosten-Limit hält: Ein Ergebnis ohne Fehler hat höchstens `max_cost` verbraucht.
//! - Deterministisch: Zwei Auswertungen nur mit Kostenbudget (eingefrorene Uhr) liefern
//!   dasselbe Ergebnis mit denselben Kosten, und die Auswertung mit echter Uhr weicht davon
//!   höchstens durch `Deadline` ab.

#![no_main]

use std::time::{Duration, Instant};

use arbitrary::Unstructured;
use beton_policy::Phase;
use beton_policy::cel::{Clock, EvalError, Limits, Meter, Program, SystemClock, compile_expr};
use beton_policy::context::Activation;
use beton_policy::types::{DECLS, Ty};
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Value as Json};

/// Zeitbudget je Auswertung (POL-002, QA-009 AC3).
const BUDGET: Duration = Duration::from_millis(10);

/// Uhr, die steht: Nur das Kostenbudget begrenzt.
struct Frozen;

impl Clock for Frozen {
    fn now_us(&self) -> u64 {
        0
    }
}

const WORDS: [&str; 12] = [
    "",
    "Bash",
    "Read",
    "shell",
    "git",
    "push",
    "--force",
    "/etc/passwd",
    "*.example.com",
    "https://api.example.com/x",
    "rm -rf /",
    "main",
];

fn string(u: &mut Unstructured) -> String {
    if u.ratio(1, 2).unwrap_or(true) {
        WORDS[u.choose_index(WORDS.len()).unwrap_or(0)].to_owned()
    } else {
        let n = u.int_in_range(0..=64).unwrap_or(0);
        (0..n)
            .map(|_| char::from(u.int_in_range(0x20..=0x7e).unwrap_or(b'a')))
            .collect()
    }
}

/// Zufälliger JSON-Wert passend zu `ty` (manchmal absichtlich falsch: der Kontext lehnt ab).
fn value(u: &mut Unstructured, ty: &Ty, depth: u32) -> Json {
    if u.ratio(1, 16).unwrap_or(false) {
        return Json::Null;
    }
    match ty {
        Ty::Bool => Json::Bool(u.arbitrary().unwrap_or(false)),
        Ty::Int => Json::from(u.arbitrary::<i64>().unwrap_or(0)),
        Ty::Uint => Json::from(u.arbitrary::<u64>().unwrap_or(0)),
        Ty::Double => serde_json::Number::from_f64(u.arbitrary::<f64>().unwrap_or(0.0))
            .map_or(Json::Null, Json::Number),
        Ty::String | Ty::Bytes => Json::String(string(u)),
        Ty::Null => Json::Null,
        Ty::Timestamp => Json::String(
            [
                "2026-10-16T08:00:00Z",
                "1970-01-01T00:00:00+02:00",
                "kein Datum",
            ][u.choose_index(3).unwrap_or(0)]
            .to_owned(),
        ),
        Ty::Duration => {
            Json::String(["90s", "1h30m", "-5m", "x"][u.choose_index(4).unwrap_or(0)].to_owned())
        }
        Ty::List(elem) => {
            let n = if depth > 3 {
                0
            } else {
                u.int_in_range(0..=6).unwrap_or(0)
            };
            Json::Array((0..n).map(|_| value(u, elem, depth + 1)).collect())
        }
        Ty::Map(_, val) => {
            let n = if depth > 3 {
                0
            } else {
                u.int_in_range(0..=4).unwrap_or(0)
            };
            Json::Object(
                (0..n)
                    .map(|_| (string(u), value(u, val, depth + 1)))
                    .collect::<Map<_, _>>(),
            )
        }
        Ty::Object(name) => {
            let mut out = Map::new();
            if depth > 4 {
                return Json::Object(out);
            }
            for (field, fty) in DECLS.objects.get(name).into_iter().flatten() {
                if u.ratio(2, 3).unwrap_or(false) {
                    out.insert((*field).to_owned(), value(u, fty, depth + 1));
                }
            }
            Json::Object(out)
        }
        Ty::Dyn => match u.choose_index(5).unwrap_or(0) {
            0 => Json::String(string(u)),
            1 => Json::from(u.arbitrary::<i64>().unwrap_or(0)),
            2 if depth < 4 => Json::Array(
                (0..u.int_in_range(0..=4).unwrap_or(0))
                    .map(|_| value(u, &Ty::Dyn, depth + 1))
                    .collect(),
            ),
            3 if depth < 4 => Json::Object(
                (0..u.int_in_range(0..=4).unwrap_or(0))
                    .map(|_| (string(u), value(u, &Ty::Dyn, depth + 1)))
                    .collect::<Map<_, _>>(),
            ),
            _ => Json::Bool(u.arbitrary().unwrap_or(true)),
        },
    }
}

fn context(u: &mut Unstructured, phase: Phase) -> Json {
    let mut out = Map::new();
    for var in &DECLS.vars {
        if (var.phases.is_empty() || var.phases.contains(&phase)) && u.ratio(3, 4).unwrap_or(true) {
            out.insert(var.name.to_owned(), value(u, &var.ty, 0));
        }
    }
    Json::Object(out)
}

fn eval_timed(
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

fuzz_target!(|data: &[u8]| {
    let (expr, ctx) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let Ok(src) = std::str::from_utf8(expr) else {
        return;
    };
    let mut u = Unstructured::new(ctx);
    let phase = if ctx.is_empty() {
        Phase::ToolCall
    } else {
        Phase::ALL[u.choose_index(Phase::ALL.len()).unwrap_or(2)]
    };
    let json = context(&mut u, phase);
    let Ok(act) = Activation::new(phase, json) else {
        return;
    };
    let Ok(program) = compile_expr(src, phase, Ty::Dyn) else {
        return;
    };
    let _ = program.tool_kinds();
    let _ = program.fields();
    let limits = Limits::default();

    // QA-009 AC3: Zeitbudget mit echter Uhr.
    let (real, mut took) = eval_timed(&program, &act, &limits);
    for _ in 0..2 {
        if took <= BUDGET {
            break;
        }
        took = took.min(eval_timed(&program, &act, &limits).1);
    }
    assert!(
        took <= BUDGET,
        "CEL-Auswertung dauert {took:?} (> {BUDGET:?}): {src:?}"
    );

    // Determinismus und Kosten-Limit ohne Zeitbudget.
    let run = || {
        let mut meter = Meter::new(&limits, &Frozen);
        let r = program.eval(&act, &mut meter).map(|v| format!("{v:?}"));
        (r, meter.used())
    };
    let (a, cost_a) = run();
    let (b, cost_b) = run();
    assert_eq!(a, b, "CEL-Auswertung nicht deterministisch: {src:?}");
    assert_eq!(cost_a, cost_b, "Kosten nicht deterministisch: {src:?}");
    if a.is_ok() {
        assert!(
            cost_a <= limits.max_cost,
            "Kosten-Limit überschritten: {src:?}"
        );
    }
    if real != Err(EvalError::Deadline) {
        assert_eq!(real, a, "Ergebnis hängt von der Uhr ab: {src:?}");
    }

    let mut meter = Meter::new(&limits, &Frozen);
    let _ = program.trace(&act, &mut meter);
});
