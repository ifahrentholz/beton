// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Event } from "./Event";

export type EventPage = { 
/**
 * Events im Envelope-Format (siehe `schemas/v1/events.schema.json`).
 */
items: Array<Event>, next_cursor: string | null, };
