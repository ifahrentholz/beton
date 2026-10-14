//! Metadaten der Secrets (SEC-001). Werte liegen nie in der Datenbank, nur im Backend
//! (`beton-secrets`). Jede Änderung schreibt Metadaten und Audit-Eintrag in einer Transaktion.

use beton_core::id::OrgId;
use beton_core::time::Timestamp;
use sqlx::Row;

use crate::Store;
use crate::audit::{NewAuditEntry, append_in};
use crate::error::{Error, Result};
use crate::sessions::parse;

/// Eine Zeile der Tabelle `secrets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMetaRecord {
    pub secret_ref: String,
    pub scope: String,
    pub provider: String,
    pub account: String,
    /// Optionaler Name (`#name` der SecretRef); leer ohne Namen.
    pub name: String,
    /// `token`, `basic` oder `oauth`.
    pub kind: String,
    /// `keychain` oder `file`.
    pub backend: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

const COLUMNS: &str = "secret_ref, scope, provider, account, name, kind, backend, created_at, \
                       updated_at, last_used_at";

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<SecretMetaRecord> {
    let used: Option<String> = row.try_get("last_used_at")?;
    Ok(SecretMetaRecord {
        secret_ref: row.try_get("secret_ref")?,
        scope: row.try_get("scope")?,
        provider: row.try_get("provider")?,
        account: row.try_get("account")?,
        name: row.try_get("name")?,
        kind: row.try_get("kind")?,
        backend: row.try_get("backend")?,
        created_at: parse(row.try_get("created_at")?)?,
        updated_at: parse(row.try_get("updated_at")?)?,
        last_used_at: used.as_deref().map(parse).transpose()?,
    })
}

impl Store {
    /// Legt Metadaten an und auditiert das; `conflict`, wenn die Ref schon existiert.
    pub async fn insert_secret_meta(
        &self,
        org: OrgId,
        meta: &SecretMetaRecord,
        audit: NewAuditEntry,
    ) -> Result<()> {
        let mut tx = self.write_tx().await?;
        let done = sqlx::query(
            "INSERT INTO secrets (org_id, secret_ref, scope, provider, account, name, kind, \
             backend, created_at, updated_at, last_used_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (org_id, secret_ref) DO NOTHING",
        )
        .bind(org.to_string())
        .bind(&meta.secret_ref)
        .bind(&meta.scope)
        .bind(&meta.provider)
        .bind(&meta.account)
        .bind(&meta.name)
        .bind(&meta.kind)
        .bind(&meta.backend)
        .bind(meta.created_at.to_string())
        .bind(meta.updated_at.to_string())
        .bind(meta.last_used_at.map(|t| t.to_string()))
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Err(Error::Conflict(format!(
                "Secret {} existiert bereits",
                meta.secret_ref
            )));
        }
        append_in(&mut tx, org, &audit, 1).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Vermerkt einen neuen Wert (`updated_at`) und auditiert das; `not_found` ohne Eintrag.
    pub async fn rotate_secret_meta(
        &self,
        org: OrgId,
        secret_ref: &str,
        at: Timestamp,
        audit: NewAuditEntry,
    ) -> Result<()> {
        let mut tx = self.write_tx().await?;
        let done =
            sqlx::query("UPDATE secrets SET updated_at = ? WHERE org_id = ? AND secret_ref = ?")
                .bind(at.to_string())
                .bind(org.to_string())
                .bind(secret_ref)
                .execute(&mut *tx)
                .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("Secret {secret_ref}")));
        }
        append_in(&mut tx, org, &audit, 1).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Entfernt die Metadaten und auditiert das; `false`, wenn es keine gab (dann ohne Audit).
    pub async fn delete_secret_meta(
        &self,
        org: OrgId,
        secret_ref: &str,
        audit: NewAuditEntry,
    ) -> Result<bool> {
        let mut tx = self.write_tx().await?;
        let done = sqlx::query("DELETE FROM secrets WHERE org_id = ? AND secret_ref = ?")
            .bind(org.to_string())
            .bind(secret_ref)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        append_in(&mut tx, org, &audit, 1).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Setzt „zuletzt genutzt“ (SEC-004 AC1).
    pub async fn touch_secret_used(
        &self,
        org: OrgId,
        secret_ref: &str,
        at: Timestamp,
    ) -> Result<()> {
        sqlx::query("UPDATE secrets SET last_used_at = ? WHERE org_id = ? AND secret_ref = ?")
            .bind(at.to_string())
            .bind(org.to_string())
            .bind(secret_ref)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn secret_meta(
        &self,
        org: OrgId,
        secret_ref: &str,
    ) -> Result<Option<SecretMetaRecord>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM secrets WHERE org_id = ? AND secret_ref = ?"
        ))
        .bind(org.to_string())
        .bind(secret_ref)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(from_row)
        .transpose()
    }

    /// Alle Metadaten der Org, sortiert nach Ref.
    pub async fn list_secret_meta(&self, org: OrgId) -> Result<Vec<SecretMetaRecord>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM secrets WHERE org_id = ? ORDER BY secret_ref"
        ))
        .bind(org.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(from_row)
        .collect()
    }
}
