//! Der Secret-Dienst (SEC-001): Metadaten in der Datenbank, Werte im Backend, jede Änderung
//! und jede Nutzung im Audit-Log.
//!
//! Werte verlassen den Dienst nur über `get_for_use`, und das ist `pub(crate)`: Von außen ist
//! es nur über einen Capability-Typ erreichbar (siehe [`crate::capability`]).

use std::sync::Arc;

use beton_core::id::{OrgId, PrincipalId, SessionId};
use beton_core::time::Timestamp;
use beton_store::{LocalIdentity, NewAuditEntry, SecretMetaRecord, Store, UsageKey};
use serde::{Deserialize, Serialize};

use crate::backend::{Backend, BackendKind};
use crate::capability::UseCapabilities;
use crate::error::{Error, Result};
use crate::redact::{KnownSecrets, KnownSource, Redactor};
use crate::{SecretRef, SecretValue};

/// Art eines Secrets (SEC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// Ein Token, z. B. API-Key oder PAT.
    Token,
    /// User und Passwort: JSON `{"username": …, "password": …}`.
    Basic,
    /// OAuth: JSON mit `access_token`, optional `refresh_token` und `expires_at`.
    Oauth,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Basic => "basic",
            Self::Oauth => "oauth",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        match s {
            "token" => Ok(Self::Token),
            "basic" => Ok(Self::Basic),
            "oauth" => Ok(Self::Oauth),
            other => Err(Error::Invalid(format!("unbekannte Secret-Art `{other}`"))),
        }
    }

    /// Prüft die Form des Werts; Meldungen nennen den Wert nie.
    fn validate(self, value: &SecretValue) -> Result<()> {
        if value.is_empty() {
            return Err(Error::Invalid("Der Wert ist leer".into()));
        }
        let field = |name: &str| -> Result<()> {
            let json: serde_json::Value = serde_json::from_slice(value.expose()).map_err(|_| {
                Error::Invalid(format!(
                    "Ein {}-Secret ist ein JSON-Objekt (Inhalt nicht angezeigt)",
                    self.as_str()
                ))
            })?;
            match json.get(name) {
                Some(serde_json::Value::String(s)) if !s.is_empty() => Ok(()),
                _ => Err(Error::Invalid(format!(
                    "Ein {}-Secret braucht das Feld `{name}`",
                    self.as_str()
                ))),
            }
        };
        match self {
            Self::Token => Ok(()),
            Self::Basic => field("username").and_then(|()| field("password")),
            Self::Oauth => field("access_token"),
        }
    }
}

/// Metadaten eines Secrets; enthält nie den Wert (SEC-001 AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretMeta {
    pub secret_ref: String,
    pub kind: SecretKind,
    /// `user`, `team` oder `org`.
    pub scope: String,
    pub provider: String,
    pub account: String,
    pub name: Option<String>,
    /// Backend, in dem der Wert liegt (`keychain`, `file`).
    pub backend: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

impl SecretMeta {
    fn from_record(r: SecretMetaRecord) -> Result<Self> {
        Ok(Self {
            secret_ref: r.secret_ref,
            kind: SecretKind::parse(&r.kind)?,
            scope: r.scope,
            provider: r.provider,
            account: r.account,
            name: (!r.name.is_empty()).then_some(r.name),
            backend: r.backend,
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_used_at: r.last_used_at,
        })
    }
}

/// Ein neues Secret.
#[derive(Debug)]
pub struct NewSecret {
    pub secret_ref: SecretRef,
    pub kind: SecretKind,
    pub value: SecretValue,
}

/// Zweck eines Abrufs; bestimmt der Capability-Typ, nicht der Aufrufer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Credential-Injection im Egress-Proxy (PRX-006); hochfrequent, im Audit aggregiert.
    Proxy,
    /// Direkt-API-Harness (HAR-011).
    DirectApi,
    /// Git-Provider-Client (GIT-002).
    GitProvider,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::DirectApi => "direct_api",
            Self::GitProvider => "git_provider",
        }
    }
}

/// Wofür ein Wert gebraucht wird: Session und Host landen im Audit-Eintrag (SEC-001 AC3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseContext {
    pub session_id: Option<SessionId>,
    pub host_id: String,
}

/// Zustand des Backends für `beton doctor`, die API und `beton secrets list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreStatus {
    pub backend: BackendKind,
    pub locked: bool,
}

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Der Secret-Dienst eines Knotens.
#[derive(Clone)]
pub struct Secrets {
    store: Store,
    org: OrgId,
    actor: PrincipalId,
    backend: Arc<dyn Backend>,
    clock: Clock,
    /// Angebundene Redaction-Pipeline (SEC-013); gemeinsam für alle Klone des Dienstes.
    redaction: Arc<Redaction>,
}

