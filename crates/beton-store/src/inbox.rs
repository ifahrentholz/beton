//! Inbox-Projektion (UX-001) und Policy-Zustand des Users (POL-009).
//!
//! Die Inbox entsteht allein aus dem Event-Log und lässt sich je Session neu aufbauen
//! (UX-001 AC4): `approval.requested` wird zum offenen Eintrag (`approval` bzw. `question`),
//! `approval.resolved` – egal von welchem Client – macht ihn `done`, `policy.notified` wird zum
//! Hinweis (`notice`, Status `info`), ebenso `usage.rate_limit_high` (Abo-Fenster fast
//! aufgebraucht, USE-004 AC3). Der Zähler der Inbox sind die offenen Einträge.
//!
//! Der Zustand der Policy-Scopes `user` und `user_day` ist kein Projektionsstand, sondern Daten
//! des Users; ein Rebuild lässt ihn unberührt.

use std::collections::BTreeMap;

use beton_core::event::{ApprovalKind, EventPayload};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Row, SqliteConnection};

use crate::sessions::{enum_to_str, parse, to_i64, to_u64};
use crate::{Error, Result, Store};

/// Ein Eintrag der Inbox.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InboxRecord {
    pub id: String,
    pub session_id: SessionId,
    pub session_title: String,
    /// `approval`, `question` oder `notice`.
    pub kind: String,
    /// `open`, `done` oder `info` (Hinweis ohne Entscheidung).
    pub status: String,
    pub approval_id: Option<String>,
    pub title: String,
    pub summary: String,
    /// Freigaben: `subject`, `options`, `expires_at`, `on_timeout`, `approval_kind`;
    /// Hinweise: `rule_id`, `scope`, `set`, `channels`.
    pub detail: Value,
    pub created_seq: u64,
    pub created_at: Timestamp,
    pub resolved_seq: Option<u64>,
    pub resolved_at: Option<Timestamp>,
    /// `decision`, `via`, `actor`, `comment?`, `answer?`, `remember?`, `on_timeout_applied?`.
    pub resolution: Option<Value>,
}

/// Filter der Inbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InboxQuery {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub session: Option<SessionId>,
    pub limit: u32,
    /// Nur Einträge vor diesem (`created_at`, `id`) – Cursor der vorigen Seite.
    pub before: Option<(String, String)>,
}

fn item_id(session: SessionId, seq: u64) -> String {
    let s = session.to_string();
    format!("inb_{}_{seq}", s.strip_prefix("ses_").unwrap_or(&s))
}

fn text(v: &Value, max: usize) -> String {
    let s = v.as_str().unwrap_or_default();
    s.chars().take(max).collect()
}

