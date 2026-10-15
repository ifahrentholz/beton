//! Proxy-Kern und Instanzmodell (PRX-001), TLS-MITM (PRX-002), Regelprüfung je Request bzw.
//! HTTP/2-Stream (PRX-003, PRX-004) und Private-IP-Schutz beim Verbindungsaufbau (PRX-007).

use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use http::header::{
    ALT_SVC, CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HOST, PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use http::uri::Authority;
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri, Version,
};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use zeroize::{Zeroize as _, Zeroizing};

use beton_core::event::{EgressBlocked, EgressSource};

use crate::ca::{BetonCa, CaInfo, LeafIssuer};
use crate::credentials::{Credentials, Plan, Prefixed, REDACT_LIMIT, ResponseRedactor, Treatment};
use crate::egress::{Counted, EgressEvents, Traffic};
use crate::ip::{PrivateAllowList, destination_allowed};
use crate::net::{ConnectError, Connector, Idle, Net, dial_checked, find_connect_error};
use crate::normalize::{Host, Scheme, normalize_path};
use crate::passthrough::{Hello, Passthrough, client_hello_sni};
use crate::rules::{Decision, DenyReason, Request as RulesRequest, RuleSet};

/// Timeout für den Verbindungsaufbau zum Upstream (inkl. DNS und TLS).
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Leerlauf-Timeout für Client- und Upstream-Verbindungen.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Höchstzahl gleichzeitiger HTTP/2-Streams pro Client-Verbindung.
pub const MAX_CONCURRENT_STREAMS: u32 = 100;

/// Einstellungen des Proxys (`proxy.*` in der Konfiguration).
#[derive(Debug, Clone, Default)]
pub struct ProxyOptions {
    /// Zusätzliche vertrauenswürdige CAs für Upstream-TLS (PEM), z. B. eine Firmen-CA.
    pub extra_ca_files: Vec<PathBuf>,
}

/// Fehler beim Aufbau des Proxys oder einer Instanz.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("proxy_config: {0}")]
    Config(String),
    #[error("proxy_io: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ca(#[from] crate::ca::CaError),
}

/// Regeln und Freigaben einer Proxy-Instanz (eine pro Session-Stufe).
#[derive(Clone, Default)]
pub struct InstanceSpec {
    /// Kennung für Logs, z. B. `ses_…/tools`; nie ein Secret.
    pub label: String,
    pub rules: RuleSet,
    pub allow_private: PrivateAllowList,
    /// Vendor-Hosts ohne TLS-Terminierung (PRX-005); nur in Stufe 1 bzw. ausdrücklich.
    pub passthrough: Passthrough,
    /// Credential-Bindings mit Platzhaltern (PRX-006); nie auf Passthrough-Verbindungen.
    pub credentials: Credentials,
    /// Herkunft für `egress.blocked` (PRX-009).
    pub source: EgressSource,
    /// Empfänger von `egress.blocked` (PRX-009 AC1); ohne Empfänger nur Logs.
    pub events: Option<Arc<dyn EgressEvents>>,
    /// Zähler für `egress.summary` (PRX-009 AC3), je Session geteilt.
    pub traffic: Option<Traffic>,
}

impl fmt::Debug for InstanceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstanceSpec")
            .field("label", &self.label)
            .field("rules", &self.rules)
            .field("passthrough", &self.passthrough)
            .field("credentials", &self.credentials)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

type ProxyBody = BoxBody<Bytes, hyper::Error>;
type HttpClient = Client<Connector, Incoming>;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// System-Roots, einmal pro Prozess geladen. Einzelne unlesbare Einträge werden übersprungen.
fn native_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| rustls_native_certs::load_native_certs().certs)
}

/// Liest zusätzliche CAs (PEM). Fehlt eine Datei oder enthält sie kein Zertifikat, ist das ein
/// Konfigurationsfehler (fail closed statt stillem Weglassen).
fn extra_roots(files: &[PathBuf]) -> Result<Vec<CertificateDer<'static>>, ProxyError> {
    let mut out = Vec::new();
    for file in files {
        let certs: Vec<_> = CertificateDer::pem_file_iter(file)
            .and_then(|it| it.collect::<Result<Vec<_>, _>>())
            .map_err(|e| {
                ProxyError::Config(format!("proxy.extra_ca_files {}: {e}", file.display()))
            })?;
        if certs.is_empty() {
            return Err(ProxyError::Config(format!(
                "proxy.extra_ca_files {}: kein Zertifikat gefunden",
                file.display()
            )));
        }
        out.extend(certs);
    }
    Ok(out)
}

/// Prüft `proxy.extra_ca_files` (beim Laden der Konfiguration): Jede Datei muss lesbar sein und
/// mindestens ein Zertifikat enthalten (PRX-002).
pub fn check_extra_ca_files(files: &[PathBuf]) -> Result<(), ProxyError> {
    extra_roots(files).map(|_| ())
}

