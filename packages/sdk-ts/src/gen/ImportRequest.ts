// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Import-Anfrage: entweder `refs` oder `last_n`.
 */
export type ImportRequest = { 
/**
 * `claude` oder `codex`.
 */
harness: string, 
/**
 * Vendor-Session-IDs aus den Kandidaten.
 */
refs?: Array<string>, 
/**
 * Die `n` zuletzt geänderten Sessions.
 */
last_n?: number, 
/**
 * Auch bereits importierte Sessions erneut importieren (neue Session).
 */
force?: boolean, };
