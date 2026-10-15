//! Usage-API (USE-005, USE-004, USE-007): `GET /v1/usage` mit Gruppierung, Filtern und
//! getrennten Summen, `GET /v1/sessions/{id}/usage` mit den Records je Turn.
//!
//! Geld und Subscription bleiben getrennt (ADR-0034, CLI-010 AC2): Jeder Eintrag hat eine
//! Abrechnungsart (`billing`); `cost_usd` gibt es nur bei `metered`, `equivalent_usd`
//! (informativ) nur bei `subscription`. Die Summen stehen je Abrechnungsart, nie zusammen.
//! Abo-Fenster (`subscriptions`) zeigen den jüngsten gemeldeten Stand; `reporting` nennt je
//! Harness, ob er Tokens bzw. Fenster meldet (sonst „nicht gemeldet“ statt 0, USE-004 AC4).
//!
//! `beton usage` ohne Daemon (USE-007 AC3) nutzt dieselben Funktionen
//! ([`UsageParams::to_query`], [`UsageReportView::new`]) direkt auf der Datenbank.

use axum::extract::{Path, State};
use beton_core::id::{ProjectId, SessionId, UserId};
use beton_core::time::Timestamp;
use beton_store::{
    Billing, HarnessReporting, SubscriptionWindow, UsageDimension, UsageGroup, UsageQuery,
    UsageRecord, UsageReport,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime, Time, UtcOffset};
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::ApiQuery;
use crate::problem::{ApiResult, Problem, ProblemCode};

/// Standard-Zeitraum ohne `from`: 30 Tage vor `to`.
pub const DEFAULT_DAYS: i64 = 30;

/// Query-Parameter von `GET /v1/usage`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UsageParams {
    /// Beginn: RFC 3339 oder `YYYY-MM-DD` (00:00 in `tz`). Default: `to` minus 30 Tage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Ende, exklusiv: RFC 3339 oder `YYYY-MM-DD` (dieser Tag zählt noch mit). Default: jetzt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Zeitzone des Users als Versatz (`+02:00`, `-05:00`, `Z`); bestimmt Tagesgrenzen von
    /// `day` und Datumsangaben. Default UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// Komma-Liste aus `day`, `session`, `project`, `harness`, `model`, `auth_source`
    /// (`user` und `team` ab M4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Lokal gibt es nur den Owner; Sichtbarkeit nach Rolle kommt mit M4 (AUTH-014).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Ab M4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Versatz `+02:00`, `-0530`, `Z` bzw. `UTC`.
pub fn parse_offset(raw: &str) -> Result<UtcOffset, String> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("z") || s.eq_ignore_ascii_case("utc") {
        return Ok(UtcOffset::UTC);
    }
    let invalid = || format!("tz `{raw}` ist ungültig (erwartet z. B. +02:00 oder Z)");
    let (sign, rest) = match s.as_bytes().first() {
        Some(b'+') => (1, &s[1..]),
        Some(b'-') => (-1, &s[1..]),
        _ => return Err(invalid()),
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    let h: i8 = digits[..2].parse().map_err(|_| invalid())?;
    let m: i8 = digits[2..].parse().map_err(|_| invalid())?;
    UtcOffset::from_hms(sign * h, sign * m, 0).map_err(|_| invalid())
}

/// Versatz als Text, z. B. `+02:00`.
pub fn format_offset(o: UtcOffset) -> String {
    let (h, m, _) = o.as_hms();
    let sign = if o.is_negative() { '-' } else { '+' };
    format!("{sign}{:02}:{:02}", h.unsigned_abs(), m.unsigned_abs())
}

fn date(raw: &str) -> Option<Date> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    Date::parse(raw, &format).ok()
}

/// Zeitpunkt aus RFC 3339 oder Datum; `end` nimmt bei einem Datum den Folgetag 00:00.
fn instant(raw: &str, offset: UtcOffset, end: bool, name: &str) -> Result<OffsetDateTime, String> {
    if let Some(d) = date(raw) {
        let start = d.with_time(Time::MIDNIGHT).assume_offset(offset);
        return Ok(if end {
            start + time::Duration::DAY
        } else {
            start
        });
    }
    raw.parse::<Timestamp>()
        .map(|t| t.as_offset_date_time())
        .map_err(|_| format!("{name} `{raw}` ist ungültig (RFC 3339 oder JJJJ-MM-TT)"))
}

