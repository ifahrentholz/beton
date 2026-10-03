//! Gespeicherte Antworten für `Idempotency-Key` (PROTO-010 AC2), 24 h gültig.

use beton_core::id::OrgId;
use beton_core::time::Timestamp;
use sqlx::Row;

use crate::Store;
use crate::error::Result;

/// So lange gilt ein Idempotency-Key.
pub const IDEMPOTENCY_TTL: time::Duration = time::Duration::hours(24);

/// Eine gespeicherte Antwort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    /// Hash über Methode, Pfad und Body des ursprünglichen Requests.
    pub request_hash: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Store {
    /// Gespeicherte Antwort zu einem Key, sofern jünger als 24 h. Ältere werden dabei entfernt.
    pub async fn idempotency_lookup(
        &self,
        org: OrgId,
        key: &str,
    ) -> Result<Option<StoredResponse>> {
        let cutoff = Timestamp::from(Timestamp::now().as_offset_date_time() - IDEMPOTENCY_TTL);
        sqlx::query("DELETE FROM idempotency_keys WHERE org_id = ? AND created_at < ?")
            .bind(org.to_string())
            .bind(cutoff.to_string())
            .execute(&self.pool)
            .await?;
        let row = sqlx::query(
            "SELECT request_hash, status, content_type, body FROM idempotency_keys \
             WHERE org_id = ? AND key = ?",
        )
        .bind(org.to_string())
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(StoredResponse {
                request_hash: row.try_get("request_hash")?,
                status: u16::try_from(row.try_get::<i64, _>("status")?).unwrap_or(500),
                content_type: row.try_get("content_type")?,
                body: row.try_get("body")?,
            })
        })
        .transpose()
    }

    /// Speichert die Antwort zu einem Key; ein bereits vorhandener Key bleibt unverändert.
    pub async fn idempotency_save(
        &self,
        org: OrgId,
        key: &str,
        response: &StoredResponse,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO idempotency_keys (org_id, key, request_hash, status, content_type, body, \
             created_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(org.to_string())
        .bind(key)
        .bind(&response.request_hash)
        .bind(i64::from(response.status))
        .bind(&response.content_type)
        .bind(&response.body)
        .bind(Timestamp::now().to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::store;

    #[tokio::test]
    async fn idempotency_keys_are_stored_per_org_and_expire() {
        let t = store().await;
        let org = t.local.org;
        let r = StoredResponse {
            request_hash: "h1".into(),
            status: 201,
            content_type: "application/json".into(),
            body: b"{}".to_vec(),
        };
        t.store.idempotency_save(org, "k1", &r).await.unwrap();
        assert_eq!(
            t.store.idempotency_lookup(org, "k1").await.unwrap(),
            Some(r.clone())
        );
        assert!(
            t.store
                .idempotency_lookup(OrgId::new(), "k1")
                .await
                .unwrap()
                .is_none()
        );

        let old =
            Timestamp::from(Timestamp::now().as_offset_date_time() - time::Duration::hours(25));
        sqlx::query("UPDATE idempotency_keys SET created_at = ? WHERE org_id = ?")
            .bind(old.to_string())
            .bind(org.to_string())
            .execute(&t.store.pool)
            .await
            .unwrap();
        assert!(
            t.store
                .idempotency_lookup(org, "k1")
                .await
                .unwrap()
                .is_none()
        );
    }
}
