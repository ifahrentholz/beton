// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SessionSandbox } from "./SessionSandbox";
import type { JsonValue } from "./serde_json/JsonValue";

export type SessionStarted = { runner_id: `run_${string}`, host_id: `hst_${string}`, harness: string, harness_version: string, capabilities: JsonValue, harness_session_ref?: string, 
/**
 * Was die Sandbox dieser Session durchsetzt (SBX-006); Grundlage für `yolo` (HAR-027).
 */
sandbox?: SessionSandbox, };
