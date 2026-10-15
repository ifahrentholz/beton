use std::process::ExitCode;

use beton_cli::cli::Cli;
use beton_cli::exit::Exit;
use clap::Parser as _;

fn main() -> ExitCode {
    // SBX-008: `beton __sandbox-exec` ist der Linux-Sandbox-Launcher. Er muss vor jedem
    // weiteren Thread laufen (`unshare(CLONE_NEWUSER)` verlangt einen einthreadigen Prozess).
    if let Some(code) = beton_sandbox::linux::launcher_dispatch() {
        return code;
    }
    // SBX-002: als `bt-exec` (bzw. `beton __exec`) ist beton der Client des Exec-Brokers in
    // Stufe 1, ohne Laufzeit und ohne Konfiguration.
    if let Some(code) = beton_sandbox::broker::exec_dispatch() {
        return code;
    }
    // Fehlerhafte Aufrufe: clap schreibt Usage und Hinweis auf stderr, Exit-Code 2 (CLI-001 AC2).
    let cli = Cli::parse();
    // Lokale Zeitzone für `beton usage`, solange der Prozess einthreadig ist (USE-007).
    beton_cli::usage_report::init_local_offset();
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
