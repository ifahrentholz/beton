//! Natives Menü (DESK-001, Entwurf `desktop-shell` Zustand „Natives Menü“), Tray und
//! Dock-Badge (DESK-004, Entwurf `desktop-tray`).

use std::sync::atomic::Ordering;

use beton_desktop::status::{daemon_line, uptime};
use beton_desktop::windows::session_of;
use tauri::menu::{
    IsMenuItem, Menu, MenuBuilder, MenuEvent, MenuItem, MenuItemBuilder, PredefinedMenuItem,
    SubmenuBuilder,
};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Wry};

use crate::core::{self, LOGS};

pub const TRAY_ID: &str = "beton";

/// Menüleiste: beton, Ablage (Entwurf), Bearbeiten (Standard für Kopieren/Einsetzen), Fenster.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let item = |id: &str, text: &str, accel: Option<&str>| {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        b.build(app)
    };
    let beton = SubmenuBuilder::new(app, "beton")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&item("quit", "beton beenden", Some("CmdOrCtrl+Q"))?)
        .build()?;
    let file = SubmenuBuilder::new(app, "Ablage")
        .item(&item("new-window", "Neues Fenster", Some("CmdOrCtrl+N"))?)
        .item(&item("new-session", "Neue Session", Some("CmdOrCtrl+T"))?)
        .separator()
        .item(&item(
            "reveal-worktree",
            "Worktree im Finder zeigen",
            Some("CmdOrCtrl+Shift+R"),
        )?)
        .item(&item(
            "attach-file",
            "Datei anhängen…",
            Some("CmdOrCtrl+O"),
        )?)
        .separator()
        .close_window_with_text("Fenster schließen")
        .build()?;
    let edit = SubmenuBuilder::new(app, "Bearbeiten")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window = SubmenuBuilder::new(app, "Fenster")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .build()?;
    MenuBuilder::new(app)
        .items(&[&beton, &file, &edit, &window])
        .build()
}

fn label(app: &AppHandle, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItemBuilder::new(text).enabled(false).build(app)
}

/// Tray-Menü aus dem aktuellen Stand (Entwurf `desktop-tray`, Zustände „Tray geöffnet“ und
/// „Daemon gestoppt“).
fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let c = core::core(app);
    let reachable = c.reachable.load(Ordering::Relaxed);
    let status = c.status();
    let since = core::since(&c).map(|s| uptime(s, std::time::SystemTime::now()));
    let header = daemon_line(
        reachable,
        status.as_ref().map(|s| s.version.as_str()).unwrap_or(""),
        since.as_deref(),
    );
    let model = c.tray.lock().map(|t| t.clone()).unwrap_or_default();
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    items.push(Box::new(label(app, &format!("beton — {header}"))?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    if !reachable {
        items.push(Box::new(label(
            app,
            "Ohne Daemon laufen keine Agents. Deine Sessions sind gespeichert.",
        )?));
        items.push(Box::new(
            MenuItemBuilder::with_id("daemon-start", "Daemon starten").build(app)?,
        ));
    } else {
        if !model.waiting.is_empty() {
            items.push(Box::new(label(app, "Wartet auf dich")?));
            for e in &model.waiting {
                items.push(Box::new(
                    MenuItemBuilder::with_id(
                        format!("session:{}", e.session_id),
                        format!("{} — {}", e.title, e.note),
                    )
                    .build(app)?,
                ));
            }
        }
        if !model.running.is_empty() {
            items.push(Box::new(label(app, "Läuft")?));
            for e in &model.running {
                items.push(Box::new(
                    MenuItemBuilder::with_id(
                        format!("session:{}", e.session_id),
                        format!("{} — {}", e.title, e.note),
                    )
                    .build(app)?,
                ));
            }
        }
        if model.waiting.is_empty() && model.running.is_empty() {
            items.push(Box::new(label(app, "Keine Session wartet oder läuft")?));
        }
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(
        MenuItemBuilder::with_id("new-session", "Neue Session…")
            .enabled(reachable)
            .build(app)?,
    ));
    items.push(Box::new(
        MenuItemBuilder::with_id("open-window", "Fenster öffnen").build(app)?,
    ));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(label(
        app,
        "Daemon · 127.0.0.1, nur dieser Rechner",
    )?));
    items.push(Box::new(
        MenuItemBuilder::with_id("daemon-restart", "Neu starten")
            .enabled(reachable)
            .build(app)?,
    ));
    items.push(Box::new(
        MenuItemBuilder::with_id("daemon-stop", "Stoppen")
            .enabled(reachable && cfg!(unix))
            .build(app)?,
    ));
    items.push(Box::new(
        MenuItemBuilder::with_id("daemon-logs", "Logs anzeigen").build(app)?,
    ));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(
        MenuItemBuilder::with_id("quit", "beton beenden").build(app)?,
    ));
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    MenuBuilder::new(app).items(&refs).build()
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon<Wry>> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("beton")
        .menu(&tray_menu(app)?)
        .show_menu_on_left_click(true)
        .build(app)
}

