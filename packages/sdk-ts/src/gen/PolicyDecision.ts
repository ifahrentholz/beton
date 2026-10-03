// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { PolicyOutcome } from "./PolicyOutcome";
import type { JsonValue } from "./serde_json/JsonValue";

export type PolicyDecision = { decision_id: string, phase: string, outcome: PolicyOutcome, modified?: JsonValue, approval_id?: `apr_${string}`, matched: Array<JsonValue>, defaults_applied: Array<JsonValue>, conflicts: Array<JsonValue>, errors: Array<JsonValue>, grants_used: Array<JsonValue>, enforcement: string, policy_set_hash: string, eval_us: number, };
