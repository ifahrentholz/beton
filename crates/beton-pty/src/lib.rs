//! Pseudo-Terminals für Workspace-Terminals (SES-019) und die Panes der TUI (TUI-002), ohne
//! tmux.
//!
//! - [`Pty`]: ein Pseudo-Terminal (Unix-PTY über `portable-pty`, unter Windows ConPTY). Das PTY
//!   hält beton in Stufe 0; den Kindprozess am Slave startet ausschließlich
//!   `beton_sandbox::Sandbox::spawn` (SBX-017), nie dieses Crate: [`Pty::slave`] liefert den
//!   Deskriptor, den die Sandbox als Controlling Terminal des Kindes einrichtet.
//! - [`Screen`]: VT-Emulator mit Scrollback (Default 10 000 Zeilen). Aus ihm entstehen
//!   Keyframes für neue Zuschauer und die Snapshots `terminal.snapshot` (DATA-011): Bytes, die
//!   ein frisches Terminal derselben Größe in denselben Zustand bringen.

pub mod pty;
pub mod screen;

pub use pty::{Pty, PtyError};
pub use screen::{Cursor, SCROLLBACK_LINES, Screen};
