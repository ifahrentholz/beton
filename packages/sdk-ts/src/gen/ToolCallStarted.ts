// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SandboxStage } from "./SandboxStage";
import type { JsonValue } from "./serde_json/JsonValue";

export type ToolCallStarted = { call_id: string, sandbox_stage?: SandboxStage, 
/**
 * Ausgeführte Argumente, wenn das Gate sie geändert hat (HAR-005 AC2).
 */
args?: JsonValue, };
