//! Testhilfen: App im Prozess mit echtem Store, lokalem Token und Loopback-Peer.

#![allow(dead_code, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, Response, header};
use beton_server::app::{self, AppParts, AppState};
use beton_server::local_auth::{BrowserLogins, LocalToken};
use beton_store::{Store, StoreOptions};
use http_body_util::BodyExt;
use tower::ServiceExt;

pub const HOST: &str = "127.0.0.1:7420";
pub const ORIGIN: &str = "http://127.0.0.1:7420";

pub struct TestApp {
    pub router: Router,
    pub token: String,
    pub store: Store,
    pub logins: Arc<BrowserLogins>,
    pub dir: tempfile::TempDir,
}

pub async fn app() -> TestApp {
    app_with(|r| r, "127.0.0.1:50000").await
}

/// App mit zusätzlichen Routen und frei wählbarem Peer.
pub async fn app_with(
    extra: impl FnOnce(Router<AppState>) -> Router<AppState>,
    peer: &str,
) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let token = LocalToken::load_or_create(dir.path()).unwrap();
    let token_text = std::fs::read_to_string(token.path())
        .unwrap()
        .trim()
        .to_owned();
    let logins = Arc::new(BrowserLogins::default());
    let (router, doc) = app::routes();
    let router = app::layered(
        extra(router),
        &doc,
        AppParts {
            store: store.clone(),
            local,
            token: Arc::new(token),
            logins: logins.clone(),
            hosts: vec![HOST.into(), "localhost:7420".into(), "[::1]:7420".into()],
            origins: vec![ORIGIN.into(), "tauri://localhost".into()],
            primary_host: HOST.into(),
            runtime: app::Runtime::new(tokio::sync::watch::channel(false).1)
                .with_default_commands(),
        },
    )
    .layer(MockConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    TestApp {
        router,
        token: token_text,
        store,
        logins,
        dir,
    }
}

pub struct Res {
    pub status: u16,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Res {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Prüft das Problem-Format (PROTO-011, API-001 AC3).
    pub fn assert_problem(&self, code: &str) {
        assert_eq!(
            self.header("content-type"),
            Some("application/problem+json"),
            "Status {}: {}",
            self.status,
            String::from_utf8_lossy(&self.body)
        );
        let p: beton_server::Problem = serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "kein Problem-Objekt ({e}): {}",
                String::from_utf8_lossy(&self.body)
            )
        });
        assert_eq!(p.status, self.status);
        assert_eq!(p.code.as_str(), code, "{p:?}");
        assert_eq!(p.kind, format!("urn:beton:problem:{code}"));
        assert!(!p.trace_id.is_empty());
    }
}

impl TestApp {
    pub fn get(&self, path: &str) -> axum::http::request::Builder {
        Request::get(path).header(header::HOST, HOST)
    }

    pub fn authed(&self, method: &str, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
    }

    pub async fn send(&self, req: Request<Body>) -> Res {
        let res: Response<Body> = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        Res {
            status,
            headers,
            body,
        }
    }

    /// Meldet sich wie ein Browser an und liefert das Cookie.
    pub async fn browser_cookie(&self) -> String {
        let res = self
            .send(
                self.authed("POST", "/v1/auth/local/codes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 201);
        let code = res.json()["code"].as_str().unwrap().to_owned();
        let res = self
            .send(
                self.get(&format!("/auth/local/redeem?code={code}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 303);
        res.header("set-cookie")
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }
}
