//! Zustand der App und was Menü, Tray, Mitteilungen und Deep-Links auslösen.
//!
//! Fenster zeigen die Web-UI des lokalen Daemons (`http://127.0.0.1:<port>/…`). Angemeldet wird
//! jedes neue Fenster mit einem Einmal-Code (AUTH-004), den die App mit dem lokalen Token holt;
//! das Token selbst steht nie in einer URL und nie im Log. Seiten des Daemons haben keinen
//! Zugriff auf Befehle der App (Capability `local-pages` gilt nur für lokale Seiten).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use beton_desktop::daemon::{Connection, DaemonError, Manager, Step};
use beton_desktop::deeplink::{self, DeepLink};
use beton_desktop::notify::Focus;
use beton_desktop::poller::{Poller, Shared, Update};
use beton_desktop::status::TrayModel;
use beton_desktop::windows::{Geometry, OpenWindow, Screen, Target, WindowStore, session_of};
use tauri::{AppHandle, Manager as _, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::startup::Startup;

/// Fenster mit lokalen Seiten der App.
pub const MAIN: &str = "main";
pub const QUIT: &str = "quit";
pub const LOGS: &str = "logs";

pub struct Core {
    pub manager: Manager,
    pub shared: Arc<Shared>,
    pub conn: Mutex<Option<Connection>>,
    pub startup: Mutex<Startup>,
    pub windows: WindowStore,
    /// `beton beenden` bestätigt: Exit nicht mehr verhindern.
    pub quitting: AtomicBool,
    pub reachable: AtomicBool,
    pub tray: Mutex<TrayModel>,
    pub badge: AtomicU32,
    /// Routen aus Deep-Links, bevor die App verbunden ist.
    pending: Mutex<Vec<String>>,
    poller: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    next_window: AtomicU32,
}

impl Core {
    pub fn new(manager: Manager) -> Self {
        let windows = WindowStore::new(manager.home());
        Self {
            manager,
            shared: Arc::new(Shared::default()),
            conn: Mutex::new(None),
            startup: Mutex::new(Startup::default()),
            windows,
            quitting: AtomicBool::new(false),
            reachable: AtomicBool::new(false),
            tray: Mutex::new(TrayModel::default()),
            badge: AtomicU32::new(0),
            pending: Mutex::new(Vec::new()),
            poller: Mutex::new(None),
            next_window: AtomicU32::new(2),
        }
    }

    pub fn base_url(&self) -> Option<String> {
        self.conn
            .lock()
            .ok()?
            .as_ref()
            .map(|c| c.status.base_url.clone())
    }

    pub fn client(&self) -> Option<beton_sdk::Client> {
        self.conn.lock().ok()?.as_ref().map(|c| c.client.clone())
    }

    pub fn status(&self) -> Option<beton_desktop::daemon::DaemonStatus> {
        self.conn.lock().ok()?.as_ref().map(|c| c.status.clone())
    }

    pub fn home_label(&self) -> String {
        let home = self.manager.home().display().to_string();
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() && home.starts_with(&h) => format!("~{}/", &home[h.len()..]),
            _ => format!("{home}/"),
        }
    }
}

pub fn core(app: &AppHandle) -> Arc<Core> {
    app.state::<Arc<Core>>().inner().clone()
}

/// Erlaubte Navigation: lokale Seiten der App und die Web-UI des Daemons. Alles andere öffnet
/// die App nicht (fremde Origins gibt es in der Web-UI nicht, ADR-0033).
fn allowed(core: &Core, url: &Url) -> bool {
    if matches!(url.scheme(), "tauri" | "about") || url.host_str() == Some("tauri.localhost") {
        return true;
    }
    let Some(base) = core.base_url() else {
        return false;
    };
    Url::parse(&base).is_ok_and(|b| b.origin() == url.origin())
}

fn screens(window: &WebviewWindow) -> Vec<Screen> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let a = m.work_area();
            Screen {
                x: a.position.x,
                y: a.position.y,
                width: a.size.width,
                height: a.size.height,
            }
        })
        .collect()
}