/// TLS-Client-Konfiguration für Upstreams: strikte webpki-Prüfung gegen `roots`, kein
/// Override.
fn upstream_tls(roots: &RootCertStore, alpn: &[&[u8]]) -> Result<Arc<ClientConfig>, ProxyError> {
    let mut config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| ProxyError::Config(e.to_string()))?
        .with_root_certificates(roots.clone())
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

struct Shared {
    ca: Arc<BetonCa>,
    net: Net,
    /// Upstream-Clients: ALPN `h2` + `http/1.1` bzw. nur `http/1.1` (für Upgrades).
    tls_auto: Arc<ClientConfig>,
    tls_h1: Arc<ClientConfig>,
}

/// Der Egress-Proxy eines Runners: eine CA, ein Upstream-Vertrauensanker, beliebig viele
/// Instanzen.
#[derive(Clone)]
pub struct EgressProxy {
    shared: Arc<Shared>,
}

impl fmt::Debug for EgressProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EgressProxy")
            .field("ca", &self.shared.ca)
            .finish_non_exhaustive()
    }
}

impl EgressProxy {
    /// Proxy mit System-Resolver, direkten Verbindungen und System-Roots plus
    /// `extra_ca_files` für Upstream-TLS.
    pub fn new(ca: BetonCa, options: &ProxyOptions) -> Result<Self, ProxyError> {
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(native_roots().iter().cloned());
        for cert in extra_roots(&options.extra_ca_files)? {
            roots
                .add(cert)
                .map_err(|e| ProxyError::Config(format!("proxy.extra_ca_files: {e}")))?;
        }
        Self::build(ca, &roots, Net::system())
    }

    /// Nur für Tests: Test-Resolver, Test-Dialer und die Test-CA der lokalen Upstreams als
    /// einziger Vertrauensanker. Nicht aus Konfiguration erreichbar.
    #[cfg(test)]
    pub(crate) fn for_tests(
        ca: BetonCa,
        upstream_roots: Vec<CertificateDer<'static>>,
        net: Net,
    ) -> Self {
        let mut roots = RootCertStore::empty();
        for r in upstream_roots {
            roots.add(r).unwrap_or_else(|e| panic!("Test-CA: {e}"));
        }
        Self::build(ca, &roots, net).unwrap_or_else(|e| panic!("Test-Proxy: {e}"))
    }

    fn build(ca: BetonCa, roots: &RootCertStore, net: Net) -> Result<Self, ProxyError> {
        Ok(Self {
            shared: Arc::new(Shared {
                ca: Arc::new(ca),
                net,
                tls_auto: upstream_tls(roots, &[b"h2", b"http/1.1"])?,
                tls_h1: upstream_tls(roots, &[b"http/1.1"])?,
            }),
        })
    }

    pub fn ca_info(&self) -> CaInfo {
        self.shared.ca.info()
    }

    /// PEM der CA dieses Proxys (Bundle für die Sandbox, PRX-008).
    pub fn ca_pem(&self) -> String {
        self.shared.ca.cert_pem()
    }

    /// CA-Bundle für die Sandbox (PRX-008): beton-CA plus System-Roots als PEM. Werkzeuge in
    /// der Sandbox sehen sonst nur Leafs der beton-CA; die System-Roots braucht der Harness für
    /// seine Passthrough-Verbindungen (PRX-005).
    pub fn ca_bundle_pem(&self) -> String {
        let mut pem = self.ca_pem();
        for root in native_roots() {
            pem.push_str(&crate::ca::pem_certificate(root.as_ref()));
        }
        pem
    }

    /// Nur für Tests anderer Crates (Feature `test-net`, nie in Produktionsbuilds): Jeder Name
    /// aus `hosts` löst auf eine öffentlich wirkende Adresse auf, die der Dialer auf den
    /// lokalen Test-Upstream abbildet; alle anderen Namen sind NXDOMAIN, alle anderen Adressen
    /// unerreichbar. Upstream-TLS vertraut nur `upstream_roots`. Die Private-IP-Sperre gilt
    /// unverändert (ADR-0033: kein Test erreicht ein echtes Netz).
    #[cfg(any(test, feature = "test-net"))]
    pub fn with_test_hosts(
        ca: BetonCa,
        upstream_roots: Vec<CertificateDer<'static>>,
        hosts: &[(&str, std::net::SocketAddr)],
    ) -> Self {
        let mut roots = RootCertStore::empty();
        for r in upstream_roots {
            roots.add(r).unwrap_or_else(|e| panic!("Test-CA: {e}"));
        }
        Self::build(ca, &roots, crate::net::Net::test_hosts(hosts))
            .unwrap_or_else(|e| panic!("Test-Proxy: {e}"))
    }

