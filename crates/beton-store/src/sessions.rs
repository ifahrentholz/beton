//! Lokale Identität, Sessions, Löschen mit Tombstones und Audit (DATA-001, DATA-008).

use beton_core::event::{
    Actor, Event, EventPayload, SessionCreated, SessionKind, SessionStatus, SessionTrigger,
};
use beton_core::id::{EventId, NodeId, OrgId, PrincipalId, ProjectId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::Store;
use crate::error::{Error, Result};

/// Die festen Identitäten des lokalen Modus (`org_local`, `usr_local`) und der eigene Knoten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalIdentity {
    pub org: OrgId,
    pub user: UserId,
    pub node: NodeId,
}

/// Parameter für eine neue Session. `id` wird vom Aufrufer vergeben (lokal `SessionId::new()`,
/// bei Replikation die ID des Home-Knotens).
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: SessionId,
    pub owner: UserId,
    pub kind: SessionKind,
    pub harness: String,
    pub cwd: String,
    pub model: Option<String>,
    pub agent_ref: Option<String>,
    pub project_id: Option<ProjectId>,
    pub parent_id: Option<SessionId>,
    pub trigger: SessionTrigger,
    pub home_node: NodeId,
}

/// Eine Session mit ihren Projektionsfeldern (Session-Liste, DATA-005).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub org_id: OrgId,
    pub owner: UserId,
    pub project_id: Option<ProjectId>,
    pub parent_id: Option<SessionId>,
    pub kind: SessionKind,
    pub harness: String,
    pub home_node_id: NodeId,
    pub epoch: u64,
    pub head_seq: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub title: String,
    pub status: SessionStatus,
    pub archived: bool,
    pub cost_micro: i64,
    pub last_activity_at: Timestamp,
}

/// Grund, aus dem jemand eine Session löschen darf (DATA-008 AC3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAuthority {
    /// Der Aufrufer muss Owner der Session sein.
    Owner,
    /// Der Aufrufer ist Org-Admin (Offboarding); geprüft von der Server-Autorisierung.
    OrgAdmin,
}

/// Spur einer gelöschten Entität; verhindert erneute Replikation (DATA-008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tombstone {
    pub kind: String,
    pub id: String,
    pub owner: PrincipalId,
    pub deleted_at: Timestamp,
    pub deleted_by: PrincipalId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub id: String,
    pub at: Timestamp,
    pub actor: PrincipalId,
    pub action: String,
    pub target_kind: String,
    pub target_id: String,
    pub details: serde_json::Value,
}

pub(crate) fn enum_to_str<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

pub(crate) fn enum_from_str<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).map_err(Error::corrupt)
}

pub(crate) fn parse<T: std::str::FromStr>(s: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(Error::corrupt)
}

pub(crate) fn opt_parse<T: std::str::FromStr>(s: Option<String>) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    s.as_deref().map(parse).transpose()
}

pub(crate) fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| Error::corrupt(format!("{v} passt nicht in i64")))
}

pub(crate) fn to_u64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(|_| Error::corrupt(format!("{v} ist negativ")))
}

const SESSION_COLUMNS: &str = "id, org_id, owner_id, project_id, parent_id, kind, harness, \
    home_node_id, epoch, head_seq, created_at, updated_at, title, status, archived, cost_micro, \
    last_activity_at";

fn session_from_row(row: &SqliteRow) -> Result<SessionRecord> {
    Ok(SessionRecord {
        id: parse(row.try_get("id")?)?,
        org_id: parse(row.try_get("org_id")?)?,
        owner: parse(row.try_get("owner_id")?)?,
        project_id: opt_parse(row.try_get("project_id")?)?,
        parent_id: opt_parse(row.try_get("parent_id")?)?,
        kind: enum_from_str(row.try_get("kind")?)?,
        harness: row.try_get("harness")?,
        home_node_id: parse(row.try_get("home_node_id")?)?,
        epoch: to_u64(row.try_get("epoch")?)?,
        head_seq: to_u64(row.try_get("head_seq")?)?,
        created_at: parse(row.try_get("created_at")?)?,
        updated_at: parse(row.try_get("updated_at")?)?,
        title: row.try_get("title")?,
        status: enum_from_str(row.try_get("status")?)?,
        archived: row.try_get("archived")?,
        cost_micro: row.try_get("cost_micro")?,
        last_activity_at: parse(row.try_get("last_activity_at")?)?,
    })
}

