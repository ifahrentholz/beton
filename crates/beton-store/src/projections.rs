//! Projektionen und Read-Modelle (DATA-005).
//!
//! Kernprojektionen werden in derselben Transaktion wie der Event-Append aktualisiert und
//! lassen sich jederzeit aus dem Log neu aufbauen (`beton admin projections rebuild`):
//! - Session-Liste: Titel, Status, Archiv-Flag, Kostensumme, letzte Aktivität, Worktree
//!   (`sessions`)
//! - offene Approvals (`approvals`)
//! - Usage-Records je `cost.delta` (`usage_records`, USE-001) mit Preisquelle und
//!   Katalogversion; Grundlage für Usage-Ansichten und die Policy-Variablen `session.cost_usd`
//!   und `user.daily_cost_usd`
//! - Usage-Aggregate Tag × Harness × Modell (`usage_daily`, je Session für saubere Rebuilds)
//! - Suchdokumente für die Volltextsuche (`search_docs` mit FTS5-Index, SES-012)
//!
//! Die Inbox (UX-001) projiziert `crate::inbox`; Kommentare (COL-005, COL-009) kommen mit ihren Features dazu.

use beton_core::event::{
    ApprovalDecision, ApprovalKind, AuthSource, EventPayload, PriceSource, SessionStatus,
};
use beton_core::id::{ApprovalId, OrgId, ProjectId, SessionId, UserId};
use beton_core::pricing;
use beton_core::time::Timestamp;
use serde::Serialize;
use sqlx::{Row, SqliteConnection};

use crate::Store;
use crate::error::Result;
use crate::sessions::{enum_from_str, enum_to_str, parse, to_i64, to_u64};

/// Ein offener Approval aus der Projektion.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalRecord {
    pub id: ApprovalId,
    pub session_id: SessionId,
    pub requested_seq: u64,
    pub kind: ApprovalKind,
    pub subject: serde_json::Value,
    pub options: Vec<String>,
    pub expires_at: Timestamp,
    pub status: String,
    pub decision: Option<ApprovalDecision>,
}

/// Usage einer Org an einem Tag je Harness und Modell (summiert über Sessions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageDaily {
    pub day: String,
    pub harness: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_micro: i64,
    pub events: u64,
}

/// Ein Usage-Record (USE-001): ein `cost.delta`, normalisiert und bewertet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageRecord {
    pub session_id: SessionId,
    pub seq: u64,
    pub ts: Timestamp,
    /// Kalendertag in UTC (`YYYY-MM-DD`); den Tag in der Zeitzone des Users ergibt `ts`.
    pub day: String,
    pub project_id: Option<ProjectId>,
    pub user_id: UserId,
    pub harness: String,
    pub model: String,
    pub auth_source: AuthSource,
    pub purpose: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Geldbetrag; leer bei `cost_source` `none` und `subscription`.
    pub cost_micro: Option<i64>,
    pub cost_source: PriceSource,
    pub pricing_version: Option<String>,
    /// API-Äquivalent einer Subscription; nur informativ.
    pub equivalent_micro: Option<i64>,
}

/// Summen über Usage-Records. Geld und API-Äquivalent bleiben getrennt (ADR-0034).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageTotals {
    pub records: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Summe der Geldbeträge (ohne `vendor_cli`, USE-001 AC3).
    pub cost_micro: i64,
    /// Summe der API-Äquivalente (nur informativ).
    pub equivalent_micro: i64,
    /// Records mit `cost_source: none` („ohne Preis“).
    pub unpriced: u64,
}

/// Auswahl für [`Store::usage_totals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageScope {
    /// Alle Records der Org.
    Org,
    /// Records einer Session (`session.cost_usd`).
    Session(SessionId),
    /// Records eines Users im Zeitraum `[from, to)`, z. B. ein Kalendertag in seiner
    /// Zeitzone (`user.daily_cost_usd`).
    UserBetween(UserId, Timestamp, Timestamp),
}

