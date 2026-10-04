//! Sicherheitsschicht vor allen Routen (AUTH-001, AUTH-002, AUTH-003).
//!
//! Reihenfolge je Request:
//! 1. `Host` muss auf der Allowlist stehen (DNS-Rebinding), außer über den Unix-Socket.
//! 2. WebSocket-Upgrades: `Origin` muss erlaubt sein; ohne `Origin` nur mit `Authorization`.
//! 3. Authentisierung: `Authorization: Bearer <lokales Token>` oder Session-Cookie. Mit Cookie
//!    brauchen zustandsändernde Requests eine erlaubte `Origin` und `Sec-Fetch-Site`
//!    `same-origin` bzw. `none` (CSRF).
//!
//! Es werden keine CORS-Header gesetzt; Preflights fremder Origins bleiben unbeantwortet.

use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::local_auth::{BrowserLogins, COOKIE_NAME, LocalToken};
use crate::problem::{Problem, ProblemCode};

/// Ohne Anmeldung erreichbar (Host-Prüfung gilt trotzdem).
pub const PUBLIC_PATHS: [&str; 2] = ["/healthz", "/auth/local/redeem"];

/// Wie ein Request authentisiert wurde; liegt als Extension am Request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authenticated {
    Bearer,
    Cookie,
}

/// Markiert Requests, die über den Unix-Socket kamen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViaSocket;

/// Erlaubte Origins; Wildcards nur als Subdomain-Präfix (`https://*.example.com`).
#[derive(Debug, Clone, Default)]
pub struct OriginPolicy {
    exact: Vec<String>,
    /// (Schema, Suffix inkl. Punkt, Port-Teil)
    wildcard: Vec<(String, String)>,
}

impl OriginPolicy {
    pub fn new<I: IntoIterator<Item = S>, S: AsRef<str>>(origins: I) -> Self {
        let mut policy = Self::default();
        for o in origins {
            let o = o.as_ref().trim().trim_end_matches('/').to_ascii_lowercase();
            if let Some((scheme, rest)) = o.split_once("://*.") {
                policy
                    .wildcard
                    .push((scheme.to_owned(), format!(".{rest}")));
            } else if !o.contains('*') {
                policy.exact.push(o);
            }
        }
        policy
    }

    pub fn allows(&self, origin: &str) -> bool {
        let origin = origin.trim().to_ascii_lowercase();
        if origin == "null" || origin.is_empty() {
            return false;
        }
        if self.exact.contains(&origin) {
            return true;
        }
        let Some((scheme, host)) = origin.split_once("://") else {
            return false;
        };
        if host.contains('/') || host.contains('@') {
            return false;
        }
        self.wildcard.iter().any(|(s, suffix)| {
            s == scheme
                && host.len() > suffix.len()
                && host.ends_with(suffix.as_str())
                && !host[..host.len() - suffix.len()].is_empty()
        })
    }
}

/// Zustand der Sicherheitsschicht.
#[derive(Debug, Clone)]
pub struct Guard {
    pub hosts: Arc<Vec<String>>,
    pub origins: Arc<OriginPolicy>,
    pub token: Arc<LocalToken>,
    pub logins: Arc<BrowserLogins>,
}

fn reject(code: ProblemCode, detail: &str) -> Response {
    let mut res = Problem::new(code).detail(detail).into_response();
    if code == ProblemCode::Unauthorized {
        res.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"beton\""),
        );
    }
    res
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, header::AUTHORIZATION)?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    header_str(headers, header::UPGRADE).is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

fn is_state_changing(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Die Sicherheitsschicht als axum-Middleware.
pub async fn guard(State(g): State<Guard>, mut req: Request, next: Next) -> Response {
    let headers = req.headers();
    let via_socket = req.extensions().get::<ViaSocket>().is_some();

    // 1. Host-Allowlist gegen DNS-Rebinding (AUTH-002 AC1).
    if !via_socket {
        let host = header_str(headers, header::HOST)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !g.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return reject(ProblemCode::HostNotAllowed, "Host-Header nicht erlaubt");
        }
    }

    // 2. WebSocket-Origin (AUTH-003).
    if is_websocket_upgrade(headers) {
        match header_str(headers, header::ORIGIN) {
            Some(origin) if !g.origins.allows(origin) => {
                return reject(ProblemCode::OriginNotAllowed, "Origin nicht erlaubt");
            }
            None if bearer(headers).is_none() => {
                return reject(
                    ProblemCode::OriginNotAllowed,
                    "WebSocket ohne Origin nur mit Authorization-Header",
                );
            }
            _ => {}
        }
    }

    // 3. Authentisierung (AUTH-001) und CSRF-Schutz für Cookies (AUTH-002 AC2).
    if PUBLIC_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    let auth = if let Some(token) = bearer(headers) {
        if !g.token.verify(token) {
            return reject(ProblemCode::Unauthorized, "Token ungültig");
        }
        Authenticated::Bearer
    } else if let Some(value) = cookie(headers, COOKIE_NAME)
        && g.logins.verify_cookie(value, SystemTime::now())
    {
        if is_state_changing(req.method()) {
            let origin_ok =
                header_str(headers, header::ORIGIN).is_some_and(|o| g.origins.allows(o));
            let site_ok = headers
                .get("sec-fetch-site")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|s| s == "same-origin" || s == "none");
            if !origin_ok || !site_ok {
                return reject(
                    ProblemCode::OriginNotAllowed,
                    "zustandsändernde Cookie-Requests nur von erlaubter Origin",
                );
            }
        }
        Authenticated::Cookie
    } else {
        // Browser-Navigation zur Web-UI: Hinweis statt JSON (Status bleibt 401).
        let wants_html =
            header_str(headers, header::ACCEPT).is_some_and(|a| a.contains("text/html"));
        if wants_html && req.method() == Method::GET && crate::web::is_ui_path(req.uri().path()) {
            let mut res = (
                axum::http::StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                crate::web::LOGIN_HINT,
            )
                .into_response();
            res.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                axum::http::HeaderValue::from_static(crate::web::CSP),
            );
            return res;
        }
        return reject(
            ProblemCode::Unauthorized,
            "lokales Token oder Anmeldung erforderlich",
        );
    };
    req.extensions_mut().insert(auth);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_003_ac3_wildcard_only_matches_subdomains_with_same_scheme() {
        let p = OriginPolicy::new(["https://*.example.com", "tauri://localhost"]);
        assert!(p.allows("https://a.example.com"));
        assert!(p.allows("https://a.b.example.com"));
        assert!(!p.allows("https://a.example.com.evil.test"));
        assert!(!p.allows("http://a.example.com"));
        assert!(!p.allows("https://example.com"));
        assert!(!p.allows("https://evilexample.com"));
        assert!(p.allows("tauri://localhost"));
        assert!(!p.allows("null"));
        assert!(!p.allows("https://a.example.com/pfad"));
    }

    #[test]
    fn cookies_and_bearer_are_parsed() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; beton_session=abc; b=2"),
        );
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer xyz"),
        );
        assert_eq!(cookie(&h, "beton_session"), Some("abc"));
        assert_eq!(cookie(&h, "fehlt"), None);
        assert_eq!(bearer(&h), Some("xyz"));
    }
}
