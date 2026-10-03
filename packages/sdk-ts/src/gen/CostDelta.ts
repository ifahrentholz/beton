// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { AuthSource } from "./AuthSource";
import type { CostSource } from "./CostSource";

export type CostDelta = { harness: string, model: string, input_tokens: number, output_tokens: number, cache_read_tokens: number, cache_write_tokens: number, cost_micro?: number, currency: string, source: CostSource, auth_source: AuthSource, purpose?: string, };
