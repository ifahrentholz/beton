fn main() {
    // Eigene Befehle nur für die lokalen Seiten (Capability `local-pages`); Seiten des Daemons
    // bekommen keine.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "startup_state",
            "daemon_upgrade",
            "quit_now",
            "working_sessions",
            "quit_choice",
            "close_self",
            "daemon_logs",
        ]),
    ))
    .expect("tauri-build");
}
