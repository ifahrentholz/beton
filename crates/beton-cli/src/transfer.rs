//! `beton import` und `beton export` (CLI-007): CLI-Front für den Import fremder Chats
//! (SES-008) und den Session-Export/-Import als Datei (SES-009, Format DATA-010).
//!
//! Gelesen und geschrieben werden nur lokale Dateien, die der User nennt (ADR-0033); die
//! Vendor-Verzeichnisse liest allein der Daemon über seine Pfad-Allowlist. Konvention
//! (CLI-001): Ergebniszeilen auf stdout, Hinweise und Fortschritt auf stderr.

use std::collections::{BTreeMap, HashMap};
use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::{Value, json};

use crate::cli::{ExportArgs, ImportArgs, ImportHarness};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

/// Höchstzahl der Vendor-Sessions je `POST /v1/imports` (wie der Server).
const IMPORT_CHUNK: usize = 500;

/// `beton export <SESSION> [-o FILE] [--with-blobs] [--with-raw]`.
pub async fn export(ctx: &Ctx, args: ExportArgs) -> CliResult {
    if args.output.is_none() && args.with_blobs && std::io::stdout().is_terminal() {
        return Err(CliError::usage(
            "Ein .tar.zst gehört nicht ins Terminal: Zieldatei mit -o angeben",
        ));
    }
    let client = ctx.client()?;
    let id = crate::sessionref::resolve(&client, &args.session).await?;
    let file = client
        .export_session(&id, args.with_raw, args.with_blobs)
        .await?;
    let Some(path) = args.output else {
        let mut out = std::io::stdout().lock();
        out.write_all(&file.bytes).context("stdout")?;
        out.flush().context("stdout")?;
        return Ok(());
    };
    write_private(&path, &file.bytes).with_context(|| format!("{} schreiben", path.display()))?;
    if ctx.global.json {
        return print_json(&json!({
            "file": path.display().to_string(),
            "session_id": id,
            "events": file.events,
            "blobs": file.blobs,
            "bytes": file.bytes.len(),
            "archive": file.archive,
        }));
    }
    let mut line = format!("✓ {} · {} Events", path.display(), de_int(file.events));
    if file.archive {
        line.push_str(&format!(
            " · {} {}",
            de_int(file.blobs),
            if file.blobs == 1 {
                "Anhang"
            } else {
                "Anhänge"
            }
        ));
    }
    line.push_str(&format!(" · {}", human_bytes(file.bytes.len() as u64)));
    ctx.note(line);
    Ok(())
}

/// Schreibt eine Exportdatei nur für den Eigentümer lesbar (0600) und erst vollständig an
/// ihren Platz (temporäre Datei, dann umbenennen).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    let tmp = path.with_file_name(format!(".{name}.part"));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `beton import …`: Datei, `--session`, `--last` oder interaktive Auswahl.
pub async fn import(ctx: &Ctx, args: ImportArgs) -> CliResult {
    if let Some(file) = &args.file {
        return import_file(ctx, file, args.force).await;
    }
    let client = ctx.client()?;
    let harnesses: Vec<&str> = match args.harness.unwrap_or(ImportHarness::All) {
        ImportHarness::Claude => vec!["claude"],
        ImportHarness::Codex => vec!["codex"],
        ImportHarness::All => vec!["claude", "codex"],
    };
    let single = harnesses.len() == 1;
    let candidates = candidates(ctx, &client, &harnesses, single).await?;
    let chosen: Vec<Value> = if let Some(n) = args.last {
        candidates.iter().take(n as usize).cloned().collect()
    } else if !args.session.is_empty() {
        let mut chosen = Vec::new();
        for wanted in &args.session {
            match candidates
                .iter()
                .find(|c| c["vendor_session_id"].as_str() == Some(wanted.as_str()))
            {
                Some(c) => chosen.push(c.clone()),
                None => {
                    return Err(CliError::new(
                        Exit::General,
                        anyhow::anyhow!(
                            "Keinen Chat `{wanted}` gefunden. Kandidaten zeigt: beton import"
                        ),
                    ));
                }
            }
        }
        chosen
    } else {
        match pick(ctx, &candidates)? {
            Some(chosen) => chosen,
            None => {
                return Err(CliError::new(
                    Exit::Interrupted,
                    anyhow::anyhow!("Abgebrochen, nichts importiert"),
                ));
            }
        }
    };
    if chosen.is_empty() {
        ctx.note("Keine Chats gefunden, nichts importiert.");
        return Ok(());
    }
    // Je Harness importieren; die Reihenfolge der Auswahl bleibt erhalten.
    let mut refs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &chosen {
        refs.entry(text(c, "harness"))
            .or_default()
            .push(text(c, "vendor_session_id"));
    }
    let mut by_ref: HashMap<(String, String), Value> = HashMap::new();
    for (harness, ids) in &refs {
        for chunk in ids.chunks(IMPORT_CHUNK) {
            let res = client
                .import_sessions(&json!({
                    "harness": harness,
                    "refs": chunk,
                    "force": args.force,
                }))
                .await?;
            for r in res["results"].as_array().into_iter().flatten() {
                by_ref.insert((harness.clone(), text(r, "vendor_session_id")), r.clone());
            }
        }
    }
    let results: Vec<(Value, Value)> = chosen
        .iter()
        .filter_map(|c| {
            by_ref
                .get(&(text(c, "harness"), text(c, "vendor_session_id")))
                .map(|r| (c.clone(), r.clone()))
        })
        .collect();
    report(ctx, &results)
}

