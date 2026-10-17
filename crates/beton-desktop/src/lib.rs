//! Logik der Desktop-App (DESK-001, DESK-002, DESK-004 bis DESK-006, UX-006) ohne GUI.
//!
//! Die Tauri-Hülle (`apps/desktop`) zeichnet nur Fenster, Tray, Menü und Mitteilungen; was
//! sie zeigt und wann, steht hier und ist ohne Fenster testbar.

pub mod daemon;
pub mod deeplink;
pub mod logs;
pub mod notify;
pub mod poller;
pub mod status;
pub mod windows;
