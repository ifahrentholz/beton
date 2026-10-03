// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ConflictFile } from "./ConflictFile";
import type { ConflictSource } from "./ConflictSource";

export type GitConflictsDetected = { base: string, base_sha: string, head_sha: string, files: Array<ConflictFile>, source: ConflictSource, };