/// Zustand der Anbindung an die Redaction-Pipeline.
#[derive(Default)]
struct Redaction {
    target: std::sync::OnceLock<Arc<Redactor>>,
    /// Serialisiert Neuaufbauten, damit kein älterer Stand einen neueren überschreibt.
    rebuild: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("org", &self.org)
            .field("backend", &self.backend.kind())
            .finish_non_exhaustive()
    }
}

/// Führt einen Backend-Aufruf außerhalb des Async-Executors aus.
async fn blocking<T: Send + 'static>(
    backend: &Arc<dyn Backend>,
    f: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
) -> Result<T> {
    let backend = backend.clone();
    tokio::task::spawn_blocking(move || f(backend.as_ref()))
        .await
        .map_err(|_| Error::Unavailable("Secret-Backend-Aufruf abgebrochen".into()))?
}

impl Secrets {
    /// Dienst für den lokalen Knoten. Die Capabilities für `get_for_use` gibt es nur hier;
    /// der Aufrufer (Komposition im Daemon) reicht sie an Proxy, Direkt-API-Harness und
    /// Git-Provider-Client weiter (SEC-001 AC2).
    pub fn new(
        store: Store,
        local: LocalIdentity,
        backend: Arc<dyn Backend>,
    ) -> (Self, UseCapabilities) {
        (
            Self {
                store,
                org: local.org,
                actor: PrincipalId::User(local.user),
                backend,
                clock: Arc::new(Timestamp::now),
                redaction: Arc::default(),
            },
            UseCapabilities::new(),
        )
    }

