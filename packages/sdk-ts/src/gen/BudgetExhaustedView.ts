// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein erreichtes Budget-Limit einer Auswertung.
 */
export type BudgetExhaustedView = { 
/**
 * Regel-ID (`max-cost` beim Session-Budget, API-006).
 */
rule_id: string, 
/**
 * Ebene des Sets.
 */
scope: string, set: string, 
/**
 * Kostenbereich: `session`, `run`, `subtree` oder `user`.
 */
budget_scope: string, 
/**
 * `deny` oder `ask`.
 */
action: string, };
