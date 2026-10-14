//! Test-Infrastruktur für Netz-Tests des Proxys – ausschließlich lokal (ADR-0033):
//!
//! - [`TestPki`]: Test-CA für lokale Test-Upstreams (gültige, abgelaufene, selbstsignierte
//!   Zertifikate).
//! - [`Upstream`]: lokaler HTTP/1.1- und HTTP/2-Server auf `127.0.0.1:0` (TLS oder Klartext)
//!   mit Echo, WebSocket-Echo, gRPC-artigen Trailern und `Connection: close`.
//! - [`ScriptedResolver`] und [`MappedDialer`]: Der Proxy sieht „öffentliche“ Adressen
//!   (z. B. `93.184.216.34`); der Dialer bildet nur ausdrücklich eingetragene Adressen auf
//!   lokale Test-Upstreams ab und lehnt alles andere ab. So erreicht kein Test je ein
//!   echtes Netz, und die Private-IP-Sperre wird ohne Ausnahme geprüft.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{HeaderMap, Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use time::{Duration, OffsetDateTime};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::net::{BoxFuture, Dial, Net, Resolve};
use crate::proxy::{EgressProxy, InstanceSpec, ProxyInstance};
use crate::{BetonCa, PrivateAllowList, RuleSet};

pub type Body = Full<Bytes>;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Test-CA für die lokalen Upstreams (nicht die beton-CA).
pub struct TestPki {
    ca_key: KeyPair,
    ca_params: CertificateParams,
    pub ca_der: CertificateDer<'static>,
}

impl TestPki {
    pub fn new() -> Self {
        let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "beton test upstream CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let now = OffsetDateTime::now_utc();
        ca_params.not_before = now - Duration::days(30);
        ca_params.not_after = now + Duration::days(30);
        let ca_der = ca_params.self_signed(&ca_key).unwrap().der().clone();
        Self {
            ca_key,
            ca_params,
            ca_der,
        }
    }

    fn server_config(
        chain: Vec<CertificateDer<'static>>,
        key: &KeyPair,
    ) -> Arc<rustls::ServerConfig> {
        let mut config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                chain,
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )
            .unwrap();
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(config)
    }

    /// Gültiges Zertifikat der Test-CA für `names`.
    pub fn valid(&self, names: &[&str]) -> Arc<rustls::ServerConfig> {
        let now = OffsetDateTime::now_utc();
        self.issue(names, now - Duration::days(1), now + Duration::days(1))
    }

    /// Abgelaufenes Zertifikat der Test-CA.
    pub fn expired(&self, names: &[&str]) -> Arc<rustls::ServerConfig> {
        let now = OffsetDateTime::now_utc();
        self.issue(names, now - Duration::days(10), now - Duration::days(1))
    }

    fn issue(
        &self,
        names: &[&str],
        not_before: OffsetDateTime,
        not_after: OffsetDateTime,
    ) -> Arc<rustls::ServerConfig> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .unwrap();
        params.not_before = not_before;
        params.not_after = not_after;
        let issuer = Issuer::from_params(&self.ca_params, &self.ca_key);
        let cert = params.signed_by(&key, &issuer).unwrap();
        Self::server_config(vec![cert.der().clone(), self.ca_der.clone()], &key)
    }

    /// Selbstsigniertes Zertifikat (keiner CA bekannt).
    pub fn self_signed(names: &[&str]) -> Arc<rustls::ServerConfig> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .unwrap();
        let cert = params.self_signed(&key).unwrap();
        Self::server_config(vec![cert.der().clone()], &key)
    }
}

/// Was ein Test-Upstream gesehen hat.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path_and_query: String,
    pub version: http::Version,
    pub headers: HeaderMap,
}