impl UsageParams {
    /// Auswahl für den Store; Fehlermeldungen nennen den Parameter.
    pub fn to_query(&self, now: OffsetDateTime) -> Result<UsageQuery, String> {
        let offset = match &self.tz {
            Some(t) => parse_offset(t)?,
            None => UtcOffset::UTC,
        };
        let to = match &self.to {
            Some(t) => instant(t, offset, true, "to")?,
            None => now,
        };
        let from = match &self.from {
            Some(f) => instant(f, offset, false, "from")?,
            None => to - time::Duration::days(DEFAULT_DAYS),
        };
        if from >= to {
            return Err("from muss vor to liegen".into());
        }
        if self.team.is_some() {
            return Err("team gibt es erst ab M4 (Teams, AUTH-014)".into());
        }
        let mut group_by = Vec::new();
        for raw in self
            .group_by
            .iter()
            .flat_map(|g| g.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let d = match raw {
                "user" | "team" => {
                    return Err(format!(
                        "group_by `{raw}` gibt es erst ab M4 (Rollen und Teams, AUTH-014)"
                    ));
                }
                other => UsageDimension::parse(other).ok_or_else(|| {
                    format!(
                        "group_by `{other}` ist ungültig (day, session, project, harness, model, \
                         auth_source)"
                    )
                })?,
            };
            if !group_by.contains(&d) {
                group_by.push(d);
            }
        }
        let parse_id = |name: &str, raw: &Option<String>| -> Result<Option<String>, String> {
            match raw {
                Some(s) if s.trim().is_empty() => Err(format!("{name} ist leer")),
                other => Ok(other.clone()),
            }
        };
        Ok(UsageQuery {
            from: from.into(),
            to: to.into(),
            utc_offset_s: offset.whole_seconds(),
            group_by,
            project: parse_id("project", &self.project)?
                .map(|p| {
                    p.parse::<ProjectId>()
                        .map_err(|_| format!("project `{p}` ist ungültig"))
                })
                .transpose()?,
            harness: parse_id("harness", &self.harness)?,
            user: parse_id("user", &self.user)?
                .map(|u| {
                    u.parse::<UserId>()
                        .map_err(|_| format!("user `{u}` ist ungültig"))
                })
                .transpose()?,
            session: parse_id("session_id", &self.session_id)?
                .map(|s| {
                    s.parse::<SessionId>()
                        .map_err(|_| format!("session_id `{s}` ist ungültig"))
                })
                .transpose()?,
        })
    }
}

fn usd(micro: i64) -> f64 {
    micro as f64 / 1_000_000.0
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Ein Eintrag: gruppierte Dimensionen (nur die angefragten) und Summen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct UsageItem {
    /// Kalendertag in der Zeitzone `tz` (`YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub day: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_title: Option<String>,
    /// Fehlt bei Sessions ohne Projekt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auth_source: Option<String>,
    /// `subscription` (Login der Vendor-CLI) oder `metered` (API-Key, Gateway, lokal).
    pub billing: String,
    /// Turns (Usage-Records).
    pub turns: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Kosten in USD; `null` bei Subscription (kein Geldbetrag).
    pub cost_usd: Option<f64>,
    /// API-Äquivalent in USD, nur informativ; `null` außer bei Subscription.
    pub equivalent_usd: Option<f64>,
    /// Turns ohne Preis („ohne Preis – Override anlegen“).
    pub unpriced_count: u64,
}

impl UsageItem {
    fn empty(billing: Billing) -> Self {
        Self {
            billing: enum_str(&billing),
            cost_usd: (billing == Billing::Metered).then_some(0.0),
            equivalent_usd: (billing == Billing::Subscription).then_some(0.0),
            ..Self::default()
        }
    }
}

impl From<&UsageGroup> for UsageItem {
    fn from(g: &UsageGroup) -> Self {
        Self {
            day: g.day.clone(),
            session_id: g.session_id.map(|s| s.to_string()),
            session_title: g.session_title.clone(),
            project_id: g.project_id.map(|p| p.to_string()),
            harness: g.harness.clone(),
            model: g.model.clone(),
            auth_source: g.auth_source.as_ref().map(enum_str),
            billing: g.billing.as_ref().map(enum_str).unwrap_or_default(),
            turns: g.turns,
            sessions: g.sessions,
            input_tokens: g.input_tokens,
            output_tokens: g.output_tokens,
            cache_read_tokens: g.cache_read_tokens,
            cache_write_tokens: g.cache_write_tokens,
            cost_usd: g.cost_micro.map(usd),
            equivalent_usd: g.equivalent_micro.map(usd),
            unpriced_count: g.unpriced,
        }
    }
}

/// Summen je Abrechnungsart; es gibt bewusst keine Gesamtsumme.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct UsageTotals {
    /// API-Key, Gateway, lokal: mit Kosten.
    pub metered: UsageItem,
    /// Login der Vendor-CLI: Tokens, kein Geld.
    pub subscription: UsageItem,
}

