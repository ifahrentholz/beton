//! Extraktoren, die Fehler als Problem-Objekte melden (PROTO-011): JSON-Bodies mit
//! JSON-Pointer auf das fehlerhafte Feld, Query-Parameter, Pagination, `If-Match`.

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use crate::problem::{FieldError, Problem, ProblemCode};

/// JSON-Body; Fehler zeigen per JSON-Pointer auf das Feld (PROTO-011 AC2).
#[derive(Debug, Clone)]
pub struct ApiJson<T>(pub T);

/// JSON-Pointer aus dem serde-Pfad, z. B. `/steps/2/name`.
fn pointer(path: &serde_path_to_error::Path) -> String {
    let mut out = String::new();
    for segment in path.iter() {
        use serde_path_to_error::Segment;
        out.push('/');
        match segment {
            Segment::Seq { index } => out.push_str(&index.to_string()),
            Segment::Map { key } => out.push_str(&key.replace('~', "~0").replace('/', "~1")),
            Segment::Enum { variant } => out.push_str(variant),
            Segment::Unknown => out.push('?'),
        }
    }
    out
}

/// Deserialisiert JSON und meldet Fehler mit Pointer.
pub fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Problem> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    serde_path_to_error::deserialize(&mut de).map_err(|e| {
        let p = pointer(e.path());
        Problem::new(ProblemCode::ValidationFailed)
            .detail("Der Request-Body ist ungültig.")
            .errors(vec![FieldError {
                pointer: if p.is_empty() { "/".into() } else { p },
                detail: e.inner().to_string(),
            }])
    })
}

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
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
        parse_json(&bytes).map(ApiJson)
    }
}

/// Query-Parameter mit Problem-Fehlern.
#[derive(Debug, Clone)]
pub struct ApiQuery<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::try_from_uri(&parts.uri)
            .map(|q| ApiQuery(q.0))
            .map_err(|e| {
                Problem::new(ProblemCode::BadRequest)
                    .detail(format!("Query-Parameter ungültig: {}", e.body_text()))
            })
    }
}

/// Pagination-Parameter (PROTO-010): `limit` 1…200 (Default 50), opaker `cursor`.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// Anzahl der Einträge, 1 bis 200 (Default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<String>,
    /// Opaker Cursor aus `next_cursor` der vorigen Seite.
    pub cursor: Option<String>,
}

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 200;

impl PageQuery {
    pub fn limit(&self) -> Result<u32, Problem> {
        match &self.limit {
            None => Ok(DEFAULT_LIMIT),
            Some(raw) => raw
                .parse::<u32>()
                .ok()
                .filter(|l| (1..=MAX_LIMIT).contains(l))
                .ok_or_else(|| {
                    Problem::new(ProblemCode::InvalidLimit)
                        .detail(format!("limit muss zwischen 1 und {MAX_LIMIT} liegen"))
                }),
        }
    }

    /// Dekodiert den Cursor in seinen Inhalt.
    pub fn cursor<C: DeserializeOwned>(&self) -> Result<Option<C>, Problem> {
        self.cursor.as_deref().map(decode_cursor).transpose()
    }
}

/// Opaker Cursor: base64url(JSON). Für Clients undurchsichtig, für den Server prüfbar.
pub fn encode_cursor<C: Serialize>(c: &C) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).unwrap_or_default())
}

pub fn decode_cursor<C: DeserializeOwned>(raw: &str) -> Result<C, Problem> {
    URL_SAFE_NO_PAD
        .decode(raw)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Problem::new(ProblemCode::InvalidCursor))
}

/// `If-Match` gegen das aktuelle ETag prüfen (PROTO-010): 412, wenn veraltet.
pub fn check_if_match(headers: &HeaderMap, current_etag: &str) -> Result<(), Problem> {
    let Some(value) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let matches = value
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == current_etag);
    if matches {
        Ok(())
    } else {
        Err(Problem::new(ProblemCode::PreconditionFailed)
            .detail("Die Ressource wurde inzwischen geändert (ETag veraltet)."))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Agent {
        name: String,
        steps: Vec<Step>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Step {
        timeout_s: u32,
    }

    #[test]
    fn proto_011_ac2_validation_errors_point_to_the_field() {
        let err =
            parse_json::<Agent>(br#"{"name":"x","steps":[{"timeout_s":1},{"timeout_s":"lang"}]}"#)
                .unwrap_err();
        assert_eq!(err.code, ProblemCode::ValidationFailed);
        assert_eq!(err.errors[0].pointer, "/steps/1/timeout_s");
    }

    #[test]
    fn page_query_validates_limit_and_cursor() {
        let q = |limit: Option<&str>, cursor: Option<&str>| PageQuery {
            limit: limit.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
        };
        assert_eq!(q(None, None).limit().unwrap(), 50);
        assert_eq!(q(Some("200"), None).limit().unwrap(), 200);
        for bad in ["0", "201", "x", "-1"] {
            assert_eq!(
                q(Some(bad), None).limit().unwrap_err().code,
                ProblemCode::InvalidLimit
            );
        }
        let c = encode_cursor(&serde_json::json!({"before": "ses_1"}));
        assert_eq!(
            q(None, Some(&c))
                .cursor::<serde_json::Value>()
                .unwrap()
                .unwrap()["before"],
            "ses_1"
        );
        assert_eq!(
            q(None, Some("kaputt!"))
                .cursor::<serde_json::Value>()
                .unwrap_err()
                .code,
            ProblemCode::InvalidCursor
        );
    }

    #[test]
    fn proto_010_ac3_stale_if_match_is_412() {
        let mut h = HeaderMap::new();
        assert!(check_if_match(&h, "\"v2\"").is_ok(), "ohne If-Match");
        h.insert(header::IF_MATCH, HeaderValue::from_static("\"v1\""));
        assert_eq!(
            check_if_match(&h, "\"v2\"").unwrap_err().code,
            ProblemCode::PreconditionFailed
        );
        h.insert(header::IF_MATCH, HeaderValue::from_static("\"v1\", \"v2\""));
        assert!(check_if_match(&h, "\"v2\"").is_ok());
    }
}
