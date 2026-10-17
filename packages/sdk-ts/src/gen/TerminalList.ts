// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { TerminalView } from "./TerminalView";

/**
 * Laufende Terminals (höchstens [`MAX_TERMINALS`] je Session, daher immer vollständig;
 * `next_cursor` bleibt leer).
 */
export type TerminalList = { items: Array<TerminalView>, next_cursor: string | null, };
