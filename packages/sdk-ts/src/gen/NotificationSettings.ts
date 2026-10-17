// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { NotificationMatrix } from "./NotificationMatrix";
import type { QuietHours } from "./QuietHours";

/**
 * Benachrichtigungs-Einstellungen eines Users (UX-006).
 */
export type NotificationSettings = { events: NotificationMatrix, quiet_hours: QuietHours, 
/**
 * Session-Titel plus erste Zeile; `false`: nur Session und Ereignis (DESK-005 AC3).
 */
preview: boolean, };
