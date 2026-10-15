// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { BudgetExhaustedView } from "./BudgetExhaustedView";

/**
 * Ergebnis einer seiteneffektfreien Auswertung.
 */
export type PolicyEvaluation = { 
/**
 * `allow`, `deny` oder `ask`.
 */
outcome: string, 
/**
 * Durchsetzung, die der Harness in dieser Phase erreicht.
 */
enforcement: string, 
/**
 * Die Entscheidung in der Form von `policy.decision` (ohne `decision_id`).
 */
decision: unknown, 
/**
 * Strukturierte Erklärung samt benutzter Kontextwerte.
 */
explanation: unknown, 
/**
 * Erklärung als Text.
 */
explain: string, 
/**
 * Budget-Limits, die diese Auswertung erreicht (POL-011, POL-012; wie `budget.exhausted`).
 */
budget_exhausted: Array<BudgetExhaustedView>, };
