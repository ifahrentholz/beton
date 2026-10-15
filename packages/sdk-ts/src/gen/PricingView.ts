// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { CatalogEntryView } from "./CatalogEntryView";
import type { PriceOverrideView } from "./PriceOverrideView";

/**
 * Antwort von `GET /v1/usage/pricing`: womit der Server neue Turns bewertet (USE-002,
 * USE-003); Ziel des Hinweises „ohne Preis“ der Usage-Seite (USE-006 AC4).
 */
export type PricingView = { 
/**
 * Version des eingebauten Katalogs; kommt mit jedem beton-Release, kein Online-Abruf.
 */
catalog_version: string, currency: string, overrides: Array<PriceOverrideView>, catalog: Array<CatalogEntryView>, };
