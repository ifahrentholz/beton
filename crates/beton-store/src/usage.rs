//! Usage-Auswertung (USE-004, USE-005, USE-007): Summen über die Usage-Records mit
//! Gruppierung und Filtern, die Abo-Fenster der Vendor-Logins und welche Harnesses überhaupt
//! etwas melden.
//!
//! Geld und Subscription bleiben getrennt (ADR-0034): Jede Gruppe gehört genau zu einer
//! Abrechnungsart (`subscription` bei `auth_source: vendor_cli`, sonst `metered`); Geld gibt
//! es nur bei `metered`, das API-Äquivalent nur bei `subscription`. Mischsummen entstehen nie
//! (CLI-010 AC2).
//!
//! Daemon (`GET /v1/usage`) und CLI ohne Daemon (USE-007 AC3) lesen über dieselben Abfragen,
//! damit beide dieselben Summen zeigen.

use std::path::Path;

use beton_core::event::AuthSource;
use beton_core::id::{OrgId, ProjectId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::sessions::{enum_from_str, parse, to_u64};
use crate::{BlobStore, Error, Result, Store, StoreOptions};

/// Dimension für `group_by` (USE-005). `user` und `team` kommen mit M4 (AUTH-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageDimension {
    Day,
    Session,
    Project,
    Harness,
    Model,
    AuthSource,
}

impl UsageDimension {
    pub const ALL: [Self; 6] = [
        Self::Day,
        Self::Session,
        Self::Project,
        Self::Harness,
        Self::Model,
        Self::AuthSource,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Session => "session",
            Self::Project => "project",
            Self::Harness => "harness",
            Self::Model => "model",
            Self::AuthSource => "auth_source",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// SQL-Ausdruck über `usage_records r` (Tag in der Zeitzone des Users: Parameter 1 bzw.
    /// je Versatzwechsel ein eigener Versatz, [`day_column`]).
    fn column(self, q: &UsageQuery) -> String {
        match self {
            Self::Day => return day_column(q),
            Self::Session => "r.session_id",
            Self::Project => "r.project_id",
            Self::Harness => "r.harness",
            Self::Model => "r.model",
            Self::AuthSource => "r.auth_source",
        }
        .to_owned()
    }
}

/// Tag eines Records in der Zeitzone des Users. Mit Sommerzeit gilt je Abschnitt zwischen zwei
/// Versatzwechseln dessen Versatz; Zeitpunkte und Versätze stammen aus [`UsageQuery`] (keine
/// Eingabe des Users), Zeitstempel vergleichen als Text wie `r.ts >= ?3`.
fn day_column(q: &UsageQuery) -> String {
    if q.offset_changes.is_empty() {
        return "date(r.ts, ?1)".into();
    }
    let mut out = String::from("date(r.ts, CASE");
    for (at, offset) in q.offset_changes.iter().rev() {
        out.push_str(&format!(
            " WHEN r.ts >= '{at}' THEN '{}'",
            offset_modifier(*offset)
        ));
    }
    out.push_str(" ELSE ?1 END)");
    out
}

/// Abrechnungsart einer Gruppe: Subscription (Login der Vendor-CLI) oder nach Verbrauch
/// (API-Key, Gateway, lokal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    Metered,
    Subscription,
}

/// Auswahl für [`Store::usage_report`]: Zeitraum `[from, to)`, Gruppierung und Filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageQuery {
    pub from: Timestamp,
    pub to: Timestamp,
    /// Versatz der Zeitzone des Users zu UTC in Sekunden zu Beginn (`from`); bestimmt mit
    /// `offset_changes` die Tagesgrenzen von `day`.
    pub utc_offset_s: i32,
    /// Versatzwechsel im Zeitraum (Sommerzeit): ab dem Zeitpunkt gilt der Versatz in Sekunden.
    /// Leer bei UTC und festen Versätzen.
    pub offset_changes: Vec<(Timestamp, i32)>,
    pub group_by: Vec<UsageDimension>,
    pub project: Option<ProjectId>,
    pub harness: Option<String>,
    pub user: Option<UserId>,
    pub session: Option<SessionId>,
}

