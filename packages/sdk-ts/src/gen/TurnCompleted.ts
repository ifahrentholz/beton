// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { JsonValue } from "./serde_json/JsonValue";

export type TurnCompleted = { turn_id: `trn_${string}`, stop_reason: string, usage_summary: JsonValue, };