/// Neues Fenster mit gespeicherter Größe und Position (DESK-001 AC4).
pub fn build_window(app: &AppHandle, label: &str, url: WebviewUrl) -> tauri::Result<WebviewWindow> {
    let c = core(app);
    let guard = c.clone();
    let window = WebviewWindowBuilder::new(app, label, url)
        .title("beton")
        .inner_size(1280.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .visible(false)
        // Dateien per Drag & Drop erreichen die Web-UI als HTML5-Drop (DESK-001 AC3).
        .disable_drag_drop_handler()
        .on_navigation(move |url| allowed(&guard, url))
        .build()?;
    if let Some(saved) = c.windows.get(label) {
        let g = saved.fit(&screens(&window));
        let _ = window.set_size(tauri::PhysicalSize::new(g.width, g.height));
        let _ = window.set_position(tauri::PhysicalPosition::new(g.x, g.y));
        if g.maximized {
            let _ = window.maximize();
        }
    } else if label != MAIN
        && let Some(main) = app.get_webview_window(MAIN)
        && let Ok(p) = main.outer_position()
    {
        let _ = window.set_position(tauri::PhysicalPosition::new(p.x + 40, p.y + 40));
    }
    let _ = window.show();
    let _ = window.set_focus();
    Ok(window)
}

/// Fenster mit lokaler Seite (Beenden, Logs).
pub fn local_window(app: &AppHandle, label: &str, page: &str, title: &str, w: f64, h: f64) {
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(title)
        .inner_size(w, h)
        .resizable(label == LOGS)
        .build();
}

/// Fenster der App, die eine Seite des Daemons zeigen.
pub fn open_windows(app: &AppHandle) -> Vec<(OpenWindow, WebviewWindow)> {
    let mut out: Vec<(OpenWindow, WebviewWindow)> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label != QUIT && label != LOGS)
        .filter_map(|(label, w)| {
            let url = w.url().ok()?.to_string();
            let focused = w.is_focused().unwrap_or(false);
            Some((
                OpenWindow {
                    label,
                    url,
                    focused,
                },
                w,
            ))
        })
        .collect();
    out.sort_by(|a, b| a.0.label.cmp(&b.0.label));
    out
}

/// Öffnet ein weiteres Fenster mit `route` (DESK-001 AC2: jedes Fenster eine eigene Session).
pub fn new_window(app: &AppHandle, route: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let c = core(&app);
        let Some(client) = c.client() else {
            c.pending.lock().map(|mut p| p.push(route)).ok();
            return;
        };
        let url = match Manager::login_url(&client, &route).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("Einmal-Code für ein neues Fenster: {e}");
                return;
            }
        };
        let Ok(url) = Url::parse(&url) else { return };
        let label = loop {
            let n = c.next_window.fetch_add(1, Ordering::Relaxed);
            let label = format!("w{n}");
            if app.get_webview_window(&label).is_none() {
                break label;
            }
        };
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = build_window(&handle, &label, WebviewUrl::External(url)) {
                tracing::warn!("Fenster öffnen: {e}");
            }
        });
    });
}

