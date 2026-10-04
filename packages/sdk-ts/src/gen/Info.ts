// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { AttachmentLimitsView } from "./AttachmentLimitsView";

/**
 * Server-Informationen.
 */
export type Info = { version: string, schema_version: number, protocol_version: number, 
/**
 * `local` (M0) oder `server`.
 */
mode: string, org_id: `org_${string}`, 
/**
 * Aktive Feature-Flags (UX-007); Funktionen hinter anderen Flags blendet die UI aus.
 */
features: Array<string>, 
/**
 * Grenzen für Anhänge (WEB-006).
 */
attachments: AttachmentLimitsView, };
