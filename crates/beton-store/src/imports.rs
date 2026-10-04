//! Herkunft importierter Sessions (SES-008): Dedup-Schlüssel `(host_id, harness,
//! vendor_session_id)`.

use std::collections::HashMap;

use beton_core::id::{NodeId, OrgId, SessionId};
use beton_core::time::Timestamp;
use sqlx::Row;

use crate::Store;
use crate::error::{Error, Result};
use crate::sessions::parse;

/// Ein Import-Eintrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportKey<'a> {
    /// Host, auf dem die Vendor-Dateien liegen.
    pub host: NodeId,
    pub harness: &'a str,
    pub vendor_session_id: &'a str,
}

impl Store {
    /// Bereits importierte Session zu einem Dedup-Schlüssel.
    pub async fn imported_session(
        &self,
        org: OrgId,
        key: &ImportKey<'_>,
    ) -> Result<Option<SessionId>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT session_id FROM session_imports WHERE org_id = ? AND host_id = ? \
             AND harness = ? AND vendor_session_id = ?",
        )
        .bind(org.to_string())
        .bind(key.host.to_string())
        .bind(key.harness)
        .bind(key.vendor_session_id)
        .fetch_optional(&self.pool)
        .await?;
        id.as_deref().map(parse).transpose()
    }

    /// Alle Importe eines Harness auf einem Host: Vendor-Session-ID → Session.
    pub async fn imported_sessions(
        &self,
        org: OrgId,
        host: NodeId,
        harness: &str,
    ) -> Result<HashMap<String, SessionId>> {
        let rows = sqlx::query(
            "SELECT vendor_session_id, session_id FROM session_imports \
             WHERE org_id = ? AND host_id = ? AND harness = ?",
        )
        .bind(org.to_string())
        .bind(host.to_string())
        .bind(harness)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get::<String, _>("vendor_session_id")?,
                    parse(&r.try_get::<String, _>("session_id")?)?,
                ))
            })
            .collect()
    }

    /// Vermerkt einen Import. Ohne `replace` ist ein vorhandener Eintrag ein Konflikt; mit
    /// `replace` (Import mit `force`) zeigt der Schlüssel danach auf die neue Session.
    pub async fn record_import(
        &self,
        org: OrgId,
        key: &ImportKey<'_>,
        session: SessionId,
        at: Timestamp,
        replace: bool,
    ) -> Result<()> {
        let sql = if replace {
            "INSERT INTO session_imports (org_id, host_id, harness, vendor_session_id, \
             session_id, imported_at) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (org_id, host_id, harness, vendor_session_id) \
             DO UPDATE SET session_id = excluded.session_id, imported_at = excluded.imported_at"
        } else {
            "INSERT INTO session_imports (org_id, host_id, harness, vendor_session_id, \
             session_id, imported_at) VALUES (?, ?, ?, ?, ?, ?)"
        };
        let result = sqlx::query(sql)
            .bind(org.to_string())
            .bind(key.host.to_string())
            .bind(key.harness)
            .bind(key.vendor_session_id)
            .bind(session.to_string())
            .bind(at.to_string())
            .execute(&self.pool)
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(Error::Conflict(format!(
                    "Vendor-Session {} ist bereits importiert",
                    key.vendor_session_id
                )))
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use beton_core::event::{SessionKind, SessionTrigger};
    use beton_core::id::{PrincipalId, UserId};

    use super::*;
    use crate::NewSession;
    use crate::testutil::{org, store};

    async fn session(t: &crate::testutil::TestStore) -> SessionId {
        let id = SessionId::new();
        t.store
            .create_session(
                org(t),
                NewSession {
                    id,
                    owner: UserId::LOCAL,
                    kind: SessionKind::Main,
                    harness: "claude".into(),
                    cwd: "/w".into(),
                    model: None,
                    agent_ref: None,
                    project_id: None,
                    parent_id: None,
                    trigger: SessionTrigger::User,
                    home_node: t.local.node,
                    harness_opts: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn ses_008_ac1_dedup_key_is_host_harness_and_vendor_session() {
        let t = store().await;
        let key = ImportKey {
            host: t.local.node,
            harness: "claude",
            vendor_session_id: "v1",
        };
        assert_eq!(t.store.imported_session(org(&t), &key).await.unwrap(), None);
        let a = session(&t).await;
        t.store
            .record_import(org(&t), &key, a, Timestamp::now(), false)
            .await
            .unwrap();
        assert_eq!(
            t.store.imported_session(org(&t), &key).await.unwrap(),
            Some(a)
        );
        // Derselbe Schlüssel ohne `replace`: Konflikt.
        let b = session(&t).await;
        let err = t
            .store
            .record_import(org(&t), &key, b, Timestamp::now(), false)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "conflict");
        // Anderer Harness bzw. andere ID: unabhängig.
        let codex = ImportKey {
            harness: "codex",
            ..key.clone()
        };
        assert_eq!(
            t.store.imported_session(org(&t), &codex).await.unwrap(),
            None
        );
        // Mit `replace` (force): zeigt auf die neue Session.
        t.store
            .record_import(org(&t), &key, b, Timestamp::now(), true)
            .await
            .unwrap();
        assert_eq!(
            t.store.imported_session(org(&t), &key).await.unwrap(),
            Some(b)
        );
        let all = t
            .store
            .imported_sessions(org(&t), t.local.node, "claude")
            .await
            .unwrap();
        assert_eq!(all.get("v1"), Some(&b));
        // Gelöschte Session gibt den Schlüssel frei.
        t.store
            .delete_session(
                org(&t),
                b,
                PrincipalId::User(UserId::LOCAL),
                crate::DeleteAuthority::Owner,
            )
            .await
            .unwrap();
        assert_eq!(t.store.imported_session(org(&t), &key).await.unwrap(), None);
    }
}
