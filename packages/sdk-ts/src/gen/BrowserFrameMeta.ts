// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ImageFormat } from "./ImageFormat";

/**
 * Metadaten eines Browser-Frames: `u16 meta_len ‖ JSON {w, h, ts, format} ‖ Bilddaten`.
 */
export type BrowserFrameMeta = { w: number, h: number, 
/**
 * Zeitpunkt der Aufnahme (Millisekunden seit Unix-Epoche).
 */
ts: number, format: ImageFormat, };
