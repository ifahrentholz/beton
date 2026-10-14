// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Event } from "./Event";

/**
 * Ergebnis einer Nachredaktion.
 */
export type RedactEventResponse = { 
/**
 * Das geschwärzte Event in Draht-Form.
 */
target: Event, 
/**
 * Das neue `event.redacted`, das den Vorgang dokumentiert.
 */
event: Event, };
