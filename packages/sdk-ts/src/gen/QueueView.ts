// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { QueueItem } from "./QueueItem";

/**
 * Stand der Queue einer Session (SES-004); immer vollständig, `next_cursor` bleibt leer.
 */
export type QueueView = { 
/**
 * Einträge in Ausführungsreihenfolge (`{id, author, text, attachments, created_at}`).
 */
items: Array<QueueItem>, 
/**
 * Nach einem Interrupt pausiert, bis jemand fortsetzt oder neuen Input sendet.
 */
paused: boolean, next_cursor: string | null, };
