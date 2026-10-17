//! Bibliotheksteil des Binaries `beton`.
//!
//! Enthält den Kommandobaum (CLI-001), die Konfiguration (CLI-008), `serve` (CLI-004) und
//! Querschnittsbausteine, die alle Modi (`serve`, Runner, CLI) teilen.

pub mod agent;
pub mod cli;
pub mod commands;
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod drive;
pub mod exit;
pub mod logging;
pub mod picker;
pub mod policy;
pub mod render;
pub mod run;
pub mod sandbox;
pub mod secrets;
pub mod serve;
pub mod sessionref;
pub mod setup;
pub mod transfer;
pub mod usage;
pub mod usage_report;
pub mod voice;
pub mod yaml_edit;

/// Version des Binaries, wie sie in Logs und `beton --version` erscheint.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
