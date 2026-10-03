// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Text-Delta einer Nachricht oder Überlegung. Mit `snapshot: true` enthält `text` den
 * gesamten bisherigen Text (Snapshot nach Reconnect, PROTO-003).
 */
export type TextDelta = { message_id: string, text: string, snapshot?: boolean, };
