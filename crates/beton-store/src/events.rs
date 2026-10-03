//! Append-only-Event-Log (DATA-002) mit Auslagerung großer Payloads (PROTO-001 AC4),
//! redigiertem `raw` (PROTO-001 AC3) und Blob-Zugriff über die Session (DATA-006).

use std::time::SystemTime;

use beton_core::event::{
    Actor, BlobRef, Event, EventBody, EventPayload, EventType, OffloadedPayload,
    PAYLOAD_INLINE_LIMIT, Persistence,
};
use beton_core::id::{OrgId, SessionId};
use beton_core::time::Timestamp;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::blobs::{GC_GRACE, GcReport};
use crate::error::{Error, Result};
use crate::sessions::{opt_parse, parse, session_exists, to_i64, to_u64};
use crate::{Store, projections};

/// Ein zum Schreiben vorbereitetes Event: Payload serialisiert bzw. ausgelagert, `raw` redigiert.
pub(crate) struct Prepared {
    pub event: Event,
    pub payload: EventPayload,
    pub payload_json: Option<String>,
    pub payload_ref: Option<BlobRef>,
    pub raw: Option<String>,
}

impl Store {
    /// Hängt dauerhafte Events an das Log einer Session an.
    ///
    /// `expected_head` ist die zuletzt bekannte `seq`, `epoch` der Fencing-Token des
    /// Home-Knotens. Die Events erhalten `expected_head + 1 …`; passt `head_seq` nicht mehr,
    /// schlägt der Append mit `seq_conflict` fehl, bei veralteter Epoch mit `stale_epoch`.
    /// Kernprojektionen werden in derselben Transaktion aktualisiert (DATA-005).
    ///
    /// Zurück kommen die Events in Draht-Form: mit `seq`, ohne `raw` und mit `payload_ref`
    /// statt `payload`, wenn die Nutzlast größer als 64 KiB ist.
    pub async fn append(
        &self,
        org: OrgId,
        session: SessionId,
        expected_head: u64,
        epoch: u64,
        events: Vec<Event>,
    ) -> Result<Vec<Event>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let prepared = self.prepare(session, events)?;
        let mut tx = self.pool.begin().await?;
        let written = self
            .write_prepared(&mut tx, org, session, expected_head, epoch, prepared)
            .await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Prüft Events, serialisiert Payloads, lagert große aus und redigiert `raw`.
    /// Läuft vor der Transaktion, weil Blob-Schreibvorgänge Dateisystem-IO sind.
    pub(crate) fn prepare(&self, session: SessionId, events: Vec<Event>) -> Result<Vec<Prepared>> {
        events
            .into_iter()
            .map(|mut event| {
                if event.session_id != session {
                    return Err(Error::InvalidEvent(format!(
                        "Event {} gehört zu {}, nicht zu {session}",
                        event.id, event.session_id
                    )));
                }
                let payload = match &event.body {
                    EventBody::Inline(p) => p.clone(),
                    EventBody::Offloaded(_) => {
                        return Err(Error::InvalidEvent(
                            "ausgelagerte Payloads werden vom Store selbst erzeugt".into(),
                        ));
                    }
                };
                if payload.persistence() == Persistence::Transient || event.transient {
                    return Err(Error::InvalidEvent(format!(
                        "{} ist transient und gehört nicht ins Log",
                        payload.type_name()
                    )));
                }
                let json = payload_json(&payload)?;
                let (payload_json, payload_ref) = if json.len() > PAYLOAD_INLINE_LIMIT {
                    let blob = self.blobs.put(json.as_bytes())?;
                    event.body = EventBody::Offloaded(OffloadedPayload {
                        event_type: payload.event_type(),
                        payload_ref: blob.clone(),
                    });
                    (None, Some(blob))
                } else {
                    (Some(json), None)
                };
                let raw = match event.raw.take() {
                    Some(raw) if self.options.store_raw => Some(self.redact_raw(&raw)?),
                    _ => None,
                };
                Ok(Prepared {
                    event,
                    payload,
                    payload_json,
                    payload_ref,
                    raw,
                })
            })
            .collect()
    }

