// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ChangeScope } from "./ChangeScope";
import type { ChangeStatus } from "./ChangeStatus";
import type { DiffHunk } from "./DiffHunk";

export type FileDiff = { path: string, old_path?: string, status: ChangeStatus, binary: boolean, scope: ChangeScope, base_sha: string, 
/**
 * Fehlt bei `uncommitted` (Arbeitsverzeichnis).
 */
head_sha?: string, 
/**
 * Unified Diff.
 */
patch: string, 
/**
 * Zeilengenau adressierbar über `old_line`/`new_line` zusammen mit `base_sha`/`head_sha`.
 */
hunks: Array<DiffHunk>, };
