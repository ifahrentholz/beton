//! Test-Launcher von `beton-sandbox` (SBX-008): versteht `__sandbox-exec` und, als `bt-exec`
//! aufgerufen, den Client des Exec-Brokers (SBX-002), wie `beton` im Produkt. Wird nicht
//! ausgeliefert.

fn main() -> std::process::ExitCode {
    if let Some(code) = beton_sandbox::broker::exec_dispatch() {
        return code;
    }
    beton_sandbox::linux::launcher_dispatch().unwrap_or_else(|| {
        eprintln!(
            "beton-sandbox-exec: erwartet `{}`",
            beton_sandbox::linux::LAUNCHER_ARG
        );
        std::process::ExitCode::from(beton_sandbox::linux::LAUNCH_FAILED)
    })
}