/// Lokaler Test-Upstream.
pub struct Upstream {
    pub addr: SocketAddr,
    pub requests: Arc<AtomicUsize>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Upstream {
    pub async fn tls(config: Arc<rustls::ServerConfig>) -> Self {
        Self::spawn(Some(config)).await
    }

    pub async fn plain() -> Self {
        Self::spawn(None).await
    }

    pub fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    async fn spawn(tls: Option<Arc<rustls::ServerConfig>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (r, s) = (requests.clone(), seen.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (r, s, tls) = (r.clone(), s.clone(), tls.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |req| handle(req, r.clone(), s.clone()));
                    let builder =
                        hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
                    match tls {
                        Some(config) => {
                            let Ok(stream) = TlsAcceptor::from(config).accept(tcp).await else {
                                return;
                            };
                            let _ = builder
                                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                                .await;
                        }
                        None => {
                            let _ = builder
                                .serve_connection_with_upgrades(TokioIo::new(tcp), service)
                                .await;
                        }
                    }
                });
            }
        });
        Self {
            addr,
            requests,
            seen,
            task,
        }
    }
}

type UpBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

fn full(b: impl Into<Bytes>) -> UpBody {
    Full::new(b.into()).boxed()
}

async fn handle(
    mut req: Request<Incoming>,
    requests: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
) -> Result<Response<UpBody>, std::convert::Infallible> {
    requests.fetch_add(1, Ordering::SeqCst);
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    seen.lock().unwrap().push(Seen {
        method: req.method().to_string(),
        path_and_query: path_and_query.clone(),
        version: req.version(),
        headers: req.headers().clone(),
    });
    let path = req.uri().path().to_owned();
    if path.starts_with("/ws") {
        let key = req
            .headers()
            .get("sec-websocket-key")
            .cloned()
            .unwrap_or(http::HeaderValue::from_static(""));
        let on_upgrade = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            let Ok(upgraded) = on_upgrade.await else {
                return;
            };
            let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                TokioIo::new(upgraded),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            use futures_util::{SinkExt as _, StreamExt as _};
            while let Some(Ok(msg)) = ws.next().await {
                if msg.is_text() || msg.is_binary() {
                    if ws.send(msg).await.is_err() {
                        return;
                    }
                } else if msg.is_close() {
                    return;
                }
            }
        });
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        return Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-accept", accept)
            .body(full(""))
            .unwrap());
    }
    if path == "/grpc" {
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());
        trailers.insert("grpc-message", "ok".parse().unwrap());
        let frames = futures_util::stream::iter(vec![
            Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(b"\0\0\0\0\0"))),
            Ok(Frame::trailers(trailers)),
        ]);
        return Ok(Response::builder()
            .header("content-type", "application/grpc")
            .body(StreamBody::new(frames).boxed())
            .unwrap());
    }
    let mut res = Response::builder()
        .header("content-type", "text/plain")
        .header("alt-svc", "h3=\":443\"; ma=86400");
    if path == "/close" {
        res = res.header("connection", "close");
    }
    let body = format!("{} {} {:?}", req.method(), path_and_query, req.version());
    Ok(res.body(full(body)).unwrap())
}

/// Test-Resolver: liefert pro Name der Reihe nach die eingetragenen Antworten (zyklisch).
#[derive(Default)]
pub struct ScriptedResolver {
    answers: Mutex<HashMap<String, (Vec<Vec<IpAddr>>, usize)>>,
    pub queries: AtomicUsize,
}

impl ScriptedResolver {
    pub fn set(&self, name: &str, answers: Vec<Vec<IpAddr>>) {
        self.answers
            .lock()
            .unwrap()
            .insert(name.to_owned(), (answers, 0));
    }
}

impl Resolve for ScriptedResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        let result = match answers.get_mut(host) {
            Some((list, next)) if !list.is_empty() => {
                let answer = list[*next % list.len()].clone();
                *next += 1;
                Ok(answer)
            }
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "NXDOMAIN")),
        };
        Box::pin(async move { result })
    }
}

/// Test-Dialer: verbindet nur eingetragene (virtuelle) Ziele mit lokalen Test-Upstreams und
/// protokolliert jeden Versuch.
#[derive(Default)]
pub struct MappedDialer {
    map: Mutex<HashMap<SocketAddr, SocketAddr>>,
    pub log: Mutex<Vec<SocketAddr>>,
}

