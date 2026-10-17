// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ChannelAccess } from "./ChannelAccess";
import type { Event } from "./Event";
import type { SessionLimits } from "./SessionLimits";
import type { JsonValue } from "./serde_json/JsonValue";

/**
 * Nachrichten vom Server.
 */
export type ServerMsg = { "t": "welcome", protocol: string, server_version: string, session_limits: SessionLimits, } | { "t": "events", session_id: `ses_${string}`, events: Array<Event>, 
/**
 * Nur bei `tail`: es gibt ältere Events.
 */
has_more?: boolean, } | { "t": "live", session_id: `ses_${string}`, head_seq: number, } | { "t": "ack", id: string, result: JsonValue, } | { "t": "nack", id: string, problem: JsonValue, } | { "t": "overflow", session_id: `ses_${string}`, resume_from: number, } | { "t": "channel_opened", id: string, channel_id: number, window: number, access: ChannelAccess, } | { "t": "channel_closed", channel_id: number, reason?: string, } | { "t": "credit", channel_id: number, bytes: number, };
