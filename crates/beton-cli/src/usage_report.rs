//! `beton usage` (USE-007, CLI-010): Verbrauch nach Tag, Session, Projekt, Harness oder Modell,
//! je Turn einer Session, als Tabelle, `--json` oder `--csv`.
//!
//! - Läuft ein Daemon (bzw. mit `--server`), kommen die Zahlen aus `GET /v1/usage` bzw.
//!   `GET /v1/sessions/{id}/usage`. Sonst liest der Befehl die lokale Datenbank nur lesend,
//!   ohne den Daemon zu starten (USE-007 AC3), mit denselben Abfragen
//!   ([`beton_server::api_usage`]); beide Wege liefern dieselben Summen.
//! - Subscriptions (Login der Vendor-CLI) zeigen Tokens und Abo-Fenster, nie einen
//!   Geldbetrag; Nutzung nach Verbrauch (API-Key, Gateway, lokal) zeigt Kosten. Beides steht
//!   getrennt, Mischsummen gibt es nicht (CLI-010 AC2). Harnesses ohne Meldung erscheinen als
//!   „nicht gemeldet“ statt 0 (USE-004 AC4).
//! - stdout enthält nur Daten (Tabellen, JSON, CSV); Hinweise gehen nach stderr (CLI-001).
//! - Zeiten und Tagesgrenzen gelten in der lokalen Zeitzone (fester Versatz beim Start).

use std::io::Write as _;
use std::sync::OnceLock;

use beton_core::id::{OrgId, SessionId};
use beton_server::api_usage::{
    SessionUsageView, SubscriptionWindowView, UsageItem, UsageParams, UsageReportView,
    format_offset,
};
use beton_store::{Store, UsageReport};
use time::{Date, OffsetDateTime, Time, UtcOffset};

use crate::cli::UsageReportArgs;
use crate::commands::Ctx;
use crate::exit::{CliError, CliResult, Exit};

static LOCAL_OFFSET: OnceLock<UtcOffset> = OnceLock::new();

/// Liest den Versatz der lokalen Zeitzone. Muss vor weiteren Threads laufen (`main`), weil
/// `time` ihn sonst auf Unix nicht sicher bestimmen kann; ohne Ergebnis gilt UTC.
pub fn init_local_offset() {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let _ = LOCAL_OFFSET.set(offset);
}

fn local_offset() -> UtcOffset {
    *LOCAL_OFFSET.get_or_init(|| UtcOffset::UTC)
}

/// Woher die Zahlen kommen.
enum Source {
    Api(beton_sdk::Client),
    /// Lokale Datenbank, nur lesend; `None`, wenn es noch keine gibt.
    Local(Option<Store>),
}

async fn source(ctx: &Ctx) -> CliResult<Source> {
    if ctx.global.server.is_some() {
        return Ok(Source::Api(ctx.client()?));
    }
    if let Some(client) = crate::daemon::running(&ctx.home).await {
        return Ok(Source::Api(client));
    }
    ctx.note("Kein Daemon aktiv: lese die lokale Datenbank (nur lesend).");
    let store = Store::open_read_only(&ctx.home)
        .await
        .map_err(|e| CliError::new(Exit::General, e))?;
    Ok(Source::Local(store))
}

/// Dauer wie `30m`, `12h`, `7d`, `2w`.
fn duration(raw: &str) -> Option<time::Duration> {
    let raw = raw.trim();
    let (n, unit) = raw.split_at(raw.len().checked_sub(1)?);
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    match unit {
        "m" => Some(time::Duration::minutes(n)),
        "h" => Some(time::Duration::hours(n)),
        "d" => Some(time::Duration::days(n)),
        "w" => Some(time::Duration::weeks(n)),
        _ => None,
    }
}

fn parse_date(raw: &str) -> Option<Date> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    Date::parse(raw.trim(), &format).ok()
}

fn ts(t: OffsetDateTime) -> String {
    beton_core::time::Timestamp::from(t).to_string()
}

