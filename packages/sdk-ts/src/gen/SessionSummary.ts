// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionWorktree } from "./SessionWorktree";

/**
 * Eine Session in der Liste.
 */
export type SessionSummary = { id: `ses_${string}`, title: string, 
/**
 * Siehe `SessionStatus` im Event-Schema.
 */
status: string, kind: string, harness: string, archived: boolean, head_seq: number, 
/**
 * Kosten in Mikro-Einheiten der Währung.
 */
cost_micro: number, created_at: string, last_activity_at: string, 
/**
 * Eigener Worktree der Session (SES-015); fehlt ohne Worktree.
 */
worktree?: SessionWorktree, };
