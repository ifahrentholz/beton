// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
export type TurnInterrupted = { turn_id: `trn_${string}`, by: `usr_${string}` | `sa_${string}`, 
/**
 * Warum, wenn nicht auf Wunsch eines Menschen: `timed_out` (`executor.timeout`, AGT-004).
 */
reason?: string, };
