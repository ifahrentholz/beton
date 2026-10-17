// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { TerminalKind } from "./TerminalKind";

export type TerminalOpened = { terminal_id: string, channel_id: number, kind: TerminalKind, cmd: string, cols: number, rows: number, 
/**
 * Anzeigename des Terminal-Tabs (SES-019, WEB-010); ändert sich mit `terminal.renamed`.
 */
name?: string, };
