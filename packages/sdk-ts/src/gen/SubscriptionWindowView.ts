// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Jüngster gemeldeter Stand eines Abo-Fensters (USE-004 AC2).
 */
export type SubscriptionWindowView = { harness: string, 
/**
 * `anthropic`, `openai`, …
 */
vendor: string, 
/**
 * `5h`, `7d`, `7d_opus`, …
 */
window: string, 
/**
 * Genutzter Anteil in Prozent.
 */
used_pct: number, 
/**
 * RFC 3339 (UTC); die Anzeige rechnet in die Zeitzone des Users um.
 */
resets_at: string, 
/**
 * Zeitpunkt der Meldung.
 */
observed_at: string, session_id: string, 
/**
 * Reset-Zeitpunkt liegt in der Vergangenheit: der Wert ist überholt.
 */
expired: boolean, };
