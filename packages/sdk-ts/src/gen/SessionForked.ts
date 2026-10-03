// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { ForkReason } from "./ForkReason";
import type { HistoryMode } from "./HistoryMode";

export type SessionForked = { from_session: `ses_${string}`, at_seq: number, reason: ForkReason, harness?: string, from_harness?: string, history_mode?: HistoryMode, fallback_reason?: string, };