impl Store {
    /// Legt `org_local`, `usr_local` und die ID dieses Knotens an, falls noch nicht vorhanden.
    pub async fn ensure_local(&self) -> Result<LocalIdentity> {
        let org = OrgId::LOCAL;
        let user = UserId::LOCAL;
        let now = Timestamp::now().to_string();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO orgs (id, name, created_at) VALUES (?, 'Lokal', ?) ON CONFLICT DO NOTHING",
        )
        .bind(org.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO users (id, org_id, display_name, created_at) VALUES (?, ?, 'Lokal', ?) \
             ON CONFLICT DO NOTHING",
        )
        .bind(user.to_string())
        .bind(org.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM nodes WHERE org_id = ? ORDER BY created_at, id LIMIT 1",
        )
        .bind(org.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let node = match existing {
            Some(id) => parse(&id)?,
            None => {
                let node = NodeId::new();
                sqlx::query(
                    "INSERT INTO nodes (id, org_id, name, created_at) VALUES (?, ?, 'lokal', ?)",
                )
                .bind(node.to_string())
                .bind(org.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
                node
            }
        };
        tx.commit().await?;
        Ok(LocalIdentity { org, user, node })
    }

    pub async fn create_org(&self, org: OrgId, name: &str) -> Result<()> {
        sqlx::query("INSERT INTO orgs (id, name, created_at) VALUES (?, ?, ?)")
            .bind(org.to_string())
            .bind(name)
            .bind(Timestamp::now().to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_user(&self, org: OrgId, user: UserId, display_name: &str) -> Result<()> {
        sqlx::query("INSERT INTO users (id, org_id, display_name, created_at) VALUES (?, ?, ?, ?)")
            .bind(user.to_string())
            .bind(org.to_string())
            .bind(display_name)
            .bind(Timestamp::now().to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_node(&self, org: OrgId, node: NodeId, name: &str) -> Result<()> {
        sqlx::query("INSERT INTO nodes (id, org_id, name, created_at) VALUES (?, ?, ?, ?)")
            .bind(node.to_string())
            .bind(org.to_string())
            .bind(name)
            .bind(Timestamp::now().to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Löscht einen User. Besitzt er noch Sessions, schlägt das fehl (DATA-001 AC3).
    pub async fn delete_user(&self, org: OrgId, user: UserId) -> Result<()> {
        let result = sqlx::query("DELETE FROM users WHERE org_id = ? AND id = ?")
            .bind(org.to_string())
            .bind(user.to_string())
            .execute(&self.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() == 0 => Err(Error::NotFound(format!("User {user}"))),
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => Err(Error::Conflict(
                format!("User {user} besitzt noch Sessions; erst übertragen"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    /// Legt eine Session an und schreibt `session.created` als `seq` 1.
    pub async fn create_session(&self, org: OrgId, new: NewSession) -> Result<SessionRecord> {
        let event = Event::new(
            new.id,
            0,
            Actor::User {
                id: PrincipalId::User(new.owner),
                device_id: None,
            },
            EventPayload::SessionCreated(SessionCreated {
                owner: PrincipalId::User(new.owner),
                kind: new.kind,
                harness: new.harness.clone(),
                agent_ref: new.agent_ref.clone(),
                model: new.model.clone(),
                cwd: new.cwd.clone(),
                project_id: new.project_id,
                parent_session_id: new.parent_id,
                trigger: new.trigger,
            }),
        );
        let created_at = event.ts.to_string();
        let prepared = self.prepare(new.id, vec![event])?;

        let mut tx = self.pool.begin().await?;
        if is_tombstoned(&mut tx, org, new.id).await? {
            return Err(Error::Tombstoned(format!("Session {}", new.id)));
        }
        if let Some(parent) = new.parent_id {
            session_exists(&mut tx, org, parent).await?;
        }
        sqlx::query(
            "INSERT INTO sessions (id, org_id, owner_id, project_id, parent_id, kind, harness, \
             home_node_id, epoch, head_seq, created_at, updated_at, title, status, archived, \
             cost_micro, last_activity_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, 0, ?, ?, '', ?, 0, 0, ?)",
        )
        .bind(new.id.to_string())
        .bind(org.to_string())
        .bind(new.owner.to_string())
        .bind(new.project_id.map(|p| p.to_string()))
        .bind(new.parent_id.map(|p| p.to_string()))
        .bind(enum_to_str(&new.kind))
        .bind(&new.harness)
        .bind(new.home_node.to_string())
        .bind(&created_at)
        .bind(&created_at)
        .bind(enum_to_str(&SessionStatus::default()))
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
        self.write_prepared(&mut tx, org, new.id, 0, 1, prepared)
            .await?;
        tx.commit().await?;
        self.session(org, new.id).await
    }

    pub async fn session(&self, org: OrgId, id: SessionId) -> Result<SessionRecord> {
        let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE org_id = ? AND id = ?");
        let row = sqlx::query(&sql)
            .bind(org.to_string())
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Session {id}")))?;
        session_from_row(&row)
    }

    /// Session-Liste, neueste Aktivität zuerst.
    pub async fn sessions(&self, org: OrgId, include_archived: bool) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE org_id = ? AND (archived = 0 OR ?) \
             ORDER BY last_activity_at DESC, id DESC"
        );
        sqlx::query(&sql)
            .bind(org.to_string())
            .bind(include_archived)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(session_from_row)
            .collect()
    }

    /// Eine Seite der Session-Liste, neueste zuerst nach ID (PROTO-010 AC1). Die Sortierung
    /// über die unveränderliche, zeitlich sortierte ID macht die Pagination stabil: Neue
    /// Sessions erscheinen vor der ersten Seite und verschieben keine späteren.
    pub async fn sessions_page(
        &self,
        org: OrgId,
        include_archived: bool,
        limit: u32,
        before: Option<SessionId>,
    ) -> Result<(Vec<SessionRecord>, Option<SessionId>)> {
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE org_id = ? AND (archived = 0 OR ?) \
             AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT ?"
        );
        let before = before.map(|b| b.to_string());
        let mut items: Vec<SessionRecord> = sqlx::query(&sql)
            .bind(org.to_string())
            .bind(include_archived)
            .bind(&before)
            .bind(&before)
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(session_from_row)
            .collect::<Result<_>>()?;
        let next = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|s| s.id)
        } else {
            None
        };
        Ok((items, next))
    }

    /// Löscht eine Session samt Side-Chats und Sub-Sessions in einer Transaktion: Events,
    /// `event_raw`, Projektionen und Blob-Referenzen. Hinterlässt je Session einen Tombstone
    /// und einen Audit-Eintrag (DATA-008). Die Blob-Dateien entfernt der nächste GC-Lauf.
    pub async fn delete_session(
        &self,
        org: OrgId,
        id: SessionId,
        by: PrincipalId,
        authority: DeleteAuthority,
    ) -> Result<Vec<Tombstone>> {
        let mut tx = self.pool.begin().await?;
        let owner: String =
            sqlx::query_scalar("SELECT owner_id FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org.to_string())
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| Error::NotFound(format!("Session {id}")))?;
        let owner: PrincipalId = parse(&owner)?;
        if authority == DeleteAuthority::Owner && owner != by {
            return Err(Error::Forbidden(format!(
                "nur der Owner darf Session {id} löschen"
            )));
        }
        let now = Timestamp::now();
        let tombstones = delete_tree(&mut tx, org, id, by, now).await?;
        let deleted: Vec<&str> = tombstones.iter().map(|t| t.id.as_str()).collect();
        sqlx::query(
            "INSERT INTO audit_entries (id, org_id, at, actor_id, action, target_kind, target_id, \
             details) VALUES (?, ?, ?, ?, 'session.delete', 'session', ?, ?)",
        )
        .bind(EventId::new().ulid().to_string())
        .bind(org.to_string())
        .bind(now.to_string())
        .bind(by.to_string())
        .bind(id.to_string())
        .bind(
            serde_json::json!({
                "deleted": deleted,
                "authority": match authority {
                    DeleteAuthority::Owner => "owner",
                    DeleteAuthority::OrgAdmin => "org_admin",
                },
            })
            .to_string(),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(tombstones)
    }

    /// Wendet einen empfangenen Tombstone an: lokale Kopie löschen, Tombstone merken
    /// (DATA-008 AC2). Idempotent.
    pub async fn apply_tombstone(&self, org: OrgId, tombstone: &Tombstone) -> Result<()> {
        if tombstone.kind != "session" {
            return Err(Error::InvalidEvent(format!(
                "Tombstone-Art `{}` wird nicht unterstützt",
                tombstone.kind
            )));
        }
        let id: SessionId = tombstone
            .id
            .parse()
            .map_err(|e| Error::InvalidEvent(format!("{e}")))?;
        let mut tx = self.pool.begin().await?;
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org.to_string())
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        if exists.is_some() {
            delete_tree(&mut tx, org, id, tombstone.deleted_by, tombstone.deleted_at).await?;
        }
        insert_tombstone(&mut tx, org, tombstone).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Tombstones nach `since`, älteste zuerst (für `GET /v1/tombstones?since=`).
    pub async fn tombstones_since(&self, org: OrgId, since: Timestamp) -> Result<Vec<Tombstone>> {
        sqlx::query(
            "SELECT kind, id, owner_id, deleted_at, deleted_by FROM tombstones \
             WHERE org_id = ? AND deleted_at > ? ORDER BY deleted_at, id",
        )
        .bind(org.to_string())
        .bind(since.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            Ok(Tombstone {
                kind: row.try_get("kind")?,
                id: row.try_get("id")?,
                owner: parse(row.try_get("owner_id")?)?,
                deleted_at: parse(row.try_get("deleted_at")?)?,
                deleted_by: parse(row.try_get("deleted_by")?)?,
            })
        })
        .collect()
    }

    /// Audit-Einträge, neueste zuerst.
    pub async fn audit_entries(&self, org: OrgId, limit: u32) -> Result<Vec<AuditEntry>> {
        sqlx::query(
            "SELECT id, at, actor_id, action, target_kind, target_id, details FROM audit_entries \
             WHERE org_id = ? ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(org.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            Ok(AuditEntry {
                id: row.try_get("id")?,
                at: parse(row.try_get("at")?)?,
                actor: parse(row.try_get("actor_id")?)?,
                action: row.try_get("action")?,
                target_kind: row.try_get("target_kind")?,
                target_id: row.try_get("target_id")?,
                details: serde_json::from_str(row.try_get("details")?)?,
            })
        })
        .collect()
    }
}

pub(crate) async fn session_exists(
    conn: &mut SqliteConnection,
    org: OrgId,
    id: SessionId,
) -> Result<()> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sessions WHERE org_id = ? AND id = ?")
            .bind(org.to_string())
            .bind(id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    found
        .map(|_| ())
        .ok_or_else(|| Error::NotFound(format!("Session {id}")))
}

async fn is_tombstoned(conn: &mut SqliteConnection, org: OrgId, id: SessionId) -> Result<bool> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM tombstones WHERE org_id = ? AND kind = 'session' AND id = ?",
    )
    .bind(org.to_string())
    .bind(id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

async fn insert_tombstone(conn: &mut SqliteConnection, org: OrgId, t: &Tombstone) -> Result<()> {
    sqlx::query(
        "INSERT INTO tombstones (org_id, kind, id, owner_id, deleted_at, deleted_by) \
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(org.to_string())
    .bind(&t.kind)
    .bind(&t.id)
    .bind(t.owner.to_string())
    .bind(t.deleted_at.to_string())
    .bind(t.deleted_by.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Löscht eine Session und alle Nachfahren (tiefste zuerst) und schreibt Tombstones.
async fn delete_tree(
    conn: &mut SqliteConnection,
    org: OrgId,
    root: SessionId,
    by: PrincipalId,
    at: Timestamp,
) -> Result<Vec<Tombstone>> {
    let rows = sqlx::query(
        "WITH RECURSIVE tree (id, owner_id, depth) AS ( \
           SELECT id, owner_id, 0 FROM sessions WHERE org_id = ?1 AND id = ?2 \
           UNION ALL \
           SELECT s.id, s.owner_id, t.depth + 1 FROM sessions s JOIN tree t ON s.parent_id = t.id \
           WHERE s.org_id = ?1 \
         ) SELECT id, owner_id FROM tree ORDER BY depth DESC, id",
    )
    .bind(org.to_string())
    .bind(root.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let mut tombstones = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let owner: String = row.try_get("owner_id")?;
        for sql in [
            "DELETE FROM event_raw WHERE org_id = ? AND session_id = ?",
            "DELETE FROM approvals WHERE org_id = ? AND session_id = ?",
            "DELETE FROM usage_daily WHERE org_id = ? AND session_id = ?",
            "DELETE FROM blob_refs WHERE org_id = ? AND session_id = ?",
            "DELETE FROM events WHERE org_id = ? AND session_id = ?",
            "DELETE FROM sessions WHERE org_id = ? AND id = ?",
        ] {
            sqlx::query(sql)
                .bind(org.to_string())
                .bind(&id)
                .execute(&mut *conn)
                .await?;
        }
        let tombstone = Tombstone {
            kind: "session".into(),
            id,
            owner: parse(&owner)?,
            deleted_at: at,
            deleted_by: by,
        };
        insert_tombstone(conn, org, &tombstone).await?;
        tombstones.push(tombstone);
    }
    Ok(tombstones)
}