    /// Schickt `raw` durch den Redaction-Hook. Ändert er nichts, bleiben die Original-Bytes
    /// erhalten (HAR-001 AC3); sonst wird der redigierte Wert kompakt serialisiert.
    fn redact_raw(&self, raw: &beton_core::event::RawJson) -> Result<String> {
        let original: Value = raw
            .to_value()
            .map_err(|e| Error::InvalidEvent(format!("raw ist kein JSON: {e}")))?;
        let mut redacted = original.clone();
        self.options.redactor.redact(&mut redacted);
        Ok(if redacted == original {
            raw.get().to_owned()
        } else {
            redacted.to_string()
        })
    }

    /// Schreibt vorbereitete Events in einer bestehenden Transaktion.
    pub(crate) async fn write_prepared(
        &self,
        conn: &mut SqliteConnection,
        org: OrgId,
        session: SessionId,
        expected_head: u64,
        epoch: u64,
        prepared: Vec<Prepared>,
    ) -> Result<Vec<Event>> {
        let count = prepared.len() as u64;
        let now = Timestamp::now().to_string();
        // Erste Anweisung ist ein Schreibzugriff: SQLite nimmt sofort den Schreib-Lock
        // (mit busy_timeout) statt eine Lesetransaktion später hochzustufen.
        let updated = sqlx::query(
            "UPDATE sessions SET head_seq = head_seq + ?, updated_at = ? \
             WHERE org_id = ? AND id = ? AND head_seq = ? AND epoch = ?",
        )
        .bind(to_i64(count)?)
        .bind(&now)
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(expected_head)?)
        .bind(to_i64(epoch)?)
        .execute(&mut *conn)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(append_rejection(conn, org, session, expected_head, epoch).await);
        }

        let mut written = Vec::with_capacity(prepared.len());
        for (i, p) in prepared.into_iter().enumerate() {
            let mut event = p.event;
            event.seq = expected_head + i as u64 + 1;
            let (actor_kind, actor_id) = actor_columns(&event.actor);
            sqlx::query(
                "INSERT INTO events (org_id, session_id, seq, id, ts, actor_kind, actor_id, actor, \
                 type, payload, payload_ref, turn_id, causation_id, epoch, redacted) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
            )
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(to_i64(event.seq)?)
            .bind(event.id.to_string())
            .bind(event.ts.to_string())
            .bind(actor_kind)
            .bind(actor_id)
            .bind(serde_json::to_string(&event.actor)?)
            .bind(event.type_name())
            .bind(p.payload_json)
            .bind(p.payload_ref.as_ref().map(BlobRef::to_string))
            .bind(event.turn_id.map(|t| t.to_string()))
            .bind(event.causation_id.map(|c| c.to_string()))
            .bind(to_i64(epoch)?)
            .execute(&mut *conn)
            .await?;
            if let Some(raw) = p.raw {
                sqlx::query(
                    "INSERT INTO event_raw (org_id, session_id, seq, raw, created_at) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(org.to_string())
                .bind(session.to_string())
                .bind(to_i64(event.seq)?)
                .bind(raw)
                .bind(&now)
                .execute(&mut *conn)
                .await?;
            }
            if let Some(blob) = &p.payload_ref {
                let size = std::fs::metadata(self.blobs.path(blob))?.len();
                reference_blob(conn, org, session, blob, size).await?;
            }
            projections::apply(conn, org, session, event.seq, event.ts, &p.payload).await?;
            written.push(event);
        }
        Ok(written)
    }

    /// Dauerhafte Events einer Session nach `after_seq`, aufsteigend, höchstens `limit`.
    pub async fn events(
        &self,
        org: OrgId,
        session: SessionId,
        after_seq: u64,
        limit: u32,
    ) -> Result<Vec<Event>> {
        let mut conn = self.pool.acquire().await?;
        session_exists(&mut conn, org, session).await?;
        sqlx::query(
            "SELECT seq, id, ts, actor, type, payload, payload_ref, turn_id, causation_id \
             FROM events WHERE org_id = ? AND session_id = ? AND seq > ? ORDER BY seq LIMIT ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(after_seq)?)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|row| event_from_row(session, row))
        .collect()
    }

    /// Das gespeicherte (redigierte) `raw` eines Events, falls vorhanden.
    pub async fn event_raw(
        &self,
        org: OrgId,
        session: SessionId,
        seq: u64,
    ) -> Result<Option<Value>> {
        let raw: Option<String> = sqlx::query_scalar(
            "SELECT raw FROM event_raw WHERE org_id = ? AND session_id = ? AND seq = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(seq)?)
        .fetch_optional(&self.pool)
        .await?;
        Ok(raw.map(|r| serde_json::from_str(&r)).transpose()?)
    }

    /// Die vollständige Nutzlast eines Events; ausgelagerte werden aus dem Blob-Store geladen.
    pub async fn resolve_payload(&self, org: OrgId, event: &Event) -> Result<EventPayload> {
        match &event.body {
            EventBody::Inline(p) => Ok(p.clone()),
            EventBody::Offloaded(o) => {
                let bytes = self
                    .session_blob(org, event.session_id, &o.payload_ref)
                    .await?;
                payload_from_json(o.event_type, &String::from_utf8_lossy(&bytes))
            }
        }
    }

    /// Speichert einen Blob für eine Session (Anhänge, Tool-Ergebnisse, Snapshots).
    pub async fn put_session_blob(
        &self,
        org: OrgId,
        session: SessionId,
        bytes: &[u8],
    ) -> Result<BlobRef> {
        let blob = self.blobs.put(bytes)?;
        let mut tx = self.pool.begin().await?;
        session_exists(&mut tx, org, session).await?;
        reference_blob(&mut tx, org, session, &blob, bytes.len() as u64).await?;
        tx.commit().await?;
        Ok(blob)
    }

    /// Liest einen Blob über die referenzierende Session (DATA-006 AC1). Ein Blob, den diese
    /// Session nicht referenziert, gilt als nicht vorhanden – auch wenn der Inhalt in einer
    /// anderen Session existiert. Einen Zugriff allein per Hash gibt es nicht.
    pub async fn session_blob(
        &self,
        org: OrgId,
        session: SessionId,
        blob: &BlobRef,
    ) -> Result<Vec<u8>> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM blob_refs WHERE org_id = ? AND session_id = ? AND sha256 = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(blob.hex())
        .fetch_optional(&self.pool)
        .await?;
        if found.is_none() {
            return Err(Error::NotFound(format!("Blob {blob} in Session {session}")));
        }
        self.blobs.read(blob)
    }

    /// Mark-and-Sweep (DATA-006): Blobs ohne Referenz, die älter als 24 h sind, werden
    /// aus der Datenbank und – wenn keine Org sie mehr kennt – aus dem Dateisystem entfernt.
    /// `now` ist injizierbar, damit Tests die Frist überspringen können.
    pub async fn gc_blobs(&self, now: SystemTime) -> Result<GcReport> {
        let cutoff = now.checked_sub(GC_GRACE).unwrap_or(SystemTime::UNIX_EPOCH);
        let cutoff_ts = Timestamp::from(time::OffsetDateTime::from(cutoff)).to_string();
        // Org-übergreifend: der Blob-Store ist ein gemeinsamer Inhaltsspeicher.
        let rows_deleted = sqlx::query(
            "DELETE FROM blobs /* org-übergreifend: GC */ WHERE created_at < ? AND NOT EXISTS ( \
               SELECT 1 FROM blob_refs r WHERE r.org_id = blobs.org_id AND r.sha256 = blobs.sha256)",
        )
        .bind(&cutoff_ts)
        .execute(&self.pool)
        .await?
        .rows_affected();
        let known: std::collections::HashSet<String> =
            sqlx::query_scalar("SELECT DISTINCT sha256 FROM blobs /* org-übergreifend: GC */")
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .collect();
        let mut files_deleted = 0;
        for (blob, modified) in self.blobs.list()? {
            if !known.contains(blob.hex()) && modified < cutoff && self.blobs.remove(&blob)? {
                files_deleted += 1;
            }
        }
        Ok(GcReport {
            rows_deleted,
            files_deleted,
        })
    }

    /// Alle dauerhaften Events einer Session mit vollständiger Nutzlast (für Rebuilds).
    pub(crate) async fn full_events(
        &self,
        conn: &mut SqliteConnection,
        org: OrgId,
        session: SessionId,
    ) -> Result<Vec<(u64, Timestamp, EventPayload)>> {
        let rows = sqlx::query(
            "SELECT seq, ts, type, payload, payload_ref FROM events \
             WHERE org_id = ? AND session_id = ? ORDER BY seq",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let event_type = event_type(row.try_get("type")?)?;
            let json = match row.try_get::<Option<String>, _>("payload")? {
                Some(json) => json,
                None => {
                    let blob: String = row.try_get("payload_ref")?;
                    let blob: BlobRef = parse(&blob)?;
                    String::from_utf8_lossy(&self.blobs.read(&blob)?).into_owned()
                }
            };
            out.push((
                to_u64(row.try_get("seq")?)?,
                parse(row.try_get("ts")?)?,
                payload_from_json(event_type, &json)?,
            ));
        }
        Ok(out)
    }

    /// Prüft, ob die Blob-Datei zu einer Referenz existiert (Tests, Diagnose).
    pub fn blob_file_exists(&self, blob: &BlobRef) -> bool {
        self.blobs.exists(blob)
    }
}