/// Eine Gruppe: Werte der gruppierten Dimensionen und die Summen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageGroup {
    pub day: Option<String>,
    pub session_id: Option<SessionId>,
    pub session_title: Option<String>,
    pub project_id: Option<ProjectId>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub auth_source: Option<AuthSource>,
    pub billing: Option<Billing>,
    /// Records, also Turns (ein `cost.delta` je Turn und Modell).
    pub turns: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Geld; nur bei `metered` (auch 0, wenn nichts einen Preis hat).
    pub cost_micro: Option<i64>,
    /// API-Äquivalent; nur bei `subscription`, informativ.
    pub equivalent_micro: Option<i64>,
    /// Records ohne Preis (`cost_source: none`).
    pub unpriced: u64,
}

/// Jüngster gemeldeter Stand eines Abo-Fensters (USE-004 AC2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubscriptionWindow {
    pub harness: String,
    pub vendor: String,
    /// Vereinheitlicht: `5h`, `7d`, `7d_opus`.
    pub window: String,
    pub used_pct: f64,
    pub resets_at: Timestamp,
    /// Zeitpunkt der Meldung.
    pub observed_at: Timestamp,
    pub session_id: SessionId,
}

/// Was ein Harness im Zeitraum gemeldet hat (USE-004 AC4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HarnessReporting {
    pub harness: String,
    /// Sessions mit Aktivität im Zeitraum.
    pub sessions: u64,
    /// Es gibt Usage-Records im Zeitraum.
    pub tokens: bool,
    /// Es gibt einen gemeldeten Stand eines Abo-Fensters.
    pub rate_limits: bool,
}

/// Ergebnis von [`Store::usage_report`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct UsageReport {
    pub groups: Vec<UsageGroup>,
    /// Summen je Abrechnungsart (höchstens zwei Einträge, nie zusammengerechnet).
    pub totals: Vec<UsageGroup>,
    pub windows: Vec<SubscriptionWindow>,
    pub reporting: Vec<HarnessReporting>,
}

/// SQLite-Modifier für den Versatz, z. B. `+7200 seconds`.
fn offset_modifier(offset_s: i32) -> String {
    format!("{offset_s:+} seconds")
}

const BILLING: &str =
    "CASE WHEN r.auth_source = 'vendor_cli' THEN 'subscription' ELSE 'metered' END";

impl Store {
    /// Summen der Usage-Records im Zeitraum, gruppiert und je Abrechnungsart getrennt
    /// (USE-005 AC1), dazu Abo-Fenster und Melde-Stand je Harness.
    pub async fn usage_report(&self, org: OrgId, q: &UsageQuery) -> Result<UsageReport> {
        let groups = self.usage_groups(org, q, &q.group_by).await?;
        let totals = self.usage_groups(org, q, &[]).await?;
        let windows = self.subscription_windows(org, q.harness.as_deref()).await?;
        let reporting = self.harness_reporting(org, q, &windows).await?;
        Ok(UsageReport {
            groups,
            totals,
            windows,
            reporting,
        })
    }

