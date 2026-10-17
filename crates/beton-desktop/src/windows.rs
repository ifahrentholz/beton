//! Fenster der App (DESK-001, DESK-004): Zustand über Neustarts und welche Session ein
//! Fenster zeigt.
//!
//! Jedes Fenster zeigt eine Route der Web-UI (`/s/<id>` usw.); welche Session es zeigt, liest
//! die App aus seiner URL. Größe und Position je Fenster liegen in
//! `<BETON_HOME>/desktop/windows.json` (nicht in `~/Library`), damit sie mit den übrigen Daten
//! von beton wandern und Tests ein temporäres Verzeichnis nutzen können.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Größe und Position eines Fensters in physischen Pixeln.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub maximized: bool,
}

/// Ein Bildschirm (Arbeitsbereich) in physischen Pixeln.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Kleinste Fenstergröße.
pub const MIN_WIDTH: u32 = 480;
pub const MIN_HEIGHT: u32 = 360;

impl Geometry {
    /// Passt die Geometrie an die vorhandenen Bildschirme an: Liegt die Titelleiste auf keinem
    /// Bildschirm mehr (z. B. externer Monitor abgesteckt), landet das Fenster auf dem ersten.
    pub fn fit(self, screens: &[Screen]) -> Self {
        let mut g = self;
        g.width = g.width.max(MIN_WIDTH);
        g.height = g.height.max(MIN_HEIGHT);
        let Some(first) = screens.first() else {
            return g;
        };
        // Ein Punkt der Titelleiste muss sichtbar sein.
        let (px, py) = (g.x.saturating_add(40), g.y.saturating_add(10));
        let visible = screens.iter().any(|s| {
            px >= s.x
                && py >= s.y
                && i64::from(px) < i64::from(s.x) + i64::from(s.width)
                && i64::from(py) < i64::from(s.y) + i64::from(s.height)
        });
        if !visible {
            g.width = g.width.min(first.width);
            g.height = g.height.min(first.height);
            g.x = first.x + i32::try_from((first.width - g.width) / 2).unwrap_or(0);
            g.y = first.y + i32::try_from((first.height - g.height) / 2).unwrap_or(0);
        }
        g
    }
}

/// Gespeicherte Fensterzustände (DESK-001 AC4).
#[derive(Debug, Clone)]
pub struct WindowStore {
    path: PathBuf,
}

