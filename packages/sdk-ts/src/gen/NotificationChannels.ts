// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Kanäle eines Ereignistyps. Inbox/In-App ist immer an und deshalb kein Feld.
 */
export type NotificationChannels = { 
/**
 * Mitteilung des Betriebssystems (DESK-005).
 */
desktop: boolean, 
/**
 * Ton zur Mitteilung.
 */
sound: boolean, 
/**
 * Zählt im Dock- bzw. Tray-Badge (DESK-004).
 */
badge: boolean, };
