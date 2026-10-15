//! End-to-End-Harness des Egress-Proxys für `http_request` und `http_connect` (QA-009).
//!
//! HTTP/1.1 parst im Proxy hyper; beton fuzzt deshalb nicht einen eigenen Parser, sondern den
//! ganzen Weg: Bytes vom Client → hyper (Request-Zeile, Header, CONNECT-Authority) → Prüfung
//! von `Proxy-Authorization`, CONNECT-Vorprüfung, Passthrough mit SNI-Prüfung, TLS-MITM,
//! Tunnel-Requests (`:authority`/`Host`), Pfad-Normalisierung, Credential-Ersetzung, Regeln,
//! Weiterleitung und Antwort-Redaction.
//!
//! Alles bleibt lokal (ADR-0033): Der Proxy läuft mit dem Test-Netz aus `beton-proxy`
//! (Feature `test-net`); Namen lösen nur auf die hier gestarteten Upstreams auf, alles andere
//! ist NXDOMAIN bzw. unerreichbar. Die Upstreams prüfen selbst, was bei ihnen ankommt, und
//! brechen den Prozess bei einer Verletzung ab (Panic → libFuzzer-Abbruch):
//!
//! - `api.test` (TLS und Klartext) bekommt nur Requests, die die Regeln erlauben, mit
//!   normalisiertem Pfad, ohne `Proxy-Authorization`, ohne Platzhalter und mit echtem Wert
//!   nur über HTTPS an den gebundenen Host;
//! - `vendor.test` (Passthrough) bekommt nur Bytes, die mit einem `ClientHello` mit SNI
//!   `vendor.test` beginnen;
//! - `denied.test` und `other.test` werden nie verbunden (keine Allow-Regel).
//!
//! Der Client prüft, dass die Antworten den echten Wert nie enthalten (PRX-010) und dass ohne
//! gültige `Proxy-Authorization` nur Fehler zurückkommen.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, HOST, UPGRADE};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::{
    BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use beton_proxy::credentials::find_placeholders;
use beton_proxy::fuzzing::{ClientHello, client_hello_sni};
use beton_proxy::{
    BetonCa, Binding, CredentialAudit, CredentialKind, CredentialSource, Credentials, EgressProxy,
    Host, InstanceSpec, Passthrough, Placeholder, ProxyInstance, RuleSet, Scheme, SecretBytes,
    normalize_path,
};

/// Platzhalter in Eingaben, den der Harness durch gültige `Proxy-Authorization` ersetzt.
pub const AUTH_MARKER: &[u8] = b"@@AUTH@@";
/// Platzhalter in Eingaben für die Credential-Platzhalter der Bindings `api` und `other`.
pub const API_MARKER: &[u8] = b"{A}";
pub const OTHER_MARKER: &[u8] = b"{O}";

/// Echte Werte der Bindings (Fixtures). Nur Großbuchstaben und Ziffern: Sie bleiben in der
/// URL unkodiert und bilden mit benachbarten Bytes nie einen Platzhalter.
pub const SECRET_API: &str = "FIXTUREAPI0SECRET0VALUE";
pub const SECRET_OTHER: &str = "FIXTUREOTHER0SECRET0VALUE";

/// Egress-Regeln der Instanz.
pub const RULES: &[&str] = &[
    "* http://api.test/**",
    "!* http://api.test/private/**",
    "* api.test/**",
    "!* api.test/private/**",
    "GET,HEAD api.test:8443/**",
    "GET *.wild.test/**",
    "!* denied.test/**",
];
/// Passthrough-Ziele der Instanz (PRX-005).
pub const PASSTHROUGH: &[&str] = &["vendor.test"];

/// Obergrenze für eine Antwort; länger ist ein Hänger.
const LIMIT: Duration = Duration::from_secs(5);

fn placeholder(c: char) -> String {
    format!("bt_cred_{}", c.to_string().repeat(32))
}

