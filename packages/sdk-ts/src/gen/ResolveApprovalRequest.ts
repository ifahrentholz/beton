// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Decision } from "./Decision";
import type { Remember } from "./Remember";

export type ResolveApprovalRequest = { decision: Decision, 
/**
 * `session` bei `allow`: gilt für denselben Befehl in dieser Session, falls die Card
 * `approve_session` anbietet (sonst 409).
 */
remember?: Remember, 
/**
 * Geänderte Argumente bei `allow` (HAR-005 AC2).
 */
updated_args?: unknown, 
/**
 * Begründung bei `deny`; das Modell sieht sie.
 */
reason?: string, 
/**
 * Antwort bei `answer`.
 */
answer?: string, };
