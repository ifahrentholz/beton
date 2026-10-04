//! Transcript-Import (HAR-023, HAR-024, SES-008): vorhandene Chats der Vendor-CLIs finden und
//! in normalisierte Events übersetzen.
//!
//! Datenschutz und Sicherheit: Gelesen werden ausschließlich Verlaufsdateien unterhalb eines
//! festen Wurzelverzeichnisses (Claude: `<config>/projects`, Codex: `<CODEX_HOME>/sessions`)
//! mit erlaubtem Dateinamen. Credential-Dateien der CLIs (Credentials-Dateien, `auth.json` von Codex)
//! liegen außerhalb dieser Wurzeln und sind damit nicht erreichbar (ADR-0005). Symlinks unter
//! der Wurzel werden nie verfolgt, Dateien haben ein Größenlimit, und ein unbekanntes Format
//! führt zu einem Fehler statt zu geratenen Events (fail closed). Zugriffe passieren nur auf
//! ausdrückliche Nutzeraktion (API `/v1/imports`), nie im Hintergrund.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use beton_core::event::{EventPayload, RawJson};
use beton_core::id::TurnId;
use beton_core::time::Timestamp;
use serde::Serialize;
use serde_json::{Value, json};

use crate::adapter::HostEnv;

/// Größte Verlaufsdatei, die importiert wird.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Längste Zeile; längere werden mit Warnung übersprungen.
pub const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;
/// So viel liest `discover()` je Datei für Titel, `cwd` und Modell.
pub const HEAD_BYTES: u64 = 64 * 1024;
/// Höchstens so viele Kandidaten je Harness (die zuletzt geänderten).
pub const MAX_CANDIDATES: usize = 2_000;

/// Eine gefundene Vendor-Session (Ergebnis von [`TranscriptImporter::discover`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSessionRef {
    /// Harness-ID, z. B. `claude`.
    pub harness: String,
    /// Session-ID der Vendor-CLI (Dedup-Schlüssel, SES-008).
    pub id: String,
    /// Verlaufsdatei.
    pub path: PathBuf,
    /// Erlaubte Wurzel, unter der `path` liegen muss.
    pub root: PathBuf,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub updated_at: Option<Timestamp>,
    pub size_bytes: u64,
}

/// Ein importiertes Event. `session_id`, `seq` und `actor` setzt der Server.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedEvent {
    pub payload: EventPayload,
    /// Ursprungs-Zeitstempel aus der Vendor-Datei (SES-008).
    pub ts: Option<Timestamp>,
    pub turn_id: Option<TurnId>,
    /// Vom Menschen (Nutzer-Nachricht, Turn-Beginn); sonst vom Agent.
    pub by_user: bool,
    /// Original-Zeile, wenn sie genau dieses eine Event ergibt.
    pub raw: Option<RawJson>,
}

/// Art einer Import-Warnung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// Zeile ist kein gültiges JSON-Objekt; übersprungen (HAR-023 AC3).
    CorruptLine,
    /// Zeile länger als [`MAX_LINE_BYTES`]; übersprungen.
    LineTooLong,
    /// Verworfene Zweige (Rewind); nur der aktive Zweig wird importiert (HAR-023 AC2).
    DiscardedBranches,
    /// Unbekannte Einträge, als `harness.unmapped` übernommen.
    Unmapped,
    /// Sub-Agent-Verlauf ohne auslösenden Tool-Call; nicht übernommen.
    OrphanSidechain,
    /// Verweis auf einen fehlenden Vorgänger; an den vorigen Eintrag angeschlossen.
    DanglingParent,
}

impl WarningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CorruptLine => "corrupt_line",
            Self::LineTooLong => "line_too_long",
            Self::DiscardedBranches => "discarded_branches",
            Self::Unmapped => "unmapped",
            Self::OrphanSidechain => "orphan_sidechain",
            Self::DanglingParent => "dangling_parent",
        }
    }
}

/// Hinweis des Parsers; enthält nie Inhalte aus dem Verlauf, nur Zeilennummern und Zahlen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportWarning {
    pub kind: WarningKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    pub detail: String,
}

