//! Farben der TUI nach den Nacht-Werten der Prototyp-Tokens (`design/prototype/src/index.css`):
//! Schalungsgelb (`signal`) heißt immer „du bist dran“; Stimmen je Harness-Familie.

use ratatui::style::{Color, Modifier, Style};

use crate::model::{Glyph, Voice};

pub const SIGNAL: Color = Color::Rgb(0xe2, 0xb3, 0x3b);
pub const SIGNAL_FG: Color = Color::Rgb(0x1d, 0x1f, 0x22);
pub const OK: Color = Color::Rgb(0x62, 0xb5, 0x80);
pub const DENY: Color = Color::Rgb(0xec, 0x7d, 0x71);
pub const WARN: Color = Color::Rgb(0xe8, 0x80, 0x3c);
pub const MUTED: Color = Color::Rgb(0x8b, 0x8e, 0x93);
pub const FG: Color = Color::Rgb(0xdc, 0xdd, 0xd8);
pub const BORDER: Color = Color::Rgb(0x55, 0x58, 0x5c);
pub const TITLE_BG: Color = Color::Rgb(0x2c, 0x2f, 0x33);
pub const SELECTED_BG: Color = Color::Rgb(0x34, 0x37, 0x3b);
pub const VOICE_CLAUDE: Color = Color::Rgb(0xad, 0x99, 0xda);
pub const VOICE_CODEX: Color = Color::Rgb(0x56, 0xb0, 0xb7);
pub const VOICE_ACP: Color = Color::Rgb(0xaa, 0xb5, 0x65);
pub const VOICE_DIRECT: Color = Color::Rgb(0x93, 0xa2, 0xbf);

pub fn dim() -> Style {
    Style::default().fg(MUTED)
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::default().fg(OK)
}

pub fn deny() -> Style {
    Style::default().fg(DENY)
}

pub fn warn() -> Style {
    Style::default().fg(WARN)
}

pub fn signal() -> Style {
    Style::default().fg(SIGNAL)
}

pub fn code() -> Style {
    Style::default().fg(VOICE_CODEX)
}

/// Taste in der Tastenleiste: invertiert.
pub fn key() -> Style {
    Style::default().fg(Color::Black).bg(FG)
}

/// Hervorgehobene Taste (`y` erlauben).
pub fn key_signal() -> Style {
    Style::default()
        .fg(SIGNAL_FG)
        .bg(SIGNAL)
        .add_modifier(Modifier::BOLD)
}

pub fn voice(v: Voice) -> Style {
    let c = match v {
        Voice::Claude => VOICE_CLAUDE,
        Voice::Codex => VOICE_CODEX,
        Voice::Acp => VOICE_ACP,
        Voice::Direct => VOICE_DIRECT,
    };
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

pub fn glyph(g: Glyph) -> Style {
    match g {
        Glyph::Waiting => signal(),
        Glyph::Running => ok(),
        Glyph::Failed => deny(),
        Glyph::Idle | Glyph::Stopped => dim(),
    }
}
