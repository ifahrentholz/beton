// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { EventType } from "./EventType";

/**
 * Platzhalter für eine in den Blob-Store ausgelagerte Nutzlast.
 */
export type OffloadedPayload = { type: EventType, payload_ref: `sha256:${string}`, };
