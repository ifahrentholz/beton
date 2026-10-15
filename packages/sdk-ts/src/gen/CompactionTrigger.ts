// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Auslöser einer Compaction (USE-009): `user` über Button, `/compact` oder API, `beton`
 * automatisch vor dem nächsten Turn (`compaction.auto: beton`), `harness` vom Harness selbst.
 */
export type CompactionTrigger = "user" | "beton" | "harness";
