use std::process::ExitCode;

use beton_cli::cli::Cli;
use beton_cli::exit::Exit;
use clap::Parser as _;

fn main() -> ExitCode {
    // Fehlerhafte Aufrufe: clap schreibt Usage und Hinweis auf stderr, Exit-Code 2 (CLI-001 AC2).
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("beton: Laufzeit nicht startbar: {e}");
            return Exit::General.into();
        }
    };
    let result = runtime.block_on(beton_cli::commands::run(cli));
    // Nicht auf blockierende Leser (stdin) warten.
    runtime.shutdown_background();
    match result {
        Ok(()) => Exit::Ok.into(),
        Err(e) => {
            eprintln!("beton: {e}");
            e.exit.into()
        }
    }
}
