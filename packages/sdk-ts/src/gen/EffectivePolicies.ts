// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Das effektive Set (POL-027 `show --effective`).
 */
export type EffectivePolicies = { policy_set_hash: string, 
/**
 * Sets mit Ebene, Herkunft, Version, Modus und Regeln (`id`, `phases`, `action`, `when`).
 */
sets: unknown[], 
/**
 * Dateien, die gerade nicht laden (Pfad).
 */
invalid: Array<string>, };
