//! Der Fuzz-Zugang (`crate::fuzzing`, QA-009) reicht die internen Funktionen unverändert
//! durch: gleiche Ergebnisse wie der Proxy-Code selbst.

use std::sync::Arc;

use http::header::{CONNECTION, HOST, PROXY_AUTHORIZATION, UPGRADE};
use http::{HeaderMap, HeaderValue, Request, Version};

use crate::credentials::{
    Binding, BoxFuture, CredentialAudit, CredentialKind, CredentialSource, Credentials,
    Placeholder, SecretBytes,
};
use crate::fuzzing::{self, ClientHello, CredentialOutcome};
use crate::normalize::{Host, Scheme};

/// `ClientHello` mit SNI `name` in einem Record.
fn hello(name: &str) -> Vec<u8> {
    let mut body = vec![3, 3];
    body.extend_from_slice(&[7u8; 32]);
    body.extend_from_slice(&[0, 0, 2, 0x13, 0x01, 1, 0]);
    let n = name.as_bytes();
    let mut list = vec![0u8];
    list.extend_from_slice(&(n.len() as u16).to_be_bytes());
    list.extend_from_slice(n);
    let mut sn = (list.len() as u16).to_be_bytes().to_vec();
    sn.extend_from_slice(&list);
    let mut ext = vec![0, 0];
    ext.extend_from_slice(&(sn.len() as u16).to_be_bytes());
    ext.extend_from_slice(&sn);
    body.extend_from_slice(&(ext.len() as u16).to_be_bytes());
    body.extend_from_slice(&ext);
    let mut hs = vec![1];
    hs.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    hs.extend_from_slice(&body);
    let mut out = vec![22, 3, 1];
    out.extend_from_slice(&(hs.len() as u16).to_be_bytes());
    out.extend_from_slice(&hs);
    out
}

#[test]
fn fuzzing_client_hello_mirrors_passthrough_parser() {
    let h = hello("Vendor.Example.");
    assert_eq!(
        fuzzing::client_hello_sni(&h),
        ClientHello::Sni(Some("vendor.example".into()))
    );
    assert_eq!(
        fuzzing::client_hello_sni(&h[..h.len() - 1]),
        ClientHello::Incomplete
    );
    assert_eq!(
        fuzzing::client_hello_sni(b"GET / HTTP/1.1\r\n"),
        ClientHello::Invalid
    );
}

#[test]
fn fuzzing_upgrade_and_hop_by_hop_mirror_proxy() {
    let req = Request::builder()
        .version(Version::HTTP_11)
        .uri("https://api.example/ws")
        .header(HOST, "api.example")
        .header(CONNECTION, "keep-alive, Upgrade")
        .header(UPGRADE, "websocket")
        .header(PROXY_AUTHORIZATION, "Basic eA==")
        .body(())
        .unwrap();
    assert!(fuzzing::is_upgrade(&req));
    let authorities = fuzzing::request_authorities(&req).unwrap();
    assert_eq!(
        authorities,
        vec![
            (Host::Domain("api.example".into()), None),
            (Host::Domain("api.example".into()), None)
        ]
    );
    let mut headers = req.headers().clone();
    fuzzing::strip_hop_by_hop(&mut headers, true);
    assert_eq!(headers.get(CONNECTION).unwrap(), "upgrade");
    assert_eq!(headers.get(UPGRADE).unwrap(), "websocket");
    assert!(!headers.contains_key(PROXY_AUTHORIZATION));

    let no_host = Request::builder().uri("/x").body(()).unwrap();
    assert!(fuzzing::request_authorities(&no_host).is_none());
}

struct Fixed;

impl CredentialSource for Fixed {
    fn value<'a>(&'a self, _binding: &'a str) -> BoxFuture<'a, Result<SecretBytes, String>> {
        Box::pin(async { Ok(SecretBytes::new(b"FIXTUREVALUE".to_vec())) })
    }
}

impl CredentialAudit for Fixed {
    fn used<'a>(&'a self, _binding: &'a str, _host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(
        &'a self,
        _binding: Option<&'a str>,
        _host: &'a str,
        _reason: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn fuzzing_credential_substitution_mirrors_plan_and_apply() {
    let placeholder = Placeholder::parse(&format!("bt_cred_{}", "a".repeat(32))).unwrap();
    let creds = Credentials::new(
        vec![Binding {
            name: "api".into(),
            kind: CredentialKind::Bearer,
            hosts: vec!["api.example".into()],
            username: None,
            placeholder: placeholder.clone(),
            implicit: false,
        }],
        Arc::new(Fixed),
        Arc::new(Fixed),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-token",
        HeaderValue::from_str(&format!("t={placeholder}")).unwrap(),
    );
    let api = Host::Domain("api.example".into());
    let applied =
        fuzzing::substitute_credentials(&creds, Scheme::Https, &api, "/v1", None, headers.clone())
            .await;
    let CredentialOutcome::Applied(out) = applied else {
        panic!("{applied:?}");
    };
    assert_eq!(out.headers.get("x-token").unwrap(), "t=FIXTUREVALUE");
    assert_eq!(out.path, "/v1");

    let evil = Host::Domain("evil.example".into());
    let refused =
        fuzzing::substitute_credentials(&creds, Scheme::Https, &evil, "/", None, headers).await;
    assert!(
        matches!(
            refused,
            CredentialOutcome::Refused {
                status: 403,
                error: "credential_host_mismatch"
            }
        ),
        "{refused:?}"
    );
    let untouched =
        fuzzing::substitute_credentials(&creds, Scheme::Https, &api, "/", None, HeaderMap::new())
            .await;
    assert!(matches!(untouched, CredentialOutcome::Untouched));
}