/// Öffnet eine Route der Web-UI: im Fenster, das die Session schon zeigt, sonst im aktiven
/// Fenster, ohne Fenster in einem neuen (DESK-004 AC2, DESK-005 AC2, DESK-006 AC1).
pub fn open_route(app: &AppHandle, route: &str) {
    let c = core(app);
    let Some(base) = c.base_url() else {
        c.pending.lock().map(|mut p| p.push(route.to_owned())).ok();
        return;
    };
    let windows = open_windows(app);
    let infos: Vec<OpenWindow> = windows.iter().map(|(o, _)| o.clone()).collect();
    let session = session_of(&format!("{base}{route}"));
    let target = match &session {
        Some(s) => beton_desktop::windows::target_for(&infos, s),
        None => match infos.iter().find(|w| w.focused).or_else(|| infos.first()) {
            Some(w) => Target::Navigate(w.label.clone()),
            None => Target::Open,
        },
    };
    let navigate = |w: &WebviewWindow| {
        // Noch auf dem Startbildschirm: Route merken, die Anmeldung öffnet sie.
        if w.url().is_ok_and(|u| !allowed_remote(&c, &u)) {
            c.pending.lock().map(|mut p| p.push(route.to_owned())).ok();
        } else if let Ok(url) = Url::parse(&format!("{base}{route}")) {
            let _ = w.navigate(url);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    };
    match target {
        Target::Focus(label) | Target::Navigate(label) => {
            if let Some((_, w)) = windows.iter().find(|(o, _)| o.label == label) {
                navigate(w);
            }
        }
        Target::Open => new_window(app, route.to_owned()),
    }
}

fn allowed_remote(core: &Core, url: &Url) -> bool {
    core.base_url()
        .and_then(|b| Url::parse(&b).ok())
        .is_some_and(|b| b.origin() == url.origin())
}

/// Ein `beton://`-Link (DESK-006). Links öffnen nur Ansichten; unbekannte Server und Koppeln
/// brauchen Server-Profile (ab M4): Die App verbindet sich dafür nie von selbst.
pub fn handle_link(app: &AppHandle, raw: &str) {
    match deeplink::parse(raw) {
        Ok(link) => match link.route() {
            Some(route) => open_route(app, &route),
            None => {
                let what = match &link {
                    DeepLink::UnknownServer { host } => format!("Server {host}"),
                    DeepLink::Pair { .. } => "Koppeln".to_owned(),
                    DeepLink::Session { .. } => String::new(),
                };
                tracing::info!(
                    "Link auf {what} nicht geöffnet: Server-Profile kommen mit M4; keine Verbindung aufgebaut"
                );
                focus_any(app);
            }
        },
        Err(e) => tracing::warn!("Link nicht geöffnet: {e}"),
    }
}

/// Holt die App nach vorn (ohne Fenster: eins öffnen).
pub fn focus_any(app: &AppHandle) {
    let windows = open_windows(app);
    if let Some((_, w)) = windows
        .iter()
        .find(|(o, _)| o.focused)
        .or_else(|| windows.first())
    {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        new_window(app, "/".into());
    }
}

/// Verbindet mit dem Daemon bzw. startet ihn und meldet das Hauptfenster an (DESK-002).
pub fn connect(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let c = core(&app);
        let progress = |s: Step| {
            if let Ok(mut st) = c.startup.lock() {
                st.apply(&s);
            }
        };
        match c.manager.connect(progress).await {
            Ok(conn) => on_connected(&app, conn).await,
            Err(DaemonError::Incompatible(v)) => {
                if let Ok(mut st) = c.startup.lock() {
                    st.incompatible = Some(v);
                }
            }
            Err(e) => {
                tracing::warn!("Daemon nicht verbunden: {e}");
                if let Ok(mut st) = c.startup.lock() {
                    st.error = Some(e.to_string());
                }
            }
        }
    });
}

async fn on_connected(app: &AppHandle, conn: Connection) {
    let c = core(app);
    let client = conn.client.clone();
    tracing::info!(
        url = %conn.status.base_url,
        started_by_app = conn.status.started_by_app,
        "mit dem lokalen Daemon verbunden"
    );
    if let Ok(mut g) = c.conn.lock() {
        *g = Some(conn);
    }
    if let Ok(mut st) = c.startup.lock() {
        st.incompatible = None;
        st.upgrading = false;
        st.connected = true;
    }
    c.reachable.store(true, Ordering::Relaxed);
    start_poller(app, client.clone());
    crate::menu::refresh_tray(app);
    let pending: Vec<String> = c
        .pending
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default();
    // Das Hauptfenster wechselt vom Startbildschirm zur Web-UI. Nach einem Neustart des
    // Daemons bleiben Fenster mit der Web-UI, wo sie sind (sie verbinden sich selbst neu);
    // offene Deep-Links öffnen ihre Session.
    let mut routes = pending.into_iter();
    if let Some(main) = app.get_webview_window(MAIN)
        && main.url().is_ok_and(|u| !allowed_remote(&c, &u))
    {
        let first = routes.next().unwrap_or_else(|| "/".into());
        match Manager::login_url(&client, &first).await {
            Ok(url) => {
                if let Ok(url) = Url::parse(&url) {
                    let _ = main.navigate(url);
                }
            }
            Err(e) => tracing::warn!("Anmeldung des Fensters: {e}"),
        }
    }
    for r in routes {
        open_route(app, &r);
    }
}

fn start_poller(app: &AppHandle, client: beton_sdk::Client) {
    let c = core(app);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let poller = Poller::new(client, c.shared.clone());
    let task = tauri::async_runtime::spawn(poller.run(tx));
    if let Ok(mut old) = c.poller.lock()
        && let Some(prev) = old.replace(task)
    {
        prev.abort();
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(update) = rx.recv().await {
            let c = core(&app);
            match update {
                Update::Badge(n) => {
                    c.badge.store(n, Ordering::Relaxed);
                    crate::menu::set_badge(&app, n);
                }
                Update::Tray(model) => {
                    if let Ok(mut t) = c.tray.lock() {
                        *t = model;
                    }
                    crate::menu::refresh_tray(&app);
                }
                Update::Notify(n) => crate::notifications::show(&app, n),
                Update::Reachable(ok) => {
                    c.reachable.store(ok, Ordering::Relaxed);
                    crate::menu::refresh_tray(&app);
                }
            }
        }
    });
}