impl UsageTotals {
    fn from_groups(groups: &[UsageGroup]) -> Self {
        let pick = |b: Billing| {
            groups
                .iter()
                .find(|g| g.billing == Some(b))
                .map_or_else(|| UsageItem::empty(b), UsageItem::from)
        };
        Self {
            metered: pick(Billing::Metered),
            subscription: pick(Billing::Subscription),
        }
    }
}

/// Jüngster gemeldeter Stand eines Abo-Fensters (USE-004 AC2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct SubscriptionWindowView {
    pub harness: String,
    /// `anthropic`, `openai`, …
    pub vendor: String,
    /// `5h`, `7d`, `7d_opus`, …
    pub window: String,
    /// Genutzter Anteil in Prozent.
    pub used_pct: f64,
    /// RFC 3339 (UTC); die Anzeige rechnet in die Zeitzone des Users um.
    pub resets_at: String,
    /// Zeitpunkt der Meldung.
    pub observed_at: String,
    pub session_id: String,
    /// Reset-Zeitpunkt liegt in der Vergangenheit: der Wert ist überholt.
    pub expired: bool,
}

impl SubscriptionWindowView {
    fn new(w: &SubscriptionWindow, now: OffsetDateTime) -> Self {
        Self {
            harness: w.harness.clone(),
            vendor: w.vendor.clone(),
            window: w.window.clone(),
            used_pct: w.used_pct,
            resets_at: w.resets_at.to_string(),
            observed_at: w.observed_at.to_string(),
            session_id: w.session_id.to_string(),
            expired: w.resets_at.as_offset_date_time() <= now,
        }
    }
}

/// Was ein Harness im Zeitraum meldet (USE-004 AC4): `false` heißt „nicht gemeldet“.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct HarnessReportingView {
    pub harness: String,
    /// Sessions mit Aktivität im Zeitraum.
    pub sessions: u64,
    /// Tokens gemeldet (Usage-Records im Zeitraum).
    pub tokens: bool,
    /// Abo-Fenster gemeldet.
    pub rate_limits: bool,
}

impl From<&HarnessReporting> for HarnessReportingView {
    fn from(h: &HarnessReporting) -> Self {
        Self {
            harness: h.harness.clone(),
            sessions: h.sessions,
            tokens: h.tokens,
            rate_limits: h.rate_limits,
        }
    }
}

/// Antwort von `GET /v1/usage` und Ausgabe von `beton usage --json` (USE-005 AC1, USE-007
/// AC1; Schema `schemas/v1/usage.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct UsageReportView {
    /// Beginn (inklusiv), RFC 3339 UTC.
    pub from: String,
    /// Ende (exklusiv), RFC 3339 UTC.
    pub to: String,
    /// Zeitzone der Tagesgrenzen, z. B. `+02:00`.
    pub tz: String,
    pub group_by: Vec<String>,
    /// Version des eingebauten Preis-Katalogs (USE-002); Records tragen ihre eigene.
    pub pricing_version: String,
    /// Eine Gruppe je Kombination der `group_by`-Werte und Abrechnungsart (kein Paging:
    /// eine Auswertung, keine Liste von Ressourcen).
    pub groups: Vec<UsageItem>,
    pub totals: UsageTotals,
    pub subscriptions: Vec<SubscriptionWindowView>,
    pub reporting: Vec<HarnessReportingView>,
}

