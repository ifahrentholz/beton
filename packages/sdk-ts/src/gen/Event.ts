// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Actor } from "./Actor";
import type { EventPayload } from "./EventPayload";
import type { OffloadedPayload } from "./OffloadedPayload";
import type { JsonValue } from "./serde_json/JsonValue";

/**
 * Ein Event im Session-Log bzw. auf dem Draht.
 */
export type Event = { 
/**
 * Envelope-Version, derzeit immer 1.
 */
v: number, id: `evt_${string}`, session_id: `ses_${string}`, 
/**
 * Lückenlose Position im Session-Log. Transiente Events tragen die letzte dauerhafte `seq`.
 */
seq: number, ts: string, actor: Actor, turn_id?: `trn_${string}`, causation_id?: `evt_${string}`, 
/**
 * Original-Payload des Harness (optional, vor Persistenz redigiert).
 */
raw?: JsonValue, 
/**
 * Nur bei transienten Events: `true`.
 */
transient?: boolean, 
/**
 * Nur bei transienten Events: pro Session und Epoch monotone Folgenummer (PROTO-003).
 */
tseq?: number, } & (EventPayload | OffloadedPayload);
