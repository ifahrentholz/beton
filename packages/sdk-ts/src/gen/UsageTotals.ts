// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { UsageItem } from "./UsageItem";

/**
 * Summen je Abrechnungsart; es gibt bewusst keine Gesamtsumme.
 */
export type UsageTotals = { 
/**
 * API-Key, Gateway, lokal: mit Kosten.
 */
metered: UsageItem, 
/**
 * Login der Vendor-CLI: Tokens, kein Geld.
 */
subscription: UsageItem, };
