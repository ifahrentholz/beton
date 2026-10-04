// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Eine Session in der Liste.
 */
export type SessionSummary = { id: `ses_${string}`, title: string, 
/**
 * Siehe `SessionStatus` im Event-Schema.
 */
status: string, kind: string, harness: string, archived: boolean, head_seq: number, 
/**
 * Kosten in Mikro-Einheiten der Währung.
 */
cost_micro: number, created_at: string, last_activity_at: string, };
