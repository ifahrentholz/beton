//! Test-Hilfsbinary des Runners. Im Produkt läuft der Runner als `beton __runner`
//! (dasselbe Binary wie der Daemon, RUN-002); beide rufen [`beton_runner::main_from_env`].
//! Wie `beton` dient es auch als MCP-Relay (`mcp serve|proxy`, HAR-009), weil der Runner
//! sein eigenes Binary als Relay in die Harness-Konfiguration schreibt.

use std::process::ExitCode;

fn main() -> ExitCode {
    // Wie `beton`: Linux-Launcher (`__sandbox-exec`, vor jedem weiteren Thread) und `bt-exec`
    // (SBX-002), damit Tests das zweistufige Modell mit diesem Binary prüfen können.
    if let Some(code) = beton_sandbox::linux::launcher_dispatch() {
        return code;
    }
    if let Some(code) = beton_sandbox::broker::exec_dispatch() {
        return code;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("beton-runner: Laufzeit nicht startbar: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|a| a == "mcp") {
            return beton_mcp::relay::main(&args[1..]).await;
        }
        beton_runner::main_from_env().await
    })
}