    /// Startet eine Instanz auf `127.0.0.1:<zufälliger Port>` mit eigenem Token, eigenem
    /// Leaf-Schlüssel und eigenem Upstream-Pool.
    pub async fn start(&self, spec: InstanceSpec) -> Result<ProxyInstance, ProxyError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| ProxyError::Config(e.to_string()))?;
        let token = Zeroizing::new(hex::encode(raw));
        raw.zeroize();
        let expected = Zeroizing::new(format!("bt:{}", token.as_str()).into_bytes());
        let allow = Arc::new(spec.allow_private);
        let client = |tls: &Arc<ClientConfig>| -> HttpClient {
            Client::builder(TokioExecutor::new())
                .pool_idle_timeout(IDLE_TIMEOUT)
                .pool_timer(TokioTimer::new())
                .timer(TokioTimer::new())
                .build(Connector {
                    net: self.shared.net.clone(),
                    tls: tls.clone(),
                    allow_private: allow.clone(),
                })
        };
        let (stop, stopped) = watch::channel(false);
        let instance = Arc::new(Instance {
            label: spec.label,
            rules: spec.rules,
            passthrough: spec.passthrough,
            credentials: spec.credentials,
            source: spec.source,
            events: spec.events,
            traffic: spec.traffic,
            net: self.shared.net.clone(),
            allow_private: allow.clone(),
            expected,
            leafs: LeafIssuer::new(self.shared.ca.clone())?,
            auto: client(&self.shared.tls_auto),
            h1: client(&self.shared.tls_h1),
            stopped,
        });
        let accept = instance.clone();
        spawn_until(instance.stopped.clone(), async move {
            loop {
                match listener.accept().await {
                    Ok((tcp, _)) => {
                        let _ = tcp.set_nodelay(true);
                        let inst = accept.clone();
                        spawn_until(accept.stopped.clone(), inst.serve_client(tcp));
                    }
                    Err(e) => {
                        tracing::warn!(instance = %accept.label, "accept: {e}");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }
        });
        tracing::debug!(instance = %instance.label, %addr, "Proxy-Instanz gestartet");
        Ok(ProxyInstance {
            addr,
            token: ProxyToken(token),
            stop,
        })
    }
}

/// Startet eine Aufgabe, die mit dem Ende der Instanz abbricht.
fn spawn_until<F>(mut stopped: watch::Receiver<bool>, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        tokio::select! {
            () = fut => {}
            _ = stopped.wait_for(|s| *s) => {}
        }
    });
}

/// Auth-Token einer Instanz (256 bit, hex); `Debug` zeigt es nie.
pub struct ProxyToken(Zeroizing<String>);

impl ProxyToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProxyToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Eine laufende Proxy-Instanz; endet mit `Drop` (Listener, Tunnel und Verbindungen).
pub struct ProxyInstance {
    addr: SocketAddr,
    token: ProxyToken,
    stop: watch::Sender<bool>,
}

impl Drop for ProxyInstance {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl fmt::Debug for ProxyInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyInstance")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl ProxyInstance {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &ProxyToken {
        &self.token
    }

    /// Proxy-URL für die Sandbox: `http://bt:<token>@127.0.0.1:<port>` (enthält das Token).
    pub fn proxy_url(&self) -> Zeroizing<String> {
        Zeroizing::new(format!(
            "http://bt:{}@127.0.0.1:{}",
            self.token.expose(),
            self.addr.port()
        ))
    }
}

/// Zustand einer Instanz.
struct Instance {
    label: String,
    rules: RuleSet,
    /// Vendor-Hosts ohne TLS-Terminierung (PRX-005).
    passthrough: Passthrough,
    /// Credential-Bindings (PRX-006).
    credentials: Credentials,
    source: EgressSource,
    events: Option<Arc<dyn EgressEvents>>,
    traffic: Option<Traffic>,
    net: Net,
    allow_private: Arc<PrivateAllowList>,
    /// Erwarteter Klartext von `Proxy-Authorization: Basic …` (`bt:<token>`).
    expected: Zeroizing<Vec<u8>>,
    leafs: LeafIssuer,
    auto: HttpClient,
    h1: HttpClient,
    stopped: watch::Receiver<bool>,
}