impl UsageReportView {
    pub fn new(q: &UsageQuery, report: &UsageReport, now: OffsetDateTime) -> Self {
        let tz = UtcOffset::from_whole_seconds(q.utc_offset_s).unwrap_or(UtcOffset::UTC);
        Self {
            from: q.from.to_string(),
            to: q.to.to_string(),
            tz: format_offset(tz),
            group_by: q.group_by.iter().map(|d| d.as_str().to_owned()).collect(),
            pricing_version: beton_core::pricing::Catalog::embedded()
                .version()
                .to_owned(),
            groups: report.groups.iter().map(UsageItem::from).collect(),
            totals: UsageTotals::from_groups(&report.totals),
            subscriptions: report
                .windows
                .iter()
                .map(|w| SubscriptionWindowView::new(w, now))
                .collect(),
            reporting: report
                .reporting
                .iter()
                .map(HarnessReportingView::from)
                .collect(),
        }
    }
}

/// Ein Turn einer Session (USE-007 AC2): Tokens, Kosten und Preisquelle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct UsageRecordView {
    pub seq: u64,
    pub ts: String,
    pub harness: String,
    pub model: String,
    pub auth_source: String,
    pub billing: String,
    /// Zweck außerhalb der Unterhaltung, z. B. `title` oder `compaction`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// `null` bei Subscription und ohne Preis.
    pub cost_usd: Option<f64>,
    /// `override`, `harness`, `catalog`, `none` oder `subscription`.
    pub cost_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pricing_version: Option<String>,
    /// API-Äquivalent (informativ), nur bei Subscription.
    pub equivalent_usd: Option<f64>,
}

impl From<&UsageRecord> for UsageRecordView {
    fn from(r: &UsageRecord) -> Self {
        let subscription = r.auth_source == beton_core::event::AuthSource::VendorCli;
        Self {
            seq: r.seq,
            ts: r.ts.to_string(),
            harness: r.harness.clone(),
            model: r.model.clone(),
            auth_source: enum_str(&r.auth_source),
            billing: enum_str(&if subscription {
                Billing::Subscription
            } else {
                Billing::Metered
            }),
            purpose: r.purpose.clone(),
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens,
            cache_write_tokens: r.cache_write_tokens,
            cost_usd: if subscription {
                None
            } else {
                r.cost_micro.map(usd)
            },
            cost_source: enum_str(&r.cost_source),
            pricing_version: r.pricing_version.clone(),
            equivalent_usd: if subscription {
                r.equivalent_micro.map(usd)
            } else {
                None
            },
        }
    }
}

/// Antwort von `GET /v1/sessions/{id}/usage` und `beton usage --session <id> --json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct SessionUsageView {
    pub session_id: String,
    pub records: Vec<UsageRecordView>,
    pub totals: UsageTotals,
}

impl SessionUsageView {
    pub fn new(session: SessionId, records: &[UsageRecord]) -> Self {
        let views: Vec<UsageRecordView> = records.iter().map(UsageRecordView::from).collect();
        let mut totals = UsageTotals {
            metered: UsageItem::empty(Billing::Metered),
            subscription: UsageItem::empty(Billing::Subscription),
        };
        for r in records {
            let subscription = r.auth_source == beton_core::event::AuthSource::VendorCli;
            let t = if subscription {
                &mut totals.subscription
            } else {
                &mut totals.metered
            };
            t.turns += 1;
            t.sessions = 1;
            t.input_tokens += r.input_tokens;
            t.output_tokens += r.output_tokens;
            t.cache_read_tokens += r.cache_read_tokens;
            t.cache_write_tokens += r.cache_write_tokens;
            if r.cost_source == beton_core::event::PriceSource::None {
                t.unpriced_count += 1;
            }
        }
        let money: i64 = records
            .iter()
            .filter(|r| r.auth_source != beton_core::event::AuthSource::VendorCli)
            .filter_map(|r| r.cost_micro)
            .sum();
        let equivalent: i64 = records
            .iter()
            .filter(|r| r.auth_source == beton_core::event::AuthSource::VendorCli)
            .filter_map(|r| r.equivalent_micro)
            .sum();
        totals.metered.cost_usd = Some(usd(money));
        totals.subscription.equivalent_usd = Some(usd(equivalent));
        Self {
            session_id: session.to_string(),
            records: views,
            totals,
        }
    }
}

