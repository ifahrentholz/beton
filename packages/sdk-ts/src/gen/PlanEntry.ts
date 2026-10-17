// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { PlanPriority } from "./PlanPriority";
import type { PlanStatus } from "./PlanStatus";

/**
 * Ein Schritt im Plan des Agents (ACP `plan`, Codex `turn/plan/updated`).
 */
export type PlanEntry = { content: string, 
/**
 * Nur, wenn der Harness sie meldet (ACP); Codex kennt keine Priorität.
 */
priority?: PlanPriority, status: PlanStatus, };
