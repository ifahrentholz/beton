//! CEL-Engine (POL-002).
//!
//! `cel` (cel-rust) liefert den Parser: AST mit Positionen und expandierten Makros (`has`,
//! `all`, `exists`, `exists_one`, `map`, `filter`). Darauf setzt beton eine eigene
//! Typprüfung gegen die Kontext-Deklaration ([`crate::types`]) und eine eigene Auswertung
//! mit Kostenbudget, weil cel-rust beides nicht anbietet:
//!
//! - **Laden:** Ausdruck ≤ 4 KiB, Syntax und Typen werden geprüft, Regex- und Glob-Literale
//!   vorkompiliert. Fehler tragen Zeile und Spalte im Ausdruck (AC1).
//! - **Auswerten:** CEL-Semantik inklusive Error-Absorption von `&&`/`||` (AC2). Jede
//!   Operation kostet; über `max_cost` (≈ 10 000) bricht der Ausdruck mit
//!   [`EvalError::CostExceeded`] ab, über das Zeitbudget pro `evaluate` (10 ms) mit
//!   [`EvalError::Deadline`]. Beides ist nicht absorbierbar und gilt als `error` (fail closed).

mod analyze;
mod check;
mod eval;
pub mod ext;
mod trace;

use std::collections::BTreeSet;
use std::time::Instant;

use crate::Phase;
use crate::context::Activation;
use crate::types::Ty;
use crate::value::Value;

pub(crate) use check::Node;
pub use trace::TracePart;

/// Ladefehler eines Ausdrucks mit 1-basierter Position im Ausdruck.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{line}:{column}: {message}")]
pub struct CompileError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

/// Fehler einer Auswertung.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    /// Laufzeitfehler im Ausdruck (fehlender Schlüssel, Division durch null, …); wird von
    /// `&&`/`||` absorbiert.
    #[error("{0}")]
    Error(String),
    /// Kostenbudget des Ausdrucks überschritten.
    #[error("Kostenlimit des Ausdrucks überschritten")]
    CostExceeded,
    /// Zeitbudget der gesamten Auswertung überschritten.
    #[error("Zeitbudget der Policy-Auswertung überschritten")]
    Deadline,
}

impl EvalError {
    /// Fehler, die einen Ausdruck sofort abbrechen (nicht absorbierbar).
    pub fn is_fatal(&self) -> bool {
        !matches!(self, EvalError::Error(_))
    }
}

/// Monotone Uhr in Mikrosekunden; austauschbar für Tests (Fehlerinjektion).
pub trait Clock: Send + Sync {
    fn now_us(&self) -> u64;
}

/// Systemuhr (monoton).
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

/// Grenzen aus POL-002.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximale Länge eines Ausdrucks in Bytes (4 KiB).
    pub max_expr_bytes: usize,
    /// Kostenbudget je Ausdruck und Auswertung (≈ 10 000 Operationen).
    pub max_cost: u64,
    /// Zeitbudget je `evaluate` in Mikrosekunden (10 ms).
    pub eval_budget_us: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_expr_bytes: 4096,
            max_cost: 10_000,
            eval_budget_us: 10_000,
        }
    }
}

/// Zählt Kosten je Ausdruck und überwacht das Zeitbudget einer gesamten Auswertung.
pub struct Meter<'a> {
    max_cost: u64,
    used: u64,
    deadline_us: u64,
    clock: Option<&'a dyn Clock>,
    ticks: u32,
}

/// Wie oft (in Operationen) die Uhr gelesen wird.
const CLOCK_EVERY: u32 = 32;

impl<'a> Meter<'a> {
    /// Startet das Zeitbudget jetzt.
    pub fn new(limits: &Limits, clock: &'a dyn Clock) -> Self {
        Self {
            max_cost: limits.max_cost,
            used: 0,
            deadline_us: clock.now_us().saturating_add(limits.eval_budget_us),
            clock: Some(clock),
            ticks: 0,
        }
    }

    /// Ohne Kosten- und Zeitgrenzen (nur für Tests und Werkzeuge).
    pub fn unlimited() -> Self {
        Self {
            max_cost: u64::MAX,
            used: 0,
            deadline_us: u64::MAX,
            clock: None,
            ticks: 0,
        }
    }

    /// Beginnt einen neuen Ausdruck: Kosten auf null, Zeitbudget weiter wie bisher.
    pub fn begin_expr(&mut self) -> Result<(), EvalError> {
        self.used = 0;
        self.check_deadline()
    }

