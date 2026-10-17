//! Hash-verkettetes Audit-Log (SEC-012).
//!
//! Jede Org hat eine eigene Kette: `seq` zählt lückenlos ab 1, und
//! `hash = SHA-256(prev_hash ‖ Eintrag)` (hex). Der erste Eintrag hat `prev_hash` aus 64 Nullen.
//! Der Eintrag geht als Folge längenpräfixierter Felder in den Hash ein, und zwar genau so,
//! wie die Felder in der Datenbank stehen (`details` als gespeicherter JSON-Text). Damit hängt
//! die Prüfung nicht von der Reihenfolge der Schlüssel beim erneuten Serialisieren ab.
//!
//! `audit_heads` hält `seq` und `hash` des letzten Eintrags; so fällt auch das Löschen des
//! letzten Eintrags auf. Wer Schreibzugriff auf die Datenbank hat, kann die Kette komplett
//! neu berechnen; die Kette erkennt nachträgliche Änderungen, sie verhindert sie nicht.
//!
//! Hochfrequente Nutzung eines Secrets (Proxy) wird je `(secret, session, host)` in
//! 5-min-Fenstern gezählt ([`Store::record_audit_usage`]). Die offenen Fenster liegen in
//! `audit_usage_windows` (übersteht einen Neustart); ein abgelaufenes Fenster wird als **ein**
//! Eintrag `secret.used` mit `count` versiegelt ([`Store::seal_audit_windows`]).
//!
//! Audit-Einträge enthalten nie Secret-Werte; das stellen die Aufrufer sicher (`beton-secrets`
//! übergibt nur Refs, Zwecke und IDs).

use std::time::Duration;

use beton_core::id::{EventId, OrgId, PrincipalId, SessionId};
use beton_core::time::Timestamp;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection as _, Row, SqliteConnection};

use crate::Store;
use crate::error::{Error, Result};
use crate::sessions::parse;

/// Länge eines Aggregationsfensters für hochfrequente Nutzung (SEC-012).
pub const AUDIT_USAGE_WINDOW: Duration = Duration::from_secs(5 * 60);

/// `prev_hash` des ersten Eintrags einer Kette.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Aktion für versiegelte Nutzungsfenster.
pub const USED_ACTION: &str = "secret.used";

/// Nutzungsfenster eines Credential-Bindings des Proxys (PRX-006 AC6): `secret_ref` ist
/// `credential:<binding>`, `host_id` der Ziel-Host. Versiegelt als `credential.used` mit
/// `target_kind: credential`.
pub const CREDENTIAL_PREFIX: &str = "credential:";

/// Aktion für versiegelte Nutzungsfenster eines Credential-Bindings.
pub const CREDENTIAL_USED_ACTION: &str = "credential.used";

/// Ein neuer Audit-Eintrag; `seq` und Hashes vergibt der Store.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEntry {
    pub at: Timestamp,
    pub actor: PrincipalId,
    /// Z. B. `secret.created`, `secret.used`, `session.delete`.
    pub action: String,
    pub target_kind: String,
    pub target_id: String,
    pub session_id: Option<SessionId>,
    pub host_id: Option<String>,
    /// Zusätzliche Angaben; nie Secret-Werte.
    pub details: serde_json::Value,
}

/// Ein gespeicherter Audit-Eintrag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub id: String,
    pub at: Timestamp,
    pub actor: PrincipalId,
    pub action: String,
    pub target_kind: String,
    pub target_id: String,
    pub session_id: Option<SessionId>,
    pub host_id: Option<String>,
    /// Anzahl zusammengefasster Vorgänge (> 1 nur bei versiegelten Nutzungsfenstern).
    pub count: u64,
    pub details: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

/// Abfrage einer Seite, neueste zuerst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    /// Nur Einträge mit `seq` kleiner als dieser Wert (Cursor der vorigen Seite).
    pub before_seq: Option<u64>,
    /// Nur diese Aktion.
    pub action: Option<String>,
    /// Nur diese Art des Ziels, z. B. `credential` (PRX-009 AC4) oder `secret`.
    pub target_kind: Option<String>,
    pub limit: u32,
}

impl Default for AuditQuery {
    fn default() -> Self {
        Self {
            before_seq: None,
            action: None,
            target_kind: None,
            limit: 100,
        }
    }
}

