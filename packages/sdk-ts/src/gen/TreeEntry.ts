// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { EntryKind } from "./EntryKind";

export type TreeEntry = { name: string, 
/**
 * Pfad relativ zum Workspace, mit `/` getrennt.
 */
path: string, kind: EntryKind, 
/**
 * Größe in Bytes (nur Dateien).
 */
size?: number, };
