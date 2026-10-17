//! Einstellungen je User (UX-006 AC3): ein JSON-Dokument je Schlüssel, z. B. `notifications`.
//!
//! Der Store kennt die Struktur nicht; der Server prüft das Dokument gegen den Rust-Typ (und
//! damit auch seine Größe), bevor er es schreibt. Weil die Einstellungen am User hängen und
//! nicht am Gerät, sieht jeder Client nach dem Neuladen denselben Stand.

use beton_core::id::{OrgId, UserId};
use beton_core::time::Timestamp;
use serde_json::Value;

use crate::Store;
use crate::error::{Error, Result};

impl Store {
    /// Das Dokument unter `key`; `None`, solange der User nichts gespeichert hat.
    pub async fn user_setting(&self, org: OrgId, user: UserId, key: &str) -> Result<Option<Value>> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT value FROM user_settings WHERE org_id = ? AND user_id = ? AND key = ?",
        )
        .bind(org.to_string())
        .bind(user.to_string())
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|text| {
            serde_json::from_str(&text)
                .map_err(|e| Error::corrupt(format!("user_settings `{key}`: {e}")))
        })
        .transpose()
    }

    /// Ersetzt das Dokument unter `key`.
    pub async fn set_user_setting(
        &self,
        org: OrgId,
        user: UserId,
        key: &str,
        value: &Value,
    ) -> Result<()> {
        let text = serde_json::to_string(value)
            .map_err(|e| Error::corrupt(format!("user_settings `{key}`: {e}")))?;
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "INSERT INTO user_settings (org_id, user_id, key, value, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (org_id, user_id, key) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at",
        )
        .bind(org.to_string())
        .bind(user.to_string())
        .bind(key)
        .bind(text)
        .bind(Timestamp::now().to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::testutil::{org, store};

    #[tokio::test]
    async fn ux_006_ac3_settings_belong_to_the_user_not_the_connection() {
        let t = store().await;
        let user = t.local.user;
        assert_eq!(
            t.store
                .user_setting(org(&t), user, "notifications")
                .await
                .unwrap(),
            None
        );
        let doc = json!({"preview": false});
        t.store
            .set_user_setting(org(&t), user, "notifications", &doc)
            .await
            .unwrap();
        t.store
            .set_user_setting(org(&t), user, "notifications", &json!({"preview": true}))
            .await
            .unwrap();
        // Ein zweiter Store auf derselben Datenbank (anderer Prozess bzw. Neustart) liest
        // denselben Stand.
        drop(t.store);
        let again = crate::Store::open(t.dir.path(), crate::StoreOptions::default())
            .await
            .unwrap();
        assert_eq!(
            again
                .user_setting(t.local.org, user, "notifications")
                .await
                .unwrap(),
            Some(json!({"preview": true}))
        );
    }
}
