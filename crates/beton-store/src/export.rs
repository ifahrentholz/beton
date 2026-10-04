//! Export-/Import-Format für Sessions (DATA-010, SES-009).
//!
//! Eine Exportdatei ist JSONL: Zeile 1 ist der Kopf [`ExportHeader`]
//! (`{"type":"beton.export","format_version":1,…}`), danach folgt je Zeile ein dauerhaftes
//! Event als PROTO-001-Envelope in `seq`-Reihenfolge. Ohne Blobs sind ausgelagerte Payloads
//! wieder eingebettet, die Datei ist also für sich vollständig. Mit Blobs entsteht ein
//! `.tar.zst` mit `session.jsonl` und `blobs/<sha256>`; ausgelagerte Payloads behalten dort
//! ihr `payload_ref`.
//!
//! Eine Importdatei ist **nicht vertrauenswürdig**. [`parse_export`] prüft sie vollständig,
//! bevor irgendetwas geschrieben wird: Größen, Archiv-Einträge (nur `session.jsonl` und
//! `blobs/<sha256>`, keine Links, keine Pfade), Blob-Hashes, jede Zeile gegen die Rust-Typen,
//! aus denen `schemas/v1/events.schema.json` generiert ist, lückenlose `seq`, Session-ID und
//! Anzahl laut Kopf. Fehler nennen die Zeile. Erst danach legt
//! [`Store::import_export`] die Session in einer Transaktion an: neue Session- und Event-IDs,
//! `seq` und Zeitstempel bleiben, unbekannte User werden zu `system/import`.
//!
//! Eine importierte Session ist auf diesem Host nicht ausführbar: Arbeitsverzeichnis,
//! Worktree, Agent-Ref und Startoptionen stammen aus der Datei. Der Store vermerkt den Import
//! in `session_file_imports`, lässt den Worktree weg und beendet offene Zustände
//! ([`seal_imported`]); der Server lehnt Runner, Fork und Workspace-Zugriffe ab.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;

use beton_core::event::{
    Actor, BlobRef, ENVELOPE_VERSION, Event, EventBody, EventPayload, Persistence, RawJson,
    SessionKind, SessionStatus, SystemComponent,
};
use beton_core::id::{EventId, NodeId, OrgId, PrincipalId, SessionId, UserId};
use beton_core::time::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};

use crate::Store;
use crate::error::{Error, Result};
use crate::events::{event_from_row, payload_from_json, reference_blob};
use crate::sessions::{enum_to_str, parse, to_i64};

/// Wert von `type` im Kopf.
pub const EXPORT_TYPE: &str = "beton.export";
/// Höchste Formatversion, die diese Implementierung liest und schreibt.
pub const FORMAT_VERSION: u32 = 1;
/// Name der JSONL-Datei im Archiv.
pub const JSONL_NAME: &str = "session.jsonl";
/// Größte Importdatei (komprimiert bzw. JSONL): 256 MiB.
pub const MAX_IMPORT_BYTES: usize = 256 * 1024 * 1024;
/// Obergrenze für den entpackten Inhalt eines Archivs (Schutz vor Zip-Bomben): 1 GiB.
pub const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;
/// Längste Zeile: 64 MiB.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;
/// Höchstzahl der Events einer Importdatei.
pub const MAX_EVENTS: u64 = 1_000_000;
/// Höchstzahl der Blobs in einem Archiv.
pub const MAX_BLOBS: usize = 10_000;
/// Magische Bytes eines zstd-Frames.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Optionen des Exports (`--with-raw`, `--with-blobs`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Redigiertes `raw` mitexportieren (Default: weggelassen, DATA-010 AC3).
    pub with_raw: bool,
    /// `.tar.zst` mit `session.jsonl` und `blobs/<sha256>` statt reinem JSONL.
    pub with_blobs: bool,
}

/// Eine fertige Exportdatei.
#[derive(Debug, Clone)]
pub struct ExportFile {
    pub bytes: Vec<u8>,
    /// Anzahl der Event-Zeilen.
    pub events: u64,
    /// Anzahl der Blobs im Archiv (0 bei JSONL).
    pub blobs: u64,
    /// `true` bei `.tar.zst`, sonst JSONL.
    pub archive: bool,
}

