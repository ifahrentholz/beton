//! Eingaben aus crossterm (Unix und Windows) und ihre Bytes für ein PTY.

use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Eingabe des Benutzers an die TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Key(KeyEvent),
    /// Eingefügter Text (Bracketed Paste).
    Paste(String),
    /// Neue Terminalgröße `(cols, rows)`.
    Resize(u16, u16),
}

/// Übersetzt ein crossterm-Event. Unter Windows meldet crossterm zu jeder Taste auch das
/// Loslassen (`KeyEventKind::Release`); das zählt nicht als Eingabe (TUI-005).
pub fn from_crossterm(ev: CtEvent) -> Option<Input> {
    match ev {
        CtEvent::Key(k) if k.kind == KeyEventKind::Release => None,
        CtEvent::Key(k) => Some(Input::Key(k)),
        CtEvent::Paste(t) => Some(Input::Paste(t)),
        CtEvent::Resize(c, r) => Some(Input::Resize(c, r)),
        _ => None,
    }
}

/// Ist das `Ctrl+G`, der Prefix der TUI?
pub fn is_prefix(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('g' | 'G'))
}

/// Bytes, die eine Taste an ein Programm im PTY schickt (xterm-Konvention). `app_cursor`:
/// Pfeiltasten im Application-Cursor-Modus (`ESC O A`), wie ihn z. B. htop setzt.
pub fn key_bytes(k: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let mut out = match k.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![c as u8 - b'a' + 1],
            '@' | ' ' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            ']' | '5' => vec![0x1d],
            '^' | '6' => vec![0x1e],
            '_' | '/' | '7' => vec![0x1f],
            '8' | '?' => vec![0x7f],
            other => other.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => cursor(b'A', app_cursor, k.modifiers),
        KeyCode::Down => cursor(b'B', app_cursor, k.modifiers),
        KeyCode::Right => cursor(b'C', app_cursor, k.modifiers),
        KeyCode::Left => cursor(b'D', app_cursor, k.modifiers),
        KeyCode::Home => cursor(b'H', app_cursor, k.modifiers),
        KeyCode::End => cursor(b'F', app_cursor, k.modifiers),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

fn cursor(code: u8, app: bool, mods: KeyModifiers) -> Vec<u8> {
    let m = 1
        + u8::from(mods.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(mods.contains(KeyModifiers::ALT))
        + 4 * u8::from(mods.contains(KeyModifiers::CONTROL));
    if m > 1 {
        return format!("\x1b[1;{m}{}", code as char).into_bytes();
    }
    if app {
        vec![0x1b, b'O', code]
    } else {
        vec![0x1b, b'[', code]
    }
}

/// Eingefügter Text für ein PTY; mit Bracketed Paste, wenn das Programm es eingeschaltet hat.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn tui_002_keys_become_xterm_bytes() {
        assert_eq!(
            key_bytes(&key(KeyCode::Char('a'), KeyModifiers::NONE), false),
            b"a"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            [3]
        );
        assert_eq!(
            key_bytes(&key(KeyCode::Char('x'), KeyModifiers::ALT), false),
            b"\x1bx"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::Enter, KeyModifiers::NONE), false),
            b"\r"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::Up, KeyModifiers::NONE), false),
            b"\x1b[A"
        );
        // htop schaltet den Application-Cursor-Modus ein.
        assert_eq!(
            key_bytes(&key(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::F(10), KeyModifiers::NONE), false),
            b"\x1b[21~"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::BackTab, KeyModifiers::SHIFT), false),
            b"\x1b[Z"
        );
        assert_eq!(
            key_bytes(&key(KeyCode::Right, KeyModifiers::CONTROL), false),
            b"\x1b[1;5C"
        );
        assert_eq!(paste_bytes("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(paste_bytes("a\nb", false), b"a\rb");
        assert!(is_prefix(&key(KeyCode::Char('g'), KeyModifiers::CONTROL)));
        assert!(!is_prefix(&key(KeyCode::Char('g'), KeyModifiers::NONE)));
    }

    #[test]
    fn tui_005_key_release_events_from_windows_are_ignored() {
        // crossterm meldet unter Windows Drücken und Loslassen; nur das Drücken zählt.
        let mut k = key(KeyCode::Char('y'), KeyModifiers::NONE);
        k.kind = KeyEventKind::Release;
        k.state = KeyEventState::NONE;
        assert_eq!(from_crossterm(CtEvent::Key(k)), None);
        k.kind = KeyEventKind::Press;
        assert_eq!(from_crossterm(CtEvent::Key(k)), Some(Input::Key(k)));
        k.kind = KeyEventKind::Repeat;
        assert_eq!(from_crossterm(CtEvent::Key(k)), Some(Input::Key(k)));
        assert_eq!(
            from_crossterm(CtEvent::Resize(120, 30)),
            Some(Input::Resize(120, 30))
        );
    }
}