impl WindowStore {
    pub fn new(beton_home: &Path) -> Self {
        Self {
            path: beton_home.join("desktop").join("windows.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Alle gespeicherten Fenster; eine fehlende oder unlesbare Datei heißt „keine“.
    pub fn load(&self) -> BTreeMap<String, Geometry> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, label: &str) -> Option<Geometry> {
        self.load().get(label).copied()
    }

    /// Speichert ein Fenster (erst vollständig in eine temporäre Datei, dann umbenannt).
    pub fn save(&self, label: &str, geometry: Geometry) -> std::io::Result<()> {
        let mut all = self.load();
        if all.get(label) == Some(&geometry) {
            return Ok(());
        }
        all.insert(label.to_owned(), geometry);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// Session, die eine URL der Web-UI zeigt (`…/s/<id>[?…]`).
pub fn session_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map_or(url, |(_, r)| r);
    let path = after_scheme
        .find('/')
        .map_or("", |i| &after_scheme[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let id = path.strip_prefix("/s/")?.trim_end_matches('/');
    (id.starts_with("ses_") && !id.contains('/')).then(|| id.to_owned())
}

/// Ein offenes Fenster: Label und aktuelle URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    pub label: String,
    pub url: String,
    pub focused: bool,
}

/// Wohin ein Klick auf eine Session (Tray, Benachrichtigung, Deep-Link) führt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Ein Fenster zeigt die Session schon: fokussieren (und zur Route navigieren).
    Focus(String),
    /// Das zuletzt fokussierte Fenster zur Session navigieren.
    Navigate(String),
    /// Kein Fenster offen: eins öffnen.
    Open,
}

/// DESK-004 AC2: Zeigt ein Fenster die Session, wird es fokussiert, sonst wechselt das aktive
/// Fenster dorthin; ohne Fenster öffnet die App eins.
pub fn target_for(windows: &[OpenWindow], session: &str) -> Target {
    if let Some(w) = windows
        .iter()
        .find(|w| session_of(&w.url).as_deref() == Some(session))
    {
        return Target::Focus(w.label.clone());
    }
    match windows
        .iter()
        .find(|w| w.focused)
        .or_else(|| windows.first())
    {
        Some(w) => Target::Navigate(w.label.clone()),
        None => Target::Open,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desk_001_ac4_window_size_and_position_survive_a_restart() {
        let home = tempfile::tempdir().unwrap();
        let store = WindowStore::new(home.path());
        assert_eq!(store.get("main"), None);
        let g = Geometry {
            x: 120,
            y: 80,
            width: 1280,
            height: 820,
            maximized: false,
        };
        store.save("main", g).unwrap();
        store.save("w2", Geometry { x: 700, ..g }).unwrap();
        // „Neustart“: neuer Store auf demselben Verzeichnis.
        let again = WindowStore::new(home.path());
        assert_eq!(again.get("main"), Some(g));
        assert_eq!(again.get("w2").unwrap().x, 700);
        assert!(
            again.path().starts_with(home.path()),
            "nichts unter ~/Library"
        );
        std::fs::write(again.path(), "kaputt").unwrap();
        assert!(
            again.load().is_empty(),
            "unlesbar = keine gespeicherten Fenster"
        );
    }

    #[test]
    fn desk_001_ac4_window_off_screen_moves_back_onto_a_screen() {
        let screens = [Screen {
            x: 0,
            y: 0,
            width: 1440,
            height: 900,
        }];
        let on = Geometry {
            x: 100,
            y: 100,
            width: 1000,
            height: 700,
            maximized: false,
        };
        assert_eq!(on.fit(&screens), on);
        let off = Geometry { x: 3000, ..on };
        let fitted = off.fit(&screens);
        assert_eq!((fitted.x, fitted.y), (220, 100));
        let tiny = Geometry {
            width: 10,
            height: 10,
            ..on
        };
        assert_eq!(tiny.fit(&screens).width, MIN_WIDTH);
    }

    #[test]
    fn desk_001_ac2_each_window_shows_its_own_session() {
        assert_eq!(
            session_of("http://127.0.0.1:7420/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?seq=3").as_deref(),
            Some("ses_01JB8Y2D0M3K4J5H6G7F8E9D0C")
        );
        assert_eq!(session_of("http://127.0.0.1:7420/inbox"), None);
        assert_eq!(session_of("http://127.0.0.1:7420/s/ses_x/files"), None);
        let windows = [
            OpenWindow {
                label: "main".into(),
                url: "http://127.0.0.1:7420/s/ses_a".into(),
                focused: false,
            },
            OpenWindow {
                label: "w2".into(),
                url: "http://127.0.0.1:7420/s/ses_b".into(),
                focused: true,
            },
        ];
        assert_eq!(session_of(&windows[0].url).as_deref(), Some("ses_a"));
        assert_eq!(session_of(&windows[1].url).as_deref(), Some("ses_b"));
    }

    #[test]
    fn desk_004_ac2_tray_click_focuses_the_window_with_that_session() {
        let windows = vec![
            OpenWindow {
                label: "main".into(),
                url: "http://127.0.0.1:7420/s/ses_a".into(),
                focused: true,
            },
            OpenWindow {
                label: "w2".into(),
                url: "http://127.0.0.1:7420/s/ses_b?seq=2".into(),
                focused: false,
            },
        ];
        assert_eq!(target_for(&windows, "ses_b"), Target::Focus("w2".into()));
        assert_eq!(
            target_for(&windows, "ses_c"),
            Target::Navigate("main".into())
        );
        assert_eq!(target_for(&[], "ses_c"), Target::Open);
    }
}