/// Zeile 1 einer Exportdatei.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportHeader {
    /// Immer `beton.export`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Formatversion; eine Implementierung lehnt höhere Versionen ab
    /// (`unsupported_format_version`).
    pub format_version: u32,
    pub exported_at: Timestamp,
    /// Version von beton, die exportiert hat (nur zur Anzeige).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beton_version: Option<String>,
    pub session: ExportSession,
}

/// Metadaten der exportierten Session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportSession {
    pub id: SessionId,
    pub title: String,
    pub harness: String,
    pub status: SessionStatus,
    pub archived: bool,
    pub created_at: Timestamp,
    /// Anzahl der folgenden Event-Zeilen (`seq` 1 bis `events`).
    pub events: u64,
    /// Die Event-Zeilen enthalten `raw` (`--with-raw`).
    #[serde(default)]
    pub raw: bool,
    /// Blobs im Archiv unter `blobs/<sha256>` (nur `.tar.zst`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<BlobRef>,
    /// Die Session wurde ihrerseits aus einer Exportdatei importiert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<ImportedFrom>,
}

/// Herkunft einer importierten Session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportedFrom {
    /// Session-ID in der exportierenden Instanz.
    pub session_id: SessionId,
    pub exported_at: Timestamp,
    /// SHA-256 (Hex) der importierten `session.jsonl`; Schlüssel der Idempotenz.
    pub export_sha256: String,
}

/// Art eines Fehlers in einer Importdatei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidKind {
    /// Formatversion höher als [`FORMAT_VERSION`] (DATA-010 AC4).
    UnsupportedFormatVersion,
    /// Lücke oder Sprung in `seq` (DATA-010 AC2).
    SeqGap,
    /// Datei oder Archiv größer als erlaubt.
    TooLarge,
    /// Sonst ungültig: Zeile, Kopf, Archiv-Eintrag, Blob.
    Invalid,
}

impl InvalidKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnsupportedFormatVersion => "unsupported_format_version",
            Self::SeqGap => "import_seq_gap",
            Self::TooLarge => "payload_too_large",
            Self::Invalid => "import_invalid",
        }
    }
}

/// Eine abgelehnte Importdatei. Es wurde nichts angelegt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct InvalidExport {
    pub kind: InvalidKind,
    /// Zeile in `session.jsonl` (1-basiert), falls der Fehler einer Zeile zuzuordnen ist.
    pub line: Option<u64>,
    /// JSON-Pointer, z. B. `/lines/419/seq`.
    pub pointer: Option<String>,
    /// Verständliche Meldung (nur Zeilennummern, Zahlen und Feldnamen, keine Inhalte).
    pub detail: String,
}

impl InvalidExport {
    fn new(kind: InvalidKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            line: None,
            pointer: None,
            detail: detail.into(),
        }
    }

    fn invalid(detail: impl Into<String>) -> Self {
        Self::new(InvalidKind::Invalid, detail)
    }

    fn at(kind: InvalidKind, line: u64, field: &str, detail: impl std::fmt::Display) -> Self {
        let field = field.trim_start_matches('/');
        Self {
            kind,
            line: Some(line),
            pointer: Some(if field.is_empty() {
                format!("/lines/{line}")
            } else {
                format!("/lines/{line}/{field}")
            }),
            detail: format!("Zeile {line}: {detail}"),
        }
    }
}

/// Eine geprüfte Importdatei.
#[derive(Debug, Clone)]
pub struct ParsedExport {
    pub header: ExportHeader,
    /// Die Events wie in der Datei, ausgelagerte Payloads bereits aus dem Archiv eingebettet.
    pub events: Vec<Event>,
    /// SHA-256 (Hex) der `session.jsonl`.
    pub sha256: String,
    /// Blobs aus dem Archiv, die keine ausgelagerte Payload sind (z. B. Anhänge).
    pub blobs: BTreeMap<BlobRef, Vec<u8>>,
}

/// Ergebnis eines Imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileImportStatus {
    Imported,
    /// Dieselbe Datei (gleicher Hash) wurde schon importiert; `session_id` ist die bestehende.
    Skipped,
}

#[derive(Debug, Clone)]
pub struct FileImportOutcome {
    pub status: FileImportStatus,
    pub session_id: SessionId,
    pub title: String,
    pub events: u64,
    pub blobs: u64,
    pub imported_from: ImportedFrom,
}

