//! Passthrough für Vendor-Hosts (PRX-005): CONNECT ohne TLS-Terminierung, Prüfung von Host,
//! Port, Private-IP-Sperre und SNI. Alle Upstreams sind lokal (ADR-0033); die Vendor-Namen
//! löst nur der Test-Resolver auf.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::support::*;
use crate::Passthrough;

const PUBLIC: &str = "93.184.216.34";
const SECRET: &str = "FIXTURE-SUBSCRIPTION-TOKEN-4711";

/// Roh-Upstream: zählt Verbindungen und empfangene Bytes, antwortet nie.
struct Sink {
    addr: SocketAddr,
    connections: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn sink() -> Sink {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    let (c, b) = (connections.clone(), bytes.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            c.fetch_add(1, Ordering::SeqCst);
            let b = b.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = tcp.read(&mut buf).await {
                    if n == 0 {
                        return;
                    }
                    b.fetch_add(n, Ordering::SeqCst);
                }
            });
        }
    });
    Sink {
        addr,
        connections,
        bytes,
        task,
    }
}

fn route_sink(h: &Harness, name: &str, sink: &Sink) {
    let ip = ip(PUBLIC);
    h.resolver.set(name, vec![vec![ip]]);
    h.dialer.map(SocketAddr::new(ip, 443), sink.addr);
}

/// Mitschnitt aller Logs (bis TRACE) dieses Threads.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn prx_005_ac1_passthrough_does_not_terminate_tls_and_logs_no_headers() {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["api.anthropic.test"])).await;
    h.route("api.anthropic.test", PUBLIC, 443, &up);
    // Stufe 1: keine Regel, nur der Vendor-Host als Passthrough.
    let a = h.instance_passthrough(&[], &["api.anthropic.test"]).await;
    let (stream, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    // Der Client vertraut nur der CA des Upstreams: Gelingt der Handshake, hat der Proxy TLS
    // nicht terminiert (er besitzt keinen Schlüssel dieser CA).
    let tls = connector(std::slice::from_ref(&h.pki.ca_der), &[b"http/1.1"])
        .connect(ServerName::try_from("api.anthropic.test").unwrap(), stream)
        .await
        .expect("TLS-Handshake mit dem echten Upstream-Zertifikat");
    let chain = tls.get_ref().1.peer_certificates().unwrap().to_vec();
    assert_eq!(chain.last().unwrap().as_ref(), h.pki.ca_der.as_ref());
    assert_ne!(chain.last().unwrap().as_ref(), h.beton_ca.as_ref());
    let mut s = h1(tls).await;
    let mut r = origin("POST", "api.anthropic.test", "/v1/messages?secret-query=1");
    r.headers_mut()
        .insert("authorization", format!("Bearer {SECRET}").parse().unwrap());
    let got = collect(s.send_request(r).await.unwrap()).await;
    assert_eq!(got.status, 200, "{}", got.body);
    // Der Upstream hat den Header unverändert bekommen …
    let seen = up.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].headers.get("authorization").unwrap(),
        &format!("Bearer {SECRET}")
    );
    // … und der „Debug-Dump“ des Proxys (alle Logs bis TRACE) zeigt keine entschlüsselten
    // Header, keinen Pfad und keine Query.
    let text = logs.text();
    assert!(text.contains("passthrough"), "{text}");
    for leak in [
        SECRET,
        "authorization",
        "Authorization",
        "/v1/messages",
        "secret-query",
    ] {
        assert!(!text.contains(leak), "`{leak}` im Log: {text}");
    }
}