pub struct Proxy {
    rt: Runtime,
    addr: SocketAddr,
    /// Wert von `Proxy-Authorization`.
    auth: String,
    client_tls: Arc<rustls::ClientConfig>,
    pub rules: RuleSet,
    pub passthrough: Passthrough,
    _proxy: EgressProxy,
    _instance: ProxyInstance,
}

static PROXY: OnceLock<Proxy> = OnceLock::new();

/// Der gemeinsame Proxy aller Läufe eines Prozesses (einmal gestartet).
pub fn proxy() -> &'static Proxy {
    PROXY.get_or_init(start)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

struct Fixed;

impl CredentialSource for Fixed {
    fn value<'a>(
        &'a self,
        binding: &'a str,
    ) -> beton_proxy::credentials::BoxFuture<'a, Result<SecretBytes, String>> {
        let v = match binding {
            "api" => SECRET_API,
            _ => SECRET_OTHER,
        };
        Box::pin(async move { Ok(SecretBytes::new(v.as_bytes().to_vec())) })
    }
}

impl CredentialAudit for Fixed {
    fn used<'a>(
        &'a self,
        _binding: &'a str,
        _host: &'a str,
    ) -> beton_proxy::credentials::BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn misuse<'a>(
        &'a self,
        _binding: Option<&'a str>,
        _host: &'a str,
        _reason: &'a str,
    ) -> beton_proxy::credentials::BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Bindings: `api` (implizit, nur `api.test`) und `other` (nur `other.test`, keine Regel).
pub fn credentials() -> Credentials {
    let binding = |name: &str, host: &str, c: char, implicit: bool| Binding {
        name: name.into(),
        kind: CredentialKind::Bearer,
        hosts: vec![host.into()],
        username: None,
        placeholder: Placeholder::parse(&placeholder(c)).expect("gültiger Platzhalter"),
        implicit,
    };
    Credentials::new(
        vec![
            binding("api", "api.test", 'a', true),
            binding("other", "other.test", 'o', false),
        ],
        Arc::new(Fixed),
        Arc::new(Fixed),
    )
    .expect("gültige Bindings")
}

/// Test-PKI der Upstreams: CA und Server-Konfiguration für `api.test` und `*.wild.test`.
fn upstream_pki() -> (CertificateDer<'static>, Arc<rustls::ServerConfig>) {
    let now = OffsetDateTime::now_utc();
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("CA-Schlüssel");
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.not_before = now - time::Duration::days(1);
    ca_params.not_after = now + time::Duration::days(30);
    let ca_der = ca_params.self_signed(&ca_key).expect("CA").der().clone();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("Leaf-Schlüssel");
    let mut params = CertificateParams::new(vec!["api.test".to_owned(), "*.wild.test".to_owned()])
        .expect("Leaf-Parameter");
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(30);
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let cert = params.signed_by(&key, &issuer).expect("Leaf");
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("TLS-Versionen")
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone(), ca_der.clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .expect("Server-Konfiguration");
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    (ca_der, Arc::new(config))
}

