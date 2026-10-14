//! Manuelle Nachredaktion eines Events (DATA-012).
//!
//! Die einzige erlaubte Änderung bestehender Events: Die Nutzlast wird ganz oder an
//! JSON-Pointer-Pfaden durch `[REDACTED:manual]` ersetzt. `seq`, `type`, `actor` und `ts`
//! bleiben, ebenso IDs (`*_id`), Aufzählungen und Zeitstempel, damit das Event lesbar bleibt;
//! `redacted = 1`. `raw` und ausgelagerte Blobs werden gelöscht. In derselben Transaktion
//! entstehen das dauerhafte `event.redacted` und der Audit-Eintrag, die Projektionen (Suche,
//! Approvals) werden für die Session neu aufgebaut. Danach werden die Daten auch physisch
//! entfernt: `secure_delete` für die Transaktion, Checkpoint des WAL, Blob-Dateien ohne
//! weitere Referenz.

use beton_core::event::{BlobRef, Event, EventType, PAYLOAD_INLINE_LIMIT};
use beton_core::id::{OrgId, SessionId};
use serde_json::Value;
use sqlx::{Connection as _, Row, SqliteConnection};

use crate::Store;
use crate::audit::NewAuditEntry;
use crate::error::{Error, Result};
use crate::events::{event_from_row, event_type, payload_from_json, reference_blob};
use crate::sessions::{parse, to_i64, to_u64};

/// Ersetzungstext der manuellen Redaktion.
pub const MANUAL_MARKER: &str = "[REDACTED:manual]";

/// Eine Nachredaktion.
#[derive(Debug)]
pub struct ManualRedaction {
    /// Das zu schwärzende Event.
    pub seq: u64,
    /// JSON-Pointer in die Nutzlast (ohne `type`); leer heißt: die ganze Nutzlast.
    pub paths: Vec<String>,
    /// Das dokumentierende `event.redacted` (`seq` vergibt der Store).
    pub record: Event,
    pub audit: NewAuditEntry,
}

/// Ergebnis: das geschwärzte Event und das neue `event.redacted`, beide in Draht-Form.
#[derive(Debug, Clone)]
pub struct ManualRedactionOutcome {
    pub target: Event,
    pub record: Event,
}

impl Store {
    /// Schwärzt ein Event nachträglich (DATA-012). `invalid_event`, wenn ein Pfad fehlt oder
    /// die Ersetzung den Typ der Nutzlast verletzen würde; dann bleibt alles unverändert.
    pub async fn redact_event(
        &self,
        org: OrgId,
        session: SessionId,
        req: ManualRedaction,
    ) -> Result<ManualRedactionOutcome> {
        let prepared = self.prepare(session, vec![req.record])?;
        let mut conn = self.pool.acquire().await?;
        // Gelöschte Inhalte werden mit Nullen überschrieben, statt in freien Seiten zu bleiben.
        sqlx::query("PRAGMA secure_delete = ON")
            .execute(&mut *conn)
            .await?;
        let result = self
            .redact_in(
                &mut conn, org, session, req.seq, &req.paths, prepared, &req.audit,
            )
            .await;
        let _ = sqlx::query("PRAGMA secure_delete = OFF")
            .execute(&mut *conn)
            .await;
        drop(conn);
        let (outcome, dropped) = result?;
        for blob in &dropped {
            self.blobs.remove(blob)?;
        }
        // Alte Seiten mit dem Inhalt stehen noch im WAL; der Checkpoint leert es (best effort,
        // ein gleichzeitiger Leser kann ihn verzögern).
        if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            tracing::warn!("WAL-Checkpoint nach Nachredaktion gescheitert: {e}");
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    async fn redact_in(
        &self,
        conn: &mut SqliteConnection,
        org: OrgId,
        session: SessionId,
        seq: u64,
        paths: &[String],
        prepared: Vec<crate::events::Prepared>,
        audit: &NewAuditEntry,
    ) -> Result<(ManualRedactionOutcome, Vec<BlobRef>)> {
        let mut tx = conn.begin_with("BEGIN IMMEDIATE").await?;
        crate::sessions::session_exists(&mut tx, org, session).await?;
        let row = sqlx::query(
            "SELECT type, payload, payload_ref FROM events \
             WHERE org_id = ? AND session_id = ? AND seq = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(seq)?)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Event {seq} in Session {session}")))?;
        let kind = event_type(row.try_get("type")?)?;
        let old_ref: Option<BlobRef> = row
            .try_get::<Option<String>, _>("payload_ref")?
            .map(|r| parse(&r))
            .transpose()?;
        let json = match (row.try_get::<Option<String>, _>("payload")?, &old_ref) {
            (Some(json), _) => json,
            (None, Some(blob)) => String::from_utf8_lossy(&self.blobs.read(blob)?).into_owned(),
            (None, None) => return Err(Error::corrupt(format!("Event {seq} ohne Nutzlast"))),
        };
        let original: Value = serde_json::from_str(&json)?;
        let (redacted, removed) = if paths.is_empty() {
            redact_whole(kind, &original)
        } else {
            redact_paths(kind, &original, paths)?
        };

