// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ToolSource } from "./ToolSource";
import type { JsonValue } from "./serde_json/JsonValue";

export type ToolCallRequested = { call_id: string, tool: string, mcp_server?: string, args: JsonValue, source: ToolSource, };
