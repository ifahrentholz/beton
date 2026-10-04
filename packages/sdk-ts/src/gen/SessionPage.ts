// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionSummary } from "./SessionSummary";

/**
 * Eine Seite der Session-Liste (Cursor-Pagination, PROTO-010).
 */
export type SessionPage = { items: Array<SessionSummary>, next_cursor: string | null, };