/// Wendet ein dauerhaftes Event auf die Projektionen an.
pub(crate) async fn apply(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    seq: u64,
    ts: Timestamp,
    payload: &EventPayload,
) -> Result<()> {
    let org_s = org.to_string();
    let session_s = session.to_string();
    let ts_s = ts.to_string();
    sqlx::query("UPDATE sessions SET last_activity_at = ? WHERE org_id = ? AND id = ?")
        .bind(&ts_s)
        .bind(&org_s)
        .bind(&session_s)
        .execute(&mut *conn)
        .await?;
    match payload {
        EventPayload::SessionStatus(s) => {
            sqlx::query("UPDATE sessions SET status = ? WHERE org_id = ? AND id = ?")
                .bind(enum_to_str(&s.status))
                .bind(&org_s)
                .bind(&session_s)
                .execute(&mut *conn)
                .await?;
        }
        EventPayload::SessionTitleChanged(t) => {
            // Ein Titel des Users bleibt: generierte Titel ersetzen ihn nie (SES-010, UX-009).
            sqlx::query(
                "UPDATE sessions SET title = ?, title_source = ? WHERE org_id = ? AND id = ? \
                 AND NOT (? = 'generated' AND title_source = 'user')",
            )
            .bind(&t.title)
            .bind(enum_to_str(&t.source))
            .bind(&org_s)
            .bind(&session_s)
            .bind(enum_to_str(&t.source))
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::SessionArchived(_) | EventPayload::SessionUnarchived(_) => {
            let archived = matches!(payload, EventPayload::SessionArchived(_));
            sqlx::query("UPDATE sessions SET archived = ? WHERE org_id = ? AND id = ?")
                .bind(archived)
                .bind(&org_s)
                .bind(&session_s)
                .execute(&mut *conn)
                .await?;
        }
        EventPayload::GitWorktreeCreated(w) => {
            sqlx::query(
                "UPDATE sessions SET worktree_path = ?, worktree_branch = ?, worktree_base = ?, \
                 worktree_base_sha = ? WHERE org_id = ? AND id = ?",
            )
            .bind(&w.path)
            .bind(&w.branch)
            .bind(&w.base)
            .bind(&w.base_sha)
            .bind(&org_s)
            .bind(&session_s)
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::ApprovalRequested(a) => {
            sqlx::query(
                "INSERT INTO approvals (id, org_id, session_id, requested_seq, kind, subject, \
                 options, expires_at, on_timeout, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'open')",
            )
            .bind(a.approval_id.to_string())
            .bind(&org_s)
            .bind(&session_s)
            .bind(to_i64(seq)?)
            .bind(enum_to_str(&a.kind))
            .bind(a.subject.to_string())
            .bind(serde_json::to_string(&a.options)?)
            .bind(a.expires_at.to_string())
            .bind(enum_to_str(&a.on_timeout))
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::ApprovalResolved(r) => {
            sqlx::query(
                "UPDATE approvals SET status = 'resolved', decision = ?, resolved_seq = ? \
                 WHERE org_id = ? AND session_id = ? AND id = ?",
            )
            .bind(enum_to_str(&r.decision))
            .bind(to_i64(seq)?)
            .bind(&org_s)
            .bind(&session_s)
            .bind(r.approval_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::CostDelta(c) => {
            // Geld nie aus Subscriptions (USE-001 AC3); ältere Events ohne Bewertung werden
            // eingestuft, aber nicht neu bewertet.
            let money = pricing::money_micro(c);
            let cost = money.unwrap_or(0);
            sqlx::query(
                "UPDATE sessions SET cost_micro = cost_micro + ? WHERE org_id = ? AND id = ?",
            )
            .bind(cost)
            .bind(&org_s)
            .bind(&session_s)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "INSERT INTO usage_daily (org_id, session_id, day, harness, model, input_tokens, \
                 output_tokens, cache_read_tokens, cache_write_tokens, cost_micro, events) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1) \
                 ON CONFLICT (session_id, day, harness, model) DO UPDATE SET \
                   input_tokens = input_tokens + excluded.input_tokens, \
                   output_tokens = output_tokens + excluded.output_tokens, \
                   cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens, \
                   cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens, \
                   cost_micro = cost_micro + excluded.cost_micro, \
                   events = events + 1 \
                 WHERE usage_daily.org_id = excluded.org_id",
            )
            .bind(&org_s)
            .bind(&session_s)
            .bind(&ts_s[..10])
            .bind(&c.harness)
            .bind(&c.model)
            .bind(to_i64(c.input_tokens)?)
            .bind(to_i64(c.output_tokens)?)
            .bind(to_i64(c.cache_read_tokens)?)
            .bind(to_i64(c.cache_write_tokens)?)
            .bind(cost)
            .execute(&mut *conn)
            .await?;
            // Usage-Record (USE-001): Projekt und User aus der Session.
            sqlx::query(
                "INSERT INTO usage_records (org_id, session_id, seq, ts, day, project_id, user_id, \
                 harness, model, auth_source, purpose, input_tokens, output_tokens, \
                 cache_read_tokens, cache_write_tokens, cost_micro, cost_source, pricing_version, \
                 equivalent_micro) \
                 SELECT ?, ?, ?, ?, ?, project_id, owner_id, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ? \
                 FROM sessions WHERE org_id = ? AND id = ?",
            )
            .bind(&org_s)
            .bind(&session_s)
            .bind(to_i64(seq)?)
            .bind(&ts_s)
            .bind(&ts_s[..10])
            .bind(&c.harness)
            .bind(&c.model)
            .bind(enum_to_str(&c.auth_source))
            .bind(&c.purpose)
            .bind(to_i64(c.input_tokens)?)
            .bind(to_i64(c.output_tokens)?)
            .bind(to_i64(c.cache_read_tokens)?)
            .bind(to_i64(c.cache_write_tokens)?)
            .bind(money)
            .bind(enum_to_str(&pricing::effective_source(c)))
            .bind(&c.pricing_version)
            .bind(c.equivalent_micro)
            .bind(&org_s)
            .bind(&session_s)
            .execute(&mut *conn)
            .await?;
        }
        _ => {}
    }
    crate::inbox::index_event(conn, org, session, seq, ts, payload).await?;
    crate::listing::index_event(conn, org, session, seq, payload).await
}

impl Store {
    /// Baut die Projektionen aus dem Log neu auf: für eine Session oder die ganze Org.
    /// Gibt die Zahl der neu aufgebauten Sessions zurück.
    pub async fn rebuild_projections(&self, org: OrgId, session: Option<SessionId>) -> Result<u64> {
        let mut tx = self.write_tx().await?;
        let ids: Vec<String> = match session {
            Some(id) => {
                crate::sessions::session_exists(&mut tx, org, id).await?;
                vec![id.to_string()]
            }
            None => {
                sqlx::query_scalar("SELECT id FROM sessions WHERE org_id = ? ORDER BY id")
                    .bind(org.to_string())
                    .fetch_all(&mut *tx)
                    .await?
            }
        };
        for id in &ids {
            let session: SessionId = parse(id)?;
            sqlx::query(
                "UPDATE sessions SET title = '', title_source = '', status = ?, archived = 0, cost_micro = 0, \
                 last_activity_at = created_at, worktree_path = NULL, worktree_branch = NULL, \
                 worktree_base = NULL, worktree_base_sha = NULL WHERE org_id = ? AND id = ?",
            )
            .bind(enum_to_str(&SessionStatus::default()))
            .bind(org.to_string())
            .bind(id)
            .execute(&mut *tx)
            .await?;
            crate::listing::purge_docs(&mut tx, org, id).await?;
            for sql in [
                "DELETE FROM approvals WHERE org_id = ? AND session_id = ?",
                "DELETE FROM inbox_items WHERE org_id = ? AND session_id = ?",
                "DELETE FROM usage_daily WHERE org_id = ? AND session_id = ?",
                "DELETE FROM usage_records WHERE org_id = ? AND session_id = ?",
            ] {
                sqlx::query(sql)
                    .bind(org.to_string())
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            for (seq, ts, payload) in self.full_events(&mut tx, org, session).await? {
                apply(&mut tx, org, session, seq, ts, &payload).await?;
            }
            if crate::export::is_file_import(&mut tx, org, id).await? {
                crate::export::seal_imported(&mut tx, org, session).await?;
            }
        }
        tx.commit().await?;
        tracing::info!(sessions = ids.len(), "Projektionen neu aufgebaut");
        Ok(ids.len() as u64)
    }

    /// Offene Approvals der Org, älteste zuerst.
    pub async fn open_approvals(&self, org: OrgId) -> Result<Vec<ApprovalRecord>> {
        sqlx::query(
            "SELECT id, session_id, requested_seq, kind, subject, options, expires_at, status, \
             decision FROM approvals WHERE org_id = ? AND status = 'open' \
             ORDER BY expires_at, id",
        )
        .bind(org.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            let subject: String = row.try_get("subject")?;
            let options: String = row.try_get("options")?;
            let decision: Option<String> = row.try_get("decision")?;
            Ok(ApprovalRecord {
                id: parse(row.try_get("id")?)?,
                session_id: parse(row.try_get("session_id")?)?,
                requested_seq: to_u64(row.try_get("requested_seq")?)?,
                kind: enum_from_str(row.try_get("kind")?)?,
                subject: serde_json::from_str(&subject)?,
                options: serde_json::from_str(&options)?,
                expires_at: parse(row.try_get("expires_at")?)?,
                status: row.try_get("status")?,
                decision: decision.as_deref().map(enum_from_str).transpose()?,
            })
        })
        .collect()
    }

    /// Usage der Org je Tag, Harness und Modell im Zeitraum `[from_day, to_day]` (`YYYY-MM-DD`).
    pub async fn usage_daily(
        &self,
        org: OrgId,
        from_day: &str,
        to_day: &str,
    ) -> Result<Vec<UsageDaily>> {
        sqlx::query(
            "SELECT day, harness, model, SUM(input_tokens) AS input_tokens, \
             SUM(output_tokens) AS output_tokens, SUM(cache_read_tokens) AS cache_read_tokens, \
             SUM(cache_write_tokens) AS cache_write_tokens, SUM(cost_micro) AS cost_micro, \
             SUM(events) AS events FROM usage_daily \
             WHERE org_id = ? AND day >= ? AND day <= ? \
             GROUP BY day, harness, model ORDER BY day, harness, model",
        )
        .bind(org.to_string())
        .bind(from_day)
        .bind(to_day)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            Ok(UsageDaily {
                day: row.try_get("day")?,
                harness: row.try_get("harness")?,
                model: row.try_get("model")?,
                input_tokens: to_u64(row.try_get("input_tokens")?)?,
                output_tokens: to_u64(row.try_get("output_tokens")?)?,
                cache_read_tokens: to_u64(row.try_get("cache_read_tokens")?)?,
                cache_write_tokens: to_u64(row.try_get("cache_write_tokens")?)?,
                cost_micro: row.try_get("cost_micro")?,
                events: to_u64(row.try_get("events")?)?,
            })
        })
        .collect()
    }

    /// Usage-Records einer Session in Log-Reihenfolge (USE-001).
    pub async fn usage_records(&self, org: OrgId, session: SessionId) -> Result<Vec<UsageRecord>> {
        sqlx::query("SELECT * FROM usage_records WHERE org_id = ? AND session_id = ? ORDER BY seq")
            .bind(org.to_string())
            .bind(session.to_string())
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let project: Option<String> = row.try_get("project_id")?;
                Ok(UsageRecord {
                    session_id: parse(row.try_get("session_id")?)?,
                    seq: to_u64(row.try_get("seq")?)?,
                    ts: parse(row.try_get("ts")?)?,
                    day: row.try_get("day")?,
                    project_id: project.as_deref().map(parse).transpose()?,
                    user_id: parse(row.try_get("user_id")?)?,
                    harness: row.try_get("harness")?,
                    model: row.try_get("model")?,
                    auth_source: enum_from_str(row.try_get("auth_source")?)?,
                    purpose: row.try_get("purpose")?,
                    input_tokens: to_u64(row.try_get("input_tokens")?)?,
                    output_tokens: to_u64(row.try_get("output_tokens")?)?,
                    cache_read_tokens: to_u64(row.try_get("cache_read_tokens")?)?,
                    cache_write_tokens: to_u64(row.try_get("cache_write_tokens")?)?,
                    cost_micro: row.try_get("cost_micro")?,
                    cost_source: enum_from_str(row.try_get("cost_source")?)?,
                    pricing_version: row.try_get("pricing_version")?,
                    equivalent_micro: row.try_get("equivalent_micro")?,
                })
            })
            .collect()
    }

    /// Summen über die Usage-Records einer Auswahl. Geld enthält nie Subscriptions
    /// (USE-001 AC3); Quelle für `session.cost_usd` und `user.daily_cost_usd` (POL-004).
    pub async fn usage_totals(&self, org: OrgId, scope: UsageScope) -> Result<UsageTotals> {
        let (filter, binds): (&str, Vec<String>) = match scope {
            UsageScope::Org => ("", Vec::new()),
            UsageScope::Session(id) => (" AND session_id = ?", vec![id.to_string()]),
            UsageScope::UserBetween(user, from, to) => (
                " AND user_id = ? AND ts >= ? AND ts < ?",
                vec![user.to_string(), from.to_string(), to.to_string()],
            ),
        };
        let sql = format!(
            "SELECT COUNT(*) AS records, \
             COALESCE(SUM(input_tokens), 0) AS input_tokens, \
             COALESCE(SUM(output_tokens), 0) AS output_tokens, \
             COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens, \
             COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens, \
             COALESCE(SUM(CASE WHEN auth_source <> 'vendor_cli' THEN cost_micro END), 0) \
               AS cost_micro, \
             COALESCE(SUM(equivalent_micro), 0) AS equivalent_micro, \
             COALESCE(SUM(CASE WHEN cost_source = 'none' THEN 1 ELSE 0 END), 0) AS unpriced \
             FROM usage_records WHERE org_id = ?{filter}"
        );
        let mut query = sqlx::query(&sql).bind(org.to_string());
        for b in &binds {
            query = query.bind(b);
        }
        let row = query.fetch_one(&self.pool).await?;
        Ok(UsageTotals {
            records: to_u64(row.try_get("records")?)?,
            input_tokens: to_u64(row.try_get("input_tokens")?)?,
            output_tokens: to_u64(row.try_get("output_tokens")?)?,
            cache_read_tokens: to_u64(row.try_get("cache_read_tokens")?)?,
            cache_write_tokens: to_u64(row.try_get("cache_write_tokens")?)?,
            cost_micro: row.try_get("cost_micro")?,
            equivalent_micro: row.try_get("equivalent_micro")?,
            unpriced: to_u64(row.try_get("unpriced")?)?,
        })
    }

    /// Vollständiger, sortierter Inhalt aller Projektionen einer Org (Vergleich vor/nach Rebuild,
    /// Diagnose).
    pub async fn projection_dump(&self, org: OrgId) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for sql in [
            "SELECT id, title, status, archived, cost_micro, last_activity_at, worktree_path, \
             worktree_branch, worktree_base, worktree_base_sha FROM sessions \
             WHERE org_id = ? ORDER BY id",
            "SELECT * FROM approvals WHERE org_id = ? ORDER BY session_id, id",
            "SELECT * FROM inbox_items WHERE org_id = ? ORDER BY session_id, id",
            "SELECT * FROM usage_daily WHERE org_id = ? ORDER BY session_id, day, harness, model",
            "SELECT * FROM usage_records WHERE org_id = ? ORDER BY session_id, seq",
            "SELECT session_id, doc, body FROM search_docs WHERE org_id = ? \
             ORDER BY session_id, doc",
        ] {
            for row in sqlx::query(sql)
                .bind(org.to_string())
                .fetch_all(&self.pool)
                .await?
            {
                use sqlx::{Column, ValueRef};
                let cells: Vec<String> = row
                    .columns()
                    .iter()
                    .map(|c| {
                        let null = row
                            .try_get_raw(c.ordinal())
                            .map(|raw| raw.is_null())
                            .unwrap_or(false);
                        if null {
                            "NULL".into()
                        } else {
                            row.try_get::<String, _>(c.ordinal())
                                .or_else(|_| {
                                    row.try_get::<i64, _>(c.ordinal()).map(|v| v.to_string())
                                })
                                .unwrap_or_else(|e| format!("?{e}"))
                        }
                    })
                    .collect();
                out.push(cells.join("|"));
            }
        }
        Ok(out)
    }
}
