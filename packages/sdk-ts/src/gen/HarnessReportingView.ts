// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Was ein Harness im Zeitraum meldet (USE-004 AC4): `false` heißt „nicht gemeldet“.
 */
export type HarnessReportingView = { harness: string, 
/**
 * Sessions mit Aktivität im Zeitraum.
 */
sessions: number, 
/**
 * Tokens gemeldet (Usage-Records im Zeitraum).
 */
tokens: boolean, 
/**
 * Abo-Fenster gemeldet.
 */
rate_limits: boolean, };
