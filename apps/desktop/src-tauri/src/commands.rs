//! Befehle der lokalen Seiten (Start, Beenden, Logs). Sie gelten nur für Fenster mit lokaler
//! Seite der App (Capability `local-pages`); jeder Befehl prüft das zusätzlich selbst.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tauri::{AppHandle, State, WebviewWindow};

use crate::core::{self, Core};
use crate::startup::StartupView;

fn ensure_local(window: &WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|e| e.to_string())?;
    if url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") {
        Ok(())
    } else {
        Err("nur für lokale Seiten der App".into())
    }
}

#[tauri::command]
pub fn startup_state(
    core: State<'_, Arc<Core>>,
    window: WebviewWindow,
) -> Result<StartupView, String> {
    ensure_local(&window)?;
    let st = core.startup.lock().map_err(|e| e.to_string())?;
    Ok(st.view(&core.home_label()))
}

#[tauri::command]
pub fn daemon_upgrade(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    ensure_local(&window)?;
    core::upgrade(&app);
    Ok(())
}

#[tauri::command]
pub fn quit_now(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    window: WebviewWindow,
) -> Result<(), String> {
    ensure_local(&window)?;
    core.quitting.store(true, Ordering::Relaxed);
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn working_sessions(
    core: State<'_, Arc<Core>>,
    window: WebviewWindow,
) -> Result<Vec<String>, String> {
    ensure_local(&window)?;
    Ok(core.shared.working())
}

/// Beenden bestätigt: Daemon weiterlaufen lassen (Default) oder stoppen.
#[tauri::command]
pub async fn quit_choice(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    window: WebviewWindow,
    stop_daemon: bool,
) -> Result<(), String> {
    ensure_local(&window)?;
    if stop_daemon && let Err(e) = core.manager.stop().await {
        return Err(e.to_string());
    }
    core.quitting.store(true, Ordering::Relaxed);
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn close_self(window: WebviewWindow) -> Result<(), String> {
    ensure_local(&window)?;
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn daemon_logs(core: State<'_, Arc<Core>>, window: WebviewWindow) -> Result<Value, String> {
    ensure_local(&window)?;
    let path = beton_desktop::logs::daemon_log(core.manager.home());
    let lines = beton_desktop::logs::tail(&path, 500).map_err(|e| e.to_string())?;
    Ok(json!({
        "path": format!("{}logs/daemon.log", core.home_label()),
        "lines": lines,
    }))
}

/// Der Link, der den Hinweis „Link auf einen anderen Server“ ausgelöst hat (DESK-006 AC2).
#[tauri::command]
pub fn link_info(core: State<'_, Arc<Core>>, window: WebviewWindow) -> Result<Value, String> {
    ensure_local(&window)?;
    let link = core.link.lock().map_err(|e| e.to_string())?.clone();
    Ok(match link {
        Some(c) => json!({
            "kind": match c.kind {
                beton_desktop::deeplink::ConfirmationKind::Server => "server",
                beton_desktop::deeplink::ConfirmationKind::Pair => "pair",
            },
            "host": c.host,
            "link": c.link,
        }),
        None => Value::Null,
    })
}

/// „Erneut versuchen“ nach einem fehlgeschlagenen Start (Entwurf `desktop-daemon`).
#[tauri::command]
pub fn retry_start(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    window: WebviewWindow,
) -> Result<(), String> {
    ensure_local(&window)?;
    core.startup.lock().map_err(|e| e.to_string())?.retry();
    core::connect(&app);
    Ok(())
}

/// „Logs anzeigen“ vom Startbildschirm.
#[tauri::command]
pub fn open_logs(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    ensure_local(&window)?;
    core::local_window(&app, core::LOGS, "logs.html", "Daemon-Logs", 900.0, 560.0);
    Ok(())
}
