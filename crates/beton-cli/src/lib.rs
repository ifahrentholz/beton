//! Bibliotheksteil des Binaries `beton`.
//!
//! Enthält Querschnittsbausteine, die alle Modi (`serve`, `host`, Runner, CLI) teilen.
//! Der Kommandobaum folgt mit CLI-001 (WP-10).

pub mod logging;

/// Version des Binaries, wie sie in Logs und `beton --version` erscheint.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