/// Kandidaten der gewählten Harnesses, zuletzt geänderte zuerst.
async fn candidates(
    ctx: &Ctx,
    client: &beton_sdk::Client,
    harnesses: &[&str],
    strict: bool,
) -> CliResult<Vec<Value>> {
    let mut all = Vec::new();
    for harness in harnesses {
        let mut cursor: Option<String> = None;
        loop {
            let page = match client
                .import_candidates(harness, Some(200), cursor.as_deref())
                .await
            {
                Ok(p) => p,
                // Ohne `--harness` fehlt ein Werkzeug nur als Hinweis.
                Err(beton_sdk::Error::Problem { code, detail, .. })
                    if !strict
                        && matches!(
                            code.as_str(),
                            "capability_unsupported" | "unavailable" | "validation_failed"
                        ) =>
                {
                    ctx.note(format!(
                        "{} übersprungen: {}",
                        beton_harness::handover::harness_label(harness),
                        detail.unwrap_or(code)
                    ));
                    break;
                }
                Err(e) => return Err(e.into()),
            };
            all.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
    }
    // RFC 3339 in UTC lässt sich als Text sortieren; ohne Zeitpunkt ans Ende.
    all.sort_by_key(|c| std::cmp::Reverse(text(c, "updated_at")));
    Ok(all)
}

/// Interaktive Auswahl; ohne Terminal die Liste und Exit-Code 2 (CLI-001).
fn pick(ctx: &Ctx, candidates: &[Value]) -> CliResult<Option<Vec<Value>>> {
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        if ctx.global.json {
            print_json(&Value::Array(candidates.to_vec()))?;
        } else {
            let mut out = std::io::stdout().lock();
            for c in candidates {
                writeln!(
                    out,
                    "{}  {}  {}",
                    text(c, "harness"),
                    text(c, "vendor_session_id"),
                    text(c, "title")
                )
                .context("stdout")?;
            }
        }
        return Err(CliError::usage(
            "Die Auswahl braucht ein Terminal. Ohne Terminal: beton import --last N, \
             --session REF oder eine Exportdatei",
        ));
    }
    if candidates.is_empty() {
        ctx.note("Keine Chats von Claude Code oder Codex gefunden.");
        return Ok(Some(Vec::new()));
    }
    ctx.note(
        "Gefundene Chats anderer Werkzeuge (nur lokal gelesen, Originale bleiben unverändert):",
    );
    let now = time::OffsetDateTime::now_utc();
    let items = candidates
        .iter()
        .map(|c| crate::picker::Item {
            harness: beton_harness::handover::harness_label(&text(c, "harness")),
            title: title_of(c),
            project: Path::new(&text(c, "cwd"))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            when: relative(&text(c, "updated_at"), now),
            size: human_bytes(c["size_bytes"].as_u64().unwrap_or(0)),
            imported: c.get("imported_session_id").is_some_and(|v| !v.is_null()),
        })
        .collect();
    let picker = crate::picker::Picker::new(items, 15);
    match crate::picker::run(picker, ctx.color()).context("Terminal")? {
        crate::picker::Outcome::Done(idx) => Ok(Some(
            idx.into_iter()
                .filter_map(|i| candidates.get(i).cloned())
                .collect(),
        )),
        _ => Ok(None),
    }
}

/// Ergebniszeilen: übernommene und übersprungene auf stdout, Fehler auf stderr.
fn report(ctx: &Ctx, results: &[(Value, Value)]) -> CliResult {
    if ctx.global.json {
        let list: Vec<Value> = results
            .iter()
            .map(|(c, r)| {
                let mut r = r.clone();
                r["harness"] = c["harness"].clone();
                r
            })
            .collect();
        print_json(&json!({ "results": list }))?;
    } else {
        let mut out = std::io::stdout().lock();
        for (c, r) in results {
            let id = text(r, "session_id");
            match r["status"].as_str() {
                Some("imported") => {
                    writeln!(out, "✓ {id}  {}", title_of(c)).context("stdout")?;
                }
                Some("skipped") => {
                    writeln!(out, "– {id}  {} (schon übernommen)", title_of(c))
                        .context("stdout")?;
                }
                _ => eprintln!(
                    "✗ {}  {}: {}",
                    text(r, "vendor_session_id"),
                    text(r, "reason"),
                    text(r, "detail")
                ),
            }
        }
    }
    let imported: Vec<&(Value, Value)> = results
        .iter()
        .filter(|(_, r)| r["status"] == "imported")
        .collect();
    let skipped = results
        .iter()
        .filter(|(_, r)| r["status"] == "skipped")
        .count();
    let failed = results.len() - imported.len() - skipped;
    if !ctx.global.json {
        let mut sources: Vec<String> = imported
            .iter()
            .map(|(c, _)| beton_harness::handover::harness_label(&text(c, "harness")))
            .collect();
        sources.dedup();
        let mut note = format!(
            "{} {} importiert",
            imported.len(),
            if imported.len() == 1 {
                "Session"
            } else {
                "Sessions"
            }
        );
        if !sources.is_empty() {
            note.push_str(&format!(" aus {}", sources.join(" und ")));
        }
        if skipped > 0 {
            note.push_str(&format!(", {skipped} schon übernommen"));
        }
        if failed > 0 {
            note.push_str(&format!(", {failed} fehlgeschlagen"));
        }
        note.push('.');
        if let Some((_, r)) = imported.first() {
            note.push_str(&format!(
                " Fortsetzen z. B. mit: beton resume {}",
                text(r, "session_id")
            ));
        }
        ctx.note(note);
    }
    if failed > 0 {
        return Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("{failed} Chats ließen sich nicht importieren"),
        ));
    }
    Ok(())
}