    /// Prüft das Zeitbudget.
    pub fn check_deadline(&self) -> Result<(), EvalError> {
        match self.clock {
            Some(c) if c.now_us() > self.deadline_us => Err(EvalError::Deadline),
            _ => Ok(()),
        }
    }

    /// Bisher verbrauchte Kosten des laufenden Ausdrucks.
    pub fn used(&self) -> u64 {
        self.used
    }

    pub(crate) fn charge(&mut self, cost: u64) -> Result<(), EvalError> {
        self.used = self.used.saturating_add(cost);
        if self.used > self.max_cost {
            return Err(EvalError::CostExceeded);
        }
        self.ticks += 1;
        if self.ticks >= CLOCK_EVERY {
            self.ticks = 0;
            self.check_deadline()?;
        }
        Ok(())
    }
}

/// Ein geladener, typgeprüfter Ausdruck.
#[derive(Debug, Clone)]
pub struct Program {
    source: String,
    node: Node,
    ty: Ty,
}

impl Program {
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Statischer Ergebnistyp.
    pub fn ty(&self) -> &Ty {
        &self.ty
    }

    /// Wertet den Ausdruck aus. Der Aufrufer ruft vorher [`Meter::begin_expr`] auf, wenn
    /// er mehrere Ausdrücke mit einem Meter auswertet.
    pub fn eval(&self, act: &Activation, meter: &mut Meter) -> Result<Value, EvalError> {
        eval::eval(&self.node, act, meter)
    }

    /// Tool-Klassen, auf die der Ausdruck beschränkt ist (`tool.kind == …`, `tool.kind in
    /// […]`, `tool.name == …`, verknüpft mit `&&`/`||`); `None`, wenn nicht bestimmbar
    /// (POL-024).
    pub fn tool_kinds(&self) -> Option<BTreeSet<String>> {
        analyze::tool_kinds(&self.node)
    }

    /// Benutzte Variablen (`session`) und Felder (`session.cost_usd`).
    pub fn fields(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        analyze::fields(&self.node, &mut out);
        out
    }

    /// Spur für Explain (POL-027): die Teilausdrücke einer `&&`- bzw. `||`-Verknüpfung auf
    /// oberster Ebene mit ihren Ergebnissen; sonst der ganze Ausdruck. Jeder Teil wird mit
    /// eigenem Kostenbudget ausgewertet, das Zeitbudget gilt weiter.
    pub fn trace(&self, act: &Activation, meter: &mut Meter) -> Vec<TracePart> {
        trace::trace(&self.source, &self.node, act, meter)
    }

    /// Wie [`Program::eval`], erwartet aber `bool`.
    pub fn eval_bool(&self, act: &Activation, meter: &mut Meter) -> Result<bool, EvalError> {
        match self.eval(act, meter)? {
            Value::Bool(b) => Ok(b),
            other => Err(EvalError::Error(format!(
                "Ausdruck liefert {} statt bool",
                other.type_name()
            ))),
        }
    }
}

/// Lädt einen Ausdruck für `phase` und prüft, dass sein Typ zu `expect` passt.
pub fn compile_expr(src: &str, phase: Phase, expect: Ty) -> Result<Program, CompileError> {
    compile_with(src, phase, expect, &Limits::default())
}

/// Lädt eine Bedingung (`when`): Ergebnistyp `bool`.
pub fn compile_bool(src: &str, phase: Phase) -> Result<Program, CompileError> {
    compile_expr(src, phase, Ty::Bool)
}

/// Wie [`compile_expr`] mit eigenen Grenzen.
pub fn compile_with(
    src: &str,
    phase: Phase,
    expect: Ty,
    limits: &Limits,
) -> Result<Program, CompileError> {
    if src.len() > limits.max_expr_bytes {
        return Err(CompileError {
            line: 1,
            column: 1,
            message: format!(
                "Ausdruck ist {} Bytes lang, erlaubt sind höchstens {} Bytes",
                src.len(),
                limits.max_expr_bytes
            ),
        });
    }
    let (node, ty) = check::check(src, phase)?;
    if !ty.assignable_to(&expect) {
        return Err(CompileError {
            line: 1,
            column: 1,
            message: format!("Ausdruck liefert {ty}, erwartet wird {expect}"),
        });
    }
    Ok(Program {
        source: src.to_owned(),
        node,
        ty,
    })
}
