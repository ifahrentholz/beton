// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { AuthStatus } from "./AuthStatus";

/**
 * Ergebnis von [`HarnessAdapter::probe`].
 */
export type ProbeReport = { installed: boolean, path?: string, version?: string, auth_status: AuthStatus, 
/**
 * `--version` lief nicht durch (Timeout, Fehler) – mit Begründung (HAR-003 AC2).
 */
probe_failed?: string, };
