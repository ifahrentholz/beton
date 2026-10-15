// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { CompactionTrigger } from "./CompactionTrigger";

export type Compaction = { before_tokens: number, after_tokens?: number, 
/**
 * Wer die Compaction ausgelöst hat (USE-009); setzt der Runner. Fehlt in Logs vor M2.
 */
trigger?: CompactionTrigger, };
