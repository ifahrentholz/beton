// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ChangeStatus } from "./ChangeStatus";

export type ChangedFile = { path: string, 
/**
 * Alter Pfad bei Umbenennung.
 */
old_path?: string, status: ChangeStatus, 
/**
 * Hinzugefügte Zeilen; fehlt bei Binärdateien.
 */
additions?: number, 
/**
 * Entfernte Zeilen; fehlt bei Binärdateien.
 */
deletions?: number, };
