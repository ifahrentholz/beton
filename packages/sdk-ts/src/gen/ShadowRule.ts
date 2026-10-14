// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ShadowLastHit } from "./ShadowLastHit";

/**
 * Eine Regel im Shadow-Betrieb mit Treffern je Tag.
 */
export type ShadowRule = { rule_id: string, scope: string, set: string, 
/**
 * Verdikt ohne Shadow-Betrieb: `deny` (würde ablehnen), `ask`, `modify`, …
 */
verdict: string, 
/**
 * Treffer je Tag in der Reihenfolge von `days`.
 */
counts: Array<number>, total: number, last?: ShadowLastHit, };
