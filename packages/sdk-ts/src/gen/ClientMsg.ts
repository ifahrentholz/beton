// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ClientInfo } from "./ClientInfo";
import type { JsonValue } from "./serde_json/JsonValue";

/**
 * Nachrichten vom Client.
 */
export type ClientMsg = { "t": "hello", 
/**
 * `1.<minor>`
 */
protocol: string, client: ClientInfo, } | { "t": "attach", id: string, session_id: `ses_${string}`, from_seq: number, 
/**
 * Auch transiente Events (Deltas, Presence) senden.
 */
transient?: boolean, 
/**
 * Nur die letzten N Events (plus `has_more`).
 */
tail?: number, } | { "t": "detach", id: string, session_id: `ses_${string}`, } | { "t": "cmd", id: string, session_id?: `ses_${string}`, name: string, args: JsonValue, idempotency_key?: string, };
