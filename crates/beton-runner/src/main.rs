//! Test-Hilfsbinary des Runners. Im Produkt läuft der Runner als `beton __runner`
//! (dasselbe Binary wie der Daemon, RUN-002); beide rufen [`beton_runner::main_from_env`].

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    beton_runner::main_from_env().await
}
