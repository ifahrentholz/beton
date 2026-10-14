// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Preisquelle eines Usage-Records (USE-001), gesetzt beim Append in der Reihenfolge der
 * Spec (11-platform-features.md, „Kostenberechnung“); `subscription` bei `vendor_cli`.
 */
export type PriceSource = "override" | "harness" | "catalog" | "none" | "subscription";
