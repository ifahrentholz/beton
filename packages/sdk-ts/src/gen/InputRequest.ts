// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { InputMode } from "./InputMode";

export type InputRequest = { text: string, 
/**
 * `queue` (Default): sofort bzw. nach dem laufenden Turn; `steer`: in den laufenden
 * Turn, falls der Harness `steering` kann (sonst `409 capability_unsupported`).
 */
mode?: InputMode, };
