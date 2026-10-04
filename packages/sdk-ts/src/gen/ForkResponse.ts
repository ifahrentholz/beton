// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ForkWorkspace } from "./ForkWorkspace";
import type { SessionSummary } from "./SessionSummary";

/**
 * Ergebnis eines Forks.
 */
export type ForkResponse = { 
/**
 * Die neue Session.
 */
session: SessionSummary, 
/**
 * Tatsächlicher Fork-Punkt in der Quelle (letztes vollständiges Turn-Ende).
 */
effective_seq: number, 
/**
 * Gewählter Workspace-Modus.
 */
workspace: ForkWorkspace, };