#[tokio::test]
async fn prx_005_ac2_client_hello_with_foreign_sni_is_disconnected() {
    let h = Harness::new();
    let up = sink().await;
    route_sink(&h, "api.anthropic.test", &up);
    let a = h.instance_passthrough(&[], &["api.anthropic.test"]).await;
    for sni in ["evil.test", "api.anthropic.test.evil.test"] {
        let (stream, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
        assert_eq!(raw.status, 200, "{raw:?}");
        let r = connector(std::slice::from_ref(&h.pki.ca_der), &[b"http/1.1"])
            .connect(ServerName::try_from(sni).unwrap(), stream)
            .await;
        assert!(r.is_err(), "SNI {sni} wurde durchgereicht");
    }
    // Ohne SNI (IP als Servername) ebenso.
    let (stream, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    let r = connector(std::slice::from_ref(&h.pki.ca_der), &[b"http/1.1"])
        .connect(ServerName::try_from("93.184.216.34").unwrap(), stream)
        .await;
    assert!(r.is_err(), "ClientHello ohne SNI wurde durchgereicht");
    // Kein Byte des ClientHello hat den Upstream erreicht.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(up.bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prx_005_ac2_garbage_instead_of_client_hello_is_disconnected() {
    let h = Harness::new();
    let up = sink().await;
    route_sink(&h, "api.anthropic.test", &up);
    let a = h.instance_passthrough(&[], &["api.anthropic.test"]).await;
    let (mut stream, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: api.anthropic.test\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("Proxy trennt die Verbindung")
        .unwrap_or(0);
    assert_eq!(n, 0);
    assert_eq!(up.bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prx_005_ac3_tools_instance_without_passthrough_rejects_vendor_host() {
    let h = Harness::new();
    let up = sink().await;
    route_sink(&h, "api.anthropic.test", &up);
    // Stufe 2: eigene Regeln, kein Passthrough.
    let a = h.instance(&["GET registry.npmjs.test/**"]).await;
    let (_, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 403, "{raw:?}");
    assert_eq!(raw.json()["error"], "egress_denied");
    assert_eq!(up.connections.load(Ordering::SeqCst), 0);
    assert!(h.dialer.dialed().is_empty());
}

#[tokio::test]
async fn prx_005_passthrough_keeps_the_private_ip_block() {
    let h = Harness::new();
    // Der Vendor-Name zeigt (Rebinding) auf eine private Adresse.
    h.resolver
        .set("api.anthropic.test", vec![vec![ip("127.0.0.1")]]);
    let a = h.instance_passthrough(&[], &["api.anthropic.test"]).await;
    let (_, raw) = connect(&a, "api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 403, "{raw:?}");
    assert_eq!(raw.json()["error"], "private_destination");
    assert!(h.dialer.dialed().is_empty());
}

#[tokio::test]
async fn prx_005_passthrough_needs_proxy_auth_and_the_exact_port() {
    let h = Harness::new();
    let up = sink().await;
    route_sink(&h, "api.anthropic.test", &up);
    let a = h.instance_passthrough(&[], &["api.anthropic.test"]).await;
    let (_, raw) = connect(&a, "api.anthropic.test:443", None).await;
    assert_eq!(raw.status, 407, "{raw:?}");
    let (_, raw) = connect(&a, "api.anthropic.test:8443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 403, "{raw:?}");
    // Subdomains sind nicht mitgemeint.
    let (_, raw) = connect(&a, "x.api.anthropic.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 403, "{raw:?}");
    assert_eq!(up.connections.load(Ordering::SeqCst), 0);
}

#[test]
fn prx_005_passthrough_entries_are_hostnames_with_optional_port() {
    let p = Passthrough::parse(&["API.Anthropic.com.", "chatgpt.com:8443"]).unwrap();
    assert_eq!(
        p.entries(),
        vec!["api.anthropic.com:443", "chatgpt.com:8443"]
    );
    for bad in [
        "127.0.0.1",
        "[::1]",
        "*.anthropic.com",
        "",
        "a.com:0",
        "a.com:x",
        "a.com/x",
    ] {
        let e = Passthrough::parse(&[bad]);
        assert!(e.is_err(), "`{bad}` wurde akzeptiert");
    }
}
