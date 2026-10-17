//! Zeichnet einen emulierten Terminal-Bildschirm (`beton_pty::Screen`, VT-Parser `vt100`) in
//! einen ratatui-Buffer (TUI-002): Zeichen, Farben (16, 256, RGB), fett, kursiv,
//! unterstrichen, invertiert, breite Zeichen.

use beton_pty::Screen;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => match i {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            i => Color::Indexed(i),
        },
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Stil einer Zelle.
pub fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()));
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.dim() {
        m |= Modifier::DIM;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    style = style.add_modifier(m);
    style
}

/// Zeichnet `screen` in `area` (oben links bündig, abgeschnitten bzw. leer aufgefüllt).
/// Liefert die Cursor-Position im Buffer, wenn sie sichtbar ist und im Bereich liegt.
pub fn render(screen: &Screen, area: Rect, buf: &mut Buffer) -> Option<Position> {
    let vt = screen.vt();
    let (rows, cols) = vt.size();
    for y in 0..area.height {
        for x in 0..area.width {
            let pos = Position::new(area.x + x, area.y + y);
            let Some(target) = buf.cell_mut(pos) else {
                continue;
            };
            target.reset();
            if y >= rows || x >= cols {
                continue;
            }
            let Some(cell) = vt.cell(y, x) else {
                continue;
            };
            if cell.is_wide_continuation() {
                // Die rechte Hälfte eines breiten Zeichens zeichnet ratatui selbst mit.
                target.set_symbol("");
                target.set_style(cell_style(cell));
                continue;
            }
            let text = cell.contents();
            target.set_symbol(if text.is_empty() { " " } else { text });
            target.set_style(cell_style(cell));
        }
    }
    let (cy, cx) = vt.cursor_position();
    (!vt.hide_cursor() && vt.scrollback() == 0 && cy < area.height && cx < area.width)
        .then(|| Position::new(area.x + cx, area.y + cy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(buf: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf.cell(Position::new(x, y)).map_or(" ", |c| c.symbol()))
            .collect()
    }

    /// Ausgabe wie von htop: alternativer Bildschirm, Cursor-Positionierung, Farben,
    /// invertierte Kopfzeile, Application-Cursor-Modus.
    pub(crate) const HTOP_LIKE: &[u8] = b"\x1b[?1049h\x1b[?1h\x1b=\x1b[H\x1b[2J\
\x1b[1;3H1 [\x1b[32m||||||||||\x1b[39m          42.1%]\
\x1b[2;1HMem[\x1b[36m|||||\x1b[39m 11.2G/32.0G]\
\x1b[4;1H\x1b[30;42m  PID USER  CPU% Command      \x1b[m\
\x1b[5;1H48121 ingo  12.4 node vite\x1b[?25l";

    #[test]
    fn tui_002_ac1_fullscreen_program_renders_with_colors_and_modes() {
        let mut screen = Screen::new(40, 8);
        screen.feed(HTOP_LIKE);
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        let cursor = render(&screen, area, &mut buf);
        assert_eq!(
            row_text(&buf, 0, 40).trim_end(),
            "  1 [||||||||||          42.1%]"
        );
        assert!(row_text(&buf, 1, 40).starts_with("Mem[|||||"));
        assert!(row_text(&buf, 3, 40).starts_with("  PID USER  CPU% Command"));
        assert!(row_text(&buf, 4, 40).starts_with("48121 ingo"));
        // Farben: grüne Balken, Kopfzeile schwarz auf grün.
        let bar = buf.cell(Position::new(5, 0)).unwrap();
        assert_eq!(bar.fg, Color::Green);
        let head = buf.cell(Position::new(2, 3)).unwrap();
        assert_eq!((head.fg, head.bg), (Color::Black, Color::Green));
        assert!(screen.vt().application_cursor(), "Pfeiltasten im App-Modus");
        assert_eq!(cursor, None, "htop versteckt den Cursor");
    }

    #[test]
    fn tui_002_ac1_resize_reflows_into_the_new_area() {
        let mut screen = Screen::new(20, 4);
        screen.feed(b"abc\r\ndef");
        let small = Rect::new(2, 1, 10, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 6));
        let cursor = render(&screen, small, &mut buf);
        assert_eq!(row_text(&buf, 1, 20), "  abc               ");
        assert_eq!(cursor, Some(Position::new(5, 2)));
        screen.resize(30, 10);
        assert_eq!(screen.size(), (30, 10));
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        render(&screen, Rect::new(0, 0, 30, 10), &mut buf);
        assert_eq!(row_text(&buf, 1, 30).trim_end(), "def");
    }

    #[test]
    fn tui_002_wide_characters_and_rgb_colors() {
        let mut screen = Screen::new(10, 2);
        screen.feed("\x1b[38;2;1;2;3m日本\x1b[m".as_bytes());
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render(&screen, area, &mut buf);
        assert_eq!(buf.cell(Position::new(0, 0)).unwrap().symbol(), "日");
        assert_eq!(buf.cell(Position::new(2, 0)).unwrap().symbol(), "本");
        assert_eq!(
            buf.cell(Position::new(0, 0)).unwrap().fg,
            Color::Rgb(1, 2, 3)
        );
    }
}