/// Baut das Tray-Menü neu (Stand des Pollers bzw. des Daemons hat sich geändert).
pub fn refresh_tray(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        match tray_menu(&handle) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(e) => tracing::warn!("Tray-Menü: {e}"),
        }
    });
}

/// Zähler im Dock (macOS) bzw. neben dem Tray-Symbol (DESK-004, UX-001 AC5).
pub fn set_badge(app: &AppHandle, n: u32) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let label = (n > 0).then(|| n.to_string());
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_title(label.as_deref());
        }
        #[cfg(target_os = "macos")]
        if let Some((_, w)) = core::open_windows(&handle).into_iter().next() {
            let _ = w.set_badge_count((n > 0).then_some(i64::from(n)));
        }
    });
}

fn focused_session(app: &AppHandle) -> Option<String> {
    let windows = core::open_windows(app);
    windows
        .iter()
        .find(|(o, _)| o.focused)
        .and_then(|(o, _)| session_of(&o.url))
}

/// Klick in Menü oder Tray.
pub fn on_menu(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_owned();
    match id.as_str() {
        "quit" => core::request_quit(app),
        "new-window" => core::new_window(app, "/".into()),
        "open-window" => core::focus_any(app),
        "new-session" => core::open_route(app, "/?new=1"),
        "reveal-worktree" => reveal_worktree(app),
        "attach-file" => {
            // Öffnet die Dateiauswahl des Composers der aktiven Session (WEB-006); gesendet
            // wird erst, wenn der User sendet.
            if let Some((_, w)) = core::open_windows(app).into_iter().find(|(o, _)| o.focused) {
                let _ =
                    w.eval(r#"document.querySelector('[data-testid="attach-input"]')?.click()"#);
            }
        }
        "daemon-start" => core::daemon_action(app, "start"),
        "daemon-restart" => core::daemon_action(app, "restart"),
        "daemon-stop" => core::daemon_action(app, "stop"),
        "daemon-logs" => core::local_window(app, LOGS, "logs.html", "Daemon-Logs", 900.0, 560.0),
        other => {
            if let Some(session) = other.strip_prefix("session:") {
                core::open_route(app, &format!("/s/{session}"));
            }
        }
    }
}

fn reveal_worktree(app: &AppHandle) {
    let Some(session) = focused_session(app) else {
        return;
    };
    let Some(client) = core::core(app).client() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let Ok(s) = client.session(&session).await else {
            return;
        };
        if let Some(path) = s["worktree"]["path"].as_str().or(s["cwd"].as_str())
            && let Err(e) = tauri_plugin_opener::reveal_item_in_dir(path)
        {
            tracing::warn!("Im Finder zeigen: {e}");
        }
    });
}
