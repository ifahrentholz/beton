//! Desktop-App von beton (DESK-001, ADR-0004): Tauri 2 um die Web-UI des lokalen Daemons.
//!
//! Eigene Logik nur für Fenster, Menü, Tray, Mitteilungen, Deep-Links und Single-Instance;
//! was gezeigt wird, entscheidet `beton-desktop`. Der Daemon ist dasselbe `beton`-Binary als
//! Sidecar (DESK-002) und läuft unabhängig von der App weiter.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod core;
mod menu;
mod notifications;
mod startup;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use beton_desktop::daemon::{Manager, Sidecar};
use tauri::{Manager as _, RunEvent, WebviewUrl};
use tauri_plugin_deep_link::DeepLinkExt;

/// `BETON_HOME`, sonst `~/.beton` (wie CLI und Daemon).
fn beton_home() -> PathBuf {
    if let Some(home) = std::env::var_os("BETON_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".beton"))
        .unwrap_or_else(|| PathBuf::from(".beton"))
}

fn sidecar(resources: Option<PathBuf>) -> Option<Sidecar> {
    let exe = std::env::current_exe().ok()?;
    let mut s = Sidecar::locate(&exe, resources.as_deref())?;
    // Entwicklung: Web-UI aus apps/web/dist, wenn nichts anderes gesetzt ist.
    if s.web_dir.is_none() && std::env::var_os("BETON_WEB_DIR").is_none() && cfg!(debug_assertions)
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
        if dev.join("index.html").is_file() {
            s.web_dir = Some(dev);
        }
    }
    Some(s)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BETON_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    notifications::init();

    let app = tauri::Builder::default()
        // Zuerst: Eine zweite Instanz reicht ihre Argumente (z. B. einen beton://-Link) an
        // die laufende weiter und beendet sich (DESK-006 AC1).
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let links: Vec<&String> = argv.iter().filter(|a| a.starts_with("beton://")).collect();
            if links.is_empty() {
                core::focus_any(app);
            }
            for link in links {
                core::handle_link(app, link);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            commands::startup_state,
            commands::daemon_upgrade,
            commands::quit_now,
            commands::working_sessions,
            commands::quit_choice,
            commands::close_self,
            commands::daemon_logs,
            commands::link_info,
            commands::retry_start,
            commands::open_logs,
        ])
        .on_menu_event(menu::on_menu)
        .setup(|app| {
            let handle = app.handle().clone();
            let resources = app.path().resource_dir().ok();
            let manager = Manager::new(beton_home(), sidecar(resources), env!("CARGO_PKG_VERSION"));
            app.manage(Arc::new(core::Core::new(manager)));
            app.set_menu(menu::app_menu(&handle)?)?;
            menu::build_tray(&handle)?;
            core::build_window(&handle, core::MAIN, WebviewUrl::App("index.html".into()))?;
            core::track_windows(&handle);
            core::connect(&handle);
            let links = handle.clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    core::handle_link(&links, url.as_str());
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    core::handle_link(&handle, url.as_str());
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Desktop-App starten");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { code, api, .. } => {
            let c = core::core(app);
            if c.quitting.load(Ordering::Relaxed) {
                return;
            }
            // Letztes Fenster geschlossen: Die App bleibt im Tray, Sessions laufen weiter
            // (DESK-002 AC4). Beenden von außen (Dock, Abmelden): fragen wie bei ⌘Q.
            api.prevent_exit();
            if code.is_some() {
                core::request_quit(app);
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => core::focus_any(app),
        _ => {}
    });
}