/// Leaf-Zertifikat eines Tunnels (unabhängig von der SNI des Clients).
#[derive(Debug)]
struct FixedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// Fehlerantwort mit `X-Beton-Egress` und JSON `{error, reason, rule_hint?}`.
fn problem(
    status: StatusCode,
    error: &str,
    reason: impl Into<String>,
    rule_hint: Option<String>,
) -> Response<ProxyBody> {
    let egress = if status.is_server_error() {
        "error"
    } else {
        "denied"
    };
    let mut body = serde_json::json!({ "error": error, "reason": reason.into() });
    if let Some(hint) = rule_hint {
        body["rule_hint"] = serde_json::Value::String(hint);
    }
    let mut res = Response::new(
        Full::new(Bytes::from(body.to_string()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-beton-egress", HeaderValue::from_static(egress));
    res
}

fn invalid(reason: impl Into<String>) -> Response<ProxyBody> {
    problem(StatusCode::BAD_REQUEST, "invalid_request", reason, None)
}

fn private_destination(ip: IpAddr) -> Response<ProxyBody> {
    problem(
        StatusCode::FORBIDDEN,
        "private_destination",
        format!("Ziel {ip} ist nicht global und nicht in egress_allow_private freigegeben"),
        None,
    )
}

/// Antwort auf einen gescheiterten Verbindungsaufbau (wie bei weitergeleiteten Requests).
fn connect_problem(e: &ConnectError) -> Response<ProxyBody> {
    match e {
        ConnectError::PrivateDestination(ip) => private_destination(*ip),
        ConnectError::InvalidHost => invalid("ungültiger Host"),
        ConnectError::Dns(_) => problem(StatusCode::BAD_GATEWAY, "dns_error", e.to_string(), None),
        ConnectError::Tls(_) => problem(
            StatusCode::BAD_GATEWAY,
            "upstream_tls_error",
            e.to_string(),
            None,
        ),
        ConnectError::Timeout => problem(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            e.to_string(),
            None,
        ),
        ConnectError::Connect(_) => problem(
            StatusCode::BAD_GATEWAY,
            "upstream_connect_error",
            e.to_string(),
            None,
        ),
    }
}

fn empty(status: StatusCode) -> Response<ProxyBody> {
    let mut res = Response::new(Empty::new().map_err(|never| match never {}).boxed());
    *res.status_mut() = status;
    res
}

/// Pfad für Logs: ohne Query, höchstens 256 Zeichen.
fn log_path(path: &str) -> &str {
    match path.char_indices().nth(256) {
        Some((i, _)) => &path[..i],
        None => path,
    }
}

/// Entfernt Hop-by-Hop-Header (RFC 9110 §7.6.1), auch die in `Connection` genannten.
/// `TE: trailers` bleibt (gRPC); bei Upgrades bleiben `Upgrade` und `Connection: upgrade`.
pub(crate) fn strip_hop_by_hop(headers: &mut HeaderMap, upgrade: bool) {
    let listed: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|n| HeaderName::from_bytes(n.trim().as_bytes()).ok())
        .collect();
    for name in listed {
        if !(upgrade && name == UPGRADE) {
            headers.remove(name);
        }
    }
    let te_trailers = headers.get_all(TE).iter().any(|v| {
        v.to_str().is_ok_and(|s| {
            s.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("trailers"))
        })
    });
    for name in [
        CONNECTION,
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
        TE,
        TRAILER,
        TRANSFER_ENCODING,
    ] {
        headers.remove(name);
    }
    headers.remove("proxy-connection");
    headers.remove("keep-alive");
    if te_trailers {
        headers.insert(TE, HeaderValue::from_static("trailers"));
    }
    if upgrade {
        headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
    } else {
        headers.remove(UPGRADE);
    }
}

pub(crate) fn is_upgrade<B>(req: &Request<B>) -> bool {
    req.version() == Version::HTTP_11
        && req.headers().contains_key(UPGRADE)
        && req.headers().get_all(CONNECTION).iter().any(|v| {
            v.to_str().is_ok_and(|s| {
                s.split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
            })
        })
}

/// Host und Port eines Requests im Tunnel: `:authority`/absolute URI und `Host`-Header; beide
/// müssen, wenn vorhanden, übereinstimmen.
pub(crate) fn request_authorities<B>(req: &Request<B>) -> Result<Vec<(Host, Option<u16>)>, ()> {
    let mut out = Vec::new();
    if let Some(a) = req.uri().authority() {
        out.push((Host::parse(a.host()).map_err(|_| ())?, a.port_u16()));
    }
    for v in req.headers().get_all(HOST) {
        let a: Authority = v.to_str().map_err(|_| ())?.parse().map_err(|_| ())?;
        out.push((Host::parse(a.host()).map_err(|_| ())?, a.port_u16()));
    }
    if out.is_empty() { Err(()) } else { Ok(out) }
}

impl Instance {
    async fn serve_client(self: Arc<Self>, tcp: TcpStream) {
        let io = TokioIo::new(Idle::new(tcp, IDLE_TIMEOUT));
        let inst = self.clone();
        let service = service_fn(move |req| {
            let inst = inst.clone();
            async move { Ok::<_, Infallible>(inst.handle(req).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(Some(Duration::from_secs(30)))
            .serve_connection(io, service)
            .with_upgrades()
            .await;
    }

    /// `Proxy-Authorization: Basic base64(bt:<token>)`, Vergleich in konstanter Zeit.
    fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(value) = headers.get(PROXY_AUTHORIZATION) else {
            return false;
        };
        let value = value.as_bytes();
        if value.len() < 6 || !value[..6].eq_ignore_ascii_case(b"basic ") {
            return false;
        }
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(value[6..].trim_ascii())
        else {
            return false;
        };
        let decoded = Zeroizing::new(decoded);
        bool::from(decoded.as_slice().ct_eq(self.expected.as_slice()))
    }

    async fn handle(self: Arc<Self>, req: Request<Incoming>) -> Response<ProxyBody> {
        if !self.authorized(req.headers()) {
            tracing::debug!(instance = %self.label, "407: Proxy-Authorization fehlt oder falsch");
            let mut res = problem(
                StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                "proxy_auth_required",
                "Proxy-Authorization fehlt oder ist ungültig",
                None,
            );
            res.headers_mut().insert(
                PROXY_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"beton\""),
            );
            return res;
        }
        if req.method() == Method::CONNECT {
            self.connect(req).await
        } else {
            self.forward_absolute(req).await
        }
    }

    fn log_deny(&self, method: &str, host: &Host, port: u16, path: &str, reason: &str) {
        self.deny_with_hint(method, host, port, path, reason, None);
    }

    /// Protokolliert eine Ablehnung (ohne Query, Header-Werte oder Bodies) und meldet sie als
    /// `egress.blocked` (PRX-009 AC1).
    fn deny_with_hint(
        &self,
        method: &str,
        host: &Host,
        port: u16,
        path: &str,
        reason: &str,
        rule_hint: Option<String>,
    ) {
        let path = log_path(path);
        tracing::debug!(
            instance = %self.label,
            method,
            host = %host,
            port,
            path,
            decision = "deny",
            reason,
            "Egress abgelehnt"
        );
        if let Some(events) = &self.events {
            events.blocked(EgressBlocked {
                method: method.to_owned(),
                host: host.to_string(),
                path: path.to_owned(),
                reason: reason.to_owned(),
                rule_hint,
                source: self.source,
                tool_call_id: None,
            });
        }
    }

    /// CONNECT: Vorprüfung (PRX-003 AC5) ohne TLS-Handshake, dann Tunnel mit MITM bzw. für
    /// Passthrough-Ziele ohne TLS-Terminierung (PRX-005).
    async fn connect(self: Arc<Self>, req: Request<Incoming>) -> Response<ProxyBody> {
        let Some(authority) = req.uri().authority().cloned() else {
            return invalid("CONNECT braucht host:port");
        };
        let Ok(host) = Host::parse(authority.host()) else {
            return invalid("ungültiger Host im CONNECT");
        };
        let port = authority.port_u16().unwrap_or(443);
        if let Host::Ip(ip) = &host
            && !destination_allowed(SocketAddr::new(*ip, port), &self.allow_private)
        {
            self.log_deny("CONNECT", &host, port, "", "private_destination");
            return private_destination(*ip);
        }
        if self.passthrough.contains(&host, port) {
            return self.passthrough(req, host, port).await;
        }
        if !self.rules.may_connect(Scheme::Https, &host, port) {
            let hint = RuleSet::rule_hint(&RulesRequest {
                method: "GET",
                scheme: Scheme::Https,
                host: &host,
                port,
                path: "/**",
            });
            self.deny_with_hint(
                "CONNECT",
                &host,
                port,
                "",
                "no_matching_rule",
                Some(hint.clone()),
            );
            return problem(
                StatusCode::FORBIDDEN,
                "egress_denied",
                format!("Keine Egress-Regel erlaubt {host}:{port}"),
                Some(hint),
            );
        }
        let leaf = match self.leafs.certified_key(&host, OffsetDateTime::now_utc()) {
            Ok(leaf) => leaf,
            Err(e) => {
                tracing::warn!(instance = %self.label, "Leaf-Zertifikat: {e}");
                return problem(StatusCode::BAD_GATEWAY, e.code(), e.to_string(), None);
            }
        };
        let inst = self.clone();
        spawn_until(self.stopped.clone(), async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    inst.serve_tunnel(TokioIo::new(upgraded), host, port, leaf)
                        .await
                }
                Err(e) => tracing::debug!("CONNECT-Upgrade: {e}"),
            }
        });
        empty(StatusCode::OK)
    }

    /// Passthrough (PRX-005): Upstream nach Auflösung und Private-IP-Prüfung verbinden, erst
    /// dann `200`; danach muss das erste, was der Client schickt, ein `ClientHello` mit der SNI
    /// des CONNECT-Hosts sein. Sonst wird getrennt, bevor ein Byte den Upstream erreicht.
    /// Protokolliert werden nur Host, Port, Entscheidung und Byte-Zahlen.
    async fn passthrough(
        self: Arc<Self>,
        req: Request<Incoming>,
        host: Host,
        port: u16,
    ) -> Response<ProxyBody> {
        let dial = tokio::time::timeout(
            CONNECT_TIMEOUT,
            dial_checked(&self.net, &self.allow_private, &host, port),
        )
        .await
        .unwrap_or(Err(ConnectError::Timeout));
        let upstream = match dial {
            Ok(tcp) => tcp,
            Err(ConnectError::PrivateDestination(ip)) => {
                self.log_deny("CONNECT", &host, port, "", "private_destination");
                return private_destination(ip);
            }
            Err(e) => {
                tracing::debug!(instance = %self.label, host = %host, port, "Passthrough: {e}");
                return connect_problem(&e);
            }
        };
        let inst = self.clone();
        spawn_until(self.stopped.clone(), async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    inst.splice(TokioIo::new(upgraded), upstream, host, port)
                        .await
                }
                Err(e) => tracing::debug!("CONNECT-Upgrade: {e}"),
            }
        });
        empty(StatusCode::OK)
    }

    async fn splice(
        self: Arc<Self>,
        mut client: TokioIo<Upgraded>,
        upstream: TcpStream,
        host: Host,
        port: u16,
    ) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut hello = Vec::with_capacity(2048);
        let mut chunk = [0u8; 4096];
        let verdict = tokio::time::timeout(CONNECT_TIMEOUT, async {
            loop {
                match client_hello_sni(&hello) {
                    Hello::Incomplete => {}
                    done => return done,
                }
                match client.read(&mut chunk).await {
                    Ok(0) | Err(_) => return Hello::Invalid,
                    Ok(n) => hello.extend_from_slice(&chunk[..n]),
                }
            }
        })
        .await
        .unwrap_or(Hello::Invalid);
        let expected = match &host {
            Host::Domain(d) => Some(d.as_str()),
            Host::Ip(_) => None,
        };
        let reason = match &verdict {
            Hello::Sni(Some(sni)) if Some(sni.as_str()) == expected => None,
            Hello::Sni(Some(_)) => Some("sni_mismatch"),
            Hello::Sni(None) => Some("sni_missing"),
            Hello::Incomplete | Hello::Invalid => Some("no_client_hello"),
        };
        if let Some(reason) = reason {
            self.log_deny("CONNECT", &host, port, "", reason);
            return;
        }
        tracing::debug!(
            instance = %self.label,
            host = %host,
            port,
            decision = "passthrough",
            "Egress durchgereicht (ohne TLS-Terminierung)"
        );
        let mut upstream = Idle::new(upstream, IDLE_TIMEOUT);
        if upstream.write_all(&hello).await.is_err() {
            return;
        }
        let mut client = Idle::new(client, IDLE_TIMEOUT);
        let copied = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        let (up, down) = copied.unwrap_or((0, 0));
        tracing::trace!(
            instance = %self.label,
            host = %host,
            port,
            bytes_up = up + hello.len() as u64,
            bytes_down = down,
            "Passthrough beendet"
        );
    }

    async fn serve_tunnel(
        self: Arc<Self>,
        io: TokioIo<Upgraded>,
        host: Host,
        port: u16,
        leaf: Arc<CertifiedKey>,
    ) {
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map(|b| {
                b.with_no_client_auth()
                    .with_cert_resolver(Arc::new(FixedCert(leaf)))
            });
        let Ok(mut config) = config else {
            return;
        };
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let accept = TlsAcceptor::from(Arc::new(config)).accept(io);
        let tls = match tokio::time::timeout(CONNECT_TIMEOUT, accept).await {
            Ok(Ok(tls)) => tls,
            Ok(Err(e)) => {
                tracing::debug!(instance = %self.label, "TLS mit dem Client: {e}");
                return;
            }
            Err(_) => return,
        };
        let tunnel = Arc::new(Tunnel {
            inst: self,
            host,
            port,
        });
        let service = service_fn(move |req| {
            let tunnel = tunnel.clone();
            async move { Ok::<_, Infallible>(tunnel.handle(req).await) }
        });
        let mut builder = auto::Builder::new(TokioExecutor::new());
        builder.http1().timer(TokioTimer::new());
        builder
            .http2()
            .timer(TokioTimer::new())
            .max_concurrent_streams(MAX_CONCURRENT_STREAMS);
        let _ = builder
            .serve_connection_with_upgrades(TokioIo::new(tls), service)
            .await;
    }

    /// Klartext-HTTP in absoluter Form (`GET http://host/pfad`); nur mit `http://`-Regel.
    async fn forward_absolute(self: Arc<Self>, req: Request<Incoming>) -> Response<ProxyBody> {
        if req.uri().scheme_str() != Some("http") {
            return invalid("Nur http:// in absoluter Form; HTTPS über CONNECT");
        }
        let Some(authority) = req.uri().authority().cloned() else {
            return invalid("Host fehlt");
        };
        let Ok(host) = Host::parse(authority.host()) else {
            return invalid("ungültiger Host");
        };
        let port = authority.port_u16().unwrap_or(80);
        if let Host::Ip(ip) = &host
            && !destination_allowed(SocketAddr::new(*ip, port), &self.allow_private)
        {
            self.log_deny(
                req.method().as_str(),
                &host,
                port,
                "",
                "private_destination",
            );
            return private_destination(*ip);
        }
        self.check_and_forward(Scheme::Http, host, port, req).await
    }

    /// Normalisiert den Pfad, prüft die Regeln und leitet weiter.
    async fn check_and_forward(
        &self,
        scheme: Scheme,
        host: Host,
        port: u16,
        req: Request<Incoming>,
    ) -> Response<ProxyBody> {
        let method = req.method().as_str().to_owned();
        let path = match normalize_path(req.uri().path()) {
            Ok(p) => p,
            Err(e) => {
                self.log_deny(&method, &host, port, "", "invalid_path");
                return invalid(e.to_string());
            }
        };
        let query = req.uri().query().map(str::to_owned);
        // PRX-006: Platzhalter vor der Regelprüfung erkennen, damit auch ein Fehlgebrauch an
        // einen sonst gesperrten Host auditiert wird.
        let plan =
            match self
                .credentials
                .plan(scheme, &host, &path, query.as_deref(), req.headers())
            {
                Ok(plan) => plan,
                Err(refusal) => {
                    self.log_deny(&method, &host, port, &path, refusal.error);
                    refusal.audit(&host).await;
                    return problem(refusal.status, refusal.error, refusal.reason, None);
                }
            };
        let rreq = RulesRequest {
            method: &method,
            scheme,
            host: &host,
            port,
            path: &path,
        };
        match self.rules.decide(&rreq) {
            Decision::Allow { rule } => {
                let bindings = plan
                    .as_ref()
                    .map(|p| p.names().join(","))
                    .unwrap_or_default();
                tracing::debug!(
                    instance = %self.label,
                    method = %method,
                    host = %host,
                    port,
                    path = log_path(&path),
                    decision = "allow",
                    rule = %rule,
                    bindings = %bindings,
                    "Egress erlaubt"
                );
                self.forward(scheme, &host, port, &path, query, plan, req)
                    .await
            }
            Decision::Deny { reason, rule } => {
                let (text, hint) = match (reason, rule) {
                    (DenyReason::DeniedByRule, Some(rule)) => {
                        (format!("Abgelehnt durch Regel `{rule}`"), None)
                    }
                    (DenyReason::MethodNotAllowed, _) => (
                        format!("Methode {method} ist über den Proxy nie erlaubt"),
                        None,
                    ),
                    _ => (
                        "Keine Egress-Regel erlaubt diesen Request".to_owned(),
                        Some(RuleSet::rule_hint(&rreq)),
                    ),
                };
                self.deny_with_hint(&method, &host, port, &path, reason.as_str(), hint.clone());
                problem(StatusCode::FORBIDDEN, "egress_denied", text, hint)
            }
        }
    }

    /// Leitet einen geprüften Request weiter. Der Upstream bekommt genau den geprüften,
    /// normalisierten Pfad (plus Query).
    #[allow(clippy::too_many_arguments)]
    async fn forward(
        &self,
        scheme: Scheme,
        host: &Host,
        port: u16,
        path: &str,
        query: Option<String>,
        plan: Option<Plan>,
        mut req: Request<Incoming>,
    ) -> Response<ProxyBody> {
        let started = std::time::Instant::now();
        let upgrade = is_upgrade(&req);
        let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
        let (mut parts, body) = req.into_parts();
        // PRX-006: Werte erst jetzt (Request erlaubt) holen, auditieren und einsetzen.
        let (upstream_path, query, redactor) = match &plan {
            Some(plan) => {
                match plan
                    .apply(host, path, query.as_deref(), &mut parts.headers)
                    .await
                {
                    Ok((p, q, r)) => (p, q, Some(r)),
                    Err(refusal) => {
                        tracing::debug!(
                            instance = %self.label,
                            host = %host,
                            port,
                            path = log_path(path),
                            error = refusal.error,
                            "Credential nicht eingesetzt"
                        );
                        return problem(refusal.status, refusal.error, refusal.reason, None);
                    }
                }
            }
            None => (path.to_owned(), query, None),
        };
        let query = query.map(|q| format!("?{q}")).unwrap_or_default();
        let uri: Uri = match format!(
            "{}://{}:{port}{upstream_path}{query}",
            scheme.as_str(),
            host.to_uri_host()
        )
        .parse()
        {
            Ok(uri) => uri,
            Err(_) => return invalid("ungültige URI"),
        };
        let sent = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        parts.uri = uri;
        parts.version = Version::HTTP_11;
        strip_hop_by_hop(&mut parts.headers, upgrade);
        parts.headers.remove(HOST);
        let client = if upgrade { &self.h1 } else { &self.auto };
        let mut res = match client.request(Request::from_parts(parts, body)).await {
            Ok(res) => res,
            Err(e) => return self.upstream_error(&e),
        };
        tracing::debug!(
            instance = %self.label,
            host = %host,
            port,
            // Der geprüfte Pfad (mit Platzhaltern, nie mit eingesetzten Werten).
            path = log_path(path),
            status = res.status().as_u16(),
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "Egress beantwortet"
        );
        let switching = upgrade && res.status() == StatusCode::SWITCHING_PROTOCOLS;
        if switching && let Some(on_client) = client_upgrade {
            let on_upstream = hyper::upgrade::on(&mut res);
            spawn_until(self.stopped.clone(), async move {
                if let (Ok(a), Ok(b)) = tokio::join!(on_client, on_upstream) {
                    let (mut a, mut b) = (TokioIo::new(a), TokioIo::new(b));
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
            });
        }
        strip_hop_by_hop(res.headers_mut(), switching);
        res.headers_mut().remove(ALT_SVC);
        if let Some(traffic) = &self.traffic {
            traffic.record(&host.to_string(), 0, sent);
        }
        let traffic = (!switching).then(|| self.traffic.clone()).flatten();
        let host = host.to_string();
        let Some(r) = redactor.filter(|_| !switching) else {
            return res.map(|b| Counted::new(b, traffic, &host, 1).boxed());
        };
        // PRX-010: zurückgespiegelte Werte durch die Platzhalter ersetzen.
        let r = Arc::new(r);
        r.headers(res.headers_mut());
        let (mut parts, body) = res.into_parts();
        match r.treatment(&mut parts.headers) {
            Treatment::Pass => {
                Response::from_parts(parts, Counted::new(body, traffic, &host, 1).boxed())
            }
            Treatment::Stream => {
                let body = Counted::new(r.stream(body), traffic, &host, 1);
                Response::from_parts(parts, body.boxed())
            }
            Treatment::Buffer { gzip } => {
                buffered(parts, body, &r, gzip, Counted::new((), traffic, &host, 1)).await
            }
        }
    }

    fn upstream_error(&self, err: &hyper_util::client::legacy::Error) -> Response<ProxyBody> {
        let found = find_connect_error(err);
        tracing::debug!(instance = %self.label, "Upstream: {err} ({found:?})");
        match found {
            Some(e) => connect_problem(e),
            None => problem(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "Fehler in der Verbindung zum Upstream",
                None,
            ),
        }
    }
}