fn start() -> Proxy {
    // libFuzzer stellt `main`; die Rust-Laufzeit setzt SIGPIPE deshalb nicht auf ignorieren.
    // Ein Schreibversuch auf eine vom Proxy geschlossene Verbindung darf den Prozess nicht
    // beenden.
    // SAFETY: Setzt nur die Signal-Disposition, bevor weitere Threads laufen.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Tokio-Laufzeit");
    let rules = RuleSet::compile(RULES).expect("gültige Regeln");
    let passthrough = Passthrough::parse(PASSTHROUGH).expect("gültiges Passthrough");
    let (proxy, instance, auth, ca_der) = rt.block_on(async {
        let (upstream_ca, upstream_tls) = upstream_pki();
        let api = listen().await;
        let vendor = listen().await;
        let tripwire = listen().await;
        let hosts = [
            ("api.test", api.local_addr().expect("Adresse")),
            ("a.wild.test", api.local_addr().expect("Adresse")),
            ("vendor.test", vendor.local_addr().expect("Adresse")),
            ("denied.test", tripwire.local_addr().expect("Adresse")),
            ("other.test", tripwire.local_addr().expect("Adresse")),
        ];
        tokio::spawn(serve_api(api, upstream_tls, rules.clone()));
        tokio::spawn(serve_vendor(vendor));
        tokio::spawn(serve_tripwire(tripwire));
        let ca = BetonCa::generate("fuzz", OffsetDateTime::now_utc()).expect("beton-CA");
        let ca_der = CertificateDer::from(ca.cert_der().to_vec());
        let proxy = EgressProxy::with_test_hosts(ca, vec![upstream_ca], &hosts);
        let instance = proxy
            .start(InstanceSpec {
                label: "fuzz".into(),
                rules: rules.clone(),
                passthrough: passthrough.clone(),
                credentials: credentials(),
                ..InstanceSpec::default()
            })
            .await
            .expect("Proxy-Instanz");
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("bt:{}", instance.token().expose()))
        );
        (proxy, instance, auth, ca_der)
    });
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca_der).expect("beton-CA als Vertrauensanker");
    let mut client_tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("TLS-Versionen")
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Proxy {
        rt,
        addr: instance.addr(),
        auth,
        client_tls: Arc::new(client_tls),
        rules,
        passthrough,
        _proxy: proxy,
        _instance: instance,
    }
}

async fn listen() -> TcpListener {
    TcpListener::bind("127.0.0.1:0")
        .await
        .expect("lokaler Upstream")
}

/// Enthält die Eingabe (auch prozent-dekodiert) einen echten Wert? Dann kann der Client ihn
/// selbst geschickt haben (libFuzzer lernt Vergleichswerte), und die Prüfungen auf
/// durchgesickerte Werte sagen nichts.
pub fn mentions_secret(data: &[u8]) -> bool {
    let decoded = percent_decode(data);
    [SECRET_API, SECRET_OTHER].iter().any(|s| {
        let s = s.as_bytes();
        crate::contains(data, s) || crate::contains(&decoded, s)
    })
}

fn percent_decode(data: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'%'
            && let (Some(h), Some(l)) = (
                data.get(i + 1).copied().and_then(hex),
                data.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// Ersetzt die Marker der Eingabe (`@@AUTH@@`, `{A}`, `{O}`).
pub fn expand(p: &Proxy, data: &[u8]) -> Vec<u8> {
    let data = crate::replace(data, AUTH_MARKER, p.auth.as_bytes());
    let data = crate::replace(&data, API_MARKER, placeholder('a').as_bytes());
    crate::replace(&data, OTHER_MARKER, placeholder('o').as_bytes())
}

/// Liest bis EOF, Fehler oder Zeitlimit; ein Zeitlimit ist ein Hänger.
async fn read_all<R: AsyncRead + Unpin>(r: &mut R, out: &mut Vec<u8>) {
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
        }
    }
}

fn within<T>(p: &Proxy, what: &str, fut: impl Future<Output = T>) -> T {
    p.rt.block_on(async {
        tokio::time::timeout(LIMIT, fut)
            .await
            .unwrap_or_else(|_| panic!("Hänger: {what} nicht innerhalb von {LIMIT:?} beendet"))
    })
}

