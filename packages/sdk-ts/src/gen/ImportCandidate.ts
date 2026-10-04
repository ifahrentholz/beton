// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Eine lokale Session einer Vendor-CLI.
 */
export type ImportCandidate = { harness: string, 
/**
 * Session-ID der Vendor-CLI; mit ihr wird importiert.
 */
vendor_session_id: string, 
/**
 * Verlaufsdatei auf dem Host (nur zur Anzeige).
 */
path: string, 
/**
 * Arbeitsverzeichnis, in dem der Chat lief.
 */
cwd?: string, title?: string, model?: string, 
/**
 * Letzte Änderung der Datei (RFC 3339).
 */
updated_at?: string, size_bytes: number, 
/**
 * Bereits importiert: die beton-Session.
 */
imported_session_id?: string, };
