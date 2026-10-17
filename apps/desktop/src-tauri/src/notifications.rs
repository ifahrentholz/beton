//! Mitteilungen des Betriebssystems (DESK-005). Ob und mit welchem Text eine Mitteilung
//! erscheint, entscheidet `beton_desktop::notify`; hier wird sie nur gezeigt. Ein Klick öffnet
//! die Session am Ereignis (bei Freigaben mit sichtbarer Approval-Bar, AC2).

use beton_desktop::notify::Notification;
use tauri::AppHandle;

/// Bundle-ID für macOS: im App-Bundle die eigene, sonst (Entwicklung) die des Terminals.
pub fn init() {
    #[cfg(target_os = "macos")]
    {
        let bundled = std::env::current_exe()
            .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
            .unwrap_or(false);
        let id = if bundled {
            "io.github.ifahrentholz.beton"
        } else {
            "com.apple.Terminal"
        };
        let _ = notify_rust::set_application(id);
    }
}

pub fn show(app: &AppHandle, n: Notification) {
    let app = app.clone();
    // Das Warten auf den Klick blockiert; je Mitteilung ein eigener Thread.
    std::thread::spawn(move || {
        let mut note = notify_rust::Notification::new();
        note.summary(&n.title).body(&n.body).appname("beton");
        if n.sound {
            note.sound_name(if cfg!(target_os = "macos") {
                "default"
            } else {
                "message-new-instant"
            });
        }
        // Klick auf die Mitteilung heißt unter Linux „default“.
        #[cfg(all(unix, not(target_os = "macos")))]
        note.action("default", "Öffnen");
        if n.waiting {
            note.action("open", "Öffnen");
        }
        let handle = match note.show() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Mitteilung nicht angezeigt: {e}");
                return;
            }
        };
        let route = n.route.clone();
        handle.wait_for_action(move |action| {
            if action == "default" || action == "open" {
                let target = app.clone();
                let _ = app.run_on_main_thread(move || crate::core::open_route(&target, &route));
            }
        });
    });
}
