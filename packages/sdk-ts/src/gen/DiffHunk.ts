// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { DiffLine } from "./DiffLine";

export type DiffHunk = { header: string, old_start: number, old_lines: number, new_start: number, new_lines: number, lines: Array<DiffLine>, };
