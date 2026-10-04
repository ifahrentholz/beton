// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ImportStatus } from "./ImportStatus";
import type { ImportWarningView } from "./ImportWarningView";

/**
 * Ergebnis je Vendor-Session.
 */
export type ImportResult = { vendor_session_id: string, status: ImportStatus, 
/**
 * Bei `skipped` bzw. `failed`, z. B. `already_imported`, `not_found`, `unknown_format`.
 */
reason?: string, detail?: string, 
/**
 * Neue bzw. (bei `already_imported`) bestehende Session.
 */
session_id?: string, 
/**
 * Übernommene Inhalts-Events.
 */
events?: number, warnings: Array<ImportWarningView>, };