/// Parameter für `GET /v1/usage` aus den Flags (Zeitraum in der Zeitzone `offset`).
pub fn params(
    args: &UsageReportArgs,
    now: OffsetDateTime,
    offset: UtcOffset,
) -> CliResult<UsageParams> {
    let mut p = UsageParams {
        tz: Some(format_offset(offset)),
        ..UsageParams::default()
    };
    if let Some(since) = &args.since {
        let from = if let Some(d) = duration(since) {
            now - d
        } else if let Some(d) = parse_date(since) {
            d.with_time(Time::MIDNIGHT).assume_offset(offset)
        } else {
            return Err(CliError::usage(format!(
                "--since `{since}` ist ungültig: Dauer wie 7d, 12h, 30m oder Datum JJJJ-MM-TT"
            )));
        };
        p.from = Some(ts(from));
        p.to = Some(ts(now));
    }
    for (name, value, target) in [
        ("--from", &args.from, &mut p.from),
        ("--to", &args.to, &mut p.to),
    ] {
        if let Some(v) = value {
            if parse_date(v).is_none() {
                return Err(CliError::usage(format!(
                    "{name} `{v}` ist ungültig: Datum JJJJ-MM-TT"
                )));
            }
            *target = Some(v.trim().to_owned());
        }
    }
    if args.from.is_some() && args.to.is_none() {
        p.to = Some(ts(now));
    }
    if !args.by.is_empty() {
        let dims: Vec<&str> = args
            .by
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        p.group_by = Some(dims.join(","));
    }
    // Vorab prüfen: dieselben Regeln wie der Server, aber als Usage-Fehler (Exit 2).
    p.to_query(now).map_err(CliError::usage)?;
    Ok(p)
}

fn query_pairs(p: &UsageParams) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (k, v) in [
        ("from", &p.from),
        ("to", &p.to),
        ("tz", &p.tz),
        ("group_by", &p.group_by),
        ("project", &p.project),
        ("harness", &p.harness),
        ("user", &p.user),
        ("team", &p.team),
        ("session_id", &p.session_id),
    ] {
        if let Some(v) = v {
            out.push((k, v.clone()));
        }
    }
    out
}

fn decode<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> CliResult<T> {
    serde_json::from_value(v).map_err(|e| {
        CliError::new(
            Exit::General,
            anyhow::anyhow!("Antwort des Servers nicht lesbar: {e}"),
        )
    })
}

async fn report(
    source: &Source,
    p: &UsageParams,
    now: OffsetDateTime,
) -> CliResult<UsageReportView> {
    match source {
        Source::Api(client) => decode(client.usage(&query_pairs(p)).await?),
        Source::Local(store) => {
            let q = p.to_query(now).map_err(CliError::usage)?;
            let report = match store {
                Some(store) => store
                    .usage_report(OrgId::LOCAL, &q)
                    .await
                    .map_err(|e| CliError::new(Exit::General, e))?,
                None => UsageReport::default(),
            };
            Ok(UsageReportView::new(&q, &report, now))
        }
    }
}

async fn session_usage(source: &Source, reference: &str) -> CliResult<SessionUsageView> {
    match source {
        Source::Api(client) => {
            let id = crate::sessionref::resolve(client, reference).await?;
            decode(client.session_usage(&id).await?)
        }
        Source::Local(store) => {
            let id: SessionId = reference.trim().parse().map_err(|_| {
                CliError::usage(format!(
                    "Ohne Daemon braucht --session die vollständige Session-ID (ses_…), nicht `{reference}`"
                ))
            })?;
            let not_found = || {
                CliError::new(
                    Exit::General,
                    anyhow::anyhow!("Session {id} nicht gefunden"),
                )
            };
            let Some(store) = store else {
                return Err(not_found());
            };
            store
                .session(OrgId::LOCAL, id)
                .await
                .map_err(|_| not_found())?;
            let records = store
                .usage_records(OrgId::LOCAL, id)
                .await
                .map_err(|e| CliError::new(Exit::General, e))?;
            Ok(SessionUsageView::new(id, &records))
        }
    }
}

