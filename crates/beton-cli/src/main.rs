use std::process::ExitCode;

fn main() -> ExitCode {
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("--version" | "-V" | "version") => {
            println!("beton {}", beton_cli::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "beton {}: Der Kommandobaum folgt mit CLI-001 (WP-10). Verfügbar: --version",
                beton_cli::VERSION
            );
            ExitCode::from(2)
        }
    }
}
