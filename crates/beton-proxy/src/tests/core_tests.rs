//! Proxy-Kern und Instanzmodell (PRX-001) sowie Regelprüfung über das Netz (PRX-003).

use std::time::{Duration, Instant};

use super::support::*;

const PUBLIC: &str = "93.184.216.34";

#[tokio::test]
async fn prx_001_ac1_missing_or_foreign_token_gets_407() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["allowed.test"])).await;
    h.route("allowed.test", PUBLIC, 443, &up);
    let a = h
        .instance(&["* allowed.test", "* http://allowed.test"])
        .await;
    let b = h.instance(&["* allowed.test"]).await;

    let foreign = auth(&b);
    let wrong_user = basic("other", a.token().expose());
    let bad = [
        None,
        Some(foreign.as_str()),
        Some(wrong_user.as_str()),
        Some("Basic !!!kein-base64"),
        Some("Bearer abc"),
        Some(""),
    ];
    for auth in bad {
        let (_s, raw) = connect(&a, "allowed.test:443", auth).await;
        assert_eq!(raw.status, 407, "CONNECT mit {auth:?}: {raw:?}");
        assert!(
            raw.header("proxy-authenticate")
                .is_some_and(|v| v.starts_with("Basic")),
            "{raw:?}"
        );
        assert_eq!(raw.header("x-beton-egress").as_deref(), Some("denied"));
        assert_eq!(raw.json()["error"], "proxy_auth_required");
        let got = plain(&a, "GET", "http://allowed.test/", auth).await;
        assert_denied(&got, 407, "proxy_auth_required");
    }
    // Nichts wurde weitergeleitet.
    assert_eq!(up.count(), 0);
    assert!(h.dialer.dialed().is_empty());
    assert_eq!(
        h.resolver.queries.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    // Mit dem eigenen Token geht es.
    let mut s = h1(tunnel(&h, &a, "allowed.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "allowed.test", "/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(up.count(), 1);
}

#[tokio::test]
async fn prx_001_ac2_parallel_sessions_do_not_influence_each_other() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["api.test"])).await;
    h.route("api.test", PUBLIC, 443, &up);
    let a = h.instance(&["* api.test"]).await;
    let b = h.instance(&["GET api.test/public/**"]).await;
    let c = h.instance(&["GET other.test"]).await;
    assert_ne!(a.addr(), b.addr());
    assert_ne!(a.token().expose(), b.token().expose());

    let mut sa = h1(tunnel(&h, &a, "api.test", 443, &[b"http/1.1"]).await).await;
    let mut sb = h1(tunnel(&h, &b, "api.test", 443, &[b"http/1.1"]).await).await;
    for _ in 0..3 {
        let ra = collect(
            sa.send_request(origin("POST", "api.test", "/private"))
                .await
                .unwrap(),
        )
        .await;
        let rb = collect(
            sb.send_request(origin("POST", "api.test", "/private"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(ra.status, 200, "in A erlaubt: {}", ra.body);
        assert_denied(&rb, 403, "egress_denied");
    }
    let rb = collect(
        sb.send_request(origin("GET", "api.test", "/public/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(rb.status, 200);
    // C hat keine Regel für api.test: schon der CONNECT scheitert.
    let (_s, raw) = connect(&c, "api.test:443", Some(&auth(&c))).await;
    assert_eq!(raw.status, 403, "{raw:?}");
    assert_eq!(up.count(), 4);
}

#[tokio::test]
async fn prx_001_instances_have_separate_leaf_keys() {
    let h = Harness::new();
    let a = h.instance(&["* x.test"]).await;
    let b = h.instance(&["* x.test"]).await;
    let spki = |stream: &tokio_rustls::client::TlsStream<tokio::net::TcpStream>| {
        stream.get_ref().1.peer_certificates().unwrap()[0].to_vec()
    };
    let ta = tunnel(&h, &a, "x.test", 443, &[b"http/1.1"]).await;
    let tb = tunnel(&h, &b, "x.test", 443, &[b"http/1.1"]).await;
    assert_ne!(spki(&ta), spki(&tb));
}

#[tokio::test]
async fn prx_001_token_and_proxy_url() {
    let h = Harness::new();
    let a = h.instance(&[]).await;
    let token = a.token().expose().to_owned();
    assert_eq!(token.len(), 64, "256 bit hex");
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        *a.proxy_url(),
        format!("http://bt:{token}@127.0.0.1:{}", a.addr().port())
    );
    assert!(a.addr().ip().is_loopback());
    assert!(!format!("{a:?} {:?}", a.token()).contains(&token));
}

#[tokio::test]
async fn prx_001_dropped_instance_stops_listening() {
    let h = Harness::new();
    let a = h.instance(&[]).await;
    let addr = a.addr();
    drop(a);
    let deadline = Instant::now() + Duration::from_secs(5);
    while tokio::net::TcpStream::connect(addr).await.is_ok() {
        assert!(
            Instant::now() < deadline,
            "Instanz lauscht nach Drop weiter"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Lastprofil aus PRX-001 AC3: `conns` parallele HTTPS-Verbindungen durch eine Instanz, je
/// Verbindung ein Aufwärm-Request und `requests` gemessene Requests.
async fn load(conns: usize, requests: usize) -> (Vec<Duration>, Vec<Duration>) {
    load_rounds(conns, requests, 1).await.remove(0)
}

/// Wie [`load`], `rounds` Runden hintereinander gegen dieselbe Instanz; je Runde erst direkt,
/// dann durch den Proxy, damit beide Messungen dieselben Bedingungen des Hosts sehen.
async fn load_rounds(
    conns: usize,
    requests: usize,
    rounds: usize,
) -> Vec<(Vec<Duration>, Vec<Duration>)> {
    let h = std::sync::Arc::new(Harness::new());
    let up = Upstream::tls(h.pki.valid(&["load.test"])).await;
    h.route("load.test", PUBLIC, 443, &up);
    let instance = std::sync::Arc::new(h.instance(&["GET load.test/**"]).await);
    let mut out = Vec::with_capacity(rounds);
    for round in 1..=rounds {
        // Referenz: dieselbe Last direkt gegen den Upstream (ohne Proxy).
        let direct = {
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..conns {
                let (pki_ca, addr) = (h.pki.ca_der.clone(), up.addr);
                tasks.spawn(async move {
                    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
                    let tls = connector(&[pki_ca], &[b"http/1.1"])
                        .connect(
                            rustls::pki_types::ServerName::try_from("load.test").unwrap(),
                            tcp,
                        )
                        .await
                        .unwrap();
                    let mut s = h1(tls).await;
                    timed(&mut s, requests).await
                });
            }
            tasks.join_all().await.concat()
        };

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..conns {
            let (h, instance) = (h.clone(), instance.clone());
            tasks.spawn(async move {
                let mut s = h1(tunnel(&h, &instance, "load.test", 443, &[b"http/1.1"]).await).await;
                timed(&mut s, requests).await
            });
        }
        let proxied = tasks.join_all().await.concat();
        assert_eq!(up.count(), round * 2 * conns * (requests + 1));
        out.push((proxied, direct));
    }
    out
}

async fn timed(
    s: &mut hyper::client::conn::http1::SendRequest<Body>,
    requests: usize,
) -> Vec<Duration> {
    let warm = collect(
        s.send_request(origin("GET", "load.test", "/warm"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(warm.status, 200, "{}", warm.body);
    let mut out = Vec::with_capacity(requests);
    for i in 0..requests {
        let start = Instant::now();
        let got = collect(
            s.send_request(origin("GET", "load.test", &format!("/r/{i}")))
                .await
                .unwrap(),
        )
        .await;
        out.push(start.elapsed());
        assert_eq!(got.status, 200, "{}", got.body);
    }
    out
}

fn p99(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[(v.len() * 99 / 100).min(v.len() - 1)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prx_001_ac3_200_parallel_https_connections_without_errors() {
    let (proxied, _) = load(200, 3).await;
    assert_eq!(proxied.len(), 600);
}

/// PRX-001 AC3 (Latenz): p99-Zusatzlatenz < 20 ms. Läuft im Release-Build als eigener
/// CI-Schritt (`cargo test -p beton-proxy --release --lib prx_001_ac3 -- --ignored`).
///
/// Gewertet wird der Median aus fünf Runden (#221): Auf geteilten CI-Runnern hebt ein einzelner
/// Stillstand des Hosts das p99 einer Runde auf über 40 ms, während die übrigen Runden bei
/// wenigen Millisekunden liegen. Eine echte Verlangsamung des Proxys trifft jede Runde und
/// damit auch den Median; der Grenzwert bleibt 20 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Benchmark; im Release-Build ausführen"]
async fn prx_001_ac3_p99_added_latency_below_20ms() {
    const ROUNDS: usize = 5;
    let mut added: Vec<Duration> = Vec::with_capacity(ROUNDS);
    for (i, (proxied, direct)) in load_rounds(200, 20, ROUNDS).await.into_iter().enumerate() {
        let (p, d) = (p99(proxied), p99(direct));
        let a = p.saturating_sub(d);
        println!(
            "Runde {}: p99 mit Proxy {p:?}, direkt {d:?}, Zusatz {a:?}",
            i + 1
        );
        added.push(a);
    }
    added.sort();
    let median = added[ROUNDS / 2];
    println!("Median der p99-Zusatzlatenz über {ROUNDS} Runden: {median:?}");
    assert!(
        median < Duration::from_millis(20),
        "p99-Zusatzlatenz (Median über {ROUNDS} Runden) {median:?}, Runden {added:?}"
    );
}

#[tokio::test]
async fn prx_003_ac5_connect_without_rule_is_rejected_before_tls() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["unknown.test"])).await;
    h.route("unknown.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET allowed.test/**"]).await;
    let (mut stream, raw) = connect(&a, "unknown.test:443", Some(&auth(&a))).await;
    // Die Antwort ist Klartext-HTTP: Es gab keinen TLS-Handshake.
    assert_eq!(raw.status, 403, "{raw:?}");
    assert_eq!(raw.header("x-beton-egress").as_deref(), Some("denied"));
    assert_eq!(raw.json()["error"], "egress_denied");
    assert!(
        raw.json()["rule_hint"]
            .as_str()
            .is_some_and(|r| r.contains("unknown.test"))
    );
    // Ein anschließender TLS-Handshake auf derselben Verbindung scheitert.
    use tokio::io::AsyncWriteExt as _;
    let _ = stream.write_all(b"\x16\x03\x01\x00\x05hello").await;
    let tls = h
        .client_tls(&[b"http/1.1"])
        .connect(
            rustls::pki_types::ServerName::try_from("unknown.test").unwrap(),
            stream,
        )
        .await;
    assert!(tls.is_err());
    assert!(h.dialer.dialed().is_empty());
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_003_ac2_paths_are_checked_normalized_in_the_tunnel() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["api.github.test"])).await;
    h.route("api.github.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET api.github.test/repos/a/**"]).await;
    let mut s = h1(tunnel(&h, &a, "api.github.test", 443, &[b"http/1.1"]).await).await;
    let send = |p: &str| origin("GET", "api.github.test", p);
    let ok = collect(s.send_request(send("/repos/a/x?q=1")).await.unwrap()).await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    let dots = collect(s.send_request(send("/repos/a/../b/x")).await.unwrap()).await;
    assert_denied(&dots, 403, "egress_denied");
    let enc = collect(s.send_request(send("/repos/a%2F..%2Fb")).await.unwrap()).await;
    assert_denied(&enc, 400, "invalid_request");
    let post = collect(
        s.send_request(origin("POST", "api.github.test", "/repos/a/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&post, 403, "egress_denied");
    assert_eq!(post.json()["rule_hint"], "POST api.github.test/repos/a/x");
    // Der Upstream sieht genau den geprüften, normalisierten Pfad.
    let seen = up.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path_and_query, "/repos/a/x?q=1");
    assert!(seen[0].headers.get("proxy-authorization").is_none());
}

#[tokio::test]
async fn prx_003_trace_is_rejected_in_the_tunnel() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["api.test"])).await;
    h.route("api.test", PUBLIC, 443, &up);
    let a = h.instance(&["* api.test"]).await;
    let mut s = h1(tunnel(&h, &a, "api.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("TRACE", "api.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 403, "egress_denied");
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_003_ac4_plain_http_needs_an_http_rule() {
    let h = Harness::new();
    let up = Upstream::plain().await;
    h.route("plain.test", PUBLIC, 80, &up);
    let https_only = h.instance(&["* plain.test"]).await;
    let got = plain(
        &https_only,
        "GET",
        "http://plain.test/x",
        Some(&auth(&https_only)),
    )
    .await;
    assert_denied(&got, 403, "egress_denied");
    assert_eq!(got.json()["rule_hint"], "GET http://plain.test/x");
    assert_eq!(up.count(), 0);

    let with_http = h.instance(&["GET http://plain.test/**"]).await;
    let got = plain(
        &with_http,
        "GET",
        "http://plain.test/x",
        Some(&auth(&with_http)),
    )
    .await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(up.count(), 1);
    // Andere Schemata in absoluter Form werden nicht weitergeleitet.
    let got = plain(
        &with_http,
        "GET",
        "https://plain.test/x",
        Some(&auth(&with_http)),
    )
    .await;
    assert_denied(&got, 400, "invalid_request");
}