/// Verbindung zum Proxy. Ohne `TIME_WAIT` beim Schließen (`SO_LINGER` 0, erst nach dem
/// Lesen bis EOF relevant): Bei Tausenden Läufen pro Sekunde gingen sonst die lokalen Ports
/// aus. Kurzzeitig fehlende Ports überbrückt ein erneuter Versuch.
async fn dial(addr: SocketAddr) -> TcpStream {
    for _ in 0..200 {
        if let Ok(tcp) = TcpStream::connect(addr).await {
            let _ = tcp.set_nodelay(true);
            let _ = tcp.set_zero_linger();
            return tcp;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Proxy nicht erreichbar");
}

/// Schickt `bytes` roh an den Proxy, schließt die Schreibseite und liefert die Antwort.
pub fn raw(p: &Proxy, bytes: &[u8]) -> Vec<u8> {
    within(p, "Klartext-Verbindung", async {
        let mut out = Vec::new();
        let mut tcp = dial(p.addr).await;
        if tcp.write_all(bytes).await.is_ok() {
            let _ = tcp.shutdown().await;
        }
        read_all(&mut tcp, &mut out).await;
        out
    })
}

/// Antwort auf ein CONNECT und, bei `200`, was danach im Tunnel zurückkam.
pub struct Tunnel {
    pub status: Option<u16>,
    pub head: Vec<u8>,
    pub inner: Vec<u8>,
}

/// CONNECT zu `authority` (mit gültiger Proxy-Authorization). Bei `200` schickt der Harness
/// `payload` entweder roh (`tls == false`, z. B. ein `ClientHello` für Passthrough) oder als
/// Klartext in einer TLS-Verbindung, die er selbst gegen die beton-CA aufbaut (`tls == true`,
/// SNI `sni`): So erreicht die Eingabe die HTTP-Verarbeitung im MITM-Tunnel. `strict`: Ein
/// gescheiterter Handshake ist ein Fehler (für Ziele, deren Leaf sicher gültig ist).
pub fn connect(p: &Proxy, authority: &[u8], payload: &[u8], tls: Option<(&str, bool)>) -> Tunnel {
    within(p, "CONNECT-Tunnel", async {
        let mut tcp = dial(p.addr).await;
        let mut req = b"CONNECT ".to_vec();
        req.extend_from_slice(authority);
        req.extend_from_slice(b" HTTP/1.1\r\nHost: ");
        req.extend_from_slice(authority);
        req.extend_from_slice(b"\r\nProxy-Authorization: ");
        req.extend_from_slice(p.auth.as_bytes());
        req.extend_from_slice(b"\r\n\r\n");
        let mut head = Vec::new();
        if tcp.write_all(&req).await.is_err() {
            let _ = tcp.shutdown().await;
            read_all(&mut tcp, &mut head).await;
            return Tunnel {
                status: status(&head),
                head,
                inner: Vec::new(),
            };
        }
        // Kopf der CONNECT-Antwort byteweise lesen, damit nichts vom Tunnel verloren geht.
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match tcp.read(&mut b).await {
                Ok(1) => head.push(b[0]),
                _ => break,
            }
        }
        let status = status(&head);
        let mut inner = Vec::new();
        if status != Some(200) || !head.ends_with(b"\r\n\r\n") {
            // Abgelehnt: Die Verbindung bleibt keep-alive offen, bis der Client schließt.
            let _ = tcp.shutdown().await;
            read_all(&mut tcp, &mut inner).await;
            return Tunnel {
                status,
                head,
                inner,
            };
        }
        match tls {
            None => {
                if tcp.write_all(payload).await.is_ok() {
                    let _ = tcp.shutdown().await;
                }
                read_all(&mut tcp, &mut inner).await;
            }
            Some((sni, strict)) => {
                let Ok(name) = ServerName::try_from(sni.to_owned()) else {
                    return Tunnel {
                        status,
                        head,
                        inner,
                    };
                };
                let connector = TlsConnector::from(p.client_tls.clone());
                let mut stream = match connector.connect(name, tcp).await {
                    Ok(s) => s,
                    Err(e) if strict => panic!("TLS mit dem Proxy (beton-CA) scheitert: {e}"),
                    Err(_) => {
                        return Tunnel {
                            status,
                            head,
                            inner,
                        };
                    }
                };
                if stream.write_all(payload).await.is_ok() {
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                }
                read_all(&mut stream, &mut inner).await;
            }
        }
        Tunnel {
            status,
            head,
            inner,
        }
    })
}

/// Statuscodes aller Statuszeilen `HTTP/1.x NNN` am Zeilenanfang.
pub fn statuses(bytes: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, w) in bytes.windows(12).enumerate() {
        let line_start = i == 0 || bytes[i - 1] == b'\n';
        if line_start
            && (w.starts_with(b"HTTP/1.1 ") || w.starts_with(b"HTTP/1.0 "))
            && let Ok(s) = std::str::from_utf8(&w[9..12])
            && let Ok(n) = s.parse::<u16>()
        {
            out.push(n);
        }
    }
    out
}