async fn append_rejection(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    expected_head: u64,
    epoch: u64,
) -> Error {
    let row = sqlx::query("SELECT head_seq, epoch FROM sessions WHERE org_id = ? AND id = ?")
        .bind(org.to_string())
        .bind(session.to_string())
        .fetch_optional(&mut *conn)
        .await;
    match row {
        Ok(None) => Error::NotFound(format!("Session {session}")),
        Ok(Some(row)) => {
            let head: i64 = row.try_get("head_seq").unwrap_or_default();
            let current: i64 = row.try_get("epoch").unwrap_or_default();
            let (head, current) = (head.max(0) as u64, current.max(0) as u64);
            if current != epoch {
                Error::StaleEpoch {
                    given: epoch,
                    current,
                }
            } else {
                Error::SeqConflict {
                    expected: expected_head,
                    actual: head,
                }
            }
        }
        Err(e) => e.into(),
    }
}

async fn reference_blob(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    blob: &BlobRef,
    size: u64,
) -> Result<()> {
    let now = Timestamp::now().to_string();
    sqlx::query(
        "INSERT INTO blobs (org_id, sha256, size, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT DO NOTHING",
    )
    .bind(org.to_string())
    .bind(blob.hex())
    .bind(to_i64(size)?)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO blob_refs (org_id, session_id, sha256, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT DO NOTHING",
    )
    .bind(org.to_string())
    .bind(session.to_string())
    .bind(blob.hex())
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn actor_columns(actor: &Actor) -> (&'static str, Option<String>) {
    match actor {
        Actor::User { id, .. } => ("user", Some(id.to_string())),
        Actor::Agent { id, .. } => ("agent", id.map(|i| i.to_string())),
        Actor::System { .. } => ("system", None),
    }
}

fn event_type(name: &str) -> Result<EventType> {
    EventType::parse(name).ok_or_else(|| Error::corrupt(format!("unbekannter Event-Typ `{name}`")))
}

/// Nur die Nutzlast als JSON (ohne `type`).
fn payload_json(payload: &EventPayload) -> Result<String> {
    let mut tagged = serde_json::to_value(payload)?;
    let inner = tagged
        .get_mut("payload")
        .map(Value::take)
        .unwrap_or(Value::Null);
    Ok(inner.to_string())
}

fn payload_from_json(event_type: EventType, json: &str) -> Result<EventPayload> {
    let payload: Value = serde_json::from_str(json)?;
    Ok(serde_json::from_value(serde_json::json!({
        "type": event_type.as_str(),
        "payload": payload,
    }))?)
}

fn event_from_row(session: SessionId, row: &SqliteRow) -> Result<Event> {
    let event_type = event_type(row.try_get("type")?)?;
    let body = match row.try_get::<Option<String>, _>("payload")? {
        Some(json) => EventBody::Inline(payload_from_json(event_type, &json)?),
        None => EventBody::Offloaded(OffloadedPayload {
            event_type,
            payload_ref: parse(row.try_get("payload_ref")?)?,
        }),
    };
    let actor: String = row.try_get("actor")?;
    Ok(Event {
        v: beton_core::event::ENVELOPE_VERSION,
        id: parse(row.try_get("id")?)?,
        session_id: session,
        seq: to_u64(row.try_get("seq")?)?,
        ts: parse(row.try_get("ts")?)?,
        actor: serde_json::from_str(&actor)?,
        body,
        turn_id: opt_parse(row.try_get("turn_id")?)?,
        causation_id: opt_parse(row.try_get("causation_id")?)?,
        raw: None,
        transient: false,
        tseq: None,
    })
}