/// Ein Modell des mitgelieferten Preis-Katalogs (USE-002); USD je 1 Mio. Tokens, wie im
/// Katalog angegeben (fehlende Cache-Raten bleiben leer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct CatalogEntryView {
    pub model: String,
    /// Weitere IDs desselben Modells, z. B. datierte Snapshots.
    pub aliases: Vec<String>,
    pub provider: String,
    pub context_window: Option<u64>,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
    /// Ab wann der Preis gilt (`YYYY-MM-DD`).
    pub valid_from: String,
}

/// Ein eigener Preis (USE-003) in Prüfreihenfolge: `pricing.overrides`, danach die Preise der
/// Provider-Konfiguration. Raten wirksam, d. h. fehlende Cache-Raten geschätzt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct PriceOverrideView {
    /// Position in der Prüfreihenfolge, ab 1; der erste Treffer gewinnt.
    pub position: u32,
    /// Glob auf den Provider; fehlt = jeder.
    pub provider: Option<String>,
    /// Glob auf das Modell; fehlt = jedes.
    pub model: Option<String>,
    /// Kostenlos (z. B. lokales Ollama).
    pub free: bool,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    /// Die Cache-Raten sind geschätzt (0,1 × bzw. 1,25 × Eingabe, USE-003 AC2).
    pub cache_estimated: bool,
    pub context_window: Option<u64>,
}

/// Antwort von `GET /v1/usage/pricing`: womit der Server neue Turns bewertet (USE-002,
/// USE-003); Ziel des Hinweises „ohne Preis“ der Usage-Seite (USE-006 AC4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS, JsonSchema)]
pub struct PricingView {
    /// Version des eingebauten Katalogs; kommt mit jedem beton-Release, kein Online-Abruf.
    pub catalog_version: String,
    pub currency: String,
    pub overrides: Vec<PriceOverrideView>,
    pub catalog: Vec<CatalogEntryView>,
}

impl PricingView {
    pub fn new(pricer: &beton_core::pricing::Pricer) -> Self {
        let usd = |micro: i64| micro as f64 / 1_000_000.0;
        Self {
            catalog_version: pricer.catalog().version().to_owned(),
            currency: "USD".into(),
            overrides: pricer
                .overrides()
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let r = o.rates();
                    PriceOverrideView {
                        position: u32::try_from(i + 1).unwrap_or(u32::MAX),
                        provider: o.matcher.provider.clone(),
                        model: o.matcher.model.clone(),
                        free: o.free,
                        input_per_mtok: usd(r.input_micro),
                        output_per_mtok: usd(r.output_micro),
                        cache_read_per_mtok: usd(r.cache_read_micro),
                        cache_write_per_mtok: usd(r.cache_write_micro),
                        cache_estimated: r.cache_estimated,
                        context_window: o.context_window,
                    }
                })
                .collect(),
            catalog: pricer
                .catalog()
                .models()
                .iter()
                .map(|m| CatalogEntryView {
                    model: m.id.clone(),
                    aliases: m.aliases.clone(),
                    provider: m.provider.clone(),
                    context_window: m.context_window,
                    input_per_mtok: m.input_per_mtok,
                    output_per_mtok: m.output_per_mtok,
                    cache_read_per_mtok: m.cache_read_per_mtok,
                    cache_write_per_mtok: m.cache_write_per_mtok,
                    valid_from: m.valid_from.clone(),
                })
                .collect(),
        }
    }
}

fn invalid(detail: String) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail)
}

/// Aggregierte Usage (USE-005): Summen je Gruppe und Abrechnungsart, Abo-Fenster, Melde-Stand.
#[utoipa::path(get, path = "/v1/usage", tag = "usage",
    params(UsageParams),
    responses((status = 200, description = "Summen, Abo-Fenster und Melde-Stand", body = UsageReportView)))]