/// Projiziert ein Event in die Inbox (aufgerufen aus `projections::apply`).
pub(crate) async fn index_event(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    seq: u64,
    ts: Timestamp,
    payload: &EventPayload,
) -> Result<()> {
    match payload {
        EventPayload::ApprovalRequested(a) => {
            let question = a.kind == ApprovalKind::Question;
            let subject = &a.subject;
            let title = match subject["title"].as_str() {
                Some(t) if !t.is_empty() => text(&subject["title"], 200),
                _ if question => "Frage des Agents".into(),
                _ => "Freigabe nötig".into(),
            };
            let summary = [
                &subject["summary"],
                &subject["question"],
                &subject["tool"]["name"],
            ]
            .into_iter()
            .find(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .map(|v| text(v, 500))
            .or_else(|| subject["tool"].as_str().map(str::to_owned))
            .unwrap_or_default();
            let detail = json!({
                "approval_kind": enum_to_str(&a.kind),
                "subject": subject,
                "options": a.options,
                "expires_at": a.expires_at,
                "on_timeout": enum_to_str(&a.on_timeout),
            });
            sqlx::query(
                "INSERT INTO inbox_items (id, org_id, session_id, kind, status, approval_id, \
                 title, summary, detail, created_seq, created_at) \
                 VALUES (?, ?, ?, ?, 'open', ?, ?, ?, ?, ?, ?)",
            )
            .bind(item_id(session, seq))
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(if question { "question" } else { "approval" })
            .bind(a.approval_id.to_string())
            .bind(title)
            .bind(summary)
            .bind(detail.to_string())
            .bind(to_i64(seq)?)
            .bind(ts.to_string())
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::ApprovalResolved(r) => {
            let mut resolution = json!({
                "decision": enum_to_str(&r.decision),
                "via": enum_to_str(&r.via),
                "actor": r.actor,
            });
            if let Some(c) = &r.comment {
                resolution["comment"] = json!(c);
            }
            if let Some(a) = &r.answer {
                resolution["answer"] = json!(a);
            }
            if let Some(m) = &r.remember {
                resolution["remember"] = json!(enum_to_str(m));
            }
            if let Some(t) = &r.on_timeout_applied {
                resolution["on_timeout_applied"] = json!(enum_to_str(t));
            }
            sqlx::query(
                "UPDATE inbox_items SET status = 'done', resolved_seq = ?, resolved_at = ?, \
                 resolution = ? WHERE org_id = ? AND session_id = ? AND approval_id = ? \
                 AND status = 'open'",
            )
            .bind(to_i64(seq)?)
            .bind(ts.to_string())
            .bind(resolution.to_string())
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(r.approval_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::PolicyNotified(n) => {
            let message = n.message.clone().unwrap_or_default();
            let title = if message.is_empty() {
                format!("Hinweis aus Regel {}", n.rule_id)
            } else {
                message.chars().take(200).collect()
            };
            sqlx::query(
                "INSERT INTO inbox_items (id, org_id, session_id, kind, status, title, summary, \
                 detail, created_seq, created_at) VALUES (?, ?, ?, 'notice', 'info', ?, ?, ?, ?, ?)",
            )
            .bind(item_id(session, seq))
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(title)
            .bind(message.chars().take(2000).collect::<String>())
            .bind(
                json!({"rule_id": n.rule_id, "scope": n.scope, "set": n.set, "channels": n.channels})
                    .to_string(),
            )
            .bind(to_i64(seq)?)
            .bind(ts.to_string())
            .execute(&mut *conn)
            .await?;
        }
        EventPayload::UsageRateLimitHigh(u) => {
            let window = beton_core::usage::window_name(&u.window);
            let harness: Option<String> =
                sqlx::query_scalar("SELECT harness FROM sessions WHERE org_id = ? AND id = ?")
                    .bind(org.to_string())
                    .bind(session.to_string())
                    .fetch_optional(&mut *conn)
                    .await?;
            let who = harness.unwrap_or_else(|| u.vendor.clone());
            let title = format!(
                "Subscription-Kontingent fast aufgebraucht: {who}, Fenster {window} zu {} %",
                pct(u.used_pct)
            );
            let summary = format!(
                "Das Fenster {window} setzt sich um {} (UTC) zurück; bis dahin kann die CLI \
                 Anfragen ablehnen.",
                u.resets_at
            );
            sqlx::query(
                "INSERT INTO inbox_items (id, org_id, session_id, kind, status, title, summary, \
                 detail, created_seq, created_at) VALUES (?, ?, ?, 'notice', 'info', ?, ?, ?, ?, ?)",
            )
            .bind(item_id(session, seq))
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(title)
            .bind(summary)
            .bind(
                json!({
                    "notification": "rate_limit_high",
                    "vendor": u.vendor,
                    "window": window,
                    "used_pct": u.used_pct,
                    "resets_at": u.resets_at,
                    "threshold_pct": u.threshold_pct,
                })
                .to_string(),
            )
            .bind(to_i64(seq)?)
            .bind(ts.to_string())
            .execute(&mut *conn)
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// Prozentwert wie in der UI: ganze Zahl, sonst eine Nachkommastelle mit Komma.
fn pct(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}").replace('.', ",")
    }
}

const COLUMNS: &str = "i.id, i.session_id, s.title AS session_title, i.kind, i.status, \
    i.approval_id, i.title, i.summary, i.detail, i.created_seq, i.created_at, i.resolved_seq, \
    i.resolved_at, i.resolution";

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<InboxRecord> {
    let json = |col: &str| -> Result<Option<Value>> {
        let raw: Option<String> = row.try_get(col)?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| Error::Corrupt(e.to_string())))
            .transpose()
    };
    let opt_seq = |col: &str| -> Result<Option<u64>> {
        let raw: Option<i64> = row.try_get(col)?;
        raw.map(to_u64).transpose()
    };
    let opt_ts = |col: &str| -> Result<Option<Timestamp>> {
        let raw: Option<String> = row.try_get(col)?;
        raw.map(|s| parse(&s)).transpose()
    };
    let session: String = row.try_get("session_id")?;
    let created: String = row.try_get("created_at")?;
    Ok(InboxRecord {
        id: row.try_get("id")?,
        session_id: parse(&session)?,
        session_title: row
            .try_get::<Option<String>, _>("session_title")?
            .unwrap_or_default(),
        kind: row.try_get("kind")?,
        status: row.try_get("status")?,
        approval_id: row.try_get("approval_id")?,
        title: row.try_get("title")?,
        summary: row.try_get("summary")?,
        detail: json("detail")?.unwrap_or(Value::Null),
        created_seq: to_u64(row.try_get("created_seq")?)?,
        created_at: parse(&created)?,
        resolved_seq: opt_seq("resolved_seq")?,
        resolved_at: opt_ts("resolved_at")?,
        resolution: json("resolution")?,
    })
}

impl Store {
    /// Einträge der Inbox, neueste zuerst.
    pub async fn inbox(&self, org: OrgId, q: &InboxQuery) -> Result<Vec<InboxRecord>> {
        let mut sql = format!(
            "SELECT {COLUMNS} FROM inbox_items i LEFT JOIN sessions s \
             ON s.id = i.session_id AND s.org_id = i.org_id WHERE i.org_id = ?"
        );
        if q.status.is_some() {
            sql.push_str(" AND i.status = ?");
        }
        if q.kind.is_some() {
            sql.push_str(" AND i.kind = ?");
        }
        if q.session.is_some() {
            sql.push_str(" AND i.session_id = ?");
        }
        if q.before.is_some() {
            sql.push_str(" AND (i.created_at < ? OR (i.created_at = ? AND i.id < ?))");
        }
        sql.push_str(" ORDER BY i.created_at DESC, i.id DESC LIMIT ?");
        let mut query = sqlx::query(&sql).bind(org.to_string());
        if let Some(s) = &q.status {
            query = query.bind(s);
        }
        if let Some(k) = &q.kind {
            query = query.bind(k);
        }
        if let Some(s) = q.session {
            query = query.bind(s.to_string());
        }
        if let Some((at, id)) = &q.before {
            query = query.bind(at).bind(at).bind(id);
        }
        let limit = if q.limit == 0 { 100 } else { q.limit.min(500) };
        query = query.bind(i64::from(limit));
        query
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(from_row)
            .collect()
    }

    /// Ein Eintrag der Inbox.
    pub async fn inbox_item(&self, org: OrgId, id: &str) -> Result<InboxRecord> {
        let sql = format!(
            "SELECT {COLUMNS} FROM inbox_items i LEFT JOIN sessions s \
             ON s.id = i.session_id AND s.org_id = i.org_id WHERE i.org_id = ? AND i.id = ?"
        );
        let row = sqlx::query(&sql)
            .bind(org.to_string())
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Inbox-Eintrag {id}")))?;
        from_row(&row)
    }

    /// Gibt es den Hinweis „Abo-Fenster fast aufgebraucht“ für dieses Fenster und diesen
    /// Reset-Zeitpunkt schon (USE-004 AC3: einmal je Fenster und Periode)?
    pub async fn rate_limit_notice_exists(
        &self,
        org: OrgId,
        vendor: &str,
        window: &str,
        resets_at: Timestamp,
    ) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inbox_items WHERE org_id = ? AND kind = 'notice' \
             AND json_extract(detail, '$.notification') = 'rate_limit_high' \
             AND json_extract(detail, '$.vendor') = ? AND json_extract(detail, '$.window') = ? \
             AND json_extract(detail, '$.resets_at') = ?",
        )
        .bind(org.to_string())
        .bind(vendor)
        .bind(beton_core::usage::window_name(window))
        .bind(resets_at.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Zähler der Inbox: offene Freigaben und Fragen (UX-001 AC3).
    pub async fn inbox_open_count(&self, org: OrgId) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inbox_items WHERE org_id = ? AND status = 'open'",
        )
        .bind(org.to_string())
        .fetch_one(&self.pool)
        .await?;
        to_u64(n)
    }

    /// Zustand der Policy-Scopes `user_day` (für `day`) und `user` eines Users (POL-009).
    pub async fn policy_user_state(
        &self,
        org: OrgId,
        user: UserId,
        day: &str,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, Value>)> {
        let rows = sqlx::query(
            "SELECT scope, key, value FROM policy_user_state WHERE org_id = ? AND user_id = ? \
             AND ((scope = 'user_day' AND day = ?) OR (scope = 'user' AND day = ''))",
        )
        .bind(org.to_string())
        .bind(user.to_string())
        .bind(day)
        .fetch_all(&self.pool)
        .await?;
        let mut user_day = BTreeMap::new();
        let mut user_all = BTreeMap::new();
        for row in rows {
            let scope: String = row.try_get("scope")?;
            let key: String = row.try_get("key")?;
            let raw: String = row.try_get("value")?;
            let value = serde_json::from_str(&raw).map_err(|e| Error::Corrupt(e.to_string()))?;
            if scope == "user_day" {
                user_day.insert(key, value);
            } else {
                user_all.insert(key, value);
            }
        }
        Ok((user_day, user_all))
    }

    /// Ändert einen Key der Scopes `user_day`/`user` atomar: `f` erhält den aktuellen Wert und
    /// liefert den neuen (`None` = löschen). `day` ist bei `user` leer.
    pub async fn update_policy_user_state<F>(
        &self,
        org: OrgId,
        user: UserId,
        scope: &str,
        day: &str,
        key: &str,
        f: F,
    ) -> Result<Option<Value>>
    where
        F: FnOnce(Option<Value>) -> std::result::Result<Option<Value>, String>,
    {
        if !matches!(scope, "user" | "user_day") {
            return Err(Error::InvalidEvent(format!(
                "Scope `{scope}` liegt nicht im Store"
            )));
        }
        let day = if scope == "user" { "" } else { day };
        let mut tx = self.write_tx().await?;
        let current: Option<String> = sqlx::query_scalar(
            "SELECT value FROM policy_user_state WHERE org_id = ? AND user_id = ? AND scope = ? \
             AND day = ? AND key = ?",
        )
        .bind(org.to_string())
        .bind(user.to_string())
        .bind(scope)
        .bind(day)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        let current = current
            .map(|s| serde_json::from_str(&s).map_err(|e| Error::Corrupt(e.to_string())))
            .transpose()?;
        let mut next = f(current).map_err(Error::InvalidEvent)?;
        // Werte können Daten aus Tool-Calls enthalten: dieselbe Redaction wie das Log (SEC-013).
        if let Some(v) = next.as_mut() {
            self.options.redactor.redact(v)?;
        }
        match &next {
            Some(v) => {
                sqlx::query(
                    "INSERT INTO policy_user_state (org_id, user_id, scope, day, key, value, \
                     updated_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (org_id, user_id, \
                     scope, day, key) DO UPDATE SET value = excluded.value, \
                     updated_at = excluded.updated_at",
                )
                .bind(org.to_string())
                .bind(user.to_string())
                .bind(scope)
                .bind(day)
                .bind(key)
                .bind(v.to_string())
                .bind(Timestamp::now().to_string())
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM policy_user_state WHERE org_id = ? AND user_id = ? AND scope = ? \
                     AND day = ? AND key = ?",
                )
                .bind(org.to_string())
                .bind(user.to_string())
                .bind(scope)
                .bind(day)
                .bind(key)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(next)
    }
}
