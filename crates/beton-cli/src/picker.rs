//! Interaktive Auswahl für `beton import` ohne Argumente (CLI-007, Screen `cli-import`,
//! Zustand „Auswahl“): Pfeiltasten bewegen, Leertaste wählt, Enter importiert, Esc bricht ab.
//!
//! Die Logik ([`Picker`]) ist vom Terminal getrennt und testbar; [`run`] schaltet das
//! Terminal in den Rohmodus, zeichnet auf stderr (stdout bleibt Nutzdaten) und stellt den
//! Modus in jedem Fall wieder her.

use std::collections::BTreeSet;

/// Eine Zeile der Auswahl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Anzeigename des Werkzeugs, z. B. `Claude Code`.
    pub harness: String,
    pub title: String,
    /// Letzte Komponente des Arbeitsverzeichnisses.
    pub project: String,
    /// Relative Zeit, z. B. `vor 2 Tg.`.
    pub when: String,
    /// Größe der Verlaufsdatei, z. B. `84 KB`.
    pub size: String,
    /// Schon übernommen (ein erneuter Import wird übersprungen).
    pub imported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Toggle,
    Enter,
    Cancel,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// Gewählte Indizes, aufsteigend.
    Done(Vec<usize>),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Picker {
    items: Vec<Item>,
    cursor: usize,
    selected: BTreeSet<usize>,
    offset: usize,
    height: usize,
}

impl Picker {
    /// `height`: höchstens so viele Zeilen gleichzeitig (mindestens 1).
    pub fn new(items: Vec<Item>, height: usize) -> Self {
        Self {
            items,
            cursor: 0,
            selected: BTreeSet::new(),
            offset: 0,
            height: height.max(1),
        }
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        if self.items.is_empty() {
            return Outcome::Cancelled;
        }
        match key {
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down => self.cursor = (self.cursor + 1).min(self.items.len() - 1),
            Key::Toggle => {
                if !self.selected.remove(&self.cursor) {
                    self.selected.insert(self.cursor);
                }
            }
            Key::Enter => return Outcome::Done(self.chosen()),
            Key::Cancel => return Outcome::Cancelled,
            Key::Other => {}
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + self.height {
            self.offset = self.cursor + 1 - self.height;
        }
        Outcome::Continue
    }

    /// Ohne Markierung importiert Enter die Zeile unter dem Cursor.
    fn chosen(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            vec![self.cursor]
        } else {
            self.selected.iter().copied().collect()
        }
    }

    /// Sichtbare Zeilen plus Fußzeile, ohne Farben.
    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        let end = (self.offset + self.height).min(self.items.len());
        for (i, item) in self.items.iter().enumerate().take(end).skip(self.offset) {
            let marker = if i == self.cursor { "› " } else { "  " };
            let check = if self.selected.contains(&i) { 'x' } else { ' ' };
            let mut line = format!(
                "{marker}[{check}] {} {} {} {} {}",
                pad(&item.harness, 12),
                pad(&item.title, 30),
                pad(&item.project, 15),
                pad(&item.when, 11),
                item.size
            );
            if item.imported {
                line.push_str("  (schon übernommen)");
            }
            out.push(line.trim_end().to_owned());
        }
        if self.items.len() > self.height {
            out.push(format!(
                "  … {} von {} Chats",
                end - self.offset,
                self.items.len()
            ));
        }
        let n = self.chosen().len();
        out.push(format!(
            "Leertaste wählt · Enter importiert {n} {} · Esc bricht ab",
            if n == 1 { "Chat" } else { "Chats" }
        ));
        out
    }
}

/// Auf `width` Zeichen auffüllen bzw. mit `…` kürzen.
fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count > width {
        let mut s: String = text.chars().take(width.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        format!("{text}{}", " ".repeat(width - count))
    }
}

/// Tasten aus den Bytes eines Lesevorgangs im Rohmodus.
pub fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b if bytes.get(i + 1) == Some(&b'[') || bytes.get(i + 1) == Some(&b'O') => {
                out.push(match bytes.get(i + 2) {
                    Some(b'A') => Key::Up,
                    Some(b'B') => Key::Down,
                    _ => Key::Other,
                });
                i += 3;
                continue;
            }
            0x1b | 0x03 | b'q' => out.push(Key::Cancel),
            b' ' => out.push(Key::Toggle),
            b'\r' | b'\n' => out.push(Key::Enter),
            b'k' => out.push(Key::Up),
            b'j' => out.push(Key::Down),
            _ => out.push(Key::Other),
        }
        i += 1;
    }
    out
}