    async fn usage_groups(
        &self,
        org: OrgId,
        q: &UsageQuery,
        dims: &[UsageDimension],
    ) -> Result<Vec<UsageGroup>> {
        let mut dims = dims.to_vec();
        dims.sort();
        dims.dedup();
        let mut select = String::new();
        let mut group = Vec::new();
        for d in &dims {
            select.push_str(&format!("{} AS g_{}, ", d.column(q), d.as_str()));
            group.push(format!("g_{}", d.as_str()));
            if *d == UsageDimension::Session {
                select.push_str("MAX(s.title) AS session_title, ");
            }
        }
        group.push("billing".into());
        // ?1 Versatz, ?2 Org, ?3 from, ?4 to, danach die Filter.
        let mut sql = format!(
            "SELECT {select}{BILLING} AS billing, COUNT(*) AS turns, \
             COUNT(DISTINCT r.session_id) AS sessions, \
             SUM(r.input_tokens) AS input_tokens, SUM(r.output_tokens) AS output_tokens, \
             SUM(r.cache_read_tokens) AS cache_read_tokens, \
             SUM(r.cache_write_tokens) AS cache_write_tokens, \
             COALESCE(SUM(CASE WHEN r.auth_source <> 'vendor_cli' THEN r.cost_micro END), 0) \
               AS cost_micro, \
             COALESCE(SUM(CASE WHEN r.auth_source = 'vendor_cli' THEN r.equivalent_micro END), 0) \
               AS equivalent_micro, \
             SUM(CASE WHEN r.cost_source = 'none' THEN 1 ELSE 0 END) AS unpriced \
             FROM usage_records r LEFT JOIN sessions s ON s.id = r.session_id \
             AND s.org_id = r.org_id \
             WHERE r.org_id = ?2 AND r.ts >= ?3 AND r.ts < ?4"
        );
        let mut binds = Vec::new();
        filters(q, false, 5, &mut sql, &mut binds);
        sql.push_str(&format!(" GROUP BY {g} ORDER BY {g}", g = group.join(", ")));
        let mut query = sqlx::query(&sql)
            .bind(offset_modifier(q.utc_offset_s))
            .bind(org.to_string())
            .bind(q.from.to_string())
            .bind(q.to.to_string());
        for b in &binds {
            query = query.bind(b);
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                let has = |d: UsageDimension| dims.contains(&d);
                let billing: Billing = enum_from_str(row.try_get("billing")?)?;
                let text = |col: &str| -> Result<Option<String>> { Ok(row.try_get(col)?) };
                let subscription = billing == Billing::Subscription;
                Ok(UsageGroup {
                    day: if has(UsageDimension::Day) {
                        text("g_day")?
                    } else {
                        None
                    },
                    session_id: if has(UsageDimension::Session) {
                        text("g_session")?.as_deref().map(parse).transpose()?
                    } else {
                        None
                    },
                    session_title: if has(UsageDimension::Session) {
                        text("session_title")?
                    } else {
                        None
                    },
                    project_id: if has(UsageDimension::Project) {
                        text("g_project")?.as_deref().map(parse).transpose()?
                    } else {
                        None
                    },
                    harness: if has(UsageDimension::Harness) {
                        text("g_harness")?
                    } else {
                        None
                    },
                    model: if has(UsageDimension::Model) {
                        text("g_model")?
                    } else {
                        None
                    },
                    auth_source: if has(UsageDimension::AuthSource) {
                        text("g_auth_source")?
                            .as_deref()
                            .map(enum_from_str)
                            .transpose()?
                    } else {
                        None
                    },
                    billing: Some(billing),
                    turns: to_u64(row.try_get("turns")?)?,
                    sessions: to_u64(row.try_get("sessions")?)?,
                    input_tokens: to_u64(row.try_get("input_tokens")?)?,
                    output_tokens: to_u64(row.try_get("output_tokens")?)?,
                    cache_read_tokens: to_u64(row.try_get("cache_read_tokens")?)?,
                    cache_write_tokens: to_u64(row.try_get("cache_write_tokens")?)?,
                    cost_micro: (!subscription)
                        .then(|| row.try_get("cost_micro"))
                        .transpose()?,
                    equivalent_micro: subscription
                        .then(|| row.try_get("equivalent_micro"))
                        .transpose()?,
                    unpriced: to_u64(row.try_get("unpriced")?)?,
                })
            })
            .collect()
    }

    /// Jüngster Stand je Harness, Anbieter und Fenster über alle Sessions der Org.
    pub async fn subscription_windows(
        &self,
        org: OrgId,
        harness: Option<&str>,
    ) -> Result<Vec<SubscriptionWindow>> {
        let mut sql = String::from(
            "SELECT w.harness, w.vendor, w.window_name, w.used_pct, w.resets_at, w.ts, \
             w.session_id FROM usage_subscription_windows w WHERE w.org_id = ?1 \
             AND w.ts = (SELECT MAX(x.ts) FROM usage_subscription_windows x \
               WHERE x.org_id = w.org_id AND x.harness = w.harness AND x.vendor = w.vendor \
               AND x.window_name = w.window_name)",
        );
        if harness.is_some() {
            sql.push_str(" AND w.harness = ?2");
        }
        sql.push_str(" ORDER BY w.harness, w.vendor, w.window_name, w.session_id");
        let mut query = sqlx::query(&sql).bind(org.to_string());
        if let Some(h) = harness {
            query = query.bind(h);
        }
        let mut out: Vec<SubscriptionWindow> = Vec::new();
        for row in query.fetch_all(&self.pool).await? {
            let w = SubscriptionWindow {
                harness: row.try_get("harness")?,
                vendor: row.try_get("vendor")?,
                window: row.try_get("window_name")?,
                used_pct: row.try_get("used_pct")?,
                resets_at: parse(row.try_get("resets_at")?)?,
                observed_at: parse(row.try_get("ts")?)?,
                session_id: parse(row.try_get("session_id")?)?,
            };
            // Gleichzeitige Meldungen mehrerer Sessions: eine genügt.
            if out.last().is_some_and(|p| {
                (&p.harness, &p.vendor, &p.window) == (&w.harness, &w.vendor, &w.window)
            }) {
                continue;
            }
            out.push(w);
        }
        Ok(out)
    }

    /// Harnesses mit Sessions oder Records im Zeitraum und was sie gemeldet haben.
    async fn harness_reporting(
        &self,
        org: OrgId,
        q: &UsageQuery,
        windows: &[SubscriptionWindow],
    ) -> Result<Vec<HarnessReporting>> {
        // Sessions mit Aktivität im Zeitraum; aus Dateien importierte haben nie Usage (DATA-010).
        let mut sql = String::from(
            "SELECT s.harness AS harness, COUNT(*) AS sessions FROM sessions s \
             WHERE s.org_id = ?1 AND s.created_at < ?3 AND s.last_activity_at >= ?2 \
             AND NOT EXISTS (SELECT 1 FROM session_file_imports f WHERE f.session_id = s.id)",
        );
        let mut binds = Vec::new();
        filters(q, true, 4, &mut sql, &mut binds);
        sql.push_str(" GROUP BY s.harness");
        let mut query = sqlx::query(&sql)
            .bind(org.to_string())
            .bind(q.from.to_string())
            .bind(q.to.to_string());
        for b in &binds {
            query = query.bind(b);
        }
        let mut out: std::collections::BTreeMap<String, HarnessReporting> = Default::default();
        for row in query.fetch_all(&self.pool).await? {
            let harness: String = row.try_get("harness")?;
            out.insert(
                harness.clone(),
                HarnessReporting {
                    harness,
                    sessions: to_u64(row.try_get("sessions")?)?,
                    tokens: false,
                    rate_limits: false,
                },
            );
        }
        let mut sql = String::from(
            "SELECT DISTINCT r.harness AS harness FROM usage_records r \
             WHERE r.org_id = ?1 AND r.ts >= ?2 AND r.ts < ?3",
        );
        let mut binds = Vec::new();
        filters(q, false, 4, &mut sql, &mut binds);
        let mut query = sqlx::query(&sql)
            .bind(org.to_string())
            .bind(q.from.to_string())
            .bind(q.to.to_string());
        for b in &binds {
            query = query.bind(b);
        }
        for row in query.fetch_all(&self.pool).await? {
            let harness: String = row.try_get("harness")?;
            out.entry(harness.clone())
                .or_insert_with(|| HarnessReporting {
                    harness,
                    sessions: 0,
                    tokens: false,
                    rate_limits: false,
                })
                .tokens = true;
        }
        for r in out.values_mut() {
            r.rate_limits = windows.iter().any(|w| w.harness == r.harness);
        }
        Ok(out.into_values().collect())
    }

    /// Öffnet die lokale Datenbank nur lesend, ohne Migrationen und ohne Dateien anzulegen
    /// (USE-007 AC3: `beton usage` ohne Daemon). `None`, wenn es noch keine Datenbank gibt.
    /// Ein älteres Schema wird abgelehnt (der Daemon migriert beim Start).
    pub async fn open_read_only(data_dir: &Path) -> Result<Option<Self>> {
        let db_path = data_dir.join("beton.db");
        if !db_path.exists() {
            return Ok(None);
        }
        // Ohne `-wal` läuft kein Schreiber: `immutable` liest dann, ohne `-shm`/`-wal` anzulegen.
        let mut wal = db_path.as_os_str().to_owned();
        wal.push("-wal");
        let idle = !Path::new(&wal).exists();
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .read_only(true)
            .immutable(idle)
            .create_if_missing(false)
            .busy_timeout(std::time::Duration::from_millis(5000));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;
        let version: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(version) FROM schema_migrations /* org-übergreifend: Schema-Version */",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(None);
        let version = version.unwrap_or(0);
        if version != crate::SCHEMA_VERSION {
            pool.close().await;
            return Err(if version > crate::SCHEMA_VERSION {
                Error::SchemaTooNew {
                    db: version,
                    binary: crate::SCHEMA_VERSION,
                }
            } else {
                Error::SchemaOutdated {
                    db: version,
                    binary: crate::SCHEMA_VERSION,
                }
            });
        }
        Ok(Some(Self {
            pool,
            blobs: BlobStore::unopened(&data_dir.join("blobs")),
            options: StoreOptions::default(),
        }))
    }
}

