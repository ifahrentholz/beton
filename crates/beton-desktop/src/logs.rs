//! Log-Ansicht des Daemons (DESK-002): die letzten Zeilen von `<BETON_HOME>/logs/daemon.log`
//! lesbar formatiert. Die Datei ist schon beim Schreiben redigiert (OBS-002); die App liest sie
//! nur und schickt sie nirgendwohin.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Höchstens so viele Bytes vom Ende der Datei.
pub const TAIL_BYTES: u64 = 256 * 1024;

pub fn daemon_log(beton_home: &Path) -> PathBuf {
    beton_home.join("logs").join("daemon.log")
}

/// Eine JSON-Zeile als `HH:MM:SS.mmm  LEVEL target: Nachricht k=v …`; andere Zeilen
/// unverändert.
pub fn format_line(line: &str) -> String {
    let Ok(Value::Object(v)) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let ts = v.get("ts").and_then(Value::as_str).unwrap_or_default();
    let time = ts
        .split_once('T')
        .map(|(_, t)| t.trim_end_matches('Z'))
        .map(|t| t.split(['+', '-']).next().unwrap_or(t))
        .map(|t| t.chars().take(12).collect::<String>())
        .unwrap_or_default();
    let level = v.get("level").and_then(Value::as_str).unwrap_or_default();
    let target = v.get("target").and_then(Value::as_str).unwrap_or_default();
    let message = v.get("message").and_then(Value::as_str).unwrap_or_default();
    let mut out = format!("{time:<12}  {level:>5} {target}: {message}");
    if let Some(Value::Object(fields)) = v.get("fields") {
        for (k, val) in fields {
            let val = match val {
                Value::String(s) if s.contains(' ') => format!("{s:?}"),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.push_str(&format!(" {k}={val}"));
        }
    }
    out
}

/// Die letzten `max` Zeilen, formatiert. Fehlt die Datei, ist die Liste leer.
pub fn tail(path: &Path, max: usize) -> std::io::Result<Vec<String>> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        // Die erste Zeile ist angeschnitten.
        lines.remove(0);
    }
    let skip = lines.len().saturating_sub(max);
    Ok(lines[skip..].iter().map(|l| format_line(l)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desk_002_log_view_formats_the_json_lines() {
        let line = r#"{"ts":"2026-10-16T09:38:02.114512Z","level":"INFO","target":"beton_server","component":"daemon","message":"listening","fields":{"addr":"127.0.0.1:7420","mode":"local"}}"#;
        assert_eq!(
            format_line(line),
            "09:38:02.114   INFO beton_server: listening addr=127.0.0.1:7420 mode=local"
        );
        assert_eq!(format_line("kein json"), "kein json");
    }

    #[test]
    fn desk_002_log_view_shows_the_last_lines_and_tolerates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = daemon_log(dir.path());
        assert!(tail(&path, 10).unwrap().is_empty());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = (0..50).map(|i| format!("zeile {i}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let last = tail(&path, 3).unwrap();
        assert_eq!(last, vec!["zeile 47", "zeile 48", "zeile 49"]);
    }
}