/// Zeigt die Auswahl auf dem Terminal (stdin im Rohmodus, Ausgabe auf stderr).
#[cfg(unix)]
pub fn run(mut picker: Picker, color: bool) -> std::io::Result<Outcome> {
    use std::io::{Read as _, Write as _};

    use rustix::termios::{LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr};

    struct Restore(rustix::termios::Termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, &self.0);
            let mut err = std::io::stderr();
            let _ = write!(err, "\x1b[?25h");
            let _ = err.flush();
        }
    }

    let original = tcgetattr(std::io::stdin())?;
    let mut raw = original.clone();
    // Ohne ISIG kommt Strg+C als Taste an: Abbruch stellt das Terminal sauber wieder her.
    raw.local_modes
        .remove(LocalModes::ICANON | LocalModes::ECHO | LocalModes::ISIG);
    raw.special_codes[SpecialCodeIndex::VMIN] = 1;
    raw.special_codes[SpecialCodeIndex::VTIME] = 0;
    tcsetattr(std::io::stdin(), OptionalActions::Now, &raw)?;
    let _restore = Restore(original);

    let mut err = std::io::stderr();
    let draw = |err: &mut std::io::Stderr, picker: &Picker| -> std::io::Result<usize> {
        let lines = picker.render();
        for (i, line) in lines.iter().enumerate() {
            let last = i + 1 == lines.len();
            if color && line.starts_with('›') {
                write!(err, "\x1b[2K\x1b[1m{line}\x1b[0m\r\n")?;
            } else if color && last {
                write!(err, "\x1b[2K\x1b[2m{line}\x1b[0m")?;
            } else if last {
                write!(err, "\x1b[2K{line}")?;
            } else {
                write!(err, "\x1b[2K{line}\r\n")?;
            }
        }
        err.flush()?;
        Ok(lines.len())
    };
    write!(err, "\x1b[?25l")?;
    let mut drawn = draw(&mut err, &picker)?;
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 32];
    loop {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            write!(err, "\r\n")?;
            return Ok(Outcome::Cancelled);
        }
        for key in keys(&buf[..n]) {
            match picker.handle(key) {
                Outcome::Continue => {}
                done => {
                    write!(err, "\r\n")?;
                    err.flush()?;
                    return Ok(done);
                }
            }
        }
        // Zurück an den Anfang der Auswahl und neu zeichnen.
        if drawn > 1 {
            write!(err, "\x1b[{}F", drawn - 1)?;
        } else {
            write!(err, "\r")?;
        }
        write!(err, "\x1b[J")?;
        drawn = draw(&mut err, &picker)?;
    }
}

/// Ohne Rohmodus (nicht Unix): Nummern zeilenweise eingeben.
#[cfg(not(unix))]
pub fn run(picker: Picker, _color: bool) -> std::io::Result<Outcome> {
    use std::io::{BufRead as _, Write as _};
    let mut err = std::io::stderr();
    for (i, item) in picker.items.iter().enumerate() {
        writeln!(
            err,
            "{:>3}  {}  {}  {}",
            i + 1,
            item.harness,
            item.title,
            item.when
        )?;
    }
    write!(err, "Nummern (z. B. 1 3), leer bricht ab: ")?;
    err.flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let chosen: Vec<usize> = line
        .split_whitespace()
        .filter_map(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=picker.items.len()).contains(n))
        .map(|n| n - 1)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(if chosen.is_empty() {
        Outcome::Cancelled
    } else {
        Outcome::Done(chosen)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str) -> Item {
        Item {
            harness: "Claude Code".into(),
            title: title.into(),
            project: "shop-frontend".into(),
            when: "vor 2 Tg.".into(),
            size: "84 KB".into(),
            imported: false,
        }
    }

    #[test]
    fn cli_007_picker_selects_with_space_and_imports_with_enter() {
        let mut p = Picker::new(vec![item("a"), item("b"), item("c")], 10);
        assert_eq!(p.handle(Key::Toggle), Outcome::Continue);
        p.handle(Key::Down);
        p.handle(Key::Down);
        p.handle(Key::Toggle);
        let lines = p.render();
        assert!(lines[0].starts_with("  [x] Claude Code  a"), "{lines:?}");
        assert!(lines[2].starts_with("› [x] Claude Code  c"), "{lines:?}");
        assert_eq!(
            lines.last().unwrap(),
            "Leertaste wählt · Enter importiert 2 Chats · Esc bricht ab"
        );
        assert_eq!(p.handle(Key::Enter), Outcome::Done(vec![0, 2]));
    }

    #[test]
    fn cli_007_picker_enter_without_selection_takes_the_cursor_and_esc_cancels() {
        let mut p = Picker::new(vec![item("a"), item("b")], 10);
        p.handle(Key::Down);
        assert!(p.render().last().unwrap().contains("importiert 1 Chat ·"));
        assert_eq!(p.clone().handle(Key::Enter), Outcome::Done(vec![1]));
        assert_eq!(p.handle(Key::Cancel), Outcome::Cancelled);
        assert_eq!(
            Picker::new(Vec::new(), 5).handle(Key::Enter),
            Outcome::Cancelled
        );
    }

    #[test]
    fn cli_007_picker_scrolls_and_truncates() {
        let items: Vec<Item> = (0..20).map(|i| item(&format!("Chat {i}"))).collect();
        let mut p = Picker::new(items, 5);
        for _ in 0..7 {
            p.handle(Key::Down);
        }
        let lines = p.render();
        assert_eq!(lines.len(), 7);
        assert!(lines[4].starts_with("› [ ] Claude Code  Chat 7"));
        assert_eq!(lines[5], "  … 5 von 20 Chats");
        assert_eq!(pad("ein sehr langer Titel", 8), "ein seh…");
    }

    #[test]
    fn cli_007_keys_from_raw_bytes() {
        assert_eq!(
            keys(b"\x1b[A\x1b[Bj k\r\x1b\x03q"),
            vec![
                Key::Up,
                Key::Down,
                Key::Down,
                Key::Toggle,
                Key::Up,
                Key::Enter,
                Key::Cancel,
                Key::Cancel,
                Key::Cancel
            ]
        );
    }
}
