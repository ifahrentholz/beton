// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein eigener Preis (USE-003) in Prüfreihenfolge: `pricing.overrides`, danach die Preise der
 * Provider-Konfiguration. Raten wirksam, d. h. fehlende Cache-Raten geschätzt.
 */
export type PriceOverrideView = { 
/**
 * Position in der Prüfreihenfolge, ab 1; der erste Treffer gewinnt.
 */
position: number, 
/**
 * Glob auf den Provider; fehlt = jeder.
 */
provider: string | null, 
/**
 * Glob auf das Modell; fehlt = jedes.
 */
model: string | null, 
/**
 * Kostenlos (z. B. lokales Ollama).
 */
free: boolean, input_per_mtok: number, output_per_mtok: number, cache_read_per_mtok: number, cache_write_per_mtok: number, 
/**
 * Die Cache-Raten sind geschätzt (0,1 × bzw. 1,25 × Eingabe, USE-003 AC2).
 */
cache_estimated: boolean, context_window: number | null, };
