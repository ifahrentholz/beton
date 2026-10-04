//! Session-Liste je User: Filter, Volltextsuche, Anpinnen und Gelesen-Stand (SES-012).
//!
//! Suche: Titel und Nachrichten (`message.completed`) sind Dokumente in `search_docs`; SQLite
//! indiziert sie mit FTS5 (externer Inhalt), Postgres (DATA-004) mit `to_tsvector('simple')`.
//! Semantik in beiden Dialekten: Groß-/Kleinschreibung egal, keine Stammformen, jedes Wort
//! der Anfrage als Wortanfang, alle Wörter im selben Dokument.

use beton_core::event::{EventPayload, MessageCompleted, SessionStatus};
use beton_core::id::{OrgId, ProjectId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::Store;
use crate::error::{Error, Result};
use crate::sessions::{SessionRecord, enum_to_str, parse, session_from_row, to_i64, to_u64};

/// Welche Sessions die Liste zeigt (Segmente der Sessions-Seite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionScope {
    /// Alle nicht archivierten (Standard der Seitenleiste).
    #[default]
    Active,
    /// Alle, auch archivierte.
    All,
    /// Eigene, nicht archivierte.
    Own,
    /// Nicht eigene, nicht archivierte (Freigaben, COL-001 ab M4; lokal leer).
    Shared,
    /// Archivierte.
    Archived,
}

/// Filter der Session-Liste (`GET /v1/sessions`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFilter {
    pub scope: SessionScope,
    pub harness: Option<String>,
    pub status: Option<SessionStatus>,
    pub project: Option<ProjectId>,
    /// Volltext über Titel und Nachrichten.
    pub query: Option<String>,
}

/// Eine Session aus Sicht eines Users.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    pub session: SessionRecord,
    pub pinned: bool,
    /// Bis hierhin hat ein Client des Users die Session angezeigt.
    pub read_seq: u64,
    /// Letzte Änderung aus Sicht des Users: Aktivität oder eigener Lese-/Pin-Stand.
    pub changed_at: Timestamp,
}

impl SessionView {
    /// Ungelesen: Es gibt Events nach dem Gelesen-Stand.
    pub fn unread(&self) -> bool {
        self.session.head_seq > self.read_seq
    }
}

const VIEW_COLUMNS: &str = "s.id, s.org_id, s.owner_id, s.project_id, s.parent_id, s.kind, \
    s.harness, s.home_node_id, s.epoch, s.head_seq, s.created_at, s.updated_at, s.title, \
    s.status, s.archived, s.cost_micro, s.last_activity_at, \
    COALESCE(u.pinned, 0) AS pinned, COALESCE(u.read_seq, 0) AS read_seq, \
    MAX(s.last_activity_at, COALESCE(u.updated_at, s.last_activity_at)) AS changed_at";

const VIEW_FROM: &str =
    "FROM sessions s LEFT JOIN session_user_state u ON u.session_id = s.id AND u.user_id = ?";

fn view_from_row(row: &SqliteRow) -> Result<SessionView> {
    Ok(SessionView {
        session: session_from_row(row)?,
        pinned: row.try_get::<i64, _>("pinned")? != 0,
        read_seq: to_u64(row.try_get("read_seq")?)?,
        changed_at: parse(row.try_get("changed_at")?)?,
    })
}

