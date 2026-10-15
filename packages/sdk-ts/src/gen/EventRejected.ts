// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ersatz für ein Event, das der Server fachlich abgelehnt hat (z. B. Nutzlast über dem
 * Blob-Limit): steht an dessen Stelle im Log, damit nichts still verschwindet (PROTO-015,
 * #288). Enthält bewusst nicht den abgelehnten Inhalt.
 */
export type EventRejected = { 
/**
 * Typ des abgelehnten Events, z. B. `message.completed`.
 */
original_type: string, 
/**
 * Runner-Sequenz des abgelehnten Events.
 */
rseq: number, 
/**
 * Problem-Code, z. B. `payload_too_large` oder `validation_failed`.
 */
reason: string, };