pub async fn run(ctx: &Ctx, args: &UsageReportArgs) -> CliResult {
    if args.csv && ctx.global.json {
        return Err(CliError::usage("--json und --csv schließen sich aus"));
    }
    let now = OffsetDateTime::now_utc();
    let offset = local_offset();
    // Flags prüfen, bevor etwas geöffnet wird.
    let default_view = args.by.is_empty()
        && args.since.is_none()
        && args.from.is_none()
        && args.to.is_none()
        && args.session.is_none();
    let p = if default_view {
        None
    } else if args.session.is_some() {
        Some(UsageParams::default())
    } else {
        Some(params(args, now, offset)?)
    };
    let source = source(ctx).await?;
    let style = Output::from(ctx, args);
    let mut out = std::io::stdout().lock();
    let io = |e: std::io::Error| CliError::new(Exit::General, e);
    if let Some(reference) = &args.session {
        let usage = session_usage(&source, reference).await?;
        close(source).await;
        match style {
            Output::Json => crate::commands::print_json(
                &serde_json::to_value(&usage).map_err(|e| CliError::new(Exit::General, e))?,
            )?,
            Output::Csv => session_csv(&usage, &mut out).map_err(io)?,
            Output::Table => {
                session_table(&usage, offset, &mut out).map_err(io)?;
                notes_session(ctx, &usage);
            }
        }
        return Ok(());
    }
    let Some(p) = p else {
        // Ohne Angaben: heute und die letzten 7 Tage je Harness (USE-007).
        let today = UsageReportArgs {
            by: vec!["harness".into(), "auth_source".into()],
            since: Some(now.to_offset(offset).date().to_string()),
            ..UsageReportArgs::default()
        };
        let week = UsageReportArgs {
            by: vec!["harness".into(), "auth_source".into()],
            since: Some("7d".into()),
            ..UsageReportArgs::default()
        };
        let week_report = report(&source, &params(&week, now, offset)?, now).await?;
        if style != Output::Table {
            close(source).await;
            return emit(&week_report, style, &mut out);
        }
        let today_report = report(&source, &params(&today, now, offset)?, now).await?;
        close(source).await;
        let weekday = weekday(now.to_offset(offset).date());
        writeln!(
            out,
            "Heute ({weekday} {})",
            german_date(now.to_offset(offset).date())
        )
        .map_err(io)?;
        tables(&today_report, &mut out).map_err(io)?;
        writeln!(out).map_err(io)?;
        writeln!(out, "Letzte 7 Tage").map_err(io)?;
        tables(&week_report, &mut out).map_err(io)?;
        windows(&week_report, now, offset, &mut out).map_err(io)?;
        notes(ctx, &week_report);
        return Ok(());
    };
    let view = report(&source, &p, now).await?;
    close(source).await;
    if style == Output::Table {
        writeln!(out, "{}", period(&view, offset)).map_err(io)?;
        tables(&view, &mut out).map_err(io)?;
        windows(&view, now, offset, &mut out).map_err(io)?;
        notes(ctx, &view);
        Ok(())
    } else {
        emit(&view, style, &mut out)
    }
}

async fn close(source: Source) {
    if let Source::Local(Some(store)) = source {
        store.close().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    Table,
    Json,
    Csv,
}

impl Output {
    fn from(ctx: &Ctx, args: &UsageReportArgs) -> Self {
        if ctx.global.json {
            Self::Json
        } else if args.csv {
            Self::Csv
        } else {
            Self::Table
        }
    }
}

fn emit(view: &UsageReportView, style: Output, out: &mut dyn std::io::Write) -> CliResult {
    let io = |e: std::io::Error| CliError::new(Exit::General, e);
    match style {
        Output::Json => {
            serde_json::to_writer_pretty(&mut *out, view)
                .map_err(|e| CliError::new(Exit::General, e))?;
            writeln!(out).map_err(io)
        }
        Output::Csv => report_csv(view, out).map_err(io),
        Output::Table => tables(view, out).map_err(io),
    }
}

// ------------------------------------------------------------------------- Formatierung

/// Ganze Zahl mit Tausenderpunkten: `1.234.567`.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Betrag wie in der UI: `1,75 $`, `0,075 $`.
pub fn money(usd: f64) -> String {
    let micro = (usd * 1_000_000.0).round() as i64;
    let s = format!("{:.6}", micro as f64 / 1_000_000.0);
    let s = s.trim_end_matches('0');
    let s = match s.split_once('.') {
        Some((int, frac)) if frac.len() < 2 => format!("{int}.{frac:0<2}"),
        _ => s.to_owned(),
    };
    format!("{} $", s.replace('.', ","))
}

fn pct(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{} %", v.round() as i64)
    } else {
        format!("{} %", format!("{v:.1}").replace('.', ","))
    }
}

/// Fortschrittsbalken mit 10 Feldern (USE-004 AC2).
pub fn bar(used_pct: f64) -> String {
    let filled = ((used_pct.clamp(0.0, 100.0) / 10.0).round()) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
}

fn weekday(d: Date) -> &'static str {
    match d.weekday() {
        time::Weekday::Monday => "Mo",
        time::Weekday::Tuesday => "Di",
        time::Weekday::Wednesday => "Mi",
        time::Weekday::Thursday => "Do",
        time::Weekday::Friday => "Fr",
        time::Weekday::Saturday => "Sa",
        time::Weekday::Sunday => "So",
    }
}

fn german_date(d: Date) -> String {
    format!("{:02}.{:02}.{}", d.day(), u8::from(d.month()), d.year())
}

fn parse_ts(raw: &str) -> Option<OffsetDateTime> {
    raw.parse::<beton_core::time::Timestamp>()
        .ok()
        .map(|t| t.as_offset_date_time())
}