fn status(head: &[u8]) -> Option<u16> {
    statuses(head).first().copied()
}

/// Der Client sieht nie einen echten Wert (PRX-006, PRX-010).
pub fn assert_no_secret(bytes: &[u8], context: &str) {
    for s in [SECRET_API, SECRET_OTHER] {
        if let Some(line) = bytes
            .split(|b| *b == b'\n')
            .find(|l| crate::contains(l, s.as_bytes()))
        {
            // Die Werte sind Fixtures; die Zeile zeigt, wo der Wert durchkam.
            panic!(
                "{context}: echter Credential-Wert in der Antwort an den Client: {:?}",
                String::from_utf8_lossy(line)
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Upstreams
// ---------------------------------------------------------------------------------------

async fn serve_tripwire(listener: TcpListener) {
    if let Ok((_, peer)) = listener.accept().await {
        panic!("Verbindung zu einem Host ohne Allow-Regel ({peer})");
    }
}

/// Passthrough-Upstream: Was ankommt, muss mit einem `ClientHello` für `vendor.test`
/// beginnen (PRX-005); sonst hätte der Proxy vor dem ersten Byte trennen müssen.
async fn serve_vendor(listener: TcpListener) {
    loop {
        let Ok((mut tcp, _)) = listener.accept().await else {
            return;
        };
        // Schließen per RST: kein `TIME_WAIT` für die Ports des Proxys.
        let _ = tcp.set_zero_linger();
        tokio::spawn(async move {
            let mut data = Vec::new();
            let _ = tokio::time::timeout(LIMIT, read_all(&mut tcp, &mut data)).await;
            if !data.is_empty() {
                let hello = client_hello_sni(&data);
                assert_eq!(
                    hello,
                    ClientHello::Sni(Some("vendor.test".into())),
                    "Passthrough hat Bytes ohne passendes ClientHello weitergereicht"
                );
            }
        });
    }
}

async fn serve_api(listener: TcpListener, tls: Arc<rustls::ServerConfig>, rules: RuleSet) {
    let rules = Arc::new(rules);
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let _ = tcp.set_nodelay(true);
        let (tls, rules) = (tls.clone(), rules.clone());
        tokio::spawn(async move {
            let mut first = [0u8; 1];
            let is_tls = matches!(tcp.peek(&mut first).await, Ok(1) if first[0] == 0x16);
            let scheme = if is_tls { Scheme::Https } else { Scheme::Http };
            let service = service_fn(move |req| {
                let rules = rules.clone();
                async move { Ok::<_, std::convert::Infallible>(api(req, scheme, &rules).await) }
            });
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            if is_tls {
                let Ok(stream) = TlsAcceptor::from(tls).accept(tcp).await else {
                    return;
                };
                let _ = builder
                    .serve_connection_with_upgrades(TokioIo::new(stream), service)
                    .await;
            } else {
                let _ = builder
                    .serve_connection_with_upgrades(TokioIo::new(tcp), service)
                    .await;
            }
        });
    }
}

/// Prüft, was beim Upstream ankommt, und spiegelt Header, Pfad und Body zurück (wie ein
/// Debug-Endpunkt; PRX-010 muss den echten Wert in der Antwort ersetzen).
async fn api(req: Request<Incoming>, scheme: Scheme, rules: &RuleSet) -> Response<Full<Bytes>> {
    for name in ["proxy-authorization", "proxy-connection", "keep-alive"] {
        assert!(
            !req.headers().contains_key(name),
            "Hop-by-Hop-Header `{name}` beim Upstream"
        );
    }
    let path = req.uri().path().to_owned();
    assert_eq!(
        normalize_path(&path).as_deref(),
        Ok(path.as_str()),
        "Upstream bekommt einen nicht normalisierten Pfad"
    );
    let authority = req
        .uri()
        .authority()
        .map(|a| a.as_str().to_owned())
        .or_else(|| {
            req.headers()
                .get(HOST)
                .and_then(|h| h.to_str().ok())
                .map(str::to_owned)
        })
        .expect("Upstream-Request ohne Host");
    let authority: http::uri::Authority = authority.parse().expect("gültige Authority");
    let host = Host::parse(authority.host()).expect("gültiger Host beim Upstream");
    let port = authority.port_u16().unwrap_or(scheme.default_port());
    let decision = rules.decide(&beton_proxy::Request {
        method: req.method().as_str(),
        scheme,
        host: &host,
        port,
        path: &path,
    });
    assert!(
        decision.is_allowed(),
        "Upstream bekommt einen Request, den die Regeln ablehnen: {} {scheme:?} {host}:{port}{path} ({decision:?})",
        req.method()
    );
    // URL und Header-Werte: Dort ersetzt der Proxy Platzhalter (Header-Namen nicht).
    let mut values = Vec::new();
    values.extend_from_slice(req.uri().to_string().as_bytes());
    for value in req.headers().values() {
        values.push(b'\n');
        values.extend_from_slice(value.as_bytes());
    }
    assert!(
        find_placeholders(&values).is_empty(),
        "Platzhalter in URL oder Header-Wert beim Upstream"
    );
    let mut seen = Vec::new();
    seen.extend_from_slice(req.uri().to_string().as_bytes());
    for (name, value) in req.headers() {
        seen.extend_from_slice(name.as_str().as_bytes());
        seen.extend_from_slice(b": ");
        seen.extend_from_slice(value.as_bytes());
        seen.push(b'\n');
    }
    assert!(
        !crate::contains(&seen, SECRET_OTHER.as_bytes()),
        "Wert von `other` an {host}"
    );
    if crate::contains(&seen, SECRET_API.as_bytes()) {
        assert!(
            scheme == Scheme::Https && host == Host::Domain("api.test".into()),
            "Wert von `api` an {scheme:?} {host}"
        );
    }
    let echo_auth = req.headers().get(AUTHORIZATION).cloned();
    let upgrade = req.headers().get(UPGRADE).cloned();
    if let Some(proto) = upgrade {
        // WebSocket-artiges Upgrade: 101, danach Echo der rohen Bytes.
        let on = hyper::upgrade::on(req);
        tokio::spawn(async move {
            if let Ok(up) = on.await {
                let mut io = TokioIo::new(up);
                let mut buf = [0u8; 4096];
                while let Ok(n) = io.read(&mut buf).await {
                    if n == 0 || io.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        });
        let mut res = Response::new(Full::new(Bytes::new()));
        *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        res.headers_mut().insert(UPGRADE, proto);
        res.headers_mut()
            .insert(CONNECTION, http::HeaderValue::from_static("upgrade"));
        if let Some(a) = echo_auth {
            res.headers_mut().insert("x-echo-authorization", a);
        }
        return res;
    }
    let body = Limited::new(req.into_body(), 64 * 1024)
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    seen.extend_from_slice(b"\n");
    seen.extend_from_slice(&body);
    let mut res = Response::new(Full::new(Bytes::from(seen)));
    res.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    if let Some(a) = echo_auth {
        res.headers_mut().insert("x-echo-authorization", a);
    }
    res
}
