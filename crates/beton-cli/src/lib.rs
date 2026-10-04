//! Bibliotheksteil des Binaries `beton`.
//!
//! Enthält den Kommandobaum (CLI-001), die Konfiguration (CLI-008), `serve` (CLI-004) und
//! Querschnittsbausteine, die alle Modi (`serve`, Runner, CLI) teilen.

pub mod cli;
pub mod commands;
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod drive;
pub mod exit;
pub mod logging;
pub mod render;
pub mod run;
pub mod serve;
pub mod sessionref;
pub mod setup;

/// Version des Binaries, wie sie in Logs und `beton --version` erscheint.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
