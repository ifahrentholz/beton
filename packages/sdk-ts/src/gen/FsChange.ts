// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { FsChangeKind } from "./FsChangeKind";

export type FsChange = { path: string, change: FsChangeKind, from?: string, };