        // Neue Nutzlast: inline oder (weiterhin zu groß) als neuer Blob.
        let json = redacted.to_string();
        let (payload, payload_ref) = if json.len() > PAYLOAD_INLINE_LIMIT {
            let blob = self.blobs.put(json.as_bytes())?;
            reference_blob(&mut tx, org, session, &blob, json.len() as u64).await?;
            (None, Some(blob))
        } else {
            (Some(json), None)
        };
        sqlx::query(
            "UPDATE events SET payload = ?, payload_ref = ?, redacted = 1 \
             WHERE org_id = ? AND session_id = ? AND seq = ?",
        )
        .bind(payload)
        .bind(payload_ref.as_ref().map(BlobRef::to_string))
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(seq)?)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM event_raw WHERE org_id = ? AND session_id = ? AND seq = ?")
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(to_i64(seq)?)
            .execute(&mut *tx)
            .await?;

        // Ausgelagerte Blobs: die alte Nutzlast und Blob-Referenzen in geschwärzten Teilen.
        let mut candidates: Vec<BlobRef> = old_ref.into_iter().collect();
        for value in &removed {
            blob_refs(value, &mut candidates);
        }
        candidates.sort_by_key(BlobRef::to_string);
        candidates.dedup();
        let mut dropped = Vec::new();
        for blob in candidates {
            if payload_ref.as_ref() == Some(&blob) {
                continue;
            }
            let used: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM events WHERE org_id = ? AND session_id = ? AND payload_ref = ? LIMIT 1",
            )
            .bind(org.to_string())
            .bind(session.to_string())
            .bind(blob.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            if used.is_some() {
                continue;
            }
            sqlx::query("DELETE FROM blob_refs WHERE org_id = ? AND session_id = ? AND sha256 = ?")
                .bind(org.to_string())
                .bind(session.to_string())
                .bind(blob.hex())
                .execute(&mut *tx)
                .await?;
            let in_org: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM blob_refs WHERE org_id = ? AND sha256 = ? LIMIT 1",
            )
            .bind(org.to_string())
            .bind(blob.hex())
            .fetch_optional(&mut *tx)
            .await?;
            if in_org.is_some() {
                continue;
            }
            sqlx::query("DELETE FROM blobs WHERE org_id = ? AND sha256 = ?")
                .bind(org.to_string())
                .bind(blob.hex())
                .execute(&mut *tx)
                .await?;
            // Die Datei ist ein gemeinsamer Inhaltsspeicher: weg nur, wenn keine Org sie kennt.
            let anywhere: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM blobs /* org-übergreifend: geteilte Datei */ WHERE sha256 = ? LIMIT 1",
            )
            .bind(blob.hex())
            .fetch_optional(&mut *tx)
            .await?;
            if anywhere.is_none() {
                dropped.push(blob);
            }
        }

        // `event.redacted`, Audit und Projektionen in derselben Transaktion.
        let head = sqlx::query("SELECT head_seq, epoch FROM sessions WHERE org_id = ? AND id = ?")
            .bind(org.to_string())
            .bind(session.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let head_seq = to_u64(head.try_get("head_seq")?)?;
        let epoch = to_u64(head.try_get("epoch")?)?;
        let mut written = self
            .write_prepared(&mut tx, org, session, head_seq, epoch, prepared)
            .await?;
        crate::audit::append_in(&mut tx, org, audit, 1).await?;
        self.rebuild_session(&mut tx, org, session).await?;
        // Gelöschte Suchdokumente auch aus den FTS-Segmenten entfernen.
        sqlx::query("INSERT INTO search_fts (search_fts) /* org-übergreifend: FTS-Wartung */ VALUES ('optimize')")
            .execute(&mut *tx)
            .await?;
        let target = sqlx::query(
            "SELECT seq, id, ts, actor, type, payload, payload_ref, turn_id, causation_id \
             FROM events WHERE org_id = ? AND session_id = ? AND seq = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(to_i64(seq)?)
        .fetch_one(&mut *tx)
        .await?;
        let target = event_from_row(session, &target)?;
        tx.commit().await?;
        let record = written
            .pop()
            .ok_or_else(|| Error::corrupt("event.redacted nicht geschrieben"))?;
        Ok((ManualRedactionOutcome { target, record }, dropped))
    }
}

fn marker() -> Value {
    Value::String(MANUAL_MARKER.to_owned())
}

/// Lässt sich der Wert noch als Nutzlast dieses Typs lesen?
fn readable(kind: EventType, value: &Value) -> bool {
    payload_from_json(kind, &value.to_string()).is_ok()
}