impl MappedDialer {
    pub fn map(&self, virtual_addr: SocketAddr, real: SocketAddr) {
        self.map.lock().unwrap().insert(virtual_addr, real);
    }

    pub fn dialed(&self) -> Vec<SocketAddr> {
        self.log.lock().unwrap().clone()
    }
}

impl Dial for MappedDialer {
    fn dial(&self, addr: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>> {
        self.log.lock().unwrap().push(addr);
        let real = self.map.lock().unwrap().get(&addr).copied();
        Box::pin(async move {
            match real {
                Some(real) => TcpStream::connect(real).await,
                None => Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "Testziel nicht eingetragen",
                )),
            }
        })
    }
}

/// Proxy mit frischer beton-CA, Test-PKI, Test-Resolver und Test-Dialer.
pub struct Harness {
    pub proxy: EgressProxy,
    pub beton_ca: CertificateDer<'static>,
    pub pki: TestPki,
    pub resolver: Arc<ScriptedResolver>,
    pub dialer: Arc<MappedDialer>,
}

impl Harness {
    pub fn new() -> Self {
        let ca = BetonCa::generate("nod_test", OffsetDateTime::now_utc()).unwrap();
        let beton_ca = CertificateDer::from(ca.cert_der().to_vec());
        let pki = TestPki::new();
        let resolver = Arc::new(ScriptedResolver::default());
        let dialer = Arc::new(MappedDialer::default());
        let net = Net {
            resolver: resolver.clone(),
            dialer: dialer.clone(),
        };
        let proxy = EgressProxy::for_tests(ca, vec![pki.ca_der.clone()], net);
        Self {
            proxy,
            beton_ca,
            pki,
            resolver,
            dialer,
        }
    }

    /// `name` löst auf `ip` auf; `ip:port` führt zum Test-Upstream.
    pub fn route(&self, name: &str, ip: &str, port: u16, upstream: &Upstream) {
        let ip: IpAddr = ip.parse().unwrap();
        self.resolver.set(name, vec![vec![ip]]);
        self.dialer.map(SocketAddr::new(ip, port), upstream.addr);
    }

    pub async fn instance(&self, rules: &[&str]) -> ProxyInstance {
        self.instance_with(rules, &[]).await
    }

    pub async fn instance_with(&self, rules: &[&str], allow_private: &[&str]) -> ProxyInstance {
        self.proxy
            .start(InstanceSpec {
                label: "test".into(),
                rules: RuleSet::compile(rules).unwrap(),
                allow_private: PrivateAllowList::parse(allow_private).unwrap(),
            })
            .await
            .unwrap()
    }

    /// TLS-Client, der nur der beton-CA vertraut.
    pub fn client_tls(&self, alpn: &[&[u8]]) -> TlsConnector {
        connector(&[self.beton_ca.clone()], alpn)
    }
}

pub fn connector(roots: &[CertificateDer<'static>], alpn: &[&[u8]]) -> TlsConnector {
    let mut store = rustls::RootCertStore::empty();
    for r in roots {
        store.add(r.clone()).unwrap();
    }
    let mut config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    TlsConnector::from(Arc::new(config))
}

pub fn basic(user: &str, token: &str) -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{token}"))
    )
}

pub fn auth(instance: &ProxyInstance) -> String {
    basic("bt", instance.token().expose())
}

/// Antwort des Proxys als Klartext-HTTP (z. B. auf CONNECT).
#[derive(Debug)]
pub struct Raw {
    pub status: u16,
    pub head: String,
    pub body: String,
}