pub async fn get_usage(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<UsageParams>,
) -> ApiResult<axum::Json<UsageReportView>> {
    let now = OffsetDateTime::now_utc();
    let q = params.to_query(now).map_err(invalid)?;
    let report = state.store.usage_report(state.local.org, &q).await?;
    Ok(axum::Json(UsageReportView::new(&q, &report, now)))
}

/// Preise, mit denen der Server bewertet (USE-002, USE-003): eigene Preise in
/// Prüfreihenfolge und der mitgelieferte Katalog.
#[utoipa::path(get, path = "/v1/usage/pricing", tag = "usage",
    responses((status = 200, description = "Eigene Preise und Preis-Katalog", body = PricingView)))]
pub async fn get_pricing(State(state): State<AppState>) -> ApiResult<axum::Json<PricingView>> {
    Ok(axum::Json(PricingView::new(state.store.pricer())))
}

/// Usage einer Session je Turn (USE-007 AC2): Tokens, Kosten, Preisquelle.
#[utoipa::path(get, path = "/v1/sessions/{id}/usage", tag = "usage",
    params(("id" = String, Path)),
    responses((status = 200, description = "Records und Summen der Session", body = SessionUsageView)))]
pub async fn get_session_usage(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SessionUsageView>> {
    let id: SessionId = id
        .parse()
        .map_err(|_| invalid(format!("Session-ID `{id}` ist ungültig")))?;
    state.store.session(state.local.org, id).await?;
    let records = state.store.usage_records(state.local.org, id).await?;
    Ok(axum::Json(SessionUsageView::new(id, &records)))
}

#[cfg(test)]
#[path = "api_usage_tests.rs"]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn params(pairs: &[(&str, &str)]) -> UsageParams {
        let mut p = UsageParams::default();
        for (k, v) in pairs {
            let v = Some((*v).to_owned());
            match *k {
                "from" => p.from = v,
                "to" => p.to = v,
                "tz" => p.tz = v,
                "group_by" => p.group_by = v,
                "team" => p.team = v,
                "user" => p.user = v,
                _ => unreachable!(),
            }
        }
        p
    }

    #[test]
    fn use_005_dates_are_whole_days_in_the_users_timezone() {
        let now = datetime!(2026-10-03 12:00 UTC);
        let q = params(&[
            ("from", "2026-10-01"),
            ("to", "2026-10-02"),
            ("tz", "+02:00"),
        ])
        .to_query(now)
        .unwrap();
        assert_eq!(q.from.to_string(), "2026-09-30T22:00:00.000Z");
        assert_eq!(q.to.to_string(), "2026-10-02T22:00:00.000Z");
        assert_eq!(q.utc_offset_s, 7200);
        let q = params(&[]).to_query(now).unwrap();
        assert_eq!(q.to.to_string(), "2026-10-03T12:00:00.000Z");
        assert_eq!(q.from.to_string(), "2026-09-03T12:00:00.000Z");
        let q = params(&[
            ("from", "2026-10-03T10:00:00Z"),
            ("group_by", "day, harness,day"),
        ])
        .to_query(now)
        .unwrap();
        assert_eq!(q.group_by, [UsageDimension::Day, UsageDimension::Harness]);
    }

    #[test]
    fn use_005_user_and_team_dimensions_come_with_m4() {
        let now = datetime!(2026-10-03 12:00 UTC);
        for p in [
            params(&[("group_by", "user")]),
            params(&[("group_by", "model,team")]),
            params(&[("team", "t1")]),
        ] {
            let err = p.to_query(now).unwrap_err();
            assert!(err.contains("ab M4"), "{err}");
        }
        assert!(params(&[("group_by", "farbe")]).to_query(now).is_err());
        assert!(params(&[("tz", "Europe/Berlin")]).to_query(now).is_err());
        assert!(
            params(&[("from", "2026-10-03"), ("to", "2026-10-01")])
                .to_query(now)
                .is_err()
        );
    }

    #[test]
    fn offsets_round_trip() {
        for s in ["+02:00", "-05:30", "+00:00"] {
            assert_eq!(format_offset(parse_offset(s).unwrap()), s);
        }
        assert_eq!(parse_offset("Z").unwrap(), UtcOffset::UTC);
        assert_eq!(format_offset(parse_offset("+0530").unwrap()), "+05:30");
    }
}