/// Reset-Zeitpunkt in der Zeitzone des Users: am selben Tag `16:19`, sonst `Mo 06.10. 09:00`.
pub fn reset_label(resets_at: &str, now: OffsetDateTime, offset: UtcOffset) -> String {
    let Some(t) = parse_ts(resets_at) else {
        return resets_at.to_owned();
    };
    let local = t.to_offset(offset);
    let hm = format!("{:02}:{:02}", local.hour(), local.minute());
    if local.date() == now.to_offset(offset).date() {
        hm
    } else {
        format!(
            "{} {:02}.{:02}. {hm}",
            weekday(local.date()),
            local.day(),
            u8::from(local.month())
        )
    }
}

fn period(view: &UsageReportView, offset: UtcOffset) -> String {
    let fmt = |raw: &str| {
        parse_ts(raw).map_or_else(
            || raw.to_owned(),
            |t| {
                let l = t.to_offset(offset);
                format!(
                    "{} {:02}:{:02}",
                    german_date(l.date()),
                    l.hour(),
                    l.minute()
                )
            },
        )
    };
    format!(
        "Zeitraum {} – {} ({})",
        fmt(&view.from),
        fmt(&view.to),
        view.tz
    )
}

fn auth_label(a: &str) -> &str {
    match a {
        "vendor_cli" => "Subscription",
        "api_key" => "API-Key",
        "gateway" => "Gateway",
        "none" => "lokal",
        other => other,
    }
}

