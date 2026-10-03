// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { TerminalKind } from "./TerminalKind";

export type TerminalOpened = { terminal_id: string, channel_id: number, kind: TerminalKind, cmd: string, cols: number, rows: number, };