/// Komprimierte, textartige Antwort mit eingesetztem Credential (PRX-010): bis
/// [`REDACT_LIMIT`] puffern, dekodieren, redigieren und unkomprimiert ausliefern; größer oder
/// nicht dekodierbar → unverändert.
async fn buffered(
    mut parts: http::response::Parts,
    mut body: Incoming,
    r: &ResponseRedactor,
    gzip: bool,
    mut count: Counted<()>,
) -> Response<ProxyBody> {
    let mut buf = bytes::BytesMut::new();
    loop {
        match body.frame().await {
            None => break,
            Some(Err(e)) => {
                tracing::debug!("Upstream-Body: {e}");
                return problem(
                    StatusCode::BAD_GATEWAY,
                    "upstream_error",
                    "Fehler beim Lesen der Antwort",
                    None,
                );
            }
            Some(Ok(frame)) => {
                if let Some(d) = frame.data_ref() {
                    buf.extend_from_slice(d);
                    count.add(d.len());
                }
                if buf.len() > REDACT_LIMIT {
                    let body = Prefixed::new(buf.freeze(), body);
                    return Response::from_parts(parts, count.around(body).boxed());
                }
            }
        }
    }
    let out = match r.decoded(&buf, gzip) {
        Some(plain) => {
            parts.headers.remove(http::header::CONTENT_ENCODING);
            parts
                .headers
                .insert(http::header::CONTENT_LENGTH, HeaderValue::from(plain.len()));
            Bytes::from(plain)
        }
        None => buf.freeze(),
    };
    drop(count);
    Response::from_parts(
        parts,
        Full::new(out).map_err(|never| match never {}).boxed(),
    )
}