/// Filter über `usage_records r` bzw. `sessions s` (`sessions: true`). Platzhalter sind
/// nummeriert ab `?{first}`: sqlx zählt namenlose `?` unabhängig von nummerierten.
fn filters(
    q: &UsageQuery,
    sessions: bool,
    first: usize,
    sql: &mut String,
    binds: &mut Vec<String>,
) {
    let (project, harness, user, session) = if sessions {
        ("s.project_id", "s.harness", "s.owner_id", "s.id")
    } else {
        ("r.project_id", "r.harness", "r.user_id", "r.session_id")
    };
    let mut push = |column: &str, value: String| {
        sql.push_str(&format!(" AND {column} = ?{}", first + binds.len()));
        binds.push(value);
    };
    if let Some(p) = q.project {
        push(project, p.to_string());
    }
    if let Some(h) = &q.harness {
        push(harness, h.clone());
    }
    if let Some(u) = q.user {
        push(user, u.to_string());
    }
    if let Some(id) = q.session {
        push(session, id.to_string());
    }
}

/// `usage.subscription` in die Projektion der Abo-Fenster (aufgerufen aus
/// `projections::apply`). Der Harness kommt aus der Session.
pub(crate) async fn index_window(
    conn: &mut sqlx::SqliteConnection,
    org: OrgId,
    session: SessionId,
    seq: u64,
    ts: Timestamp,
    u: &beton_core::event::UsageSubscription,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO usage_subscription_windows (org_id, session_id, harness, vendor, \
         window_name, used_pct, resets_at, seq, ts) \
         SELECT ?, ?, harness, ?, ?, ?, ?, ?, ? FROM sessions WHERE org_id = ? AND id = ? \
         ON CONFLICT (session_id, vendor, window_name) DO UPDATE SET \
           used_pct = excluded.used_pct, resets_at = excluded.resets_at, seq = excluded.seq, \
           ts = excluded.ts \
         WHERE excluded.seq > usage_subscription_windows.seq",
    )
    .bind(org.to_string())
    .bind(session.to_string())
    .bind(&u.vendor)
    .bind(beton_core::usage::window_name(&u.window))
    .bind(u.used_pct)
    .bind(u.resets_at.to_string())
    .bind(crate::sessions::to_i64(seq)?)
    .bind(ts.to_string())
    .bind(org.to_string())
    .bind(session.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}