/// `beton import FILE`: Exportdatei als neue Session (SES-009).
async fn import_file(ctx: &Ctx, path: &PathBuf, force: bool) -> CliResult {
    let meta = std::fs::metadata(path).with_context(|| format!("{} lesen", path.display()))?;
    let max = beton_store::export::MAX_IMPORT_BYTES as u64;
    if !meta.is_file() {
        return Err(CliError::usage(format!(
            "{} ist keine Datei",
            path.display()
        )));
    }
    if meta.len() > max {
        return Err(CliError::new(
            Exit::General,
            anyhow::anyhow!(
                "{} ist größer als {} MiB",
                path.display(),
                max / 1024 / 1024
            ),
        ));
    }
    let bytes = std::fs::read(path).with_context(|| format!("{} lesen", path.display()))?;
    let client = ctx.client()?;
    let res = client.import_file(bytes, force).await?;
    if ctx.global.json {
        return print_json(&res);
    }
    let id = text(&res, "session_id");
    let title = text(&res, "title");
    let mut out = std::io::stdout().lock();
    if res["status"] == "skipped" {
        writeln!(out, "– {id}  {title} (schon importiert)").context("stdout")?;
        drop(out);
        ctx.note("Diese Datei wurde schon importiert. Als weitere Session: --force");
    } else {
        writeln!(out, "✓ {id}  {title} (inhaltsgleich, neue ID)").context("stdout")?;
        drop(out);
        ctx.note(
            "Die importierte Session ist nur lesbar: Ihr Arbeitsverzeichnis stammt aus der Datei.",
        );
    }
    Ok(())
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn title_of(c: &Value) -> String {
    let t = text(c, "title");
    if t.trim().is_empty() {
        "(ohne Titel)".into()
    } else {
        t
    }
}

/// `1284` → `1.284`.
pub fn de_int(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// `2_202_009` → `2,1 MB`.
pub fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f < KB {
        format!("{n} B")
    } else if f < KB * KB {
        format!("{:.0} KB", f / KB)
    } else if f < KB * KB * KB {
        format!("{:.1} MB", f / KB / KB).replace('.', ",")
    } else {
        format!("{:.1} GB", f / KB / KB / KB).replace('.', ",")
    }
}

/// `vor 2 Tg.` relativ zu `now`; ohne lesbaren Zeitpunkt leer.
pub fn relative(rfc3339: &str, now: time::OffsetDateTime) -> String {
    let Ok(t) =
        time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
    else {
        return String::new();
    };
    let secs = (now - t).whole_seconds().max(0);
    match secs {
        0..60 => "gerade eben".into(),
        60..3_600 => format!("vor {} Min.", secs / 60),
        3_600..86_400 => format!("vor {} Std.", secs / 3_600),
        86_400..604_800 => format!("vor {} Tg.", secs / 86_400),
        604_800..2_592_000 => format!("vor {} Wo.", secs / 604_800),
        _ => format!("vor {} Mon.", secs / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_007_numbers_and_sizes_in_german() {
        assert_eq!(de_int(0), "0");
        assert_eq!(de_int(1284), "1.284");
        assert_eq!(de_int(1_234_567), "1.234.567");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(84 * 1024), "84 KB");
        assert_eq!(human_bytes(2_202_009), "2,1 MB");
    }

    #[test]
    fn cli_007_relative_times() {
        let now = time::macros::datetime!(2026-10-04 12:00 UTC);
        assert_eq!(relative("2026-10-04T11:59:30Z", now), "gerade eben");
        assert_eq!(relative("2026-10-04T11:15:00Z", now), "vor 45 Min.");
        assert_eq!(relative("2026-10-02T12:00:00Z", now), "vor 2 Tg.");
        assert_eq!(relative("2026-09-24T12:00:00Z", now), "vor 1 Wo.");
        assert_eq!(relative("kaputt", now), "");
    }

    #[test]
    fn cli_007_export_file_is_private_and_complete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        write_private(&path, b"{}\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!dir.path().join(".s.jsonl.part").exists());
    }
}