/// Ein TLS-terminierter Tunnel zu `host:port`; jeder Request (bzw. HTTP/2-Stream) wird
/// einzeln geprüft (PRX-004).
struct Tunnel {
    inst: Arc<Instance>,
    host: Host,
    port: u16,
}

impl Tunnel {
    async fn handle(&self, req: Request<Incoming>) -> Response<ProxyBody> {
        let method = req.method().clone();
        if method == Method::TRACE || method == Method::CONNECT {
            self.inst.log_deny(
                method.as_str(),
                &self.host,
                self.port,
                "",
                "method_not_allowed",
            );
            return problem(
                StatusCode::FORBIDDEN,
                "egress_denied",
                format!("Methode {method} ist im Tunnel nie erlaubt"),
                None,
            );
        }
        // `:authority` bzw. `Host` muss dem CONNECT-Ziel entsprechen, sonst könnte
        // Connection-Coalescing Regeln umgehen.
        let Ok(authorities) = request_authorities(&req) else {
            return invalid("Host bzw. :authority fehlt oder ist ungültig");
        };
        if authorities
            .iter()
            .any(|(h, p)| *h != self.host || p.unwrap_or(443) != self.port)
        {
            self.inst.log_deny(
                method.as_str(),
                &self.host,
                self.port,
                "",
                "authority_mismatch",
            );
            return problem(
                StatusCode::MISDIRECTED_REQUEST,
                "misdirected_request",
                format!(
                    "Host bzw. :authority entspricht nicht dem CONNECT-Ziel {}:{}",
                    self.host, self.port
                ),
                None,
            );
        }
        self.inst
            .check_and_forward(Scheme::Https, self.host.clone(), self.port, req)
            .await
    }
}
