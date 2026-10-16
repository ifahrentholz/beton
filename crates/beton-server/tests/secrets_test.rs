//! `POST /v1/secrets/test` (SEC-004, SEC-011 AC3): Prüfung beim Provider über den Proxy-Pfad
//! gegen lokale Attrappen von `api.github.com` und einer GitLab-Instanz mit Test-CA
//! (ADR-0033, nie gegen github.com). Der Wert ist ein Fixture; er erscheint in keiner
//! Antwort, der Upstream bekommt ihn nur vom Proxy.

#![allow(clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use beton_secrets::backend::MemoryBackend;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const TOKEN: &str = "ghp_FIXTUREsecretsTEST0123456789abcdef";
const GITLAB: &str = "gitlab.example.test";

type Seen = Arc<Mutex<Vec<String>>>;

/// TLS-Upstream: `/user` (GitHub) und `/api/v4/personal_access_tokens/self` (GitLab);
/// Scopes aus dem Pfad-Präfix der Attrappe.
async fn upstream(
    scopes: &'static str,
    seen: Seen,
) -> (SocketAddr, rustls::pki_types::CertificateDer<'static>) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PKCS_ECDSA_P256_SHA256,
    };
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "beton secrets-test CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf = CertificateParams::new(vec!["api.github.com".to_owned(), GITLAB.to_owned()])
        .unwrap()
        .signed_by(&key, &Issuer::from_params(&ca, &ca_key))
        .unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone(), ca_cert.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
    .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, seen) = (acceptor.clone(), seen.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).into_owned();
                let header = |n: &str| {
                    head.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case(n)
                            .then(|| v.trim().to_owned())
                    })
                };
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let auth = header("authorization").or_else(|| header("private-token"));
                seen.lock()
                    .unwrap()
                    .push(format!("{path} {}", auth.clone().unwrap_or_default()));
                let (status, extra, body) = match path.as_str() {
                    "/user" if auth.as_deref() == Some(&format!("token {TOKEN}")) => (
                        200,
                        format!("X-OAuth-Scopes: {scopes}\r\n"),
                        json!({"login": "me", "echo": auth}).to_string(),
                    ),
                    "/api/v4/personal_access_tokens/self" if auth.as_deref() == Some(TOKEN) => {
                        let list: Vec<&str> = scopes.split(", ").collect();
                        (
                            200,
                            String::new(),
                            json!({"name": "beton", "scopes": list}).to_string(),
                        )
                    }
                    _ => (
                        401,
                        String::new(),
                        json!({"message": "Bad credentials"}).to_string(),
                    ),
                };
                let res = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(res.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    (addr, ca_cert.der().clone())
}

async fn setup(scopes: &'static str) -> (common::TestApp, Seen) {
    let seen = Seen::default();
    let (addr, upstream_ca) = upstream(scopes, seen.clone()).await;
    let ca =
        beton_proxy::BetonCa::generate("secrets-test", time::OffsetDateTime::now_utc()).unwrap();
    let proxy = beton_proxy::EgressProxy::with_test_hosts(
        ca,
        vec![upstream_ca],
        &[("api.github.com", addr), (GITLAB, addr)],
    );
    let t = common::app_runtime(move |mut r| {
        r.secret_backend = Arc::new(MemoryBackend::default());
        r.sessions.secret_tester = Some(Arc::new(
            beton_server::secret_test::SecretTester::with_proxy(proxy),
        ));
        r
    })
    .await;
    for r in [
        "secret://user/github.com/default",
        &format!("secret://user/{GITLAB}/default"),
    ] {
        let res = t
            .send(
                t.authed("POST", "/v1/secrets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"secret_ref": r, "kind": "token", "value": TOKEN}).to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, 201, "{}", String::from_utf8_lossy(&res.body));
    }
    (t, seen)
}

async fn test(t: &common::TestApp, r: &str) -> serde_json::Value {
    let res = t
        .send(
            t.authed("POST", "/v1/secrets/test")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "secret_ref": r }).to_string()))
                .unwrap(),
        )
        .await;
    let text = String::from_utf8_lossy(&res.body).into_owned();
    assert_eq!(res.status, 200, "{text}");
    assert!(!text.contains(TOKEN), "Wert in der Antwort: {text}");
    res.json()
}

#[tokio::test]
async fn sec_011_ac3_secrets_test_reports_missing_scopes_via_the_proxy_path() {
    let (t, seen) = setup("read:org, gist").await;
    let gh = test(&t, "secret://user/github.com/default").await;
    assert_eq!(gh["status"], 200, "{gh}");
    assert_eq!(gh["ok"], false);
    assert_eq!(gh["login"], "me");
    assert_eq!(gh["scopes"], json!(["read:org", "gist"]));
    assert_eq!(gh["missing"], json!(["repo"]));
    let gl = test(&t, &format!("secret://user/{GITLAB}/default")).await;
    assert_eq!(gl["provider"], GITLAB);
    assert_eq!(gl["missing"], json!(["api"]), "{gl}");
    // Der Upstream bekam den echten Wert vom Proxy, nie einen Platzhalter.
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(
        seen.iter()
            .all(|s| s.contains(TOKEN) && !s.contains("bt_cred_")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn sec_004_secrets_test_passes_with_all_scopes_and_rejects_unknown_kinds() {
    let (t, _seen) = setup("repo, api").await;
    let gh = test(&t, "secret://user/github.com/default").await;
    assert_eq!(gh["ok"], true, "{gh}");
    assert_eq!(gh["missing"], json!([]));
    let res = t
        .send(
            t.authed("POST", "/v1/secrets/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"secret_ref": "secret://user/npm/default"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 400);
}
