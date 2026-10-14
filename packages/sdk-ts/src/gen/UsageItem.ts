// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein Eintrag: gruppierte Dimensionen (nur die angefragten) und Summen.
 */
export type UsageItem = { 
/**
 * Kalendertag in der Zeitzone `tz` (`YYYY-MM-DD`).
 */
day?: string, session_id?: string, session_title?: string, 
/**
 * Fehlt bei Sessions ohne Projekt.
 */
project_id?: string, harness?: string, model?: string, auth_source?: string, 
/**
 * `subscription` (Login der Vendor-CLI) oder `metered` (API-Key, Gateway, lokal).
 */
billing: string, 
/**
 * Turns (Usage-Records).
 */
turns: number, sessions: number, input_tokens: number, output_tokens: number, cache_read_tokens: number, cache_write_tokens: number, 
/**
 * Kosten in USD; `null` bei Subscription (kein Geldbetrag).
 */
cost_usd: number | null, 
/**
 * API-Äquivalent in USD, nur informativ; `null` außer bei Subscription.
 */
equivalent_usd: number | null, 
/**
 * Turns ohne Preis („ohne Preis – Override anlegen“).
 */
unpriced_count: number, };