/// Daemon neu starten, stoppen bzw. starten (Tray, DESK-002).
pub fn daemon_action(app: &AppHandle, action: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let c = core(&app);
        let result = match action {
            "stop" => c.manager.stop().await.map(|()| None),
            "restart" => c.manager.restart(|_| {}).await.map(Some),
            _ => c.manager.connect(|_| {}).await.map(Some),
        };
        match result {
            Ok(Some(conn)) => on_connected(&app, conn).await,
            Ok(None) => {
                c.reachable.store(false, Ordering::Relaxed);
                crate::menu::refresh_tray(&app);
            }
            Err(e) => tracing::warn!("Daemon ({action}): {e}"),
        }
    });
}

/// Upgrade bei inkompatiblem Daemon (DESK-002 AC3): wartet, bis keine Session mehr arbeitet,
/// stoppt den alten und startet den mitgelieferten Daemon.
pub fn upgrade(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let c = core(&app);
        if let Ok(mut st) = c.startup.lock() {
            if st.upgrading {
                return;
            }
            st.upgrading = true;
        }
        loop {
            let working = match c.manager.probe().await {
                Some((client, _)) => client
                    .all_sessions(false)
                    .await
                    .map(|all| {
                        all.iter()
                            .filter(|s| {
                                matches!(
                                    s["status"].as_str(),
                                    Some("running" | "waiting_approval" | "starting")
                                )
                            })
                            .count()
                    })
                    .unwrap_or(0),
                None => 0,
            };
            if let Ok(mut st) = c.startup.lock() {
                st.waiting = working;
            }
            if working == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        match c.manager.restart(|_| {}).await {
            Ok(conn) => on_connected(&app, conn).await,
            Err(e) => {
                if let Ok(mut st) = c.startup.lock() {
                    st.upgrading = false;
                    if !matches!(e, DaemonError::Incompatible(_)) {
                        st.incompatible = None;
                        st.error = Some(e.to_string());
                    }
                }
            }
        }
    });
}

/// `beton beenden`: Mit laufenden Sessions fragt die App, ob der Daemon weiterlaufen soll
/// (Default) oder gestoppt wird (DESK-002).
pub fn request_quit(app: &AppHandle) {
    let c = core(app);
    if !c.reachable.load(Ordering::Relaxed) || c.shared.working().is_empty() {
        c.quitting.store(true, Ordering::Relaxed);
        app.exit(0);
        return;
    }
    local_window(app, QUIT, "quit.html", "beton beenden?", 460.0, 330.0);
}

/// Sichtbare Sessions fokussierter Fenster (DESK-005) und Fenstergeometrie (DESK-001 AC4),
/// alle 250 ms.
pub fn track_windows(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: BTreeMap<String, Geometry> = BTreeMap::new();
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let c = core(&app);
            let windows = open_windows(&app);
            let mut visible = std::collections::HashSet::new();
            for (info, w) in &windows {
                let shown = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
                if info.focused
                    && shown
                    && let Some(s) = session_of(&info.url)
                {
                    visible.insert(s);
                }
                if let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) {
                    let g = Geometry {
                        x: pos.x,
                        y: pos.y,
                        width: size.width,
                        height: size.height,
                        maximized: w.is_maximized().unwrap_or(false),
                    };
                    if shown && last.get(&info.label) != Some(&g) {
                        last.insert(info.label.clone(), g);
                        if let Err(e) = c.windows.save(&info.label, g) {
                            tracing::debug!("Fensterzustand speichern: {e}");
                        }
                    }
                }
            }
            if let Ok(mut f) = c.shared.focus.lock() {
                *f = Focus { visible };
            }
        }
    });
}

/// Uhrzeit des Daemon-Starts für das Tray.
pub fn since(core: &Core) -> Option<SystemTime> {
    core.status().and_then(|s| s.since)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desk_001_only_local_pages_and_the_daemon_origin_are_navigable() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(Manager::new(dir.path().to_path_buf(), None, "0.0.0"));
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(allowed(&core, &url("tauri://localhost/index.html")));
        assert!(allowed(&core, &url("http://tauri.localhost/quit.html")));
        assert!(
            !allowed(&core, &url("http://127.0.0.1:7420/")),
            "vor dem Verbinden"
        );
        assert!(!allowed(&core, &url("https://example.com/")));
    }
}
