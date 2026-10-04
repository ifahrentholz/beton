// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
export type SearchHit = { path: string, 
/**
 * Zeile (1-basiert), nur bei `mode=content`.
 */
line?: number, 
/**
 * Spalte (1-basiert, Zeichen), nur bei `mode=content`.
 */
column?: number, 
/**
 * Zeilentext (gekürzt), nur bei `mode=content`.
 */
text?: string, };