/// Eine Seite des Audit-Logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPage {
    pub items: Vec<AuditEntry>,
    /// `before_seq` für die nächste Seite; `None` am Ende.
    pub next_before: Option<u64>,
}

/// Ergebnis von [`Store::verify_audit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditVerification {
    /// Anzahl der vorhandenen Einträge.
    pub entries: u64,
    /// Erste ungültige Position (`seq`); `None`, wenn die Kette intakt ist.
    pub first_invalid: Option<u64>,
    /// Grund für die erste ungültige Position.
    pub reason: Option<String>,
}

/// Schlüssel eines Nutzungsfensters (SEC-012: je `(secret, session, host)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageKey {
    pub secret_ref: String,
    pub session_id: Option<SessionId>,
    pub host_id: String,
    pub actor: PrincipalId,
    /// Zweck des Abrufs, z. B. `proxy`.
    pub purpose: String,
}

/// Die Felder eines Eintrags in der gespeicherten Form.
struct Stored<'a> {
    org: &'a str,
    seq: u64,
    id: &'a str,
    at: &'a str,
    actor: &'a str,
    action: &'a str,
    target_kind: &'a str,
    target_id: &'a str,
    session_id: &'a str,
    host_id: &'a str,
    count: u64,
    details: &'a str,
}

fn chain_hash(prev: &str, e: &Stored<'_>) -> String {
    let mut h = Sha256::new();
    h.update(b"beton-audit-v1");
    let seq = e.seq.to_string();
    let count = e.count.to_string();
    for field in [
        prev,
        e.org,
        &seq,
        e.id,
        e.at,
        e.actor,
        e.action,
        e.target_kind,
        e.target_id,
        e.session_id,
        e.host_id,
        &count,
        e.details,
    ] {
        h.update((field.len() as u64).to_be_bytes());
        h.update(field.as_bytes());
    }
    hex::encode(h.finalize())
}

fn to_i64(n: u64) -> Result<i64> {
    i64::try_from(n).map_err(|_| Error::corrupt("Zahl zu groß"))
}

/// Hängt einen Eintrag innerhalb einer laufenden Schreib-Transaktion an die Kette.
pub(crate) async fn append_in(
    conn: &mut SqliteConnection,
    org: OrgId,
    entry: &NewAuditEntry,
    count: u64,
) -> Result<AuditEntry> {
    let org_s = org.to_string();
    let head: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, hash FROM audit_heads WHERE org_id = ?")
            .bind(&org_s)
            .fetch_optional(&mut *conn)
            .await?;
    let (prev_seq, prev_hash) = match head {
        Some((seq, hash)) => (
            u64::try_from(seq).map_err(|_| Error::corrupt("audit_heads.seq negativ"))?,
            hash,
        ),
        None => (0, GENESIS_HASH.to_owned()),
    };
    let seq = prev_seq + 1;
    let id = EventId::new().ulid().to_string();
    let at = entry.at.to_string();
    let actor = entry.actor.to_string();
    let session = entry.session_id.map(|s| s.to_string()).unwrap_or_default();
    let host = entry.host_id.clone().unwrap_or_default();
    let details = serde_json::to_string(&entry.details)?;
    let hash = chain_hash(
        &prev_hash,
        &Stored {
            org: &org_s,
            seq,
            id: &id,
            at: &at,
            actor: &actor,
            action: &entry.action,
            target_kind: &entry.target_kind,
            target_id: &entry.target_id,
            session_id: &session,
            host_id: &host,
            count,
            details: &details,
        },
    );
    sqlx::query(
        "INSERT INTO audit_entries (id, org_id, at, actor_id, action, target_kind, target_id, \
         details, seq, session_id, host_id, count, prev_hash, hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&org_s)
    .bind(&at)
    .bind(&actor)
    .bind(&entry.action)
    .bind(&entry.target_kind)
    .bind(&entry.target_id)
    .bind(&details)
    .bind(to_i64(seq)?)
    .bind(entry.session_id.map(|s| s.to_string()))
    .bind(entry.host_id.clone())
    .bind(to_i64(count)?)
    .bind(&prev_hash)
    .bind(&hash)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO audit_heads (org_id, seq, hash) VALUES (?, ?, ?) \
         ON CONFLICT (org_id) DO UPDATE SET seq = excluded.seq, hash = excluded.hash",
    )
    .bind(&org_s)
    .bind(to_i64(seq)?)
    .bind(&hash)
    .execute(&mut *conn)
    .await?;
    Ok(AuditEntry {
        seq,
        id,
        at: entry.at,
        actor: entry.actor,
        action: entry.action.clone(),
        target_kind: entry.target_kind.clone(),
        target_id: entry.target_id.clone(),
        session_id: entry.session_id,
        host_id: entry.host_id.clone(),
        count,
        details: entry.details.clone(),
        prev_hash,
        hash,
    })
}

