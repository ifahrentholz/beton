// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { EgressSource } from "./EgressSource";

export type EgressBlocked = { method: string, host: string, path: string, reason: string, rule_hint?: string, source: EgressSource, tool_call_id?: string, };
