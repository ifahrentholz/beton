// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Ein Modell des mitgelieferten Preis-Katalogs (USE-002); USD je 1 Mio. Tokens, wie im
 * Katalog angegeben (fehlende Cache-Raten bleiben leer).
 */
export type CatalogEntryView = { model: string, 
/**
 * Weitere IDs desselben Modells, z. B. datierte Snapshots.
 */
aliases: Array<string>, provider: string, context_window: number | null, input_per_mtok: number, output_per_mtok: number, cache_read_per_mtok: number | null, cache_write_per_mtok: number | null, 
/**
 * Ab wann der Preis gilt (`YYYY-MM-DD`).
 */
valid_from: string, };