/// Verkettet Einträge aus der Zeit vor Migration 0010 (DATA-008: ohne `seq`), je Org in der
/// Reihenfolge ihres Zeitstempels. Läuft im Migrator auf dessen Verbindung; idempotent und ohne
/// Schreibzugriff, wenn es nichts zu tun gibt.
pub(crate) async fn backfill(conn: &mut SqliteConnection) -> Result<()> {
    let chained: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('audit_entries') WHERE name = 'seq' \
         /* org-übergreifend: Schema */",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if chained.is_none() {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT id, org_id, at, actor_id, action, target_kind, target_id, details \
         FROM audit_entries WHERE seq IS NULL ORDER BY org_id, at, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = conn.begin().await?;
    for row in rows {
        let id: String = row.try_get("id")?;
        let org: OrgId = parse(row.try_get("org_id")?)?;
        let details: String = row.try_get("details")?;
        let entry = NewAuditEntry {
            at: parse(row.try_get("at")?)?,
            actor: parse(row.try_get("actor_id")?)?,
            action: row.try_get("action")?,
            target_kind: row.try_get("target_kind")?,
            target_id: row.try_get("target_id")?,
            session_id: None,
            host_id: None,
            details: serde_json::from_str(&details)?,
        };
        sqlx::query("DELETE FROM audit_entries WHERE org_id = ? AND id = ?")
            .bind(org.to_string())
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        append_in(&mut tx, org, &entry, 1).await?;
    }
    tx.commit().await?;
    Ok(())
}

fn entry_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AuditEntry> {
    let seq: i64 = row.try_get("seq")?;
    let count: i64 = row.try_get("count")?;
    let session: Option<String> = row.try_get("session_id")?;
    Ok(AuditEntry {
        seq: u64::try_from(seq).map_err(|_| Error::corrupt("seq negativ"))?,
        id: row.try_get("id")?,
        at: parse(row.try_get("at")?)?,
        actor: parse(row.try_get("actor_id")?)?,
        action: row.try_get("action")?,
        target_kind: row.try_get("target_kind")?,
        target_id: row.try_get("target_id")?,
        session_id: session.as_deref().map(parse).transpose()?,
        host_id: row.try_get("host_id")?,
        count: u64::try_from(count).map_err(|_| Error::corrupt("count negativ"))?,
        details: serde_json::from_str(row.try_get("details")?)?,
        prev_hash: row.try_get("prev_hash")?,
        hash: row.try_get("hash")?,
    })
}

const ENTRY_COLUMNS: &str = "seq, id, at, actor_id, action, target_kind, target_id, session_id, \
                             host_id, count, details, prev_hash, hash";

impl Store {
    /// Hängt einen Eintrag an die Kette der Org (SEC-012).
    pub async fn append_audit(&self, org: OrgId, entry: NewAuditEntry) -> Result<AuditEntry> {
        let mut tx = self.write_tx().await?;
        let e = append_in(&mut tx, org, &entry, 1).await?;
        tx.commit().await?;
        Ok(e)
    }

