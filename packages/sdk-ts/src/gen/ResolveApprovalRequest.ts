// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Decision } from "./Decision";

export type ResolveApprovalRequest = { decision: Decision, 
/**
 * Geänderte Argumente bei `allow` (HAR-005 AC2).
 */
updated_args?: unknown, 
/**
 * Begründung bei `deny`; das Modell sieht sie.
 */
reason?: string, };