/// Ganze Nutzlast: jeder String wird ersetzt, außer IDs (`id`, `*_id`) und Werten, deren Typ
/// ein Ersetzen nicht zulässt (Aufzählungen, Zeitstempel, typisierte IDs).
fn redact_whole(kind: EventType, original: &Value) -> (Value, Vec<Value>) {
    let mut leaves = Vec::new();
    string_leaves(original, String::new(), &mut leaves);
    let mut all = original.clone();
    for p in &leaves {
        if let Some(slot) = all.pointer_mut(p) {
            *slot = marker();
        }
    }
    let removed = |ps: &[String]| -> Vec<Value> {
        ps.iter()
            .filter_map(|p| original.pointer(p).cloned())
            .collect()
    };
    if readable(kind, &all) {
        return (all, removed(&leaves));
    }
    // Einzeln prüfen, welche Strings sich ersetzen lassen.
    let mut current = original.clone();
    let mut done = Vec::new();
    for p in leaves {
        let mut trial = current.clone();
        if let Some(slot) = trial.pointer_mut(&p) {
            *slot = marker();
        }
        if readable(kind, &trial) {
            current = trial;
            done.push(p);
        }
    }
    (current, removed(&done))
}

/// Pfade: der Wert am Pointer wird ersetzt; jeder Pfad muss existieren und die Nutzlast muss
/// danach noch lesbar sein.
fn redact_paths(
    kind: EventType,
    original: &Value,
    paths: &[String],
) -> Result<(Value, Vec<Value>)> {
    let mut value = original.clone();
    let mut removed = Vec::new();
    for p in paths {
        if !p.starts_with('/') {
            return Err(Error::InvalidEvent(format!(
                "`{p}` ist kein JSON-Pointer (beginnt mit `/`)"
            )));
        }
        let slot = value.pointer_mut(p).ok_or_else(|| {
            Error::InvalidEvent(format!("Pfad `{p}` gibt es in der Nutzlast nicht"))
        })?;
        removed.push(std::mem::replace(slot, marker()));
    }
    if !readable(kind, &value) {
        return Err(Error::InvalidEvent(format!(
            "Die Pfade lassen sich nicht schwärzen, ohne den Typ von `{}` zu verletzen",
            kind.as_str()
        )));
    }
    Ok((value, removed))
}

/// JSON-Pointer aller Strings außer IDs.
fn string_leaves(value: &Value, at: String, out: &mut Vec<String>) {
    match value {
        Value::String(_) => out.push(at),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                string_leaves(item, format!("{at}/{i}"), out);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key == "id" || key.ends_with("_id") {
                    continue;
                }
                let escaped = key.replace('~', "~0").replace('/', "~1");
                string_leaves(item, format!("{at}/{escaped}"), out);
            }
        }
        _ => {}
    }
}

/// Blob-Referenzen (`sha256:…`) in einem Wert.
fn blob_refs(value: &Value, out: &mut Vec<BlobRef>) {
    match value {
        Value::String(s) => {
            if let Ok(blob) = s.parse::<BlobRef>() {
                out.push(blob);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| blob_refs(v, out)),
        Value::Object(map) => map.values().for_each(|v| blob_refs(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use beton_core::event::{ApprovalKind, ApprovalRequested, EventPayload, TimeoutAction};
    use beton_core::id::ApprovalId;
    use serde_json::json;

    use super::*;

    fn value(p: &EventPayload) -> Value {
        serde_json::to_value(p).unwrap()["payload"].take()
    }

    #[test]
    fn data_012_whole_payload_keeps_ids_enums_and_timestamps() {
        let id = ApprovalId::new();
        let p = EventPayload::ApprovalRequested(ApprovalRequested {
            approval_id: id,
            kind: ApprovalKind::Tool,
            subject: json!({ "tool": "Bash", "args": { "command": "echo hunter2" }, "call_id": "c1" }),
            options: vec!["allow".into(), "deny".into()],
            on_timeout: TimeoutAction::Deny,
            ..ApprovalRequested::default()
        });
        let original = value(&p);
        let (redacted, removed) = redact_whole(EventType::ApprovalRequested, &original);
        assert!(readable(EventType::ApprovalRequested, &redacted));
        assert_eq!(redacted["approval_id"], json!(id.to_string()));
        assert_eq!(redacted["kind"], original["kind"]);
        assert_eq!(redacted["on_timeout"], original["on_timeout"]);
        assert_eq!(redacted["expires_at"], original["expires_at"]);
        assert_eq!(redacted["subject"]["call_id"], "c1");
        assert_eq!(redacted["subject"]["args"]["command"], MANUAL_MARKER);
        assert_eq!(redacted["options"], json!([MANUAL_MARKER, MANUAL_MARKER]));
        assert!(!redacted.to_string().contains("hunter2"));
        assert!(removed.contains(&json!("echo hunter2")));
    }

    #[test]
    fn data_012_paths_must_exist_and_keep_the_type() {
        let p = EventPayload::ApprovalRequested(ApprovalRequested::default());
        let original = value(&p);
        let kind = EventType::ApprovalRequested;
        assert!(redact_paths(kind, &original, &["/subject".into()]).is_ok());
        for bad in ["/kind", "/gibt-es-nicht", "subject", ""] {
            assert!(
                redact_paths(kind, &original, &[bad.into()]).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn data_012_blob_references_in_removed_parts_are_found() {
        let hex = "a".repeat(64);
        let mut out = Vec::new();
        blob_refs(
            &json!({ "result_ref": format!("sha256:{hex}"), "x": ["sha256:kurz"] }),
            &mut out,
        );
        assert_eq!(out, vec![BlobRef::from_hex(&hex).unwrap()]);
    }
}
