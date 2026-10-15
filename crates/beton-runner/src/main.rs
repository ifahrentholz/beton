//! Test-Hilfsbinary des Runners. Im Produkt läuft der Runner als `beton __runner`
//! (dasselbe Binary wie der Daemon, RUN-002); beide rufen [`beton_runner::main_from_env`].
//! Wie `beton` dient es auch als MCP-Relay (`mcp serve|proxy`, HAR-009), weil der Runner
//! sein eigenes Binary als Relay in die Harness-Konfiguration schreibt, und als Kommando des
//! `PreToolUse`-Hooks (`hook claude …`, HAR-005).

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "mcp") {
        return beton_mcp::relay::main(&args[1..]).await;
    }
    // Wie `beton hook …` (HAR-005): der Runner schreibt sein eigenes Binary als Kommando des
    // `PreToolUse`-Hooks in die Settings der CLI.
    if args.first().is_some_and(|a| a == "hook") {
        use std::io::{Read as _, Write as _};
        let mut input = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut input);
        let out = beton_harness::hook::client(&args[1..], &input);
        let _ = std::io::stdout().write_all(out.stdout.as_bytes());
        let _ = std::io::stderr().write_all(out.stderr.as_bytes());
        return ExitCode::from(u8::try_from(out.exit).unwrap_or(2));
    }
    beton_runner::main_from_env().await
}