/// FTS5-Anfrage aus Nutzereingabe: jedes Wort in Anführungszeichen (keine Operatoren aus der
/// Eingabe) mit Präfix-Stern; Leerzeichen heißt UND. `None` bei leerer Eingabe.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Klartext einer Nachricht für den Suchindex.
fn message_text(m: &MessageCompleted) -> String {
    m.content
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Aktualisiert die Suchdokumente für ein Event (Teil der Projektionen, DATA-005).
pub(crate) async fn index_event(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    seq: u64,
    payload: &EventPayload,
) -> Result<()> {
    match payload {
        EventPayload::SessionTitleChanged(t) => {
            index_doc(conn, org, session, "title", &t.title).await
        }
        EventPayload::MessageCompleted(m) => {
            let text = message_text(m);
            if text.trim().is_empty() {
                return Ok(());
            }
            index_doc(conn, org, session, &format!("msg:{seq}"), &text).await
        }
        _ => Ok(()),
    }
}

async fn index_doc(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    doc: &str,
    body: &str,
) -> Result<()> {
    let (org_s, session_s) = (org.to_string(), session.to_string());
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM search_docs WHERE org_id = ? AND session_id = ? AND doc = ?",
    )
    .bind(&org_s)
    .bind(&session_s)
    .bind(doc)
    .fetch_optional(&mut *conn)
    .await?;
    let id = match existing {
        Some(id) => {
            // Externer Inhalt: FTS5 braucht zum Löschen den alten Text.
            sqlx::query(
                "INSERT INTO search_fts (search_fts, rowid, body) \
                 SELECT 'delete', id, body FROM search_docs WHERE org_id = ? AND id = ?",
            )
            .bind(&org_s)
            .bind(id)
            .execute(&mut *conn)
            .await?;
            sqlx::query("UPDATE search_docs SET body = ? WHERE org_id = ? AND id = ?")
                .bind(body)
                .bind(&org_s)
                .bind(id)
                .execute(&mut *conn)
                .await?;
            id
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO search_docs (org_id, session_id, doc, body) VALUES (?, ?, ?, ?) \
                 RETURNING id",
            )
            .bind(&org_s)
            .bind(&session_s)
            .bind(doc)
            .bind(body)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    sqlx::query(
        "INSERT INTO search_fts (rowid, body) \
         SELECT id, body FROM search_docs WHERE org_id = ? AND id = ?",
    )
    .bind(&org_s)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Entfernt alle Suchdokumente einer Session (Löschen, Rebuild).
pub(crate) async fn purge_docs(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO search_fts (search_fts, rowid, body) \
         SELECT 'delete', id, body FROM search_docs WHERE org_id = ? AND session_id = ?",
    )
    .bind(org.to_string())
    .bind(session)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM search_docs WHERE org_id = ? AND session_id = ?")
        .bind(org.to_string())
        .bind(session)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Zusätzliche Bedingungen und Parameter eines Filters (nach `s.org_id = ?`, ohne Pin).
fn filter_sql(org: OrgId, user: UserId, f: &SessionFilter) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut args = Vec::new();
    match f.scope {
        SessionScope::Active => sql.push_str(" AND s.archived = 0"),
        SessionScope::All => {}
        SessionScope::Own => {
            sql.push_str(" AND s.archived = 0 AND s.owner_id = ?");
            args.push(user.to_string());
        }
        SessionScope::Shared => {
            sql.push_str(" AND s.archived = 0 AND s.owner_id <> ?");
            args.push(user.to_string());
        }
        SessionScope::Archived => sql.push_str(" AND s.archived = 1"),
    }
    if let Some(h) = &f.harness {
        sql.push_str(" AND s.harness = ?");
        args.push(h.clone());
    }
    if let Some(st) = &f.status {
        sql.push_str(" AND s.status = ?");
        args.push(enum_to_str(st));
    }
    if let Some(p) = &f.project {
        sql.push_str(" AND s.project_id = ?");
        args.push(p.to_string());
    }
    if let Some(q) = f.query.as_deref().and_then(fts_query) {
        // Erst der FTS-Index, dann die Dokumente: Die Treffermenge entsteht genau einmal.
        sql.push_str(
            " AND s.id IN (SELECT d.session_id FROM search_docs d WHERE d.org_id = ? \
             AND d.id IN (SELECT rowid FROM search_fts WHERE search_fts MATCH ?))",
        );
        args.push(org.to_string());
        args.push(q);
    }
    (sql, args)
}

impl Store {
    /// Eine Session aus Sicht eines Users.
    pub async fn session_view(
        &self,
        org: OrgId,
        user: UserId,
        id: SessionId,
    ) -> Result<SessionView> {
        let sql = format!("SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE s.org_id = ? AND s.id = ?");
        let row = sqlx::query(&sql)
            .bind(user.to_string())
            .bind(org.to_string())
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Session {id}")))?;
        view_from_row(&row)
    }

    /// Eine Seite der Session-Liste: Auf der ersten Seite stehen die angepinnten Sessions
    /// oben (nach Aktivität, SES-012 AC3), danach die übrigen neueste zuerst nach ID
    /// (stabile Cursor-Pagination, PROTO-010). `before` ist der Cursor der vorigen Seite.
    pub async fn session_list(
        &self,
        org: OrgId,
        user: UserId,
        filter: &SessionFilter,
        limit: u32,
        before: Option<SessionId>,
    ) -> Result<(Vec<SessionView>, Option<SessionId>)> {
        let (cond, args) = filter_sql(org, user, filter);
        let mut items = Vec::new();
        if before.is_none() {
            let sql = format!(
                "SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE s.org_id = ?{cond} AND COALESCE(u.pinned, 0) = 1 \
                 ORDER BY s.last_activity_at DESC, s.id DESC"
            );
            let mut q = sqlx::query(&sql)
                .bind(user.to_string())
                .bind(org.to_string());
            for a in &args {
                q = q.bind(a);
            }
            for row in q.fetch_all(&self.pool).await? {
                items.push(view_from_row(&row)?);
            }
        }
        let sql = format!(
            "SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE s.org_id = ?{cond} AND COALESCE(u.pinned, 0) = 0 \
             AND (? IS NULL OR s.id < ?) ORDER BY s.id DESC LIMIT ?"
        );
        let before = before.map(|b| b.to_string());
        let mut q = sqlx::query(&sql)
            .bind(user.to_string())
            .bind(org.to_string());
        for a in &args {
            q = q.bind(a);
        }
        let mut rest: Vec<SessionView> = q
            .bind(&before)
            .bind(&before)
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(view_from_row)
            .collect::<Result<_>>()?;
        let next = if rest.len() > limit as usize {
            rest.truncate(limit as usize);
            rest.last().map(|s| s.session.id)
        } else {
            None
        };
        items.extend(rest);
        Ok((items, next))
    }

    /// Sessions, die sich für den User nach `since` geändert haben (Aktivität, Gelesen-Stand,
    /// Pin), älteste Änderung zuerst, auch archivierte (Listen-Deltas, WEB-003, SES-012 AC2).
    pub async fn sessions_changed_after(
        &self,
        org: OrgId,
        user: UserId,
        since: Timestamp,
        limit: u32,
    ) -> Result<Vec<SessionView>> {
        let sql = format!(
            "SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE s.org_id = ? AND s.id IN ( \
               SELECT id FROM sessions WHERE org_id = ? AND last_activity_at > ? \
               UNION SELECT session_id FROM session_user_state \
               WHERE org_id = ? AND user_id = ? AND updated_at > ?) \
             ORDER BY changed_at ASC, s.id ASC LIMIT ?"
        );
        let since = since.to_string();
        sqlx::query(&sql)
            .bind(user.to_string())
            .bind(org.to_string())
            .bind(org.to_string())
            .bind(&since)
            .bind(org.to_string())
            .bind(user.to_string())
            .bind(&since)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(view_from_row)
            .collect()
    }

    /// Anpinnen bzw. lösen (je User).
    pub async fn set_pinned(
        &self,
        org: OrgId,
        user: UserId,
        id: SessionId,
        pinned: bool,
    ) -> Result<SessionView> {
        self.upsert_user_state(org, user, id, Some(pinned), None)
            .await?;
        self.session_view(org, user, id).await
    }

    /// Gelesen bis `seq` (höchstens `head_seq`); der Stand sinkt nie (SES-012 AC2).
    pub async fn mark_read(
        &self,
        org: OrgId,
        user: UserId,
        id: SessionId,
        seq: u64,
    ) -> Result<SessionView> {
        self.upsert_user_state(org, user, id, None, Some(seq))
            .await?;
        self.session_view(org, user, id).await
    }

    async fn upsert_user_state(
        &self,
        org: OrgId,
        user: UserId,
        id: SessionId,
        pinned: Option<bool>,
        read: Option<u64>,
    ) -> Result<()> {
        let mut tx = self.write_tx().await?;
        let head: Option<i64> =
            sqlx::query_scalar("SELECT head_seq FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org.to_string())
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let head = to_u64(head.ok_or_else(|| Error::NotFound(format!("Session {id}")))?)?;
        let current: Option<(i64, i64)> = sqlx::query_as(
            "SELECT pinned, read_seq FROM session_user_state \
             WHERE org_id = ? AND session_id = ? AND user_id = ?",
        )
        .bind(org.to_string())
        .bind(id.to_string())
        .bind(user.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let (old_pinned, old_read) = current
            .map(|(p, r)| (p != 0, u64::try_from(r).unwrap_or(0)))
            .unwrap_or((false, 0));
        let new_pinned = pinned.unwrap_or(old_pinned);
        let new_read = read.map_or(old_read, |r| r.min(head).max(old_read));
        if current.is_some() && new_pinned == old_pinned && new_read == old_read {
            return Ok(());
        }
        let now = Timestamp::now().to_string();
        sqlx::query(
            "INSERT INTO session_user_state (org_id, session_id, user_id, pinned, read_seq, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (session_id, user_id) DO UPDATE SET pinned = excluded.pinned, \
             read_seq = excluded.read_seq, updated_at = excluded.updated_at",
        )
        .bind(org.to_string())
        .bind(id.to_string())
        .bind(user.to_string())
        .bind(new_pinned)
        .bind(to_i64(new_read)?)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Letztes Event eines Typs, z. B. `queue.updated` für den Queue-Stand (SES-004).
    pub async fn last_event_of_type(
        &self,
        org: OrgId,
        session: SessionId,
        type_name: &str,
    ) -> Result<Option<beton_core::event::Event>> {
        let seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM events WHERE org_id = ? AND session_id = ? AND type = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(type_name)
        .fetch_one(&self.pool)
        .await?;
        let Some(seq) = seq else { return Ok(None) };
        let seq = to_u64(seq)?;
        Ok(self
            .events(org, session, seq - 1, 1)
            .await?
            .into_iter()
            .next())
    }
}
