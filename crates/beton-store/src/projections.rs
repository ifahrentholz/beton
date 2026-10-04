//! Projektionen und Read-Modelle (DATA-005).
//!
//! Kernprojektionen werden in derselben Transaktion wie der Event-Append aktualisiert und
//! lassen sich jederzeit aus dem Log neu aufbauen (`beton admin projections rebuild`):
//! - Session-Liste: Titel, Status, Archiv-Flag, Kostensumme, letzte Aktivität (`sessions`)
//! - offene Approvals (`approvals`)
//! - Usage-Aggregate Tag × Harness × Modell (`usage_daily`, je Session für saubere Rebuilds)
//!
//! Kommentare und Inbox (COL-005, COL-009) kommen mit ihren Features dazu.

use beton_core::event::{ApprovalDecision, ApprovalKind, EventPayload, SessionStatus};
use beton_core::id::{ApprovalId, OrgId, SessionId};
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
pub struct UsageRecord {
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
            sqlx::query("UPDATE sessions SET title = ? WHERE org_id = ? AND id = ?")
                .bind(&t.title)
                .bind(&org_s)
                .bind(&session_s)
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
                 WHERE org_id = ? AND id = ?",
            )
            .bind(enum_to_str(&r.decision))
            .bind(to_i64(seq)?)
            .bind(&org_s)
            .bind(r.approval_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::CostDelta(c) => {
            let cost = c.cost_micro.unwrap_or(0);
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
        }
        _ => {}
    }
    Ok(())
}

impl Store {
    /// Baut die Projektionen aus dem Log neu auf: für eine Session oder die ganze Org.
    /// Gibt die Zahl der neu aufgebauten Sessions zurück.
    pub async fn rebuild_projections(&self, org: OrgId, session: Option<SessionId>) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
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
                "UPDATE sessions SET title = '', status = ?, archived = 0, cost_micro = 0, \
                 last_activity_at = created_at WHERE org_id = ? AND id = ?",
            )
            .bind(enum_to_str(&SessionStatus::default()))
            .bind(org.to_string())
            .bind(id)
            .execute(&mut *tx)
            .await?;
            for sql in [
                "DELETE FROM approvals WHERE org_id = ? AND session_id = ?",
                "DELETE FROM usage_daily WHERE org_id = ? AND session_id = ?",
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
    ) -> Result<Vec<UsageRecord>> {
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
            Ok(UsageRecord {
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

    /// Vollständiger, sortierter Inhalt aller Projektionen einer Org (Vergleich vor/nach Rebuild,
    /// Diagnose).
    pub async fn projection_dump(&self, org: OrgId) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for sql in [
            "SELECT id, title, status, archived, cost_micro, last_activity_at FROM sessions \
             WHERE org_id = ? ORDER BY id",
            "SELECT * FROM approvals WHERE org_id = ? ORDER BY id",
            "SELECT * FROM usage_daily WHERE org_id = ? ORDER BY session_id, day, harness, model",
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
