// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { CompactionTrigger } from "./CompactionTrigger";

export type Compaction = { before_tokens: number, after_tokens?: number, 
/**
 * Wer die Compaction ausgelöst hat (USE-009); setzt der Runner. Fehlt in Logs vor M2.
 */
trigger?: CompactionTrigger, 
/**
 * Schwelle der Automatik als Anteil des Kontextfensters (`compaction.threshold`), nur bei
 * `trigger: beton`; der Verlauf zeigt „automatisch ab 80 %“ (USE-009).
 */
threshold?: number, };
