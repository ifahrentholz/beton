// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SandboxStage } from "./SandboxStage";
import type { JsonValue } from "./serde_json/JsonValue";

export type SandboxStarted = { stage: SandboxStage, backend: string, caps: JsonValue, degraded: boolean, };
