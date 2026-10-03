// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { JsonValue } from "./serde_json/JsonValue";

export type TerminalSnapshot = { terminal_id: string, blob_ref: string, cols: number, rows: number, cursor: JsonValue, };