/// Herkunftsvermerk einer importierten Session (`session_file_imports`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileImportRecord {
    pub imported_from: ImportedFrom,
    pub imported_at: Timestamp,
}

/// Prüft eine Importdatei (JSONL oder `.tar.zst`) vollständig. Liest nichts außer `bytes`
/// und verwendet keine Pfade aus der Datei.
pub fn parse_export(bytes: &[u8]) -> std::result::Result<ParsedExport, InvalidExport> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(InvalidExport::new(
            InvalidKind::TooLarge,
            format!(
                "Die Datei ist größer als {} MiB",
                MAX_IMPORT_BYTES / 1024 / 1024
            ),
        ));
    }
    let (jsonl, archive_blobs, archive) = if bytes.starts_with(&ZSTD_MAGIC) {
        let (jsonl, blobs) = unpack(bytes)?;
        (jsonl, blobs, true)
    } else {
        (bytes.to_vec(), BTreeMap::new(), false)
    };
    let sha256 = hex::encode(Sha256::digest(&jsonl));
    let (header, events, used) = parse_jsonl(&jsonl, &archive_blobs)?;
    let listed: BTreeSet<&BlobRef> = header.session.blobs.iter().collect();
    let present: BTreeSet<&BlobRef> = archive_blobs.keys().collect();
    if listed != present {
        return Err(InvalidExport::at(
            InvalidKind::Invalid,
            1,
            "session/blobs",
            if archive {
                "Die Blob-Liste im Kopf passt nicht zu den Blobs im Archiv"
            } else {
                "Der Kopf nennt Blobs, aber die Datei ist kein Archiv (.tar.zst)"
            },
        ));
    }
    let blobs = archive_blobs
        .into_iter()
        .filter(|(b, _)| !used.contains(b))
        .collect();
    Ok(ParsedExport {
        header,
        events,
        sha256,
        blobs,
    })
}

/// Liest ein `.tar.zst`: genau eine `session.jsonl` und Blobs unter `blobs/<sha256>`.
type Unpacked = (Vec<u8>, BTreeMap<BlobRef, Vec<u8>>);

fn unpack(bytes: &[u8]) -> std::result::Result<Unpacked, InvalidExport> {
    let bad = |d: &str| InvalidExport::invalid(format!("Archiv ungültig: {d}"));
    let decoder = zstd::stream::read::Decoder::new(bytes).map_err(|_| bad("kein zstd"))?;
    let limited = Limited {
        inner: decoder,
        left: MAX_UNPACKED_BYTES,
    };
    let mut archive = tar::Archive::new(limited);
    let mut jsonl: Option<Vec<u8>> = None;
    let mut blobs = BTreeMap::new();
    let entries = archive.entries().map_err(|e| unpack_error(&e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| unpack_error(&e))?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            if name == "blobs" || name == "blobs/" {
                continue;
            }
            return Err(bad("unerwartetes Verzeichnis im Archiv"));
        }
        if !kind.is_file() {
            // Keine Links, Geräte oder Sonstiges: nichts aus dem Archiv zeigt auf den Host.
            return Err(bad("nur reguläre Dateien sind erlaubt"));
        }
        let size = entry.header().size().map_err(|_| bad("Größe unlesbar"))?;
        let limit = if name == JSONL_NAME {
            MAX_IMPORT_BYTES as u64
        } else {
            crate::blobs::DEFAULT_MAX_BLOB_BYTES
        };
        if size > limit {
            return Err(InvalidExport::new(
                InvalidKind::TooLarge,
                "Archiv ungültig: ein Eintrag ist zu groß",
            ));
        }
        let mut data = Vec::with_capacity(usize::try_from(size).unwrap_or(0).min(1 << 20));
        (&mut entry)
            .take(size)
            .read_to_end(&mut data)
            .map_err(|e| unpack_error(&e))?;
        if name == JSONL_NAME {
            if jsonl.replace(data).is_some() {
                return Err(bad("session.jsonl kommt doppelt vor"));
            }
            continue;
        }
        let blob = name
            .strip_prefix("blobs/")
            .and_then(|hex| BlobRef::from_hex(hex).ok())
            .ok_or_else(|| {
                bad("unerwarteter Eintrag (erlaubt: session.jsonl und blobs/<sha256>)")
            })?;
        if hex::encode(Sha256::digest(&data)) != blob.hex() {
            return Err(bad("ein Blob passt nicht zu seinem Hash"));
        }
        if blobs.insert(blob, data).is_some() {
            return Err(bad("ein Blob kommt doppelt vor"));
        }
        if blobs.len() > MAX_BLOBS {
            return Err(InvalidExport::new(
                InvalidKind::TooLarge,
                format!("Archiv ungültig: mehr als {MAX_BLOBS} Blobs"),
            ));
        }
    }
    let jsonl = jsonl.ok_or_else(|| bad("session.jsonl fehlt"))?;
    Ok((jsonl, blobs))
}

