//! REST-API für Secrets (SEC-004) und das Audit-Log (SEC-012).
//!
//! Werte gehen nur **in** den Server (Anlegen, Rotieren, Passphrase); keine Antwort enthält
//! einen Wert (SEC-004 AC3, OpenAPI-Snapshot-Test). Requests auf `/v1/secrets` werden nie
//! für Idempotenz gespeichert, und Parserfehler zitieren den Body nicht.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use beton_core::id::SessionId;
use beton_core::time::Timestamp;
use beton_secrets::{NewSecret, SecretKind, SecretRef, SecretValue};
use beton_store::AuditQuery;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::extract::{ApiQuery, PageQuery, decode_cursor, encode_cursor};
use crate::problem::{ApiResult, FieldError, Problem, ProblemCode};

/// Präfix der Secret-Routen (ohne Idempotenz-Speicher).
pub const SECRETS_PREFIX: &str = "/v1/secrets";

/// Ein Secret-Wert aus einem Request: wird sofort zu [`SecretValue`] (genullt beim Drop).
pub struct ValueInput(pub SecretValue);

impl<'de> Deserialize<'de> for ValueInput {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|s| Self(SecretValue::from(s)))
    }
}

impl std::fmt::Debug for ValueInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

/// JSON-Body für Secret-Routen: Fehler nennen nur das Feld, nie den Inhalt.
pub struct SecretJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for SecretJson<T> {
    type Rejection = Problem;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        if !is_json {
            return Err(Problem::new(ProblemCode::UnsupportedMediaType)
                .detail("Content-Type application/json erwartet"));
        }
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|_| Problem::new(ProblemCode::PayloadTooLarge))?;
        let body = zeroize::Zeroizing::new(bytes.to_vec());
        let mut de = serde_json::Deserializer::from_slice(&body);
        serde_path_to_error::deserialize(&mut de)
            .map(SecretJson)
            .map_err(|e| {
                let p = e.path().to_string();
                Problem::new(ProblemCode::ValidationFailed)
                    .detail("Der Request-Body ist ungültig (Inhalt nicht angezeigt).")
                    .errors(vec![FieldError {
                        pointer: format!("/{}", p.trim_start_matches('.').replace('.', "/")),
                        detail: "ungültig".into(),
                    }])
            })
    }
}

impl From<beton_secrets::Error> for Problem {
    fn from(e: beton_secrets::Error) -> Self {
        use beton_secrets::Error as E;
        let code = match &e {
            E::Locked => ProblemCode::SecretsLocked,
            E::Unavailable(_) => ProblemCode::SecretsUnavailable,
            E::Corrupt(_) => {
                let p = Self::new(ProblemCode::SecretsCorrupt);
                tracing::error!(trace_id = %p.trace_id, "{e}");
                return p;
            }
            E::NotFound(_) => ProblemCode::NotFound,
            E::Exists(_) => ProblemCode::Conflict,
            E::InvalidRef(_) => ProblemCode::InvalidSecretRef,
            E::Invalid(_) => ProblemCode::ValidationFailed,
            E::Store(_) | E::Io(_) => return Self::internal(&e),
        };
        // Meldungen des Secret-Stores nennen nie Werte (SEC-012 AC2).
        Self::new(code).detail(e.to_string())
    }
}

fn parse_ref(raw: &str) -> ApiResult<SecretRef> {
    raw.parse::<SecretRef>().map_err(Problem::from)
}

/// Metadaten eines Secrets; nie der Wert.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretMetaView {
    /// `secret://<user|team:<slug>|org>/<provider>/<account>[#name]`
    pub secret_ref: String,
    /// `token`, `basic` oder `oauth`.
    pub kind: String,
    pub scope: String,
    pub provider: String,
    pub account: String,
    pub name: Option<String>,
    /// `keychain` oder `file`.
    pub backend: String,
    pub created_at: String,
    pub updated_at: String,
    /// Letzter Abruf (SEC-004 AC1); `null`, solange nie genutzt.
    pub last_used_at: Option<String>,
}

impl From<beton_secrets::SecretMeta> for SecretMetaView {
    fn from(m: beton_secrets::SecretMeta) -> Self {
        Self {
            secret_ref: m.secret_ref,
            kind: m.kind.as_str().into(),
            scope: m.scope,
            provider: m.provider,
            account: m.account,
            name: m.name,
            backend: m.backend,
            created_at: m.created_at.to_string(),
            updated_at: m.updated_at.to_string(),
            last_used_at: m.last_used_at.map(|t| t.to_string()),
        }
    }
}