impl ImportWarning {
    pub fn at(kind: WarningKind, line: u64, detail: impl Into<String>) -> Self {
        Self {
            kind,
            line: Some(line),
            count: None,
            detail: detail.into(),
        }
    }

    pub fn counted(kind: WarningKind, count: u64, detail: impl Into<String>) -> Self {
        Self {
            kind,
            line: None,
            count: Some(count),
            detail: detail.into(),
        }
    }
}

/// Ergebnis von [`TranscriptImporter::parse`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedTranscript {
    /// Native Session-Referenz (Claude-Session-UUID, Codex-Thread-ID).
    pub native_session_ref: String,
    /// Projektpfad, in dem der Chat lief.
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
    /// CLI-Version laut Datei, falls angegeben.
    pub cli_version: Option<String>,
    pub events: Vec<ImportedEvent>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Pfad außerhalb der Allowlist, Symlink oder kein reguläres File (fail closed).
    #[error("not_allowed: {0}")]
    NotAllowed(String),
    #[error("too_large: {size} Bytes, erlaubt sind höchstens {max}")]
    TooLarge { size: u64, max: u64 },
    /// Format nicht erkannt; es wird nichts geraten.
    #[error("unknown_format: {0}")]
    UnknownFormat(String),
    /// Verzeichnis der CLI unbekannt (weder Variable noch `HOME`).
    #[error("unconfigured: {0}")]
    Unconfigured(String),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

impl ImportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAllowed(_) => "not_allowed",
            Self::TooLarge { .. } => "too_large",
            Self::UnknownFormat(_) => "unknown_format",
            Self::Unconfigured(_) => "unconfigured",
            Self::Io(_) => "io_error",
        }
    }
}

/// Importiert Chats einer Vendor-CLI (HAR-023, HAR-024).
pub trait TranscriptImporter: Send + Sync {
    /// Findet lokale Sessions; liest nur Verlaufsdateien (HAR-023 AC4).
    fn discover(&self, env: &HostEnv) -> Result<Vec<ExternalSessionRef>, ImportError>;
    /// Übersetzt eine Session in normalisierte Events plus Warnungen.
    fn parse(&self, r: &ExternalSessionRef) -> Result<ImportedTranscript, ImportError>;
    /// Kann die Vendor-CLI diese Session aus `cwd` heraus nativ fortsetzen (Datei vorhanden)?
    /// Prüft nur die Existenz, liest nichts (SES-008 AC2).
    fn native_resumable(&self, env: &HostEnv, cwd: &Path, vendor_session_id: &str) -> bool {
        let _ = (env, cwd, vendor_session_id);
        false
    }
}

/// Protokoll der geöffneten Dateien (für Tests von HAR-023 AC4).
pub type AccessLog = Arc<Mutex<Vec<PathBuf>>>;

/// Lesezugriff auf Verlaufsdateien unter einer festen Wurzel. Erlaubt sind nur reguläre
/// Dateien mit erlaubtem Namen; Symlinks unterhalb der Wurzel werden nie verfolgt.
#[derive(Clone)]
pub struct VendorFiles {
    root: PathBuf,
    allow: fn(&str) -> bool,
    audit: Option<AccessLog>,
}

impl std::fmt::Debug for VendorFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VendorFiles")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Eine gefundene Datei.
#[derive(Debug, Clone)]
pub struct FoundFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<Timestamp>,
}

