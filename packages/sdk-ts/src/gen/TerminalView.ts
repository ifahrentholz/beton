// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { TerminalKind } from "./TerminalKind";

/**
 * Ein laufendes Terminal der Session.
 */
export type TerminalView = { terminal_id: string, 
/**
 * Kanal des Runners; Clients öffnen das Terminal per `open_channel {kind: terminal, ref:
 * terminal_id}` und bekommen eine eigene Kanal-ID.
 */
channel_id: number, name: string, kind: TerminalKind, cmd: string, cols: number, rows: number, };