/// Zustand des Secret-Backends.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretStoreView {
    /// `keychain`, `file` oder `none`.
    pub backend: String,
    /// `true`: Werte sind nicht zugänglich (`beton secrets unlock`).
    pub locked: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SecretList {
    /// Sortiert nach Ref.
    pub items: Vec<SecretMetaView>,
    pub next_cursor: Option<String>,
    pub store: SecretStoreView,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecretCursor {
    after: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSecretRequest {
    pub secret_ref: String,
    /// `token`, `basic` (JSON mit `username`, `password`) oder `oauth` (JSON mit
    /// `access_token`).
    #[schema(value_type = String)]
    pub kind: SecretKind,
    /// Der Wert; wird nie zurückgegeben.
    #[schema(value_type = String, format = Password, write_only)]
    pub value: ValueInput,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateSecretRequest {
    pub secret_ref: String,
    #[schema(value_type = String, format = Password, write_only)]
    pub value: ValueInput,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnlockRequest {
    /// Passphrase des Datei-Stores; legt ihn beim ersten Mal an.
    #[schema(value_type = String, format = Password, write_only)]
    pub passphrase: ValueInput,
    /// Neue Passphrase (Wechsel, SEC-003 AC3).
    #[schema(value_type = Option<String>, format = Password, write_only)]
    #[serde(default)]
    pub new_passphrase: Option<ValueInput>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SecretRefQuery {
    pub secret_ref: String,
}

fn store_view(state: &AppState) -> SecretStoreView {
    let s = state.secrets.status();
    SecretStoreView {
        backend: s.backend.as_str().into(),
        locked: s.locked,
    }
}

#[utoipa::path(get, path = "/v1/secrets", tag = "secrets",
    params(PageQuery),
    responses((status = 200, description = "Metadaten der Secrets, nie Werte", body = SecretList)))]
pub async fn list_secrets(
    State(state): State<AppState>,
    ApiQuery(page): ApiQuery<PageQuery>,
) -> ApiResult<axum::Json<SecretList>> {
    let limit = page.limit()? as usize;
    let after = page.cursor::<SecretCursor>()?.map(|c| c.after);
    let mut items: Vec<SecretMetaView> = state
        .secrets
        .list_meta()
        .await?
        .into_iter()
        .filter(|m| after.as_deref().is_none_or(|a| m.secret_ref.as_str() > a))
        .map(SecretMetaView::from)
        .take(limit + 1)
        .collect();
    let next_cursor = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|m| {
            encode_cursor(&SecretCursor {
                after: m.secret_ref.clone(),
            })
        })
    } else {
        None
    };
    Ok(axum::Json(SecretList {
        items,
        next_cursor,
        store: store_view(&state),
    }))
}

#[utoipa::path(post, path = "/v1/secrets", tag = "secrets",
    request_body = CreateSecretRequest,
    responses((status = 201, description = "Secret angelegt", body = SecretMetaView),
              (status = 409, description = "Ref existiert bereits (`conflict`)", body = Problem, content_type = "application/problem+json"),
              (status = 423, description = "Datei-Store gesperrt (`secrets_locked`)", body = Problem, content_type = "application/problem+json"),
              (status = 503, description = "Kein Backend (`secrets_unavailable`)", body = Problem, content_type = "application/problem+json")))]
pub async fn create_secret(
    State(state): State<AppState>,
    SecretJson(req): SecretJson<CreateSecretRequest>,
) -> ApiResult<Response> {
    let meta = state
        .secrets
        .put(NewSecret {
            secret_ref: parse_ref(&req.secret_ref)?,
            kind: req.kind,
            value: req.value.0,
        })
        .await?;
    Ok((StatusCode::CREATED, axum::Json(SecretMetaView::from(meta))).into_response())
}

#[utoipa::path(put, path = "/v1/secrets", tag = "secrets",
    request_body = RotateSecretRequest,
    responses((status = 200, description = "Wert ersetzt", body = SecretMetaView),
              (status = 404, description = "Unbekannte Ref", body = Problem, content_type = "application/problem+json"),
              (status = 423, description = "Datei-Store gesperrt (`secrets_locked`)", body = Problem, content_type = "application/problem+json")))]
pub async fn rotate_secret(
    State(state): State<AppState>,
    SecretJson(req): SecretJson<RotateSecretRequest>,
) -> ApiResult<axum::Json<SecretMetaView>> {
    let meta = state
        .secrets
        .rotate(&parse_ref(&req.secret_ref)?, req.value.0)
        .await?;
    Ok(axum::Json(meta.into()))
}

#[utoipa::path(delete, path = "/v1/secrets", tag = "secrets",
    params(SecretRefQuery),
    responses((status = 204, description = "Wert und Metadaten entfernt"),
              (status = 404, description = "Unbekannte Ref", body = Problem, content_type = "application/problem+json"),
              (status = 423, description = "Datei-Store gesperrt (`secrets_locked`)", body = Problem, content_type = "application/problem+json")))]
