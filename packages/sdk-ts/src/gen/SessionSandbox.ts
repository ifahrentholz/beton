// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
export type SessionSandbox = { 
/**
 * Tools laufen in Stufe 2, getrennt vom Harness.
 */
tool_sandbox: boolean, 
/**
 * Netz der Tools nur über den Egress-Proxy (oder gar keins).
 */
egress_proxy: boolean, };
