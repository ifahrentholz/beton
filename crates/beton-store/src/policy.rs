//! Policy-Daten im Store (03-policies.md):
//!
//! - **Set-Snapshots (POL-027 AC2):** Manifest eines effektiven Sets je Session und
//!   `policy_set_hash`; die Inhalte der Sets liegen inhaltsadressiert im Blob-Store der
//!   Session (Version = `sha256:` des Inhalts). Explain baut daraus das damals gültige Set auf.
//! - **Shadow-Treffer (POL-027 AC4):** Projektion der `dry_run`-Treffer aus `policy.decision`
//!   je Session, Tag, Regel und Verdikt; aggregiert über die Org für die Shadow-Ansicht.

use beton_core::event::PolicyDecision;
use beton_core::id::{OrgId, SessionId};
use beton_core::time::Timestamp;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};

use crate::Store;
use crate::error::{Error, Result};
use crate::sessions::{parse, session_exists, to_i64, to_u64};

/// Höchstens so viele Sets je Snapshot.
pub const MAX_SNAPSHOT_SETS: usize = 256;
/// Größter Inhalt eines Sets (wie die größte Policy-Datei, 1 MiB).
pub const MAX_SNAPSHOT_SOURCE: usize = 1 << 20;

/// Treffer einer Regel im Shadow-Betrieb an einem Tag, summiert über die Sessions der Org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShadowHits {
    /// Kalendertag in UTC (`YYYY-MM-DD`).
    pub day: String,
    pub scope: String,
    pub set_name: String,
    pub rule_id: String,
    /// Verdikt, das die Regel ohne `dry_run` gehabt hätte (`deny` = „würde ablehnen“).
    pub verdict: String,
    pub hits: u64,
    /// Letzter Treffer dieses Tages.
    pub last_session_id: SessionId,
    pub last_seq: u64,
    pub last_at: Timestamp,
}

fn version_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

