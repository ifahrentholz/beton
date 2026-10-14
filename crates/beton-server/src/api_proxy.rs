//! REST-API des Egress-Proxys (PRX-002): Rotation der beton-CA.
//!
//! Antworten enthalten nur öffentliche Angaben (Subject, Fingerabdruck, Gültigkeit), nie den
//! Schlüssel. Laufende Sessions behalten ihre CA bis zum Neustart.

use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::problem::{ApiResult, Problem};

/// Öffentliche Angaben zur beton-CA.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProxyCaView {
    /// `beton egress CA <installation-id>`
    pub subject: String,
    /// SHA-256 über das DER-Zertifikat, hex.
    pub fingerprint_sha256: String,
    pub not_before: String,
    pub not_after: String,
}

impl From<beton_proxy::CaInfo> for ProxyCaView {
    fn from(i: beton_proxy::CaInfo) -> Self {
        Self {
            subject: i.subject,
            fingerprint_sha256: i.fingerprint_sha256,
            not_before: i.not_before.to_string(),
            not_after: i.not_after.to_string(),
        }
    }
}

impl From<beton_proxy::CaError> for Problem {
    fn from(e: beton_proxy::CaError) -> Self {
        match e {
            beton_proxy::CaError::Secrets(e) => e.into(),
            other => Self::internal(&other),
        }
    }
}

/// Neue beton-CA erzeugen (`beton proxy ca rotate`); die Rotation steht im Audit-Log.
#[utoipa::path(post, path = "/v1/proxy/ca/rotate", tag = "proxy",
    responses((status = 200, description = "Neue CA erzeugt; neue Sessions nutzen sie", body = ProxyCaView),
              (status = 423, description = "Secret-Store gesperrt (`secrets_locked`)", body = Problem, content_type = "application/problem+json"),
              (status = 503, description = "Kein Backend (`secrets_unavailable`)", body = Problem, content_type = "application/problem+json")))]
pub async fn rotate_ca(State(state): State<AppState>) -> ApiResult<axum::Json<ProxyCaView>> {
    let cas = beton_proxy::CaStore::new(state.secrets.clone(), state.store.clone(), state.local);
    Ok(axum::Json(cas.rotate().await?.into()))
}
