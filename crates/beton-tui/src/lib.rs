//! `beton tui` (TUI-001 … TUI-005): Vollbild-Oberfläche im Terminal mit ratatui und crossterm,
//! nach dem abgenommenen Prototyp (`design/prototype/src/screens/tui/`).
//!
//! - [`model`]: Zustand einer Session aus denselben Events wie die Web-UI (TUI-001 AC1).
//! - [`app`]: Tastenlogik ohne IO, Prefix `Ctrl+G`, Freigaben per `y`/`n`/`s`/`d` (TUI-004).
//! - [`view`]: Darstellung (Session-Ansicht, Panes, Hinweis unter 80 × 24).
//! - [`runtime`]: Ereignisschleife mit höchstens 30 Bildern pro Sekunde und Diff-Rendering
//!   (TUI-001 AC2); Zugriff auf den Server nur über `beton-sdk`.
//! - [`layout`], [`vt`]: Panes ohne tmux; PTYs laufen im Runner (`beton-pty`), die TUI rendert
//!   sie über einen VT-Parser (TUI-002).
//!
//! Die TUI startet selbst keine Prozesse.

pub mod app;
pub mod context;
pub mod keys;
pub mod layout;
pub mod model;
pub mod runtime;
pub mod testing;
pub mod text;
pub mod theme;
pub mod view;
pub mod vt;

pub use runtime::{Options, run};
