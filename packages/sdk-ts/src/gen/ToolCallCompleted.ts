// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ToolStatus } from "./ToolStatus";
import type { JsonValue } from "./serde_json/JsonValue";

export type ToolCallCompleted = { call_id: string, status: ToolStatus, result?: JsonValue, result_ref?: string, duration_ms: number, };
