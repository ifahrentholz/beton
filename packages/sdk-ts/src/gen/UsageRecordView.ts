// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein Turn einer Session (USE-007 AC2): Tokens, Kosten und Preisquelle.
 */
export type UsageRecordView = { seq: number, ts: string, harness: string, model: string, auth_source: string, billing: string, 
/**
 * Zweck außerhalb der Unterhaltung, z. B. `title` oder `compaction`.
 */
purpose?: string, input_tokens: number, output_tokens: number, cache_read_tokens: number, cache_write_tokens: number, 
/**
 * `null` bei Subscription und ohne Preis.
 */
cost_usd: number | null, 
/**
 * `override`, `harness`, `catalog`, `none` oder `subscription`.
 */
cost_source: string, pricing_version?: string, 
/**
 * API-Äquivalent (informativ), nur bei Subscription.
 */
equivalent_usd: number | null, };
