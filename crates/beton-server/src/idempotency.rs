//! `Idempotency-Key` für POST-Requests (PROTO-010 AC2).
//!
//! Gleicher Key und gleicher Request → gespeicherte Antwort wird wiederholt
//! (`Idempotent-Replayed: true`). Gleicher Key mit anderem Request → 422
//! `idempotency_key_reused`. Keys gelten 24 h und pro Org.

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use beton_core::id::OrgId;
use beton_store::{Store, StoredResponse};
use sha2::{Digest, Sha256};

use crate::problem::{Problem, ProblemCode};

pub const HEADER: &str = "idempotency-key";
/// Größter Body, der für Idempotenz gepuffert wird.
const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Idempotency {
    pub store: Store,
    pub org: OrgId,
}

pub async fn layer(State(state): State<Idempotency>, req: Request, next: Next) -> Response {
    if req.method() != Method::POST {
        return next.run(req).await;
    }
    let Some(key) = req
        .headers()
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
    else {
        return next.run(req).await;
    };
    if key.is_empty() || key.len() > 255 {
        return Problem::new(ProblemCode::BadRequest)
            .detail("Idempotency-Key muss 1 bis 255 Zeichen lang sein")
            .into_response();
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Problem::new(ProblemCode::PayloadTooLarge).into_response();
    };
    let mut hasher = Sha256::new();
    hasher.update(parts.method.as_str());
    hasher.update([0]);
    hasher.update(
        parts
            .uri
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or_default(),
    );
    hasher.update([0]);
    hasher.update(&bytes);
    let request_hash = hex::encode(hasher.finalize());

    match state.store.idempotency_lookup(state.org, &key).await {
        Err(e) => return Problem::from(e).into_response(),
        Ok(Some(stored)) if stored.request_hash != request_hash => {
            return Problem::new(ProblemCode::IdempotencyKeyReused)
                .detail("Dieser Idempotency-Key wurde mit einem anderen Request verwendet.")
                .into_response();
        }
        Ok(Some(stored)) => return replay(stored),
        Ok(None) => {}
    }

    let res = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if res.status().is_server_error() {
        return res;
    }
    let (parts, body) = res.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Problem::new(ProblemCode::Internal).into_response();
    };
    let stored = StoredResponse {
        request_hash,
        status: parts.status.as_u16(),
        content_type: parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_owned(),
        body: bytes.to_vec(),
    };
    if let Err(e) = state.store.idempotency_save(state.org, &key, &stored).await {
        tracing::warn!("Idempotency-Antwort nicht gespeichert: {e}");
    }
    Response::from_parts(parts, Body::from(bytes))
}

fn replay(stored: StoredResponse) -> Response {
    let mut res = Response::new(Body::from(stored.body));
    *res.status_mut() = StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK);
    if let Ok(ct) = HeaderValue::from_str(&stored.content_type) {
        res.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    res.headers_mut()
        .insert("idempotent-replayed", HeaderValue::from_static("true"));
    res
}