    pub fn org(&self) -> OrgId {
        self.org
    }

    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }

    pub fn status(&self) -> StoreStatus {
        StoreStatus {
            backend: self.backend.kind(),
            locked: self.backend.locked(),
        }
    }

    fn audit(&self, action: &str, r: &SecretRef, details: serde_json::Value) -> NewAuditEntry {
        NewAuditEntry {
            at: (self.clock)(),
            actor: self.actor,
            action: action.into(),
            target_kind: "secret".into(),
            target_id: r.to_string(),
            session_id: None,
            host_id: None,
            details,
        }
    }

    async fn meta(&self, r: &SecretRef) -> Result<SecretMetaRecord> {
        self.store
            .secret_meta(self.org, &r.to_string())
            .await?
            .ok_or_else(|| Error::NotFound(r.to_string()))
    }

    /// Alle Metadaten, nie Werte (SEC-001 AC1).
    pub async fn list_meta(&self) -> Result<Vec<SecretMeta>> {
        self.store
            .list_secret_meta(self.org)
            .await?
            .into_iter()
            .map(SecretMeta::from_record)
            .collect()
    }

    /// Legt ein Secret an; `conflict`, wenn die Ref schon existiert (dann bleibt der alte Wert).
    pub async fn put(&self, new: NewSecret) -> Result<SecretMeta> {
        new.kind.validate(&new.value)?;
        let r = new.secret_ref;
        if self
            .store
            .secret_meta(self.org, &r.to_string())
            .await?
            .is_some()
        {
            return Err(Error::Exists(r.to_string()));
        }
        let value = new.value;
        let key = r.clone();
        blocking(&self.backend, move |b| b.put(&key, &value)).await?;
        let now = (self.clock)();
        let record = SecretMetaRecord {
            secret_ref: r.to_string(),
            scope: r.scope.kind().into(),
            provider: r.provider.clone(),
            account: r.account.clone(),
            name: r.name.clone().unwrap_or_default(),
            kind: new.kind.as_str().into(),
            backend: self.backend.kind().as_str().into(),
            created_at: now,
            updated_at: now,
            last_used_at: None,
        };
        let audit = self.audit(
            "secret.created",
            &r,
            serde_json::json!({ "kind": new.kind.as_str(), "backend": record.backend }),
        );
        if let Err(e) = self
            .store
            .insert_secret_meta(self.org, &record, audit)
            .await
        {
            // Ohne Metadaten darf kein Wert zurückbleiben.
            let key = r.clone();
            let _ = blocking(&self.backend, move |b| b.delete(&key)).await;
            return Err(match e {
                beton_store::Error::Conflict(_) => Error::Exists(r.to_string()),
                other => other.into(),
            });
        }
        self.after_change("created").await;
        SecretMeta::from_record(record)
    }

    /// Ersetzt den Wert eines vorhandenen Secrets (`beton secrets rotate`).
    pub async fn rotate(&self, r: &SecretRef, value: SecretValue) -> Result<SecretMeta> {
        let meta = self.meta(r).await?;
        SecretKind::parse(&meta.kind)?.validate(&value)?;
        let key = r.clone();
        blocking(&self.backend, move |b| b.put(&key, &value)).await?;
        let now = (self.clock)();
        self.store
            .rotate_secret_meta(
                self.org,
                &r.to_string(),
                now,
                self.audit("secret.rotated", r, serde_json::json!({})),
            )
            .await?;
        self.after_change("rotated").await;
        SecretMeta::from_record(self.meta(r).await?)
    }

    /// Entfernt Wert und Metadaten (SEC-002 AC3). Erst der Wert, dann die Metadaten: Scheitert
    /// das Backend (z. B. gesperrt), bleibt alles, wie es war.
    pub async fn delete(&self, r: &SecretRef) -> Result<()> {
        self.meta(r).await?;
        let key = r.clone();
        let existed = blocking(&self.backend, move |b| b.delete(&key)).await?;
        self.store
            .delete_secret_meta(
                self.org,
                &r.to_string(),
                self.audit(
                    "secret.deleted",
                    r,
                    serde_json::json!({ "value_found": existed }),
                ),
            )
            .await?;
        self.after_change("deleted").await;
        Ok(())
    }

    /// Entsperrt den Datei-Store (SEC-003); auditiert Erfolg und Fehlschlag.
    pub async fn unlock(&self, passphrase: SecretValue) -> Result<()> {
        let result = blocking(&self.backend, move |b| b.unlock(&passphrase)).await;
        self.audit_store_event("secrets.unlocked", "secrets.unlock_failed", &result)
            .await?;
        if result.is_ok() {
            self.after_change("unlocked").await;
        }
        result
    }

    /// Wechselt die Passphrase des Datei-Stores (SEC-003 AC3).
    pub async fn change_passphrase(&self, old: SecretValue, new: SecretValue) -> Result<()> {
        let result = blocking(&self.backend, move |b| b.change_passphrase(&old, &new)).await;
        self.audit_store_event(
            "secrets.passphrase_changed",
            "secrets.passphrase_change_failed",
            &result,
        )
        .await?;
        result
    }

    async fn audit_store_event(&self, ok: &str, failed: &str, result: &Result<()>) -> Result<()> {
        let (action, details) = match result {
            Ok(()) => (ok, serde_json::json!({})),
            Err(e) => (failed, serde_json::json!({ "code": e.code() })),
        };
        self.store
            .append_audit(
                self.org,
                NewAuditEntry {
                    at: (self.clock)(),
                    actor: self.actor,
                    action: action.into(),
                    target_kind: "secret_store".into(),
                    target_id: self.backend.kind().as_str().into(),
                    session_id: None,
                    host_id: None,
                    details,
                },
            )
            .await?;
        Ok(())
    }

    /// Bindet die Redaction-Pipeline an (nur über [`crate::RedactionCap`]) und lädt die
    /// bekannten Werte.
    pub(crate) async fn attach_redaction(&self, redactor: Arc<Redactor>) -> Result<()> {
        if self.redaction.target.set(redactor.clone()).is_err() {
            redactor.fail();
            return Err(Error::Invalid(
                "Die Redaction-Pipeline ist bereits angebunden".into(),
            ));
        }
        self.refresh_redaction("attach").await
    }

    /// Baut den Matcher der angebundenen Pipeline neu (nach Anlegen, Rotieren, Löschen,
    /// Entsperren). Ohne Pipeline nichts zu tun.
    ///
    /// - Store gesperrt oder nicht erreichbar: die zuletzt bekannten Werte bleiben aktiv; der
    ///   Store gibt dann auch für Sessions keinen Wert heraus.
    /// - Jeder andere Fehler (Datenbank, Matcher, Audit): Pipeline blockiert (fail closed).
    async fn refresh_redaction(&self, reason: &str) -> Result<()> {
        let Some(redactor) = self.redaction.target.get() else {
            return Ok(());
        };
        let _serial = self.redaction.rebuild.lock().await;
        let result = self.rebuild_redaction(redactor, reason).await;
        if result.is_err() {
            redactor.fail();
        }
        result
    }

    async fn rebuild_redaction(&self, redactor: &Redactor, reason: &str) -> Result<()> {
        let (known, details) = match self.known_values().await {
            Ok((values, missing)) => {
                let known = KnownSecrets::build(&values).map_err(Error::Unavailable)?;
                let details = serde_json::json!({
                    "reason": reason,
                    "store": "ok",
                    "values": known.values(),
                    "missing": missing,
                });
                (Some(known), details)
            }
            Err(e @ (Error::Locked | Error::Unavailable(_) | Error::Corrupt(_))) => (
                None,
                serde_json::json!({ "reason": reason, "store": e.code(), "values": serde_json::Value::Null }),
            ),
            Err(other) => return Err(other),
        };
        // Erst das Audit, dann der neue Matcher: ohne Eintrag kein Aufbau (wie `get_for_use`).
        self.store
            .append_audit(
                self.org,
                NewAuditEntry {
                    at: (self.clock)(),
                    actor: self.actor,
                    action: "secrets.redaction_loaded".into(),
                    target_kind: "secret_store".into(),
                    target_id: self.backend.kind().as_str().into(),
                    session_id: None,
                    host_id: None,
                    details,
                },
            )
            .await?;
        match known {
            Some(known) => redactor.install(known, KnownSource::Store),
            None => redactor.mark_store_unavailable(),
        }
        Ok(())
    }

    /// Alle bekannten Werte (bzw. ihre geheimen Teile). Werte ohne Eintrag im Backend zählen
    /// als `missing`; sie lassen sich auch nicht nutzen.
    async fn known_values(&self) -> Result<(Vec<SecretValue>, usize)> {
        if self.backend.locked() {
            return Err(Error::Locked);
        }
        let mut values = Vec::new();
        let mut missing = 0;
        for meta in self.list_meta().await? {
            let r: SecretRef = meta.secret_ref.parse()?;
            match blocking(&self.backend, move |b| b.get(&r)).await {
                Ok(value) => values.extend(crate::redact::parts(meta.kind, &value)),
                Err(Error::NotFound(_)) => missing += 1,
                Err(e) => return Err(e),
            }
        }
        Ok((values, missing))
    }

    /// Nach einer Änderung: Matcher neu bauen. Scheitert das, ist die Pipeline blockiert
    /// (fail closed) und der Fehler steht im Log; die Änderung selbst bleibt gültig.
    async fn after_change(&self, reason: &str) {
        if let Err(e) = self.refresh_redaction(reason).await {
            tracing::error!(
                code = e.code(),
                reason,
                "Redaction-Matcher nicht neu aufgebaut; Ausgaben bleiben blockiert"
            );
        }
    }

    /// Versiegelt abgelaufene Nutzungsfenster (vor dem Lesen des Audit-Logs).
    pub async fn seal_usage(&self, now: Timestamp) -> Result<u64> {
        Ok(self.store.seal_audit_windows(self.org, now).await?)
    }

    /// Gibt den Wert für einen Zweck heraus und auditiert das (SEC-001 AC3). Fail closed: Ohne
    /// Metadaten, ohne Backend oder ohne Audit-Eintrag gibt es keinen Wert.
    pub(crate) async fn get_for_use(
        &self,
        r: &SecretRef,
        purpose: Purpose,
        ctx: UseContext,
    ) -> Result<SecretValue> {
        self.meta(r).await?;
        let key = r.clone();
        let value = blocking(&self.backend, move |b| b.get(&key)).await?;
        let now = (self.clock)();
        match purpose {
            Purpose::Proxy => {
                self.store
                    .record_audit_usage(
                        self.org,
                        &UsageKey {
                            secret_ref: r.to_string(),
                            session_id: ctx.session_id,
                            host_id: ctx.host_id,
                            actor: self.actor,
                            purpose: purpose.as_str().into(),
                        },
                        now,
                    )
                    .await?;
            }
            Purpose::DirectApi | Purpose::GitProvider => {
                self.store
                    .append_audit(
                        self.org,
                        NewAuditEntry {
                            at: now,
                            actor: self.actor,
                            action: beton_store::audit::USED_ACTION.into(),
                            target_kind: "secret".into(),
                            target_id: r.to_string(),
                            session_id: ctx.session_id,
                            host_id: Some(ctx.host_id),
                            details: serde_json::json!({ "purpose": purpose.as_str() }),
                        },
                    )
                    .await?;
            }
        }
        self.store
            .touch_secret_used(self.org, &r.to_string(), now)
            .await?;
        Ok(value)
    }
}
