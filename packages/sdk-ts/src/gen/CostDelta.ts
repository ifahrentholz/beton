// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { AuthSource } from "./AuthSource";
import type { CostSource } from "./CostSource";
import type { PriceSource } from "./PriceSource";

export type CostDelta = { harness: string, model: string, input_tokens: number, output_tokens: number, cache_read_tokens: number, cache_write_tokens: number, cost_micro?: number, currency: string, source: CostSource, auth_source: AuthSource, purpose?: string, 
/**
 * Preisquelle; setzt der Server beim Append (USE-001, USE-003). Fehlt sie, ist das Event
 * noch nicht bewertet (Adapter) oder stammt aus der Zeit vor M2.
 */
cost_source?: PriceSource, 
/**
 * `catalog_version` des Preis-Katalogs, wenn er zur Bewertung diente (USE-002 AC1).
 */
pricing_version?: string, 
/**
 * API-Äquivalent einer Subscription-Nutzung laut Katalog; nur informativ, nie Teil von
 * Geld-Summen (ADR-0034).
 */
equivalent_micro?: number, };