fn unpack_error(e: &std::io::Error) -> InvalidExport {
    if e.kind() == std::io::ErrorKind::FileTooLarge {
        InvalidExport::new(
            InvalidKind::TooLarge,
            format!(
                "Archiv ungültig: entpackt größer als {} MiB",
                MAX_UNPACKED_BYTES / 1024 / 1024
            ),
        )
    } else {
        InvalidExport::invalid("Archiv ungültig: kein lesbares .tar.zst")
    }
}

/// Liest höchstens `left` Bytes und meldet danach einen Fehler statt still abzuschneiden.
struct Limited<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.left = self.left.checked_sub(n as u64).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::FileTooLarge, "Archiv zu groß")
        })?;
        Ok(n)
    }
}

/// Kopf und Events aus `session.jsonl`. Liefert außerdem die Blobs, die als ausgelagerte
/// Payload verwendet wurden.
type ParsedLines = (ExportHeader, Vec<Event>, HashSet<BlobRef>);

fn parse_jsonl(
    jsonl: &[u8],
    blobs: &BTreeMap<BlobRef, Vec<u8>>,
) -> std::result::Result<ParsedLines, InvalidExport> {
    let mut lines: Vec<&[u8]> = jsonl.split(|b| *b == b'\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let Some(first) = lines.first() else {
        return Err(InvalidExport::invalid("Die Datei ist leer"));
    };
    let header = parse_header(first)?;
    let declared = header.session.events;
    if declared > MAX_EVENTS {
        return Err(InvalidExport::new(
            InvalidKind::TooLarge,
            format!("Mehr als {MAX_EVENTS} Ereignisse"),
        ));
    }
    let mut events = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
    let mut ids = HashSet::new();
    let mut used = HashSet::new();
    for (i, raw_line) in lines.iter().enumerate().skip(1) {
        let line = i as u64 + 1;
        let expected = i as u64;
        if expected > declared {
            return Err(InvalidExport::at(
                InvalidKind::Invalid,
                line,
                "",
                format!("mehr Ereignisse als die {declared} im Kopf"),
            ));
        }
        let event = parse_event_line(line, raw_line)?;
        if event.seq != expected {
            let detail = if event.seq > expected {
                format!(
                    "auf Ereignis {} folgt direkt {} – Ereignis {expected} fehlt",
                    expected - 1,
                    event.seq
                )
            } else {
                format!(
                    "Ereignis {} steht an falscher Stelle oder doppelt (erwartet {expected})",
                    event.seq
                )
            };
            return Err(InvalidExport::at(InvalidKind::SeqGap, line, "seq", detail));
        }
        let invalid =
            |field: &str, d: &str| InvalidExport::at(InvalidKind::Invalid, line, field, d);
        if event.v != ENVELOPE_VERSION {
            return Err(invalid("v", "unbekannte Envelope-Version"));
        }
        if event.session_id != header.session.id {
            return Err(invalid(
                "session_id",
                "gehört zu einer anderen Session als der Kopf",
            ));
        }
        if event.transient || event.tseq.is_some() {
            return Err(invalid(
                "transient",
                "transiente Events gehören nicht in einen Export",
            ));
        }
        if event.body.event_type().persistence() != Persistence::Durable {
            return Err(invalid("type", "transienter Event-Typ"));
        }
        if !ids.insert(event.id) {
            return Err(invalid("id", "Event-ID kommt doppelt vor"));
        }
        let event = match event.body {
            EventBody::Inline(_) => event,
            EventBody::Offloaded(ref o) => {
                let Some(bytes) = blobs.get(&o.payload_ref) else {
                    return Err(invalid(
                        "payload_ref",
                        "der ausgelagerte Payload fehlt im Archiv",
                    ));
                };
                let payload = std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|json| payload_from_json(o.event_type, json).ok())
                    .ok_or_else(|| {
                        invalid("payload_ref", "der ausgelagerte Payload ist ungültig")
                    })?;
                used.insert(o.payload_ref.clone());
                Event {
                    body: EventBody::Inline(payload),
                    ..event
                }
            }
        };
        if expected == 1 && !matches!(event.payload(), Some(EventPayload::SessionCreated(_))) {
            return Err(invalid(
                "type",
                "das erste Ereignis muss session.created sein",
            ));
        }
        events.push(event);
    }
    if (events.len() as u64) != declared {
        return Err(InvalidExport::at(
            InvalidKind::Invalid,
            lines.len() as u64 + 1,
            "",
            format!(
                "die Datei endet nach {} Ereignissen, der Kopf nennt {declared}",
                events.len()
            ),
        ));
    }
    Ok((header, events, used))
}