impl VendorFiles {
    /// Wurzel öffnen; `None`, wenn sie nicht existiert. Die Wurzel selbst darf ein Symlink sein
    /// (Konfiguration des Nutzers); darunter nichts.
    pub fn open(
        root: &Path,
        allow: fn(&str) -> bool,
        audit: Option<AccessLog>,
    ) -> Result<Option<Self>, ImportError> {
        let root = match std::fs::canonicalize(root) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !std::fs::metadata(&root)?.is_dir() {
            return Err(ImportError::NotAllowed(
                "Wurzel ist kein Verzeichnis".to_owned(),
            ));
        }
        Ok(Some(Self { root, allow, audit }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reguläre Dateien mit erlaubtem Namen bis `max_depth` Ebenen unter der Wurzel
    /// (1 = direkt darin). Symlinks werden übergangen.
    pub fn walk(&self, max_depth: usize) -> Vec<FoundFile> {
        let mut out = Vec::new();
        let mut stack = vec![(self.root.clone(), 1usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                // `file_type()` folgt Symlinks nicht.
                let Ok(ft) = entry.file_type() else { continue };
                let path = entry.path();
                if ft.is_dir() {
                    if depth < max_depth {
                        stack.push((path, depth + 1));
                    }
                } else if ft.is_file() {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else { continue };
                    if !(self.allow)(name) {
                        continue;
                    }
                    let Ok(meta) = entry.metadata() else { continue };
                    out.push(FoundFile {
                        path,
                        size: meta.len(),
                        modified: meta
                            .modified()
                            .ok()
                            .map(|t| Timestamp::from(time::OffsetDateTime::from(t))),
                    });
                }
            }
        }
        out
    }

    /// Prüft einen Pfad gegen die Allowlist: unter der Wurzel, nur normale Komponenten, kein
    /// Symlink auf dem Weg, reguläre Datei mit erlaubtem Namen.
    pub fn check(&self, path: &Path) -> Result<std::fs::Metadata, ImportError> {
        let rel = path
            .strip_prefix(&self.root)
            .map_err(|_| ImportError::NotAllowed("Pfad außerhalb der Wurzel".to_owned()))?;
        let mut cur = self.root.clone();
        let mut meta = None;
        for c in rel.components() {
            let Component::Normal(part) = c else {
                return Err(ImportError::NotAllowed("Pfad mit `..` o. ä.".to_owned()));
            };
            cur.push(part);
            let m = std::fs::symlink_metadata(&cur)?;
            if m.file_type().is_symlink() {
                return Err(ImportError::NotAllowed("Symlink im Pfad".to_owned()));
            }
            meta = Some(m);
        }
        let meta = meta.ok_or_else(|| ImportError::NotAllowed("Pfad ist die Wurzel".into()))?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !meta.is_file() || !(self.allow)(name) {
            return Err(ImportError::NotAllowed(
                "keine erlaubte Verlaufsdatei".to_owned(),
            ));
        }
        Ok(meta)
    }

    /// Liest höchstens `limit` Bytes einer erlaubten Datei. `whole`: die ganze Datei, mit
    /// Fehler, wenn sie größer als [`MAX_FILE_BYTES`] ist.
    pub fn read(&self, path: &Path, limit: u64, whole: bool) -> Result<Vec<u8>, ImportError> {
        use std::io::Read as _;
        let checked = self.check(path)?;
        if whole && checked.len() > MAX_FILE_BYTES {
            return Err(ImportError::TooLarge {
                size: checked.len(),
                max: MAX_FILE_BYTES,
            });
        }
        if let Some(log) = &self.audit
            && let Ok(mut l) = log.lock()
        {
            l.push(path.to_path_buf());
        }
        let file = std::fs::File::open(path)?;
        // Zwischen Prüfung und Öffnen ausgetauscht (z. B. gegen einen Symlink)? Dann ablehnen.
        let opened = file.metadata()?;
        if !same_file(&checked, &opened) {
            return Err(ImportError::NotAllowed(
                "Datei während des Öffnens verändert".to_owned(),
            ));
        }
        let cap = if whole { MAX_FILE_BYTES } else { limit };
        let mut buf = Vec::new();
        file.take(cap).read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino() && b.is_file()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    b.is_file() && a.len() == b.len()
}

/// Eine Zeile der Datei.
#[derive(Debug)]
pub enum Line<'a> {
    Json {
        no: u64,
        text: &'a str,
        value: Value,
    },
    Bad {
        no: u64,
        kind: WarningKind,
    },
}

/// Zerlegt JSONL in Zeilen (1-basiert). Leere Zeilen fallen weg; kaputte, zu lange oder
/// Nicht-Objekte werden als [`Line::Bad`] gemeldet.
pub fn lines(bytes: &[u8]) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    for (i, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let no = i as u64 + 1;
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if raw.len() > MAX_LINE_BYTES {
            out.push(Line::Bad {
                no,
                kind: WarningKind::LineTooLong,
            });
            continue;
        }
        let Ok(text) = std::str::from_utf8(raw) else {
            out.push(Line::Bad {
                no,
                kind: WarningKind::CorruptLine,
            });
            continue;
        };
        match serde_json::from_str::<Value>(text) {
            Ok(value) if value.is_object() => out.push(Line::Json { no, text, value }),
            _ => out.push(Line::Bad {
                no,
                kind: WarningKind::CorruptLine,
            }),
        }
    }
    out
}

/// Warnung zu einer schlechten Zeile.
pub fn bad_line_warning(no: u64, kind: WarningKind) -> ImportWarning {
    let detail = match kind {
        WarningKind::LineTooLong => format!("Zeile {no} ist zu lang und wurde übersprungen"),
        _ => format!("Zeile {no} ist beschädigt und wurde übersprungen"),
    };
    ImportWarning::at(kind, no, detail)
}

/// Zeitstempel aus einem Feld, falls gültig.
pub fn ts_of(v: &Value) -> Option<Timestamp> {
    v.as_str().and_then(|s| s.parse().ok())
}

/// Titel aus einem Text: erste nicht leere Zeile, höchstens 80 Zeichen.
pub fn title_from(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut t: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        t.push('…');
    }
    Some(t)
}

/// Prüft, ob ein Text wie eine UUID aussieht (`8-4-4-4-12` Hex-Zeichen).
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Turn-Rahmen beim Import: Nutzer-Eingaben beginnen einen Turn, das Ende ergibt
/// `turn.completed` bzw. `turn.interrupted`. Tool-Calls ohne Ergebnis werden am Turn-Ende als
/// `cancelled` geschlossen.
#[derive(Debug, Default)]
pub struct TurnBuilder {
    pub events: Vec<ImportedEvent>,
    turn: Option<TurnId>,
    last_ts: Option<Timestamp>,
    interrupted: bool,
    open_calls: Vec<String>,
}

impl TurnBuilder {
    pub fn turn(&self) -> Option<TurnId> {
        self.turn
    }

