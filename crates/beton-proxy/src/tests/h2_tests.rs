//! HTTP/2, WebSocket und Protokollgrenzen (PRX-004).

use futures_util::{SinkExt as _, StreamExt as _};
use http::Version;
use tokio_tungstenite::tungstenite::Message;

use super::support::*;

const PUBLIC: &str = "93.184.216.34";

#[tokio::test]
async fn prx_004_ac1_each_h2_stream_is_checked_separately() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["api.github.test"])).await;
    h.route("api.github.test", PUBLIC, 443, &up);
    let a = h
        .instance(&["* api.github.test/**", "!DELETE api.github.test/**"])
        .await;
    let tls = tunnel(&h, &a, "api.github.test", 443, &[b"h2", b"http/1.1"]).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let mut s = h2(tls).await;

    let ok = collect(
        s.send_request(get("https://api.github.test/repos/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(ok.version, Version::HTTP_2);
    // Upstream spricht, was er kann – hier HTTP/2.
    assert!(ok.body.ends_with("HTTP/2.0"), "{}", ok.body);

    let denied = collect(
        s.send_request(req("DELETE", "https://api.github.test/repos/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&denied, 403, "egress_denied");

    // Dieselbe Verbindung lebt weiter, auch für parallele Streams.
    let mut tasks = Vec::new();
    for i in 0..10 {
        let mut s = s.clone();
        tasks.push(tokio::spawn(async move {
            collect(
                s.send_request(get(&format!("https://api.github.test/p/{i}")))
                    .await
                    .unwrap(),
            )
            .await
            .status
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    assert_eq!(up.count(), 11);
}

#[tokio::test]
async fn prx_004_ac2_foreign_authority_on_h2_gets_421() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["allowed.test", "other.test"])).await;
    h.route("allowed.test", PUBLIC, 443, &up);
    h.route("other.test", "93.184.216.35", 443, &up);
    let a = h.instance(&["* allowed.test", "* other.test"]).await;
    let mut s = h2(tunnel(&h, &a, "allowed.test", 443, &[b"h2"]).await).await;
    let got = collect(s.send_request(get("https://other.test/x")).await.unwrap()).await;
    assert_denied(&got, 421, "misdirected_request");
    let got = collect(
        s.send_request(get("https://allowed.test:8443/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 421, "misdirected_request");
    // Die Verbindung bleibt nutzbar.
    let got = collect(s.send_request(get("https://allowed.test/x")).await.unwrap()).await;
    assert_eq!(got.status, 200);
    assert_eq!(up.count(), 1);
}

#[tokio::test]
async fn prx_004_foreign_host_header_on_h1_gets_421() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["allowed.test"])).await;
    h.route("allowed.test", PUBLIC, 443, &up);
    let a = h.instance(&["* allowed.test", "* other.test"]).await;
    let mut s = h1(tunnel(&h, &a, "allowed.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "other.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 421, "misdirected_request");
    let got = collect(
        s.send_request(origin("GET", "ALLOWED.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(got.status, 200);
    assert_eq!(up.count(), 1);
}

async fn websocket(
    h: &Harness,
    a: &crate::proxy::ProxyInstance,
    host: &str,
    path: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let tls = tunnel(h, a, host, 443, &[b"http/1.1"]).await;
    tokio_tungstenite::client_async(format!("wss://{host}{path}"), tls)
        .await
        .map(|(ws, _)| ws)
}

#[tokio::test]
async fn prx_004_ac3_websocket_to_allowed_host_echoes() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["ws.test"])).await;
    h.route("ws.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET ws.test/ws/allowed/**"]).await;
    let mut ws = websocket(&h, &a, "ws.test", "/ws/allowed/echo")
        .await
        .unwrap();
    for text in ["hallo", "beton"] {
        ws.send(Message::text(text)).await.unwrap();
        let echo = ws.next().await.unwrap().unwrap();
        assert_eq!(echo.into_text().unwrap().as_str(), text);
    }
    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn prx_004_ac3_websocket_upgrade_without_rule_gets_403() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["ws.test"])).await;
    h.route("ws.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET ws.test/ws/allowed/**"]).await;
    let err = websocket(&h, &a, "ws.test", "/ws/forbidden")
        .await
        .unwrap_err();
    match err {
        tokio_tungstenite::tungstenite::Error::Http(res) => {
            assert_eq!(res.status(), 403);
            assert_eq!(res.headers()["x-beton-egress"], "denied");
        }
        other => panic!("403 erwartet, bekam {other:?}"),
    }
    // Ein Host ganz ohne Regel scheitert schon am CONNECT.
    let (_s, raw) = connect(&a, "nows.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 403);
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_004_ac4_alt_svc_is_removed() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["alt.test"])).await;
    h.route("alt.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET alt.test", "GET http://alt.test"]).await;
    let mut s2 = h2(tunnel(&h, &a, "alt.test", 443, &[b"h2"]).await).await;
    let got = collect(s2.send_request(get("https://alt.test/")).await.unwrap()).await;
    assert_eq!(got.status, 200);
    assert!(got.headers.get("alt-svc").is_none(), "{:?}", got.headers);
    let mut s1 = h1(tunnel(&h, &a, "alt.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s1.send_request(origin("GET", "alt.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(got.status, 200);
    assert!(got.headers.get("alt-svc").is_none(), "{:?}", got.headers);
    // Der Test-Upstream sendet den Header wirklich.
    assert!(up.count() >= 2);
    let plain_up = Upstream::plain().await;
    h.route("alt.test", PUBLIC, 80, &plain_up);
    let got = plain(&a, "GET", "http://alt.test/", Some(&auth(&a))).await;
    assert_eq!(got.status, 200);
    assert!(got.headers.get("alt-svc").is_none());
}

#[tokio::test]
async fn prx_004_grpc_trailers_pass_through_h2() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["grpc.test"])).await;
    h.route("grpc.test", PUBLIC, 443, &up);
    let a = h.instance(&["POST grpc.test/**"]).await;
    let mut s = h2(tunnel(&h, &a, "grpc.test", 443, &[b"h2"]).await).await;
    let mut r = req("POST", "https://grpc.test/grpc");
    r.headers_mut()
        .insert("content-type", "application/grpc".parse().unwrap());
    r.headers_mut().insert("te", "trailers".parse().unwrap());
    let got = collect(s.send_request(r).await.unwrap()).await;
    assert_eq!(got.status, 200);
    let trailers = got.trailers.expect("Trailer erwartet");
    assert_eq!(trailers["grpc-status"], "0");
    assert_eq!(trailers["grpc-message"], "ok");
    // `te: trailers` erreicht den Upstream (gRPC verlangt es).
    assert_eq!(up.seen()[0].headers["te"], "trailers");
}