fn parse_header(line: &[u8]) -> std::result::Result<ExportHeader, InvalidExport> {
    let not_export = || {
        InvalidExport::at(
            InvalidKind::Invalid,
            1,
            "type",
            "keine beton-Exportdatei (Kopf `beton.export` fehlt)",
        )
    };
    if line.len() > MAX_LINE_BYTES {
        return Err(not_export());
    }
    let value: serde_json::Value = serde_json::from_slice(line).map_err(|_| not_export())?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some(EXPORT_TYPE) {
        return Err(not_export());
    }
    let version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .filter(|v| *v >= 1)
        .ok_or_else(|| {
            InvalidExport::at(
                InvalidKind::Invalid,
                1,
                "format_version",
                "`format_version` fehlt oder ist ungültig",
            )
        })?;
    if version > u64::from(FORMAT_VERSION) {
        let from = value
            .get("beton_version")
            .and_then(serde_json::Value::as_str)
            .filter(|v| v.len() <= 64 && v.chars().all(|c| c.is_ascii_graphic()))
            .map(|v| format!(" (aus beton {v})"))
            .unwrap_or_default();
        return Err(InvalidExport::at(
            InvalidKind::UnsupportedFormatVersion,
            1,
            "format_version",
            format!(
                "Die Datei hat Exportformat {version}{from}. Diese Version von beton liest bis \
                 Format {FORMAT_VERSION}. Aktualisiere beton und importiere die Datei dann \
                 erneut. Es wurde nichts angelegt."
            ),
        ));
    }
    typed(1, &value)
}

fn typed<T: serde::de::DeserializeOwned>(
    line: u64,
    value: &serde_json::Value,
) -> std::result::Result<T, InvalidExport> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path = pointer(e.path());
        InvalidExport::at(
            InvalidKind::Invalid,
            line,
            &path,
            format!(
                "ungültig bei `{}`: {}",
                if path.is_empty() { "/" } else { &path },
                short(&e.inner().to_string())
            ),
        )
    })
}

