//! Auslieferung der Web-UI (WEB-001): statische Dateien aus dem Build von `apps/web`, für
//! Client-Routen (`/s/<id>` …) immer `index.html`. Alles vom eigenen Origin (ADR-0033);
//! die Content-Security-Policy erlaubt keine fremden Origins.

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// Content-Security-Policy der Web-UI: nur der eigene Origin.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                       img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; \
                       worker-src 'self' blob:; object-src 'none'; base-uri 'none'; \
                       frame-ancestors 'none'; form-action 'self'";

/// Seite, wenn kein Build der Web-UI vorliegt.
const NOT_BUNDLED: &str = "<!doctype html><html lang=\"de\"><meta charset=\"utf-8\">\
<title>beton</title><body style=\"font-family:system-ui;margin:3rem;max-width:40rem\">\
<h1>Web-UI nicht gefunden</h1><p>Dieser Daemon läuft, aber ohne gebaute Web-UI. \
Für die Entwicklung: <code>pnpm build</code> in <code>apps/web</code> und \
<code>BETON_WEB_DIR=apps/web/dist beton serve</code>.</p></body></html>";

/// Seite für Browser ohne Anmeldung (statt JSON-Problem).
pub const LOGIN_HINT: &str = "<!doctype html><html lang=\"de\"><meta charset=\"utf-8\">\
<title>beton – Anmeldung</title><body style=\"font-family:system-ui;margin:3rem;max-width:40rem\">\
<h1>Nicht angemeldet</h1><p>Öffne die Web-UI aus dem Terminal mit <code>beton open</code>; \
der Befehl meldet diesen Browser mit einem Einmal-Link an.</p></body></html>";

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Relativer, bereinigter Pfad; `None` bei `..`, absoluten Teilen oder Backslashes.
fn safe_relative(uri_path: &str) -> Option<PathBuf> {
    if uri_path.contains('\\') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(uri_path.trim_start_matches('/')).components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Gehört der Pfad der Web-UI (und nicht der API)?
pub fn is_ui_path(path: &str) -> bool {
    !(path == "/v1" || path.starts_with("/v1/") || path.starts_with("/auth/") || path == "/healthz")
}

fn with_headers(mut res: Response, path: &Path) -> Response {
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(content_type(path)) {
        h.insert(header::CONTENT_TYPE, v);
    }
    let immutable = path.starts_with("assets");
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if path.extension().is_some_and(|e| e == "html") {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
    res
}

/// Liefert eine Datei der Web-UI bzw. `index.html` für Client-Routen.
pub async fn serve(dir: Option<&Path>, method: &Method, uri: &Uri) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    if !is_ui_path(uri.path()) {
        return None;
    }
    let Some(dir) = dir else {
        let res = (StatusCode::OK, NOT_BUNDLED).into_response();
        return Some(with_headers(res, Path::new("index.html")));
    };
    let rel = safe_relative(uri.path())?;
    let candidate = dir.join(&rel);
    let (file, rel) = if !rel.as_os_str().is_empty() && candidate.is_file() {
        (candidate, rel)
    } else if rel.extension().is_none() {
        // Client-Route (`/`, `/s/<id>` …): die App übernimmt.
        (dir.join("index.html"), PathBuf::from("index.html"))
    } else {
        return None;
    };
    let bytes = tokio::fs::read(&file).await.ok()?;
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    Some(with_headers((StatusCode::OK, body).into_response(), &rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_is_rejected() {
        assert_eq!(
            safe_relative("/assets/a.js"),
            Some(PathBuf::from("assets/a.js"))
        );
        assert_eq!(safe_relative("/"), Some(PathBuf::new()));
        assert_eq!(safe_relative("/../etc/passwd"), None);
        assert_eq!(safe_relative("/assets/../../x"), None);
        assert_eq!(safe_relative("/a\\..\\b"), None);
    }

    #[test]
    fn api_paths_are_not_ui() {
        assert!(!is_ui_path("/v1/sessions"));
        assert!(!is_ui_path("/auth/local/redeem"));
        assert!(is_ui_path("/s/ses_1"));
        assert!(is_ui_path("/"));
    }
}
