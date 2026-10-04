// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ImportStatus } from "./ImportStatus";
import type { ImportedFromView } from "./ImportedFromView";

/**
 * Ergebnis von `POST /v1/sessions/import`.
 */
export type SessionImportResult = { 
/**
 * `imported` oder `skipped` (dieselbe Datei wurde schon importiert).
 */
status: ImportStatus, 
/**
 * Bei `skipped`: `already_imported`.
 */
reason?: string, 
/**
 * Neue bzw. (bei `already_imported`) bestehende Session.
 */
session_id: string, title: string, 
/**
 * Anzahl der Events.
 */
events: number, 
/**
 * Anzahl der Blobs aus dem Archiv.
 */
blobs: number, imported_from: ImportedFromView, };