impl Store {
    /// Speichert den Snapshot eines effektiven Sets für eine Session. Jeder Inhalt muss zu
    /// seiner Version passen, und jede Version im Manifest braucht ihren Inhalt. Liefert
    /// `false`, wenn der Snapshot schon existierte.
    pub async fn put_policy_snapshot(
        &self,
        org: OrgId,
        session: SessionId,
        policy_set_hash: &str,
        manifest: &Value,
        sources: &[(String, Vec<u8>)],
    ) -> Result<bool> {
        let invalid = |m: String| Error::InvalidEvent(format!("Policy-Snapshot: {m}"));
        if !policy_set_hash.starts_with("sha256:") {
            return Err(invalid("policy_set_hash ist kein sha256".into()));
        }
        if manifest["policy_set_hash"].as_str() != Some(policy_set_hash) {
            return Err(invalid("Manifest gehört zu einem anderen Set".into()));
        }
        let sets = manifest["sets"]
            .as_array()
            .ok_or_else(|| invalid("Manifest ohne `sets`".into()))?;
        if sets.len() > MAX_SNAPSHOT_SETS || sources.len() > MAX_SNAPSHOT_SETS {
            return Err(invalid(format!("mehr als {MAX_SNAPSHOT_SETS} Sets")));
        }
        for (version, bytes) in sources {
            if bytes.len() > MAX_SNAPSHOT_SOURCE {
                return Err(invalid(format!("Inhalt {version} ist größer als 1 MiB")));
            }
            if version_of(bytes) != *version {
                return Err(invalid(format!("Inhalt passt nicht zur Version {version}")));
            }
        }
        for set in sets {
            let version = set["version"].as_str().unwrap_or_default();
            if !sources.iter().any(|(v, _)| v == version) {
                return Err(invalid(format!("Inhalt zu Version `{version}` fehlt")));
            }
        }
        let mut blobs = Vec::with_capacity(sources.len());
        for (_, bytes) in sources {
            blobs.push((self.blobs.put(bytes)?, bytes.len() as u64));
        }
        let mut tx = self.write_tx().await?;
        session_exists(&mut tx, org, session).await?;
        for (blob, size) in &blobs {
            crate::events::reference_blob(&mut tx, org, session, blob, *size).await?;
        }
        let inserted = sqlx::query(
            "INSERT INTO policy_set_snapshots (org_id, session_id, policy_set_hash, manifest, \
             created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(policy_set_hash)
        .bind(serde_json::to_string(manifest)?)
        .bind(Timestamp::now().to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(inserted > 0)
    }

    /// Manifest und Inhalte (Version → Inhalt) eines Snapshots der Session; `None`, wenn für
    /// diesen `policy_set_hash` keiner gespeichert ist. Inhalte werden über die Session gelesen
    /// (DATA-006 AC1) und gegen ihren Hash geprüft.
    pub async fn policy_snapshot(
        &self,
        org: OrgId,
        session: SessionId,
        policy_set_hash: &str,
    ) -> Result<Option<(Value, Vec<(String, Vec<u8>)>)>> {
        let manifest: Option<String> = sqlx::query_scalar(
            "SELECT manifest FROM policy_set_snapshots \
             WHERE org_id = ? AND session_id = ? AND policy_set_hash = ?",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(policy_set_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let manifest: Value = serde_json::from_str(&manifest)?;
        let mut sources = Vec::new();
        for set in manifest["sets"].as_array().into_iter().flatten() {
            let version = set["version"].as_str().unwrap_or_default();
            if sources
                .iter()
                .any(|(v, _): &(String, Vec<u8>)| v == version)
            {
                continue;
            }
            let hex = version.strip_prefix("sha256:").unwrap_or_default();
            let blob = beton_core::event::BlobRef::from_hex(hex).map_err(Error::corrupt)?;
            let bytes = self.session_blob(org, session, &blob).await?;
            sources.push((version.to_owned(), bytes));
        }
        Ok(Some((manifest, sources)))
    }

    /// Shadow-Treffer der Org je Tag, Regel und Verdikt im Zeitraum `[from_day, to_day]`.
    pub async fn policy_shadow(
        &self,
        org: OrgId,
        from_day: &str,
        to_day: &str,
    ) -> Result<Vec<ShadowHits>> {
        // SQLite: Spalten ohne Aggregat stammen aus der Zeile mit MAX(last_at).
        sqlx::query(
            "SELECT day, scope, set_name, rule_id, verdict, SUM(hits) AS hits, \
             MAX(last_at) AS last_at, session_id, last_seq FROM policy_shadow_daily \
             WHERE org_id = ? AND day >= ? AND day <= ? \
             GROUP BY day, scope, set_name, rule_id, verdict \
             ORDER BY scope, set_name, rule_id, verdict, day",
        )
        .bind(org.to_string())
        .bind(from_day)
        .bind(to_day)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            Ok(ShadowHits {
                day: row.try_get("day")?,
                scope: row.try_get("scope")?,
                set_name: row.try_get("set_name")?,
                rule_id: row.try_get("rule_id")?,
                verdict: row.try_get("verdict")?,
                hits: to_u64(row.try_get("hits")?)?,
                last_session_id: parse(row.try_get("session_id")?)?,
                last_seq: to_u64(row.try_get("last_seq")?)?,
                last_at: parse(row.try_get("last_at")?)?,
            })
        })
        .collect()
    }
}

/// Projektion: zählt die `dry_run`-Treffer eines `policy.decision` (POL-027 AC4).
pub(crate) async fn project_decision(
    conn: &mut SqliteConnection,
    org: OrgId,
    session: SessionId,
    seq: u64,
    ts: Timestamp,
    decision: &PolicyDecision,
) -> Result<()> {
    let ts_s = ts.to_string();
    for m in decision
        .matched
        .iter()
        .filter(|m| m["dry_run"].as_bool() == Some(true))
    {
        let text = |k: &str| m[k].as_str().unwrap_or_default().to_owned();
        sqlx::query(
            "INSERT INTO policy_shadow_daily (org_id, session_id, day, scope, set_name, rule_id, \
             verdict, hits, last_seq, last_at) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?) \
             ON CONFLICT (org_id, session_id, day, scope, set_name, rule_id, verdict) DO UPDATE SET \
               hits = policy_shadow_daily.hits + 1, \
               last_seq = MAX(policy_shadow_daily.last_seq, excluded.last_seq), \
               last_at = MAX(policy_shadow_daily.last_at, excluded.last_at)",
        )
        .bind(org.to_string())
        .bind(session.to_string())
        .bind(&ts_s[..10])
        .bind(text("scope"))
        .bind(text("set"))
        .bind(text("rule_id"))
        .bind(text("verdict"))
        .bind(to_i64(seq)?)
        .bind(&ts_s)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
