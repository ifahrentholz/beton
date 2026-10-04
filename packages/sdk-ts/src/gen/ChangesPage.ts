// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ChangeScope } from "./ChangeScope";
import type { ChangedFile } from "./ChangedFile";

export type ChangesPage = { scope: ChangeScope, 
/**
 * Vergleichsbasis: `HEAD` (uncommitted), Merge-Base (branch) bzw. Snapshot vor dem Turn.
 */
base_sha: string, 
/**
 * Vergleichsstand: `HEAD` (branch) bzw. Snapshot nach dem Turn; fehlt bei `uncommitted`
 * (Arbeitsverzeichnis).
 */
head_sha?: string, 
/**
 * Turn bei `scope=turn`.
 */
turn_id?: string, items: Array<ChangedFile>, next_cursor: string | null, };
