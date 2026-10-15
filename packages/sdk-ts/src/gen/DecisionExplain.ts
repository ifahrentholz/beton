// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Explain einer gespeicherten Entscheidung.
 */
export type DecisionExplain = { session_id: string, 
/**
 * Seq des `policy.decision`.
 */
seq: number, decision: unknown, explanation: unknown, explain: string, 
/**
 * Warum der Snapshot nicht nutzbar war (fehlt oder beschädigt).
 */
snapshot_error?: string, };
