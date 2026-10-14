// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Wo die Phase `model_request` geprüft wird (POL-003, POL-024): `turn` – der Runner vor
 * jedem Turn bzw. jeder Eingabe; `call` – der Adapter vor jedem Model-Request
 * (Direkt-API); `observe_only` – gar nicht durchsetzbar.
 */
export type ModelRequestGate = "observe_only" | "turn" | "call";
