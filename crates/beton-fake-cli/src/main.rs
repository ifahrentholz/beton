//! Deterministische Fake-Vendor-CLI für Tests (HAR-026, QA-002).
//!
//! Die Simulation des stream-json-Protokolls folgt mit WP-07; bis dahin beendet sich das
//! Binary mit einem Hinweis.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("beton-fake-cli: Szenarien folgen mit HAR-026 (WP-07)");
    ExitCode::from(2)
}
