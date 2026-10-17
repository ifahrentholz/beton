//! Gemeinsame Hilfen der Integrationstests von `beton-policy`.

use beton_policy::cel::Limits;
use beton_policy::engine::EvalOptions;

/// Zeitbudget je `evaluate` für Tests, die Entscheidungen prüfen und keine Laufzeit (10 s).
///
/// Mit dem echten Budget von 10 ms (POL-002) reißt eine unoptimierte Auswertung auf
/// überbuchten CI-Runnern die Grenze, die Engine lehnt mit der Regel `engine` ab (fail closed)
/// und der Test wird zufällig rot (#314). Großzügig, aber nie ohne Zeitbudget; derselbe Wert
/// wie in `beton-policy test` (`policytest::test_limits`).
const DECISION_TEST_BUDGET_US: u64 = 10_000_000;

/// Auswerteoptionen für Entscheidungs-Tests: Grenzen aus POL-002 mit großzügigem Zeitbudget.
///
/// Kostenbudget und Ausdrucksgröße bleiben unverändert, ebenso die Systemuhr. Die Grenzen im
/// Betrieb (`Limits::default`, `Limits::runtime`) ändert das nicht. Tests, die das Zeitbudget
/// selbst prüfen (`pol_002_time_budget_exceeded_is_an_error`, `pol_007_time_budget_exceeded_denies`,
/// `cel_bounds.rs`, `bench.rs`), nutzen diese Hilfe bewusst nicht.
pub fn opts<'a>() -> EvalOptions<'a> {
    EvalOptions {
        limits: Limits {
            eval_budget_us: DECISION_TEST_BUDGET_US,
            ..Limits::default()
        },
        ..EvalOptions::default()
    }
}
