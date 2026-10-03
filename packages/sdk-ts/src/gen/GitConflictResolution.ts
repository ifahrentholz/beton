// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { Actor } from "./Actor";
import type { ConflictPhase } from "./ConflictPhase";
import type { ConflictResolutionKind } from "./ConflictResolutionKind";
import type { MergeStrategy } from "./MergeStrategy";

export type GitConflictResolution = { phase: ConflictPhase, strategy?: MergeStrategy, path?: string, hunk?: number, resolution?: ConflictResolutionKind, actor: Actor, };
