//! Zeilenweise Ein- und Ausgabe mit Fehlerinjektion, gemeinsam für alle Protokolle.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use beton_harness::scenario::Faults;
use serde_json::Value;

/// Warum eine Simulation endet.
pub enum Stop {
    Crash(u8),
    Eof,
    Io(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Ziel von [`record_context`]: `--record <datei>`, sonst `BETON_FAKE_RECORD`.
static RECORD: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// Legt das Ziel der Kontext-Aufzeichnung fest (einmal beim Start).
pub fn set_record(path: Option<std::path::PathBuf>) {
    let _ = RECORD.set(path.or_else(|| std::env::var_os("BETON_FAKE_RECORD").map(Into::into)));
}

/// Protokolliert, was beim Modell als Kontext ankäme (AGT-005): eine JSON-Zeile
/// `{"source": …, "text": …}` je Eingabe in die Aufzeichnungsdatei, falls gesetzt.
/// `source`: `append_system_prompt` (Claude), `developer_instructions` (Codex) oder `user`.
pub fn record_context(source: &str, text: &str) {
    let Some(Some(path)) = RECORD.get() else {
        return;
    };
    let line = serde_json::json!({"source": source, "text": text}).to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// FNV-1a: stabiler Hash ohne Abhängigkeit, für deterministische IDs.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// stdin/stdout einer Simulation. Zählt ausgegebene Zeilen für die Fehlerinjektion.
pub struct Lines<R, W> {
    input: R,
    out: W,
    pub faults: Faults,
    lines: u32,
}

impl<R: BufRead, W: Write> Lines<R, W> {
    pub fn new(input: R, out: W, faults: Faults) -> Self {
        Self {
            input,
            out,
            faults,
            lines: 0,
        }
    }

    /// Schreibt eine Zeile und wendet die Fehlerinjektion an.
    pub fn emit(&mut self, value: &Value) -> Result<(), Stop> {
        self.lines += 1;
        if self.faults.malformed_line == Some(self.lines) {
            writeln!(self.out, "{{\"type\":\"assistant\",\"message\":")?;
        } else {
            writeln!(self.out, "{value}")?;
        }
        self.out.flush()?;
        if self.faults.crash_after == Some(self.lines) {
            return Err(Stop::Crash(1));
        }
        if self.faults.hang_after == Some(self.lines) {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        Ok(())
    }

    /// Nächste JSON-Zeile von stdin; ungültige Zeilen werden übersprungen.
    pub fn read(&mut self) -> Result<Value, Stop> {
        loop {
            let mut line = String::new();
            if self.input.read_line(&mut line)? == 0 {
                return Err(Stop::Eof);
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(v) => return Ok(v),
                Err(e) => eprintln!("beton-fake-cli: ungültige Eingabe ignoriert: {e}"),
            }
        }
    }
}

/// Text in Stücke von `chunk` Zeichen; ohne `chunk` ein Stück.
pub fn chunks(text: &str, chunk: Option<usize>) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let size = chunk.unwrap_or(usize::MAX).min(chars.len().max(1));
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}