fn parse_event_line(line: u64, raw: &[u8]) -> std::result::Result<Event, InvalidExport> {
    if raw.len() > MAX_LINE_BYTES {
        return Err(InvalidExport::new(
            InvalidKind::TooLarge,
            format!(
                "Zeile {line}: länger als {} MiB",
                MAX_LINE_BYTES / 1024 / 1024
            ),
        ));
    }
    if raw.is_empty() {
        return Err(InvalidExport::at(
            InvalidKind::Invalid,
            line,
            "",
            "leere Zeile",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(raw)
        .map_err(|_| InvalidExport::at(InvalidKind::Invalid, line, "", "kein gültiges JSON"))?;
    typed(line, &value)
}

/// JSON-Pointer aus einem serde-Pfad, z. B. `payload/text`.
fn pointer(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    path.iter()
        .map(|s| match s {
            Segment::Seq { index } => index.to_string(),
            Segment::Map { key } => key.replace('~', "~0").replace('/', "~1"),
            Segment::Enum { variant } => variant.clone(),
            Segment::Unknown => "?".into(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Fehlermeldungen von serde können Werte aus der Datei zitieren; gekürzt auf eine Zeile.
fn short(s: &str) -> String {
    let first = s.lines().next().unwrap_or_default();
    let mut out: String = first.chars().take(160).collect();
    if first.chars().count() > 160 {
        out.push('…');
    }
    out
}

impl Store {
    /// Exportiert eine Session (DATA-010): Kopf plus dauerhafte Events bis zum aktuellen
    /// `head_seq`. Payloads sind die gespeicherten (redigierten); `raw` nur mit `with_raw`,
    /// dabei noch einmal durch den Redaction-Hook.
    pub async fn export_session(
        &self,
        org: OrgId,
        session: SessionId,
        opts: ExportOptions,
    ) -> Result<ExportFile> {
        let record = self.session(org, session).await?;
        let head = record.head_seq;
        let imported_from = self
            .file_import(org, session)
            .await?
            .map(|f| f.imported_from);
        let mut lines = Vec::with_capacity(usize::try_from(head).unwrap_or(0));
        let mut after = 0u64;
        while after < head {
            let rows = sqlx::query(
                "SELECT e.seq, e.id, e.ts, e.actor, e.type, e.payload, e.payload_ref, e.turn_id, \
                 e.causation_id, r.raw FROM events e LEFT JOIN event_raw r \
                 ON r.org_id = e.org_id AND r.session_id = e.session_id AND r.seq = e.seq \
                 WHERE e.org_id = ? AND e.session_id = ? AND e.seq > ? AND e.seq <= ? \
                 ORDER BY e.seq LIMIT 1000",
            )
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(to_i64(after)?)
            .bind(to_i64(head)?)
            .fetch_all(&self.pool)
            .await?;
            let Some(last) = rows.last() else { break };
            after = crate::sessions::to_u64(last.try_get("seq")?)?;
            for row in &rows {
                let mut event = event_from_row(session, row)?;
                if let EventBody::Offloaded(o) = &event.body
                    && !opts.with_blobs
                {
                    let bytes = self.blobs.read(&o.payload_ref)?;
                    let payload =
                        payload_from_json(o.event_type, &String::from_utf8_lossy(&bytes))?;
                    event.body = EventBody::Inline(payload);
                }
                if opts.with_raw
                    && let Some(raw) = row.try_get::<Option<String>, _>("raw")?
                {
                    let raw = RawJson::from_string(raw).map_err(Error::corrupt)?;
                    let redacted = self.redact_raw(&raw)?;
                    event.raw = Some(RawJson::from_string(redacted).map_err(Error::corrupt)?);
                }
                lines.push(serde_json::to_string(&event)?);
            }
        }
        let blobs: Vec<BlobRef> = if opts.with_blobs {
            sqlx::query_scalar::<_, String>(
                "SELECT sha256 FROM blob_refs WHERE org_id = ? AND session_id = ? ORDER BY sha256",
            )
            .bind(org.to_string())
            .bind(session.to_string())
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|h| BlobRef::from_hex(h).map_err(Error::corrupt))
            .collect::<Result<_>>()?
        } else {
            Vec::new()
        };
        let exported_at = Timestamp::now();
        let header = ExportHeader {
            kind: EXPORT_TYPE.to_owned(),
            format_version: FORMAT_VERSION,
            exported_at,
            beton_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            session: ExportSession {
                id: session,
                title: record.title,
                harness: record.harness,
                status: record.status,
                archived: record.archived,
                created_at: record.created_at,
                events: lines.len() as u64,
                raw: opts.with_raw,
                blobs: blobs.clone(),
                imported_from,
            },
        };
        let mut jsonl = serde_json::to_string(&header)?.into_bytes();
        jsonl.push(b'\n');
        for line in &lines {
            jsonl.extend_from_slice(line.as_bytes());
            jsonl.push(b'\n');
        }
        let events = lines.len() as u64;
        if !opts.with_blobs {
            return Ok(ExportFile {
                bytes: jsonl,
                events,
                blobs: 0,
                archive: false,
            });
        }
        let mut contents = Vec::with_capacity(blobs.len());
        for blob in &blobs {
            contents.push((blob.clone(), self.blobs.read(blob)?));
        }
        let mtime = u64::try_from(exported_at.as_offset_date_time().unix_timestamp()).unwrap_or(0);
        let bytes = pack(&jsonl, &contents, mtime)?;
        Ok(ExportFile {
            bytes,
            events,
            blobs: contents.len() as u64,
            archive: true,
        })
    }

    /// Legt aus einer geprüften Importdatei eine neue Session an (DATA-010). Ohne `force`
    /// liefert eine schon importierte Datei (gleicher Hash) die bestehende Session.
    pub async fn import_export(
        &self,
        org: OrgId,
        owner: UserId,
        home_node: NodeId,
        parsed: ParsedExport,
        force: bool,
    ) -> Result<FileImportOutcome> {
        let imported_from = ImportedFrom {
            session_id: parsed.header.session.id,
            exported_at: parsed.header.exported_at,
            export_sha256: parsed.sha256.clone(),
        };
        let events_count = parsed.events.len() as u64;
        let blobs_count = parsed.header.session.blobs.len() as u64;
        if !force && let Some(existing) = self.imported_by_hash(org, &parsed.sha256).await? {
            let record = self.session(org, existing).await?;
            return Ok(FileImportOutcome {
                status: FileImportStatus::Skipped,
                session_id: existing,
                title: record.title,
                events: record.head_seq,
                blobs: blobs_count,
                imported_from,
            });
        }
        let known: HashSet<String> =
            sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE org_id = ?")
                .bind(org.to_string())
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .collect();
        let id = SessionId::new();
        let ids: HashMap<EventId, EventId> = parsed
            .events
            .iter()
            .map(|e| (e.id, EventId::new()))
            .collect();
        let Some(EventPayload::SessionCreated(created)) =
            parsed.events.first().and_then(Event::payload).cloned()
        else {
            return Err(Error::InvalidEvent("session.created fehlt".into()));
        };
        let created_at = parsed.events.first().map(|e| e.ts).unwrap_or_default();
        let events: Vec<Event> = parsed
            .events
            .into_iter()
            .map(|mut e| {
                e.id = ids.get(&e.id).copied().unwrap_or_else(EventId::new);
                e.session_id = id;
                e.causation_id = e.causation_id.map(|c| ids.get(&c).copied().unwrap_or(c));
                e.actor = map_actor(e.actor, &known);
                e
            })
            .collect();
        for bytes in parsed.blobs.values() {
            self.blobs.put(bytes)?;
        }
        let prepared = self.prepare(id, events)?;
        let now = Timestamp::now();
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "INSERT INTO sessions (id, org_id, owner_id, project_id, parent_id, kind, harness, \
             home_node_id, epoch, head_seq, created_at, updated_at, title, status, archived, \
             cost_micro, last_activity_at) \
             VALUES (?, ?, ?, NULL, NULL, ?, ?, ?, 1, 0, ?, ?, '', ?, 0, 0, ?)",
        )
        .bind(id.to_string())
        .bind(org.to_string())
        .bind(owner.to_string())
        .bind(enum_to_str(&SessionKind::Main))
        .bind(&created.harness)
        .bind(home_node.to_string())
        .bind(created_at.to_string())
        .bind(now.to_string())
        .bind(enum_to_str(&SessionStatus::default()))
        .bind(created_at.to_string())
        .execute(&mut *tx)
        .await?;
        self.write_prepared(&mut tx, org, id, 0, 1, prepared)
            .await?;
        for (blob, bytes) in &parsed.blobs {
            reference_blob(&mut tx, org, id, blob, bytes.len() as u64).await?;
        }
        sqlx::query(
            "INSERT INTO session_file_imports (session_id, org_id, export_sha256, \
             source_session_id, exported_at, imported_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(org.to_string())
        .bind(&imported_from.export_sha256)
        .bind(imported_from.session_id.to_string())
        .bind(imported_from.exported_at.to_string())
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;
        seal_imported(&mut tx, org, id).await?;
        tx.commit().await?;
        let record = self.session(org, id).await?;
        Ok(FileImportOutcome {
            status: FileImportStatus::Imported,
            session_id: id,
            title: record.title,
            events: events_count,
            blobs: blobs_count,
            imported_from,
        })
    }

    /// Herkunft einer aus einer Exportdatei importierten Session; `None` für alle anderen.
    pub async fn file_import(
        &self,
        org: OrgId,
        session: SessionId,
    ) -> Result<Option<FileImportRecord>> {
        let row = sqlx::query(
            "SELECT export_sha256, source_session_id, exported_at, imported_at \
             FROM session_file_imports WHERE org_id = ? AND session_id = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(FileImportRecord {
                imported_from: ImportedFrom {
                    session_id: parse(r.try_get("source_session_id")?)?,
                    exported_at: parse(r.try_get("exported_at")?)?,
                    export_sha256: r.try_get("export_sha256")?,
                },
                imported_at: parse(r.try_get("imported_at")?)?,
            })
        })
        .transpose()
    }

    /// Jüngste Session, die aus einer Datei mit diesem Hash importiert wurde.
    async fn imported_by_hash(&self, org: OrgId, sha256: &str) -> Result<Option<SessionId>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT session_id FROM session_file_imports WHERE org_id = ? AND export_sha256 = ? \
             ORDER BY imported_at DESC, session_id DESC LIMIT 1",
        )
        .bind(org.to_string())
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await?;
        id.as_deref().map(parse).transpose()
    }
}

/// Unbekannte User (andere Instanz oder Org) werden zu `system/import` (DATA-010).
fn map_actor(actor: Actor, known: &HashSet<String>) -> Actor {
    match actor {
        Actor::User {
            id: PrincipalId::User(user),
            ..
        } if known.contains(&user.to_string()) => actor,
        Actor::User { .. } => Actor::System {
            component: SystemComponent::Import,
        },
        other => other,
    }
}

/// Eine importierte Session läuft auf diesem Host nicht: kein Worktree (der Pfad stammt aus
/// der Datei und könnte beim Löschen entfernt werden), Status `stopped`, keine offenen
/// Freigaben und keine Usage-Aggregate (die Kosten entstanden in der Quelle). Läuft beim
/// Import und nach jedem Projektions-Rebuild.
pub(crate) async fn seal_imported(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
) -> Result<()> {
    let (org, session) = (org.to_string(), session.to_string());
    sqlx::query(
        "UPDATE sessions SET worktree_path = NULL, worktree_branch = NULL, worktree_base = NULL, \
         worktree_base_sha = NULL, status = CASE WHEN status IN (?, ?) THEN status ELSE ? END \
         WHERE org_id = ? AND id = ?",
    )
    .bind(enum_to_str(&SessionStatus::Stopped))
    .bind(enum_to_str(&SessionStatus::Failed))
    .bind(enum_to_str(&SessionStatus::Stopped))
    .bind(&org)
    .bind(&session)
    .execute(&mut *conn)
    .await?;
    for sql in [
        "UPDATE approvals SET status = 'expired' WHERE org_id = ? AND session_id = ? \
         AND status = 'open'",
        "DELETE FROM usage_daily WHERE org_id = ? AND session_id = ?",
    ] {
        sqlx::query(sql)
            .bind(&org)
            .bind(&session)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Ist die Session aus einer Exportdatei importiert?
pub(crate) async fn is_file_import(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: &str,
) -> Result<bool> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM session_file_imports WHERE org_id = ? AND session_id = ?",
    )
    .bind(org.to_string())
    .bind(session)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

/// Packt `session.jsonl` und Blobs deterministisch als `.tar.zst`.
fn pack(jsonl: &[u8], blobs: &[(BlobRef, Vec<u8>)], mtime: u64) -> Result<Vec<u8>> {
    let encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
    let mut tar = tar::Builder::new(encoder);
    let mut append = |name: &str, data: &[u8]| -> Result<()> {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_mtime(mtime);
        header.set_cksum();
        tar.append_data(&mut header, name, data)?;
        Ok(())
    };
    append(JSONL_NAME, jsonl)?;
    for (blob, data) in blobs {
        append(&format!("blobs/{}", blob.hex()), data)?;
    }
    let encoder = tar.into_inner()?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
pub(crate) fn pack_for_tests(jsonl: &[u8], entries: &[(&str, &[u8], tar::EntryType)]) -> Vec<u8> {
    let encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    let mut tar = tar::Builder::new(encoder);
    let mut all: Vec<(&str, &[u8], tar::EntryType)> =
        vec![(JSONL_NAME, jsonl, tar::EntryType::Regular)];
    all.extend_from_slice(entries);
    for (name, data, kind) in all {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(if kind.is_file() { data.len() as u64 } else { 0 });
        header.set_mode(0o600);
        if kind.is_symlink() || kind.is_hard_link() {
            header.set_link_name("/etc/passwd").unwrap();
        }
        // Absichtlich ohne die Prüfungen von `append_data`, damit auch böse Namen entstehen.
        let name_bytes = name.as_bytes();
        let gnu = header.as_gnu_mut().unwrap();
        gnu.name[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_cksum();
        let body: &[u8] = if kind.is_file() { data } else { &[] };
        tar.append(&header, body).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
