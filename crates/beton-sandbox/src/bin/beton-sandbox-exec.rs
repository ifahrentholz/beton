//! Test-Launcher von `beton-sandbox` (SBX-008): versteht nur `__sandbox-exec`, wie
//! `beton __sandbox-exec` im Produkt. Wird nicht ausgeliefert.

fn main() -> std::process::ExitCode {
    beton_sandbox::linux::launcher_dispatch().unwrap_or_else(|| {
        eprintln!(
            "beton-sandbox-exec: erwartet `{}`",
            beton_sandbox::linux::LAUNCHER_ARG
        );
        std::process::ExitCode::from(beton_sandbox::linux::LAUNCH_FAILED)
    })
}