    /// Hängt ein Event an den laufenden Turn (beginnt bei Bedarf einen ohne Nutzer-Nachricht).
    pub fn push(&mut self, payload: EventPayload, ts: Option<Timestamp>, raw: Option<RawJson>) {
        if self.turn.is_none() {
            self.begin(ts);
        }
        match &payload {
            EventPayload::ToolCallRequested(c) => self.open_calls.push(c.call_id.clone()),
            EventPayload::ToolCallCompleted(c) => self.open_calls.retain(|o| *o != c.call_id),
            _ => {}
        }
        if ts.is_some() {
            self.last_ts = ts;
        }
        self.events.push(ImportedEvent {
            payload,
            ts: ts.or(self.last_ts),
            turn_id: self.turn,
            by_user: false,
            raw,
        });
    }

    /// Beginnt einen neuen Turn mit einer Nutzer-Nachricht.
    pub fn user_message(
        &mut self,
        message_id: String,
        content: Vec<Value>,
        ts: Option<Timestamp>,
        raw: Option<RawJson>,
    ) {
        self.finish();
        self.begin(ts);
        self.events.push(ImportedEvent {
            payload: EventPayload::MessageCompleted(beton_core::event::MessageCompleted {
                message_id,
                role: beton_core::event::MessageRole::User,
                content,
                author: None,
            }),
            ts: ts.or(self.last_ts),
            turn_id: self.turn,
            by_user: true,
            raw,
        });
    }

    fn begin(&mut self, ts: Option<Timestamp>) {
        let turn = TurnId::new();
        self.turn = Some(turn);
        self.interrupted = false;
        if ts.is_some() {
            self.last_ts = ts;
        }
        self.events.push(ImportedEvent {
            payload: EventPayload::TurnStarted(beton_core::event::TurnStarted {
                turn_id: turn,
                input_id: None,
                author: beton_core::id::PrincipalId::User(beton_core::id::UserId::LOCAL),
            }),
            ts: self.last_ts,
            turn_id: Some(turn),
            by_user: true,
            raw: None,
        });
    }

    /// Markiert den laufenden Turn als vom Nutzer abgebrochen.
    pub fn interrupt(&mut self, ts: Option<Timestamp>) {
        if self.turn.is_some() {
            self.interrupted = true;
            if ts.is_some() {
                self.last_ts = ts;
            }
        }
    }

