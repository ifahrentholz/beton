// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionWorktree } from "./SessionWorktree";

/**
 * Eine Session im Sub-Agent-Baum.
 */
export type SubagentNode = { id: string, 
/**
 * Parent im Baum; fehlt an der Wurzel ohne Parent.
 */
parent_id?: string, 
/**
 * Abstand zur angefragten Session (0 = sie selbst).
 */
depth: number, title: string, 
/**
 * Name des Agents laut `agent.resolved`.
 */
agent?: string, harness: string, model?: string, 
/**
 * Status der Session (`SessionStatus`).
 */
status: string, 
/**
 * Stand des aktuellen Auftrags vom Parent: `running`, `completed`, `failed`,
 * `interrupted`, `cancelled`.
 */
task?: string, 
/**
 * Eigene Kosten in Mikro-Einheiten (bei Subscription 0).
 */
cost_micro: number, 
/**
 * Kosten samt aller Nachfahren.
 */
subtree_cost_micro: number, 
/**
 * Eigene Tokens (Eingabe inkl. Cache plus Ausgabe).
 */
tokens: number, 
/**
 * Tokens samt aller Nachfahren.
 */
subtree_tokens: number, 
/**
 * `vendor_cli` (Subscription), `api_key` … laut letztem `cost.delta`.
 */
auth_source?: string, worktree?: SessionWorktree, created_at: string, };