impl Raw {
    pub fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// Liest Status, Kopf und (per Content-Length) den Body einer Antwort.
pub async fn read_raw(stream: &mut TcpStream) -> Raw {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).await.unwrap();
        assert!(n > 0, "Verbindung vor Ende des Kopfs geschlossen");
        buf.push(byte[0]);
    }
    let head = String::from_utf8(buf).unwrap();
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.unwrap();
    Raw {
        status,
        head,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

/// Sendet einen CONNECT; bei 200 bleibt der Stream als Tunnel offen.
pub async fn connect(
    instance: &ProxyInstance,
    authority: &str,
    auth: Option<&str>,
) -> (TcpStream, Raw) {
    let mut stream = TcpStream::connect(instance.addr()).await.unwrap();
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(auth) = auth {
        req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let raw = read_raw(&mut stream).await;
    (stream, raw)
}

/// CONNECT plus TLS-Handshake mit der beton-CA als einzigem Vertrauensanker.
pub async fn tunnel(
    h: &Harness,
    instance: &ProxyInstance,
    host: &str,
    port: u16,
    alpn: &[&[u8]],
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let (stream, raw) = connect(instance, &format!("{host}:{port}"), Some(&auth(instance))).await;
    assert_eq!(raw.status, 200, "CONNECT {host}:{port}: {raw:?}");
    h.client_tls(alpn)
        .connect(ServerName::try_from(host.to_owned()).unwrap(), stream)
        .await
        .unwrap()
}

/// HTTP/1.1-Verbindung durch einen Tunnel.
pub async fn h1(
    stream: tokio_rustls::client::TlsStream<TcpStream>,
) -> hyper::client::conn::http1::SendRequest<Body> {
    let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(conn.with_upgrades());
    sender
}

/// HTTP/2-Verbindung durch einen Tunnel.
pub async fn h2(
    stream: tokio_rustls::client::TlsStream<TcpStream>,
) -> hyper::client::conn::http2::SendRequest<Body> {
    let (sender, conn) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    sender
}

/// Antwort komplett gelesen.
pub struct Got {
    pub status: StatusCode,
    pub version: http::Version,
    pub headers: HeaderMap,
    pub body: String,
    pub trailers: Option<HeaderMap>,
}

impl Got {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

pub async fn collect(res: Response<Incoming>) -> Got {
    let (parts, body) = res.into_parts();
    let collected = body.collect().await.unwrap();
    let trailers = collected.trailers().cloned();
    Got {
        status: parts.status,
        version: parts.version,
        headers: parts.headers,
        body: String::from_utf8_lossy(&collected.to_bytes()).into_owned(),
        trailers,
    }
}

pub fn get(uri: &str) -> Request<Body> {
    req("GET", uri)
}

pub fn req(method: &str, uri: &str) -> Request<Body> {
    let uri: http::Uri = uri.parse().unwrap();
    let mut b = Request::builder().method(method).uri(uri.clone());
    if let Some(a) = uri.authority() {
        b = b.header("host", a.as_str());
    }
    b.body(Body::default()).unwrap()
}

/// H1-Request mit Origin-Form-Pfad und Host-Header (wie curl im Tunnel).
pub fn origin(method: &str, host: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .body(Body::default())
        .unwrap()
}

/// Klartext-Request über den Proxy (absolute Form), mit oder ohne Auth.
pub async fn plain(instance: &ProxyInstance, method: &str, url: &str, auth: Option<&str>) -> Got {
    let stream = TcpStream::connect(instance.addr()).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(conn);
    let mut r = req(method, url);
    if let Some(auth) = auth {
        r.headers_mut()
            .insert("proxy-authorization", auth.parse().unwrap());
    }
    collect(sender.send_request(r).await.unwrap()).await
}

pub fn assert_denied(got: &Got, status: u16, error: &str) {
    assert_eq!(got.status.as_u16(), status, "Body: {}", got.body);
    assert_eq!(
        got.headers
            .get("x-beton-egress")
            .and_then(|v| v.to_str().ok()),
        Some(
            if status == 403 || status == 407 || status == 421 || status == 400 {
                "denied"
            } else {
                "error"
            }
        ),
        "{:?}",
        got.headers
    );
    assert_eq!(got.json()["error"], error, "Body: {}", got.body);
    assert!(got.json()["reason"].is_string(), "Body: {}", got.body);
}

pub fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}