    /// IDs der Tool-Calls ohne Ergebnis im laufenden Turn.
    pub fn open_calls(&self) -> &[String] {
        &self.open_calls
    }

    /// Schließt den laufenden Turn.
    pub fn finish(&mut self) {
        let Some(turn) = self.turn.take() else { return };
        for call_id in std::mem::take(&mut self.open_calls) {
            self.events.push(ImportedEvent {
                payload: EventPayload::ToolCallCompleted(beton_core::event::ToolCallCompleted {
                    call_id,
                    status: beton_core::event::ToolStatus::Cancelled,
                    result: None,
                    result_ref: None,
                    duration_ms: 0,
                }),
                ts: self.last_ts,
                turn_id: Some(turn),
                by_user: false,
                raw: None,
            });
        }
        let payload = if std::mem::take(&mut self.interrupted) {
            EventPayload::TurnInterrupted(beton_core::event::TurnInterrupted {
                turn_id: turn,
                by: beton_core::id::PrincipalId::User(beton_core::id::UserId::LOCAL),
                reason: None,
            })
        } else {
            EventPayload::TurnCompleted(beton_core::event::TurnCompleted {
                turn_id: turn,
                stop_reason: "end_turn".into(),
                usage_summary: json!({}),
            })
        };
        self.events.push(ImportedEvent {
            payload,
            ts: self.last_ts,
            turn_id: Some(turn),
            by_user: false,
            raw: None,
        });
    }
}

/// Golden-Form eines Imports: eine Zeile je Event mit Typ, Nutzlast (IDs normalisiert),
/// Ursprungs-Zeitstempel, Turn, Herkunft und ob `raw` mitkam.
pub fn golden_events(t: &ImportedTranscript, workdir: &str) -> String {
    let mut normalizer = crate::golden::Normalizer::new(workdir);
    let mut out = String::new();
    for e in &t.events {
        let mut v = serde_json::to_value(&e.payload).unwrap_or(Value::Null);
        v = normalizer.value(v);
        if let Some(obj) = v.as_object_mut() {
            if let Some(turn) = e.turn_id {
                obj.insert(
                    "turn_id".into(),
                    Value::String(normalizer.text(&turn.to_string())),
                );
            }
            obj.insert(
                "ts".into(),
                e.ts.map_or(Value::Null, |ts| Value::String(ts.to_string())),
            );
            obj.insert("by_user".into(), Value::Bool(e.by_user));
            obj.insert("has_raw".into(), Value::Bool(e.raw.is_some()));
        }
        out.push_str(&v.to_string());
        out.push('\n');
    }
    out
}

/// Metadaten und Warnungen eines Imports als Golden-Text.
pub fn golden_summary(t: &ImportedTranscript, workdir: &str) -> String {
    let summary = json!({
        "native_session_ref": t.native_session_ref,
        "cwd": t.cwd,
        "model": t.model,
        "title": t.title,
        "started_at": t.started_at.map(|ts| ts.to_string()),
        "updated_at": t.updated_at.map(|ts| ts.to_string()),
        "cli_version": t.cli_version,
        "warnings": t.warnings,
    });
    // Zeitstempel bleiben stehen (Ursprungszeiten sind Teil der Erwartung), nur Pfade werden
    // normalisiert.
    let text = serde_json::to_string_pretty(&summary).unwrap_or_default();
    let mut out = text.replace(workdir, "<workdir>");
    out.push('\n');
    out
}

/// Vergleicht einen Import mit `expected.events.jsonl` und `expected.summary.json` im
/// Fall-Verzeichnis; mit `BETON_BLESS=1` werden die Erwartungen neu geschrieben.
pub fn check_golden(dir: &Path, t: &ImportedTranscript, workdir: &str) -> Result<(), String> {
    let bless = std::env::var(crate::golden::BLESS_ENV).is_ok_and(|v| v == "1");
    let files = [
        ("expected.events.jsonl", golden_events(t, workdir)),
        ("expected.summary.json", golden_summary(t, workdir)),
    ];
    let mut diff = String::new();
    for (name, got) in files {
        let path = dir.join(name);
        if bless {
            std::fs::write(&path, &got).map_err(|e| e.to_string())?;
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == got => {}
            Ok(expected) => diff.push_str(&crate::golden::line_diff(name, &expected, &got)),
            Err(_) => diff.push_str(&format!(
                "{name} fehlt (mit {}=1 erzeugen)\n",
                crate::golden::BLESS_ENV
            )),
        }
    }
    if diff.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: {diff}\nAbsichtliche Änderung? Mit {}=1 neu schreiben.",
            dir.display(),
            crate::golden::BLESS_ENV
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn allow_jsonl(name: &str) -> bool {
        name.ends_with(".jsonl")
    }

    #[test]
    fn lines_report_corrupt_and_skip_empty() {
        let text = b"{\"a\":1}\n\n{kaputt\n[1]\n{\"b\":2}\r\n";
        let l = lines(text);
        assert_eq!(l.len(), 4);
        assert!(matches!(l[0], Line::Json { no: 1, .. }));
        assert!(matches!(
            l[1],
            Line::Bad {
                no: 3,
                kind: WarningKind::CorruptLine
            }
        ));
        assert!(matches!(
            l[2],
            Line::Bad {
                no: 4,
                kind: WarningKind::CorruptLine
            }
        ));
        assert!(matches!(l[3], Line::Json { no: 5, .. }));
    }

    #[test]
    fn uuid_and_title_helpers() {
        assert!(is_uuid("11111111-2222-4333-8444-555555555555"));
        assert!(!is_uuid("../../.credentials"));
        assert!(!is_uuid("11111111-2222-4333-8444-55555555555g"));
        assert_eq!(
            title_from("\n  Hallo Welt \nmehr").as_deref(),
            Some("Hallo Welt")
        );
        let long = "x".repeat(100);
        assert_eq!(title_from(&long).unwrap().chars().count(), 81);
    }

    #[cfg(unix)]
    #[test]
    fn vendor_files_never_follow_symlinks_or_leave_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.jsonl"), "{}\n").unwrap();
        std::fs::write(root.join("sub/b.txt"), "{}\n").unwrap();
        std::fs::write(dir.path().join("secret.jsonl"), "geheim").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.jsonl"), root.join("sub/c.jsonl"))
            .unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("outside")).unwrap();
        let log: AccessLog = Arc::default();
        let files = VendorFiles::open(&root, allow_jsonl, Some(log.clone()))
            .unwrap()
            .unwrap();
        let found: Vec<String> = files
            .walk(3)
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["a.jsonl"]);
        let root = files.root().to_path_buf();
        assert!(files.read(&root.join("sub/a.jsonl"), 10, false).is_ok());
        for bad in [
            root.join("sub/b.txt"),
            root.join("sub/c.jsonl"),
            root.join("outside/secret.jsonl"),
            root.join("sub/../../secret.jsonl"),
            dir.path().join("secret.jsonl"),
        ] {
            let err = files.read(&bad, 10, true).unwrap_err();
            assert!(
                matches!(err, ImportError::NotAllowed(_)),
                "{}: {err}",
                bad.display()
            );
        }
        assert_eq!(*log.lock().unwrap(), [root.join("sub/a.jsonl")]);
        assert!(
            VendorFiles::open(&dir.path().join("fehlt"), allow_jsonl, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn turn_builder_closes_open_calls_and_marks_interrupts() {
        use beton_core::event::{ToolCallRequested, ToolSource};
        let mut b = TurnBuilder::default();
        b.user_message(
            "u1".into(),
            vec![json!({"type":"text","text":"Hi"})],
            None,
            None,
        );
        b.push(
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: "c1".into(),
                tool: "Bash".into(),
                mcp_server: None,
                args: json!({}),
                source: ToolSource::Harness,
                parent_call_id: None,
            }),
            None,
            None,
        );
        b.interrupt(None);
        b.finish();
        let types: Vec<&str> = b.events.iter().map(|e| e.payload.type_name()).collect();
        assert_eq!(
            types,
            [
                "turn.started",
                "message.completed",
                "tool.call.requested",
                "tool.call.completed",
                "turn.interrupted"
            ]
        );
        assert!(b.events[0].by_user && b.events[1].by_user && !b.events[2].by_user);
    }
}
