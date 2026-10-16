// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Schutz der Tools einer Session (SBX-006, HAR-027): Stufe 2 und Netz nur über den Proxy.
 */
export type SessionSandbox = { 
/**
 * Tools laufen in Stufe 2, getrennt vom Harness.
 */
tool_sandbox: boolean, 
/**
 * Netz der Tools nur über den Egress-Proxy (oder gar keins).
 */
egress_proxy: boolean, };