/// Spalte einer Dimension: Überschrift und Wert je Eintrag.
type DimColumn = (&'static str, fn(&UsageItem) -> String);

/// Spalten der gruppierten Dimensionen.
fn dim_columns(view: &UsageReportView) -> Vec<DimColumn> {
    let mut cols: Vec<DimColumn> = Vec::new();
    for d in &view.group_by {
        match d.as_str() {
            "day" => cols.push(("TAG", |i| i.day.clone().unwrap_or_default())),
            "session" => cols.push(("SESSION", |i| {
                let id = i.session_id.clone().unwrap_or_default();
                match i.session_title.as_deref().filter(|t| !t.is_empty()) {
                    Some(t) => format!("{id} {}", crate::render::sanitize(t)),
                    None => id,
                }
            })),
            "project" => cols.push(("PROJEKT", |i| {
                i.project_id.clone().unwrap_or_else(|| "–".into())
            })),
            "harness" => cols.push(("HARNESS", |i| i.harness.clone().unwrap_or_default())),
            "model" => cols.push(("MODELL", |i| i.model.clone().unwrap_or_default())),
            "auth_source" => cols.push(("ANMELDUNG", |i| {
                auth_label(i.auth_source.as_deref().unwrap_or_default()).to_owned()
            })),
            _ => {}
        }
    }
    cols
}

/// Eine Tabelle mit links- (Text) und rechtsbündigen (Zahlen) Spalten.
fn table(
    header: &[&str],
    right_from: usize,
    rows: &[Vec<String>],
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    let widths: Vec<usize> = (0..header.len())
        .map(|c| {
            rows.iter()
                .map(|r| r[c].chars().count())
                .chain([header[c].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<&str>| -> String {
        let mut s = String::new();
        for (c, cell) in cells.iter().enumerate() {
            if c > 0 {
                s.push_str("  ");
            }
            let pad = widths[c].saturating_sub(cell.chars().count());
            if c >= right_from {
                s.push_str(&" ".repeat(pad));
                s.push_str(cell);
            } else {
                s.push_str(cell);
                if c + 1 < cells.len() {
                    s.push_str(&" ".repeat(pad));
                }
            }
        }
        s
    };
    writeln!(out, "{}", line(header.to_vec()))?;
    for r in rows {
        writeln!(out, "{}", line(r.iter().map(String::as_str).collect()))?;
    }
    Ok(())
}

/// Zwei getrennte Tabellen: Subscriptions (Tokens, kein Geld) und nach Verbrauch (Kosten).
pub fn tables(view: &UsageReportView, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    let dims = dim_columns(view);
    let sub: Vec<&UsageItem> = view
        .items
        .iter()
        .filter(|i| i.billing == "subscription")
        .collect();
    let metered: Vec<&UsageItem> = view
        .items
        .iter()
        .filter(|i| i.billing == "metered")
        .collect();
    if sub.is_empty() && metered.is_empty() {
        return writeln!(out, "Kein Verbrauch im Zeitraum.");
    }
    let tokens = |i: &UsageItem| {
        vec![
            grouped(i.turns),
            grouped(i.input_tokens),
            grouped(i.output_tokens),
            grouped(i.cache_read_tokens),
            grouped(i.cache_write_tokens),
        ]
    };
    let mut first = true;
    if !sub.is_empty() {
        first = false;
        writeln!(out, "Subscriptions · Tokens, kein Geldbetrag")?;
        let mut header: Vec<&str> = dims.iter().map(|(h, _)| *h).collect();
        let right = header.len().max(1);
        header.extend([
            "TURNS",
            "EINGABE",
            "AUSGABE",
            "CACHE LESEN",
            "CACHE SCHREIBEN",
        ]);
        let mut rows: Vec<Vec<String>> = sub
            .iter()
            .map(|i| {
                let mut r: Vec<String> = dims.iter().map(|(_, f)| f(i)).collect();
                r.extend(tokens(i));
                r
            })
            .collect();
        if dims.is_empty() {
            header.insert(0, "");
            for r in &mut rows {
                r.insert(0, "Summe".into());
            }
        } else if sub.len() > 1 {
            let mut r = vec![String::new(); dims.len()];
            r[0] = "Summe".into();
            r.extend(tokens(&view.totals.subscription));
            rows.push(r);
        }
        table(&header, right, &rows, out)?;
    }
    if !metered.is_empty() {
        if !first {
            writeln!(out)?;
        }
        writeln!(out, "Nach Verbrauch · API-Key, Gateway, lokal")?;
        let mut header: Vec<&str> = dims.iter().map(|(h, _)| *h).collect();
        let right = header.len().max(1);
        header.extend([
            "TURNS",
            "EINGABE",
            "AUSGABE",
            "CACHE LESEN",
            "CACHE SCHREIBEN",
            "KOSTEN",
            "OHNE PREIS",
        ]);
        let row = |i: &UsageItem| {
            let mut r = tokens(i);
            r.push(i.cost_usd.map_or_else(|| "–".into(), money));
            r.push(grouped(i.unpriced_count));
            r
        };
        let mut rows: Vec<Vec<String>> = metered
            .iter()
            .map(|i| {
                let mut r: Vec<String> = dims.iter().map(|(_, f)| f(i)).collect();
                r.extend(row(i));
                r
            })
            .collect();
        if dims.is_empty() {
            header.insert(0, "");
            for r in &mut rows {
                r.insert(0, "Summe".into());
            }
        } else if metered.len() > 1 {
            let mut r = vec![String::new(); dims.len()];
            r[0] = "Summe".into();
            r.extend(row(&view.totals.metered));
            rows.push(r);
        }
        table(&header, right, &rows, out)?;
    }
    Ok(())
}

/// Abo-Fenster mit Balken und Reset-Uhrzeit; Harnesses ohne Meldung als „nicht gemeldet“.
pub fn windows(
    view: &UsageReportView,
    now: OffsetDateTime,
    offset: UtcOffset,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    // Harnesses mit Subscription-Nutzung oder ganz ohne Meldung gehören in diesen Abschnitt.
    let subscription_harnesses: Vec<String> = view
        .items
        .iter()
        .filter(|i| i.billing == "subscription")
        .filter_map(|i| i.harness.clone())
        .collect();
    let silent: Vec<&str> = view
        .reporting
        .iter()
        .filter(|r| !r.rate_limits && (!r.tokens || subscription_harnesses.contains(&r.harness)))
        .map(|r| r.harness.as_str())
        .collect();
    if view.subscriptions.is_empty() && silent.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "Abo-Fenster")?;
    let mut rows: Vec<Vec<String>> = view
        .subscriptions
        .iter()
        .map(|w: &SubscriptionWindowView| {
            let state = if w.expired {
                format!(
                    "zurückgesetzt seit {}",
                    reset_label(&w.resets_at, now, offset)
                )
            } else {
                format!("Reset {}", reset_label(&w.resets_at, now, offset))
            };
            vec![
                w.harness.clone(),
                w.window.clone(),
                bar(w.used_pct),
                pct(w.used_pct),
                state,
            ]
        })
        .collect();
    for harness in silent {
        let what = match view.reporting.iter().find(|r| r.harness == harness) {
            Some(r) if !r.tokens => "nicht gemeldet (weder Tokens noch Fenster)",
            _ => "nicht gemeldet",
        };
        rows.push(vec![
            harness.to_owned(),
            "–".into(),
            String::new(),
            String::new(),
            what.into(),
        ]);
    }
    let widths: Vec<usize> = (0..5)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    for r in rows {
        let line = if r[2].is_empty() {
            // „nicht gemeldet“: ohne Balken und Prozent.
            format!(
                "{:w0$}  {:w1$}  {}",
                r[0],
                r[1],
                r[4],
                w0 = widths[0],
                w1 = widths[1]
            )
        } else {
            format!(
                "{:w0$}  {:w1$}  {:w2$}  {:>w3$}  {}",
                r[0],
                r[1],
                r[2],
                r[3],
                r[4],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
                w3 = widths[3]
            )
        };
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

fn notes(ctx: &Ctx, view: &UsageReportView) {
    if view.items.iter().any(|i| i.billing == "subscription") {
        let eq = view.totals.subscription.equivalent_usd.unwrap_or(0.0);
        ctx.note(format!(
            "Hinweis: Subscription-Nutzung wird nicht abgerechnet; API-Äquivalent laut Katalog {} \
             ca. {} (nur informativ).",
            view.pricing_version,
            money(eq)
        ));
    }
    let unpriced = view.totals.metered.unpriced_count;
    if unpriced > 0 {
        ctx.note(format!(
            "Hinweis: {unpriced} Turn(s) ohne Preis – eigenen Preis unter pricing.overrides anlegen \
             (Herleitung: beton usage pricing explain --model <M>)."
        ));
    }
}

fn notes_session(ctx: &Ctx, usage: &SessionUsageView) {
    if usage.totals.subscription.turns > 0 {
        ctx.note(format!(
            "Hinweis: Subscription-Nutzung wird nicht abgerechnet; API-Äquivalent ca. {} (nur \
             informativ).",
            money(usage.totals.subscription.equivalent_usd.unwrap_or(0.0))
        ));
    }
}

/// Turns einer Session: Tokens, Kosten und Preisquelle (USE-007 AC2).
pub fn session_table(
    usage: &SessionUsageView,
    offset: UtcOffset,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    if usage.records.is_empty() {
        return writeln!(
            out,
            "Session {}: kein Verbrauch gemeldet.",
            usage.session_id
        );
    }
    let header = [
        "#",
        "ZEIT",
        "MODELL",
        "ANMELDUNG",
        "EINGABE",
        "AUSGABE",
        "CACHE LESEN",
        "CACHE SCHREIBEN",
        "KOSTEN",
        "PREISQUELLE",
    ];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (n, r) in usage.records.iter().enumerate() {
        let time = parse_ts(&r.ts).map_or_else(String::new, |t| {
            let l = t.to_offset(offset);
            format!("{:02}:{:02}", l.hour(), l.minute())
        });
        let model = match &r.purpose {
            Some(p) => format!("{} ({p})", r.model),
            None => r.model.clone(),
        };
        rows.push(vec![
            (n + 1).to_string(),
            time,
            model,
            auth_label(&r.auth_source).to_owned(),
            grouped(r.input_tokens),
            grouped(r.output_tokens),
            grouped(r.cache_read_tokens),
            grouped(r.cache_write_tokens),
            r.cost_usd.map_or_else(|| "–".into(), money),
            r.cost_source.clone(),
        ]);
    }
    // Zahlen rechtsbündig ab EINGABE; Preisquelle wieder links (letzte Spalte).
    let widths: Vec<usize> = (0..header.len())
        .map(|c| {
            rows.iter()
                .map(|r| r[c].chars().count())
                .chain([header[c].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[&str]| -> String {
        let mut s = String::new();
        for (c, cell) in cells.iter().enumerate() {
            if c > 0 {
                s.push_str("  ");
            }
            let pad = widths[c].saturating_sub(cell.chars().count());
            let right = c == 0 || (4..=8).contains(&c);
            if right {
                s.push_str(&" ".repeat(pad));
                s.push_str(cell);
            } else {
                s.push_str(cell);
                if c + 1 < cells.len() {
                    s.push_str(&" ".repeat(pad));
                }
            }
        }
        s
    };
    writeln!(out, "{}", line(&header))?;
    for r in &rows {
        let cells: Vec<&str> = r.iter().map(String::as_str).collect();
        writeln!(out, "{}", line(&cells))?;
    }
    for (label, t) in [
        ("Summe Subscription", &usage.totals.subscription),
        ("Summe nach Verbrauch", &usage.totals.metered),
    ] {
        if t.turns == 0 {
            continue;
        }
        let cost = if t.billing == "metered" {
            format!(", Kosten {}", t.cost_usd.map_or_else(|| "–".into(), money))
        } else {
            String::new()
        };
        writeln!(
            out,
            "{label}: {} Turns, Eingabe {}, Ausgabe {}, Cache lesen {}, Cache schreiben {}{cost}",
            grouped(t.turns),
            grouped(t.input_tokens),
            grouped(t.output_tokens),
            grouped(t.cache_read_tokens),
            grouped(t.cache_write_tokens),
        )?;
    }
    Ok(())
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn opt_num(v: Option<f64>) -> String {
    v.map(|v| format!("{v}")).unwrap_or_default()
}

/// CSV der Gruppen: Dimensionen, Abrechnungsart, Summen; leere Zelle für `null`.
pub fn report_csv(view: &UsageReportView, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    let mut header: Vec<String> = view.group_by.clone();
    if view.group_by.iter().any(|d| d == "session") {
        header.push("session_title".into());
    }
    header.extend(
        [
            "billing",
            "turns",
            "sessions",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "cost_usd",
            "equivalent_usd",
            "unpriced_count",
        ]
        .map(str::to_owned),
    );
    writeln!(out, "{}", header.join(","))?;
    for i in &view.items {
        let mut cells: Vec<String> = Vec::new();
        for d in &view.group_by {
            cells.push(
                match d.as_str() {
                    "day" => i.day.clone(),
                    "session" => i.session_id.clone(),
                    "project" => i.project_id.clone(),
                    "harness" => i.harness.clone(),
                    "model" => i.model.clone(),
                    "auth_source" => i.auth_source.clone(),
                    _ => None,
                }
                .unwrap_or_default(),
            );
        }
        if view.group_by.iter().any(|d| d == "session") {
            cells.push(i.session_title.clone().unwrap_or_default());
        }
        cells.extend([
            i.billing.clone(),
            i.turns.to_string(),
            i.sessions.to_string(),
            i.input_tokens.to_string(),
            i.output_tokens.to_string(),
            i.cache_read_tokens.to_string(),
            i.cache_write_tokens.to_string(),
            opt_num(i.cost_usd),
            opt_num(i.equivalent_usd),
            i.unpriced_count.to_string(),
        ]);
        let cells: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
        writeln!(out, "{}", cells.join(","))?;
    }
    Ok(())
}

/// CSV der Turns einer Session.
pub fn session_csv(usage: &SessionUsageView, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(
        out,
        "seq,ts,harness,model,auth_source,billing,purpose,input_tokens,output_tokens,\
         cache_read_tokens,cache_write_tokens,cost_usd,cost_source,pricing_version,equivalent_usd"
    )?;
    for r in &usage.records {
        let cells = [
            r.seq.to_string(),
            r.ts.clone(),
            r.harness.clone(),
            r.model.clone(),
            r.auth_source.clone(),
            r.billing.clone(),
            r.purpose.clone().unwrap_or_default(),
            r.input_tokens.to_string(),
            r.output_tokens.to_string(),
            r.cache_read_tokens.to_string(),
            r.cache_write_tokens.to_string(),
            opt_num(r.cost_usd),
            r.cost_source.clone(),
            r.pricing_version.clone().unwrap_or_default(),
            opt_num(r.equivalent_usd),
        ];
        let cells: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
        writeln!(out, "{}", cells.join(","))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use beton_server::api_usage::{HarnessReportingView, UsageTotals};
    use time::macros::datetime;

    fn item(harness: &str, billing: &str, input: u64, cost: Option<f64>) -> UsageItem {
        UsageItem {
            harness: Some(harness.into()),
            billing: billing.into(),
            turns: 1,
            sessions: 1,
            input_tokens: input,
            output_tokens: 10,
            cost_usd: cost,
            equivalent_usd: (billing == "subscription").then_some(1.5),
            ..UsageItem::default()
        }
    }

    fn view(items: Vec<UsageItem>) -> UsageReportView {
        UsageReportView {
            from: "2026-10-01T00:00:00.000Z".into(),
            to: "2026-10-02T00:00:00.000Z".into(),
            tz: "+02:00".into(),
            group_by: vec!["harness".into()],
            pricing_version: "2026-10-01".into(),
            totals: UsageTotals::default(),
            items,
            subscriptions: Vec::new(),
            reporting: Vec::new(),
        }
    }

    fn text(f: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>) -> String {
        let mut buf = Vec::new();
        f(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn use_004_ac2_window_as_bar_with_reset_time_in_the_users_timezone() {
        let mut v = view(vec![item("claude", "subscription", 100, None)]);
        v.subscriptions = vec![SubscriptionWindowView {
            harness: "claude".into(),
            vendor: "anthropic".into(),
            window: "5h".into(),
            used_pct: 72.0,
            resets_at: "2026-10-03T14:19:00.000Z".into(),
            observed_at: "2026-10-03T12:00:00.000Z".into(),
            session_id: "ses_x".into(),
            expired: false,
        }];
        let now = datetime!(2026-10-03 12:05 UTC);
        let plus2 = UtcOffset::from_hms(2, 0, 0).unwrap();
        let out = text(|b| windows(&v, now, plus2, b));
        assert_eq!(
            out,
            "\nAbo-Fenster\nclaude  5h  ███████░░░  72 %  Reset 16:19\n"
        );
        // Anderer Tag: Wochentag und Datum.
        assert_eq!(
            reset_label("2026-10-05T07:00:00.000Z", now, plus2),
            "Mo 05.10. 09:00"
        );
        assert_eq!(bar(0.0), "░░░░░░░░░░");
        assert_eq!(bar(100.0), "██████████");
        assert_eq!(bar(140.0), "██████████");
    }

    #[test]
    fn use_004_ac4_harness_without_reporting_is_not_reported_instead_of_zero() {
        let mut v = view(vec![item("claude", "subscription", 100, None)]);
        v.reporting = vec![
            HarnessReportingView {
                harness: "acp:gemini".into(),
                sessions: 2,
                tokens: false,
                rate_limits: false,
            },
            HarnessReportingView {
                harness: "claude".into(),
                sessions: 1,
                tokens: true,
                rate_limits: false,
            },
        ];
        let out = text(|b| windows(&v, datetime!(2026-10-03 12:05 UTC), UtcOffset::UTC, b));
        assert!(
            out.contains("acp:gemini  –  nicht gemeldet (weder Tokens noch Fenster)"),
            "{out}"
        );
        assert!(out.contains("claude      –  nicht gemeldet\n"), "{out}");
        assert!(!out.contains(" 0 %"), "{out}");
    }

    #[test]
    fn cli_010_ac2_subscription_and_metered_are_never_summed() {
        let mut v = view(vec![
            item("claude", "subscription", 1_000, None),
            item("ollama", "metered", 2_000, Some(0.0)),
            item("direct:litellm", "metered", 3_000, Some(1.25)),
        ]);
        v.totals.metered = UsageItem {
            billing: "metered".into(),
            turns: 2,
            input_tokens: 5_000,
            cost_usd: Some(1.25),
            ..UsageItem::default()
        };
        let out = text(|b| tables(&v, b));
        let (sub, metered) = out.split_once("Nach Verbrauch").unwrap();
        assert!(
            sub.starts_with("Subscriptions · Tokens, kein Geldbetrag"),
            "{out}"
        );
        assert!(!sub.contains('$'), "kein Geld bei Subscriptions: {out}");
        assert!(metered.contains("1,25 $"), "{out}");
        // Die Summe gilt nur für die Zeilen nach Verbrauch: 5.000, nicht 6.000.
        assert!(metered.contains("Summe"), "{out}");
        assert!(metered.contains("5.000"), "{out}");
        assert!(!out.contains("6.000"), "{out}");
    }

    #[test]
    fn use_007_since_accepts_durations_and_dates() {
        let now = datetime!(2026-10-03 12:00 UTC);
        let plus2 = UtcOffset::from_hms(2, 0, 0).unwrap();
        let p = params(
            &UsageReportArgs {
                since: Some("1d".into()),
                by: vec!["model".into()],
                ..UsageReportArgs::default()
            },
            now,
            plus2,
        )
        .unwrap();
        assert_eq!(p.from.as_deref(), Some("2026-10-02T12:00:00.000Z"));
        assert_eq!(p.to.as_deref(), Some("2026-10-03T12:00:00.000Z"));
        assert_eq!(p.tz.as_deref(), Some("+02:00"));
        assert_eq!(p.group_by.as_deref(), Some("model"));
        let p = params(
            &UsageReportArgs {
                since: Some("2026-10-01".into()),
                ..UsageReportArgs::default()
            },
            now,
            plus2,
        )
        .unwrap();
        assert_eq!(p.from.as_deref(), Some("2026-09-30T22:00:00.000Z"));
        let p = params(
            &UsageReportArgs {
                from: Some("2026-10-01".into()),
                ..UsageReportArgs::default()
            },
            now,
            plus2,
        )
        .unwrap();
        assert_eq!(p.from.as_deref(), Some("2026-10-01"));
        assert_eq!(p.to.as_deref(), Some("2026-10-03T12:00:00.000Z"));
        for bad in ["gestern", "0d", "7x"] {
            let args = UsageReportArgs {
                since: Some(bad.into()),
                ..UsageReportArgs::default()
            };
            assert_eq!(params(&args, now, plus2).unwrap_err().exit, Exit::Usage);
        }
    }

    #[test]
    fn formatting() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(1_234_567), "1.234.567");
        assert_eq!(money(1.75), "1,75 $");
        assert_eq!(money(0.075), "0,075 $");
        assert_eq!(pct(37.4), "37,4 %");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
    }
}
