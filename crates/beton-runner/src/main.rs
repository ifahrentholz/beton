//! Test-Hilfsbinary des Runners. Im Produkt läuft der Runner als `beton __runner`
//! (dasselbe Binary wie der Daemon, RUN-002); beide rufen [`beton_runner::main_from_env`].
//! Wie `beton` dient es auch als MCP-Relay (`mcp serve|proxy`, HAR-009), weil der Runner
//! sein eigenes Binary als Relay in die Harness-Konfiguration schreibt.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "mcp") {
        return beton_mcp::relay::main(&args[1..]).await;
    }
    beton_runner::main_from_env().await
}
