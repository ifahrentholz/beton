//! HAR-011 AC2: Im Stufenmodell sieht der Loop nur einen Platzhalter `bt_cred_*`; der Request
//! geht über die Proxy-Instanz der Session, die den echten Key für den Host des Providers
//! einsetzt. Gegenstelle ist der lokale Mock hinter einem TLS-Terminator mit Test-CA; der
//! Proxy bildet den Namen über das Feature `test-net` auf ihn ab (ADR-0033).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::EventPayload;
use beton_harness::process::RealLauncher;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessAdapter, HostEnv, NormalizedEvent,
    SessionSpec, Shutdown, UserInput,
};
use beton_harness_direct::config::{KeyRef, ModelConfig, Provider, WireKind};
use beton_harness_direct::mock::{self, MockServer, Say};
use beton_harness_direct::secret::{ApiKey, KeyMap};
use beton_harness_direct::{DirectAdapter, DirectOptions, Egress};
use beton_proxy::credentials::BoxFuture;
use tokio::sync::mpsc;

const REAL: &str = "bt-fake-key-REALMARKER-a1b2c3d4";
const SECRET: &str = "secret://user/openrouter/default";
const HOST: &str = "api.provider.test";

struct Allow;

#[async_trait]
impl Gate for Allow {
    async fn decide(&self, _r: GateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }

    async fn model_request(
        &self,
        _r: beton_harness::ModelGateRequest,
    ) -> beton_harness::ModelGateDecision {
        beton_harness::ModelGateDecision::Allow {
            model: None,
            reasoning_effort: None,
        }
    }

    async fn tool_result(&self, _r: beton_harness::ResultGateRequest) -> GateDecision {
        GateDecision::Allow { updated_args: None }
    }
}

struct Source;

impl beton_proxy::CredentialSource for Source {
    fn value<'a>(&'a self, _b: &'a str) -> BoxFuture<'a, Result<beton_proxy::SecretBytes, String>> {
        Box::pin(async { Ok(beton_proxy::SecretBytes::new(REAL.as_bytes().to_vec())) })
    }
}

#[derive(Default)]
struct Audit(Mutex<Vec<(String, String)>>);

impl beton_proxy::CredentialAudit for Audit {
    fn used<'a>(&'a self, binding: &'a str, host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        self.0.lock().unwrap().push((binding.into(), host.into()));
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(&'a self, _b: Option<&'a str>, _h: &'a str, _r: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// TLS vor dem Mock (Test-CA, Name [`HOST`]).
async fn tls_front(mock: SocketAddr) -> (SocketAddr, rustls::pki_types::CertificateDer<'static>) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PKCS_ECDSA_P256_SHA256,
    };
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "beton direct test CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf = CertificateParams::new(vec![HOST.to_owned()])
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
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let Ok(mut up) = tokio::net::TcpStream::connect(mock).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut up).await;
            });
        }
    });
    (addr, ca_cert.der().clone())
}

fn provider(key: KeyRef) -> Provider {
    Provider {
        name: "openrouter".into(),
        wire: WireKind::Openai,
        base_url: format!("https://{HOST}/v1").parse().unwrap(),
        key,
        prompt_caching: false,
        models: vec![ModelConfig {
            id: "mock-model".into(),
            context_window: None,
            pricing: None,
        }],
    }
}

fn ctx() -> AdapterContext {
    AdapterContext {
        stage: None,
        gate: Arc::new(Allow),
        launcher: Arc::new(RealLauncher),
        env: HostEnv::default(),
    }
}

async fn turn(rx: &mut mpsc::Receiver<NormalizedEvent>) -> Vec<EventPayload> {
    let mut out = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let end = matches!(
            e.payload,
            EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_)
        );
        out.push(e.payload);
        if end {
            return out;
        }
    }
}

#[tokio::test]
async fn har_011_ac2_loop_sends_only_the_placeholder_and_the_proxy_injects_the_key() {
    let server = MockServer::start().await.unwrap();
    server.push(
        "/v1/chat/completions",
        mock::openai("chatcmpl-1", &[Say::Text("hallo".into(), 1)]),
    );
    let mock_addr: SocketAddr = server
        .base_url("")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    let (front, upstream_ca) = tls_front(mock_addr).await;
    let ca =
        beton_proxy::BetonCa::generate("direct-test", time::OffsetDateTime::now_utc()).unwrap();
    let proxy = beton_proxy::EgressProxy::with_test_hosts(ca, vec![upstream_ca], &[(HOST, front)]);
    let placeholder = beton_proxy::Placeholder::generate().unwrap();
    let audit = Arc::new(Audit::default());
    let instance = proxy
        .start(beton_proxy::InstanceSpec {
            label: "ses_direct/loop".into(),
            rules: beton_proxy::RuleSet::compile(&[format!("* {HOST}/**")]).unwrap(),
            credentials: beton_proxy::Credentials::new(
                vec![beton_proxy::Binding {
                    name: "provider:openrouter".into(),
                    kind: beton_proxy::CredentialKind::Bearer,
                    hosts: vec![HOST.into()],
                    username: None,
                    placeholder: placeholder.clone(),
                    implicit: false,
                }],
                Arc::new(Source),
                audit.clone(),
            )
            .unwrap(),
            ..beton_proxy::InstanceSpec::default()
        })
        .await
        .unwrap();
    let keys = KeyMap::new(BTreeMap::from([(
        SECRET.to_owned(),
        ApiKey::new(placeholder.as_str()).unwrap(),
    )]));
    let a = DirectAdapter::new(
        provider(KeyRef::Secret(SECRET.into())),
        &DirectOptions {
            keys: Arc::new(keys),
            discover_models: false,
            egress: Some(Egress {
                proxy_url: instance.proxy_url().to_string(),
                ca_pem: proxy.ca_pem(),
            }),
        },
    );
    let work = tempfile::tempdir().unwrap();
    let mut s = a
        .start(
            SessionSpec {
                workdir: work.path().to_path_buf(),
                ..SessionSpec::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    let mut rx = s.events().unwrap();
    s.send(UserInput::from("hallo?")).await.unwrap();
    let events = turn(&mut rx).await;
    s.shutdown(Shutdown::Kill).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EventPayload::TurnCompleted(_))),
        "{events:?}"
    );
    // Beim Anbieter kam der echte Key an, eingesetzt vom Proxy …
    let reqs = server.requests_to("/v1/chat/completions");
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0].header("authorization"),
        Some(format!("Bearer {REAL}").as_str())
    );
    assert_eq!(
        audit.0.lock().unwrap().as_slice(),
        [("provider:openrouter".to_owned(), HOST.to_owned())]
    );
    // … der Loop selbst hatte nur den Platzhalter, und kein Event enthält den Key.
    for e in &events {
        assert!(!serde_json::to_string(e).unwrap().contains(REAL), "{e:?}");
    }
}

#[tokio::test]
async fn har_011_secret_key_without_the_proxy_refuses_to_start() {
    let a = DirectAdapter::new(
        provider(KeyRef::Secret(SECRET.into())),
        &DirectOptions {
            keys: Arc::new(KeyMap::default()),
            discover_models: false,
            egress: None,
        },
    );
    let work = tempfile::tempdir().unwrap();
    let err = a
        .start(
            SessionSpec {
                workdir: work.path().to_path_buf(),
                ..SessionSpec::default()
            },
            ctx(),
        )
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("Egress-Proxy"), "{err}");
}
