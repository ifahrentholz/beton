// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { DiffLineKind } from "./DiffLineKind";

export type DiffLine = { kind: DiffLineKind, 
/**
 * Zeilennummer auf der alten Seite (fehlt bei hinzugefügten Zeilen).
 */
old_line?: number, 
/**
 * Zeilennummer auf der neuen Seite (fehlt bei entfernten Zeilen).
 */
new_line?: number, text: string, 
/**
 * Zeile endet ohne Zeilenumbruch.
 */
no_newline: boolean, };
