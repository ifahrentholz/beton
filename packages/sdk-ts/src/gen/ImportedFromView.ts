// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Herkunft einer importierten Session.
 */
export type ImportedFromView = { 
/**
 * Session-ID in der exportierenden Instanz.
 */
session_id: string, 
/**
 * Zeitpunkt des Exports (RFC 3339).
 */
exported_at: string, 
/**
 * SHA-256 der `session.jsonl`; Schlüssel der Idempotenz.
 */
export_sha256: string, };
