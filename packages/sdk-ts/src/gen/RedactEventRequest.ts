// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Manuelle Nachredaktion eines Events (DATA-012).
 */
export type RedactEventRequest = { 
/**
 * JSON-Pointer in die Nutzlast (ohne `type`), z. B. `/content/0/text`. Ohne Angabe wird
 * die ganze Nutzlast geschwärzt; IDs, Aufzählungen und Zeitstempel bleiben lesbar.
 */
paths?: Array<string>, 
/**
 * Grund, z. B. „Secret-Leak“ (Pflicht, höchstens 500 Zeichen); steht im
 * `event.redacted` und im Audit-Log.
 */
reason: string, };