pub async fn delete_secret(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<SecretRefQuery>,
) -> ApiResult<StatusCode> {
    state.secrets.delete(&parse_ref(&q.secret_ref)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/v1/secrets/unlock", tag = "secrets",
    request_body = UnlockRequest,
    responses((status = 204, description = "Entsperrt (bzw. angelegt oder Passphrase gewechselt)"),
              (status = 423, description = "Passphrase falsch (`secrets_locked`)", body = Problem, content_type = "application/problem+json"),
              (status = 500, description = "Datei beschädigt (`secrets_corrupt`)", body = Problem, content_type = "application/problem+json")))]
pub async fn unlock_secrets(
    State(state): State<AppState>,
    SecretJson(req): SecretJson<UnlockRequest>,
) -> ApiResult<StatusCode> {
    match req.new_passphrase {
        Some(new) => {
            state
                .secrets
                .change_passphrase(req.passphrase.0, new.0)
                .await?;
        }
        None => state.secrets.unlock(req.passphrase.0).await?,
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Ein Audit-Eintrag (SEC-012); enthält nie Secret-Werte.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntryView {
    /// Position in der Hash-Kette, lückenlos ab 1.
    pub seq: u64,
    pub id: String,
    pub at: String,
    /// Handelnder Principal.
    pub actor: String,
    pub action: String,
    pub target_kind: String,
    pub target_id: String,
    pub session_id: Option<String>,
    pub host_id: Option<String>,
    /// Anzahl zusammengefasster Vorgänge (aggregierte Proxy-Nutzung).
    pub count: u64,
    #[schema(value_type = Object)]
    pub details: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

impl From<beton_store::AuditEntry> for AuditEntryView {
    fn from(e: beton_store::AuditEntry) -> Self {
        Self {
            seq: e.seq,
            id: e.id,
            at: e.at.to_string(),
            actor: e.actor.to_string(),
            action: e.action,
            target_kind: e.target_kind,
            target_id: e.target_id,
            session_id: e.session_id.map(|s: SessionId| s.to_string()),
            host_id: e.host_id,
            count: e.count,
            details: e.details,
            prev_hash: e.prev_hash,
            hash: e.hash,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPageView {
    /// Neueste zuerst.
    pub items: Vec<AuditEntryView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditFilter {
    /// Nur diese Aktion, z. B. `secret.used`.
    pub action: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuditCursor {
    before_seq: u64,
}

#[utoipa::path(get, path = "/v1/audit", tag = "audit",
    params(PageQuery, AuditFilter),
    responses((status = 200, description = "Audit-Einträge, neueste zuerst", body = AuditPageView)))]
pub async fn list_audit(
    State(state): State<AppState>,
    ApiQuery(page): ApiQuery<PageQuery>,
    ApiQuery(filter): ApiQuery<AuditFilter>,
) -> ApiResult<axum::Json<AuditPageView>> {
    let limit = page.limit()?;
    let before = match page.cursor.as_deref() {
        Some(raw) => Some(decode_cursor::<AuditCursor>(raw)?.before_seq),
        None => None,
    };
    // Abgelaufene Nutzungsfenster zuerst versiegeln, damit sie sichtbar sind.
    state.secrets.seal_usage(Timestamp::now()).await?;
    let result = state
        .store
        .audit_page(
            state.local.org,
            &AuditQuery {
                before_seq: before,
                action: filter.action,
                limit,
            },
        )
        .await?;
    Ok(axum::Json(AuditPageView {
        items: result.items.into_iter().map(Into::into).collect(),
        next_cursor: result
            .next_before
            .map(|before_seq| encode_cursor(&AuditCursor { before_seq })),
    }))
}

/// Ergebnis der Kettenprüfung (SEC-012 AC1).
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditVerifyView {
    pub ok: bool,
    pub entries: u64,
    /// Erste ungültige Position (`seq`).
    pub first_invalid: Option<u64>,
    pub reason: Option<String>,
}

#[utoipa::path(get, path = "/v1/audit/verify", tag = "audit",
    responses((status = 200, description = "Ergebnis der Prüfung der Hash-Kette", body = AuditVerifyView)))]
pub async fn verify_audit(State(state): State<AppState>) -> ApiResult<axum::Json<AuditVerifyView>> {
    state.secrets.seal_usage(Timestamp::now()).await?;
    let v = state.store.verify_audit(state.local.org).await?;
    Ok(axum::Json(AuditVerifyView {
        ok: v.first_invalid.is_none(),
        entries: v.entries,
        first_invalid: v.first_invalid,
        reason: v.reason,
    }))
}
