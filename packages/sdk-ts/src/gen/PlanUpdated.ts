// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { PlanEntry } from "./PlanEntry";

/**
 * Vollständiger Plan nach jeder Änderung (wie ACP und Codex ihn melden).
 */
export type PlanUpdated = { entries: Array<PlanEntry>, 
/**
 * Begründung der Änderung, wenn der Harness eine liefert (Codex `explanation`).
 */
explanation?: string, };
