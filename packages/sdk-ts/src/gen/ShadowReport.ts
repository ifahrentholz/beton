// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ShadowRule } from "./ShadowRule";

/**
 * Shadow-Treffer der letzten Tage (POL-027 AC4).
 */
export type ShadowReport = { 
/**
 * Tage (UTC, `YYYY-MM-DD`), ältester zuerst, heute zuletzt.
 */
days: Array<string>, rules: Array<ShadowRule>, };
