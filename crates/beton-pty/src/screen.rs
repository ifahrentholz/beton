//! VT-Emulator eines Terminals mit Scrollback (SES-019) und Keyframes bzw. Snapshots
//! (PROTO-007, DATA-011).

/// Scrollback je Terminal (SES-019: serverseitig 10 000 Zeilen).
pub const SCROLLBACK_LINES: usize = 10_000;

/// Cursor eines Bildschirmzustands (`terminal.snapshot.cursor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// Der emulierte Bildschirm eines Terminals.
pub struct Screen {
    parser: vt100::Parser,
    scrollback: usize,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (rows, cols) = self.parser.screen().size();
        f.debug_struct("Screen")
            .field("cols", &cols)
            .field("rows", &rows)
            .finish_non_exhaustive()
    }
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_scrollback(cols, rows, SCROLLBACK_LINES)
    }

    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, scrollback),
            scrollback,
        }
    }

    /// Ausgabe des Kindes verarbeiten.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    pub fn cursor(&self) -> Cursor {
        let s = self.parser.screen();
        let (row, col) = s.cursor_position();
        Cursor {
            row,
            col,
            visible: !s.hide_cursor(),
        }
    }

    /// Text des sichtbaren Bildschirms (ohne Formatierung, Zeilen mit `\n`).
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    /// Zeilen im Scrollback (ohne den sichtbaren Bildschirm).
    pub fn scrollback_len(&mut self) -> usize {
        let screen = self.parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        let n = screen.scrollback();
        screen.set_scrollback(0);
        n
    }

    /// Text des Scrollbacks, älteste Zeile zuerst (für Tests und Suche).
    pub fn scrollback_text(&mut self) -> Vec<String> {
        let (_, cols) = self.parser.screen().size();
        let total = self.scrollback_len();
        let screen = self.parser.screen_mut();
        let mut out = Vec::with_capacity(total);
        for offset in (1..=total).rev() {
            screen.set_scrollback(offset);
            out.push(screen.rows(0, cols).next().unwrap_or_default());
        }
        screen.set_scrollback(0);
        out
    }

    /// Bytes, die ein frisches Terminal derselben Größe in diesen Zustand bringen: Scrollback
    /// (älteste Zeile zuerst, mit Farben), dann der sichtbare Bildschirm samt Cursor und
    /// Eingabemodi. Grundlage für Keyframes neuer Zuschauer und für `terminal.snapshot`.
    pub fn keyframe(&mut self) -> Vec<u8> {
        let (rows, cols) = self.parser.screen().size();
        // Zurücksetzen: Inhalt, Attribute und Modi des Empfängers gelten nicht weiter.
        let mut out: Vec<u8> = b"\x1bc".to_vec();
        if !self.parser.screen().alternate_screen() {
            let total = self.scrollback_len();
            let screen = self.parser.screen_mut();
            for offset in (1..=total).rev() {
                screen.set_scrollback(offset);
                let wrapped = screen.row_wrapped(0);
                if let Some(row) = screen.rows_formatted(0, cols).next() {
                    out.extend_from_slice(&row);
                }
                out.extend_from_slice(b"\x1b[m");
                if !wrapped {
                    out.extend_from_slice(b"\r\n");
                }
            }
            screen.set_scrollback(0);
            if total > 0 {
                // Die Zeilen des Scrollbacks aus dem sichtbaren Bereich schieben.
                for _ in 0..rows.saturating_sub(1) {
                    out.extend_from_slice(b"\r\n");
                }
            }
        } else {
            // Vollbildprogramm (z. B. vim): dessen Bildschirm; der Scrollback des normalen
            // Bildschirms erscheint nach dessen Ende wieder beim nächsten Keyframe.
            out.extend_from_slice(b"\x1b[?1049h");
        }
        let screen = self.parser.screen();
        out.extend_from_slice(&screen.state_formatted());
        out.extend_from_slice(&screen.cursor_state_formatted());
        out
    }

    /// Der emulierte Bildschirm mit Zellen, Farben und Modi, z. B. zum Zeichnen eines Panes der
    /// TUI (TUI-002). Zeigt den mit [`Screen::set_scrollback`] gewählten Ausschnitt.
    pub fn vt(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Ausschnitt `offset` Zeilen zurück im Scrollback (0: aktueller Bildschirm); begrenzt
    /// auf den vorhandenen Scrollback.
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.screen_mut().set_scrollback(offset);
    }

    /// Größe des Scrollbacks dieses Emulators.
    pub fn scrollback_capacity(&self) -> usize {
        self.scrollback
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn replay(s: &mut Screen) -> Screen {
        let (cols, rows) = s.size();
        let mut fresh = Screen::new(cols, rows);
        fresh.feed(&s.keyframe());
        fresh
    }

    #[test]
    fn ses_019_scrollback_keeps_10000_lines() {
        let mut s = Screen::new(40, 10);
        for i in 0..12_000 {
            s.feed(format!("Zeile {i}\r\n").as_bytes());
        }
        assert_eq!(s.scrollback_len(), SCROLLBACK_LINES);
        let back = s.scrollback_text();
        assert_eq!(back.len(), SCROLLBACK_LINES);
        assert_eq!(back.last().unwrap().trim_end(), "Zeile 11990");
    }

    #[test]
    fn proto_007_keyframe_reproduces_screen_and_scrollback() {
        let mut s = Screen::new(30, 5);
        for i in 0..20 {
            s.feed(format!("\x1b[3{}mZeile {i}\x1b[m\r\n", i % 8).as_bytes());
        }
        s.feed(b"$ echo hallo");
        let mut r = replay(&mut s);
        assert_eq!(r.text(), s.text());
        assert_eq!(r.scrollback_text(), s.scrollback_text());
        assert_eq!(r.cursor(), s.cursor());
        assert_eq!(
            r.cursor(),
            Cursor {
                row: 4,
                col: 12,
                visible: true
            }
        );
        // Farben bleiben erhalten.
        let back = r.parser.screen();
        assert_eq!(
            back.cell(0, 0).unwrap().fgcolor(),
            s.parser.screen().cell(0, 0).unwrap().fgcolor()
        );
    }

    #[test]
    fn proto_007_keyframe_of_wrapped_lines_and_a_fresh_terminal() {
        let mut s = Screen::new(10, 4);
        s.feed(b"abcdefghijklmnopqrstuvwxyz\r\n1\r\n2\r\n3\r\n4\r\n");
        let mut r = replay(&mut s);
        assert_eq!(r.text(), s.text());
        assert_eq!(r.scrollback_text(), s.scrollback_text());
        let mut empty = Screen::new(80, 24);
        let mut r = replay(&mut empty);
        assert_eq!(r.text(), empty.text());
        assert_eq!(r.scrollback_len(), 0);
    }

    #[test]
    fn web_010_ac2_fullscreen_programs_keep_their_screen_and_resize() {
        let mut s = Screen::new(40, 10);
        s.feed(b"vor vim\r\n");
        // Alternativer Bildschirm wie bei vim, Statuszeile unten.
        s.feed(b"\x1b[?1049h\x1b[H\x1b[2J~\x1b[10;1H\"x.txt\" 0L");
        let r = replay(&mut s);
        assert!(r.parser.screen().alternate_screen());
        assert_eq!(r.text(), s.text());
        s.resize(60, 20);
        assert_eq!(s.size(), (60, 20));
    }

    #[test]
    fn hidden_cursor_is_part_of_the_state() {
        let mut s = Screen::new(20, 4);
        s.feed(b"\x1b[?25lx");
        let r = replay(&mut s);
        assert!(!r.cursor().visible);
    }
}