    /// Zählt eine Nutzung im offenen 5-min-Fenster ihres Schlüssels. Abgelaufene Fenster der
    /// Org werden dabei zuerst versiegelt.
    pub async fn record_audit_usage(
        &self,
        org: OrgId,
        key: &UsageKey,
        at: Timestamp,
    ) -> Result<()> {
        let mut tx = self.write_tx().await?;
        seal_in(&mut tx, org, at).await?;
        let session = key.session_id.map(|s| s.to_string()).unwrap_or_default();
        sqlx::query(
            "INSERT INTO audit_usage_windows (org_id, secret_ref, session_id, host_id, actor_id, \
             purpose, window_start, last_at, count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1) \
             ON CONFLICT (org_id, secret_ref, session_id, host_id) \
             DO UPDATE SET count = count + 1, last_at = excluded.last_at",
        )
        .bind(org.to_string())
        .bind(&key.secret_ref)
        .bind(&session)
        .bind(&key.host_id)
        .bind(key.actor.to_string())
        .bind(&key.purpose)
        .bind(at.to_string())
        .bind(at.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Versiegelt alle Fenster der Org, die zu `now` abgelaufen sind, als je einen Eintrag
    /// `secret.used` mit `count`. Liefert die Anzahl versiegelter Fenster.
    pub async fn seal_audit_windows(&self, org: OrgId, now: Timestamp) -> Result<u64> {
        let mut tx = self.write_tx().await?;
        let n = seal_in(&mut tx, org, now).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// Eine Seite des Audit-Logs, neueste zuerst.
    /// Gibt es einen versiegelten Eintrag `action` zur Session `session`? Z. B. die
    /// Bestätigung von `backend: none` (SBX-006), die für Resume weiter gilt.
    pub async fn audit_has_session_entry(
        &self,
        org: OrgId,
        action: &str,
        session: SessionId,
    ) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT seq FROM audit_entries WHERE org_id = ? AND seq IS NOT NULL AND action = ? \
             AND session_id = ? LIMIT 1",
        )
        .bind(org.to_string())
        .bind(action)
        .bind(session.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn audit_page(&self, org: OrgId, query: &AuditQuery) -> Result<AuditPage> {
        let limit = query.limit.clamp(1, 1000);
        let rows = sqlx::query(&format!(
            "SELECT {ENTRY_COLUMNS} FROM audit_entries WHERE org_id = ? AND seq IS NOT NULL \
             AND (? IS NULL OR seq < ?) AND (? IS NULL OR action = ?) \
             AND (? IS NULL OR target_kind = ?) ORDER BY seq DESC LIMIT ?"
        ))
        .bind(org.to_string())
        .bind(query.before_seq.map(to_i64).transpose()?)
        .bind(query.before_seq.map(to_i64).transpose()?)
        .bind(query.action.as_deref())
        .bind(query.action.as_deref())
        .bind(query.target_kind.as_deref())
        .bind(query.target_kind.as_deref())
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await?;
        let mut items = rows
            .iter()
            .map(entry_from_row)
            .collect::<Result<Vec<_>>>()?;
        let next_before = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|e| e.seq)
        } else {
            None
        };
        Ok(AuditPage { items, next_before })
    }

    /// Prüft die Kette der Org vollständig (SEC-012 AC1): lückenlose `seq`, `prev_hash` gleich
    /// dem Hash des Vorgängers, neu berechneter Hash gleich dem gespeicherten, Kopf gleich dem
    /// letzten Eintrag.
    pub async fn verify_audit(&self, org: OrgId) -> Result<AuditVerification> {
        let org_s = org.to_string();
        let rows = sqlx::query(
            "SELECT seq, id, at, actor_id, action, target_kind, target_id, session_id, host_id, \
             count, details, prev_hash, hash FROM audit_entries WHERE org_id = ? \
             ORDER BY seq IS NULL, seq",
        )
        .bind(&org_s)
        .fetch_all(&self.pool)
        .await?;
        let head: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, hash FROM audit_heads WHERE org_id = ?")
                .bind(&org_s)
                .fetch_optional(&self.pool)
                .await?;
        let entries = rows.len() as u64;
        let invalid = |seq: u64, reason: &str| AuditVerification {
            entries,
            first_invalid: Some(seq),
            reason: Some(reason.to_owned()),
        };
        let mut prev = GENESIS_HASH.to_owned();
        let mut expected: u64 = 1;
        for row in &rows {
            let seq: Option<i64> = row.try_get("seq")?;
            let Some(seq) = seq.and_then(|s| u64::try_from(s).ok()) else {
                return Ok(invalid(expected, "Eintrag ohne Position in der Kette"));
            };
            if seq != expected {
                return Ok(invalid(expected, "Eintrag fehlt"));
            }
            let text = |c: &str| -> Result<String> {
                Ok(row.try_get::<Option<String>, _>(c)?.unwrap_or_default())
            };
            let count: i64 = row.try_get("count")?;
            let stored_prev = text("prev_hash")?;
            let stored_hash = text("hash")?;
            if stored_prev != prev {
                return Ok(invalid(seq, "prev_hash passt nicht zum Vorgänger"));
            }
            let (id, at, actor, action, kind, target, session, host, details) = (
                text("id")?,
                text("at")?,
                text("actor_id")?,
                text("action")?,
                text("target_kind")?,
                text("target_id")?,
                text("session_id")?,
                text("host_id")?,
                text("details")?,
            );
            let hash = chain_hash(
                &prev,
                &Stored {
                    org: &org_s,
                    seq,
                    id: &id,
                    at: &at,
                    actor: &actor,
                    action: &action,
                    target_kind: &kind,
                    target_id: &target,
                    session_id: &session,
                    host_id: &host,
                    count: u64::try_from(count).unwrap_or(u64::MAX),
                    details: &details,
                },
            );
            if hash != stored_hash {
                return Ok(invalid(seq, "Inhalt geändert (Hash passt nicht)"));
            }
            prev = hash;
            expected += 1;
        }
        let last = expected - 1;
        match head {
            Some((head_seq, head_hash)) => {
                let head_seq = u64::try_from(head_seq).unwrap_or(0);
                if head_seq > last {
                    return Ok(invalid(last + 1, "Eintrag fehlt (Ende der Kette)"));
                }
                if head_seq < last {
                    return Ok(invalid(head_seq + 1, "Eintrag hinter dem Kopf der Kette"));
                }
                if head_hash != prev {
                    return Ok(invalid(
                        last,
                        "Kopf der Kette passt nicht zum letzten Eintrag",
                    ));
                }
            }
            None if last > 0 => {
                return Ok(invalid(1, "Kopf der Kette fehlt"));
            }
            None => {}
        }
        Ok(AuditVerification {
            entries,
            first_invalid: None,
            reason: None,
        })
    }

    /// Audit-Einträge, neueste zuerst.
    pub async fn audit_entries(&self, org: OrgId, limit: u32) -> Result<Vec<AuditEntry>> {
        Ok(self
            .audit_page(
                org,
                &AuditQuery {
                    limit,
                    ..AuditQuery::default()
                },
            )
            .await?
            .items)
    }
}

async fn seal_in(conn: &mut SqliteConnection, org: OrgId, now: Timestamp) -> Result<u64> {
    let org_s = org.to_string();
    let rows = sqlx::query(
        "SELECT secret_ref, session_id, host_id, actor_id, purpose, window_start, last_at, count \
         FROM audit_usage_windows WHERE org_id = ? ORDER BY window_start, secret_ref",
    )
    .bind(&org_s)
    .fetch_all(&mut *conn)
    .await?;
    let now_t = now.as_offset_date_time();
    let mut sealed = 0;
    for row in rows {
        let start: Timestamp = parse(row.try_get("window_start")?)?;
        if start.as_offset_date_time() + AUDIT_USAGE_WINDOW > now_t {
            continue;
        }
        let secret_ref: String = row.try_get("secret_ref")?;
        let session: String = row.try_get("session_id")?;
        let host: String = row.try_get("host_id")?;
        let last: String = row.try_get("last_at")?;
        let count: i64 = row.try_get("count")?;
        let mut details = serde_json::json!({
            "purpose": row.try_get::<String, _>("purpose")?,
            "first_at": start.to_string(),
            "last_at": last,
        });
        let (action, target_kind, target_id) = match secret_ref.strip_prefix(CREDENTIAL_PREFIX) {
            Some(binding) => {
                details["binding"] = serde_json::Value::String(binding.to_owned());
                (CREDENTIAL_USED_ACTION, "credential", binding.to_owned())
            }
            None => (USED_ACTION, "secret", secret_ref.clone()),
        };
        let entry = NewAuditEntry {
            at: start,
            actor: parse(row.try_get("actor_id")?)?,
            action: action.into(),
            target_kind: target_kind.into(),
            target_id,
            session_id: (!session.is_empty()).then(|| parse(&session)).transpose()?,
            host_id: (!host.is_empty()).then(|| host.clone()),
            details,
        };
        append_in(
            conn,
            org,
            &entry,
            u64::try_from(count).map_err(|_| Error::corrupt("count negativ"))?,
        )
        .await?;
        sqlx::query(
            "DELETE FROM audit_usage_windows WHERE org_id = ? AND secret_ref = ? \
             AND session_id = ? AND host_id = ?",
        )
        .bind(&org_s)
        .bind(&secret_ref)
        .bind(&session)
        .bind(&host)
        .execute(&mut *conn)
        .await?;
        sealed += 1;
    }
    Ok(sealed)
}
