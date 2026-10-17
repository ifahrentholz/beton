// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Was ein Client auf einem Kanal darf. Lokal hat der Owner immer `drive`; Freigaben mit
 * Rollen (`view`, `comment_approve`, COL-001) bilden ab M4 auf `read` ab.
 */
export type ChannelAccess = "read" | "drive";
