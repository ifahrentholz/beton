// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { SandboxStage } from "./SandboxStage";
import type { ViolationKind } from "./ViolationKind";

export type SandboxViolation = { stage: SandboxStage, kind: ViolationKind, target?: string, suppressed_count: number, };
