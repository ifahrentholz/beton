// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { PolicyOutcome } from "./PolicyOutcome";

export type PolicyQueried = { subject_hash: string, tool: string, kind: string, outcome: PolicyOutcome, rules: Array<string>, limited: boolean, };
