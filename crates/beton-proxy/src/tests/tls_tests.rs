//! TLS-MITM mit der beton-CA und strikte Upstream-Validierung (PRX-002 AC1, AC2).

use rustls::pki_types::ServerName;

use super::support::*;
use crate::proxy::{EgressProxy, ProxyOptions};

const PUBLIC: &str = "93.184.216.34";

#[tokio::test]
async fn prx_002_ac1_client_in_sandbox_validates_against_beton_ca() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["example.test"])).await;
    h.route("example.test", PUBLIC, 443, &up);
    let a = h.instance(&["GET example.test/**"]).await;

    // Wie curl in der Sandbox: Vertrauensanker ist (nur) die beton-CA.
    let tls = tunnel(&h, &a, "example.test", 443, &[b"http/1.1"]).await;
    let chain = tls.get_ref().1.peer_certificates().unwrap().to_vec();
    assert_eq!(chain.last().unwrap().as_ref(), h.beton_ca.as_ref());
    let mut s = h1(tls).await;
    let got = collect(
        s.send_request(origin("GET", "example.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(got.status, 200, "{}", got.body);

    // Ein Client, der nur der CA des Upstreams vertraut, lehnt das MITM-Zertifikat ab: Der
    // Proxy terminiert TLS selbst.
    let (stream, raw) = connect(&a, "example.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200);
    let err = connector(&[h.pki.ca_der.clone()], &[b"http/1.1"])
        .connect(ServerName::try_from("example.test").unwrap(), stream)
        .await;
    assert!(err.is_err());
}

async fn assert_upstream_tls_error(config: std::sync::Arc<rustls::ServerConfig>, name: &str) {
    let h = Harness::new();
    let up = Upstream::tls(config).await;
    h.route(name, PUBLIC, 443, &up);
    let a = h.instance(&[&format!("* {name}")]).await;
    let mut s = h1(tunnel(&h, &a, name, 443, &[b"http/1.1"]).await).await;
    let got = collect(s.send_request(origin("GET", name, "/")).await.unwrap()).await;
    assert_denied(&got, 502, "upstream_tls_error");
    // Der Upstream hat keinen Request gesehen.
    assert_eq!(up.count(), 0);
    // Auch ein zweiter Versuch geht nicht durch (kein „Override“).
    let got = collect(s.send_request(origin("GET", name, "/")).await.unwrap()).await;
    assert_denied(&got, 502, "upstream_tls_error");
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_002_ac2_self_signed_upstream_gets_502() {
    assert_upstream_tls_error(
        TestPki::self_signed(&["selfsigned.test"]),
        "selfsigned.test",
    )
    .await;
}

#[tokio::test]
async fn prx_002_ac2_expired_upstream_gets_502() {
    let pki = TestPki::new();
    let h = Harness::new();
    // Eigene Harness-PKI nutzen, damit die Kette sonst gültig wäre.
    let up = Upstream::tls(h.pki.expired(&["expired.test"])).await;
    drop(pki);
    h.route("expired.test", PUBLIC, 443, &up);
    let a = h.instance(&["* expired.test"]).await;
    let mut s = h1(tunnel(&h, &a, "expired.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "expired.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 502, "upstream_tls_error");
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_002_ac2_upstream_with_wrong_name_gets_502() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["other.test"])).await;
    h.route("wrongname.test", PUBLIC, 443, &up);
    let a = h.instance(&["* wrongname.test"]).await;
    let mut s = h1(tunnel(&h, &a, "wrongname.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "wrongname.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 502, "upstream_tls_error");
    assert_eq!(up.count(), 0);
}

#[test]
fn prx_002_extra_ca_files_are_loaded_or_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let pki = TestPki::new();
    let pem = {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(pki.ca_der.as_ref());
        format!("-----BEGIN CERTIFICATE-----\n{b64}\n-----END CERTIFICATE-----\n")
    };
    let good = dir.path().join("corp-ca.pem");
    std::fs::write(&good, pem).unwrap();
    let ca = || crate::BetonCa::generate("nod_x", time::OffsetDateTime::now_utc()).unwrap();
    EgressProxy::new(
        ca(),
        &ProxyOptions {
            extra_ca_files: vec![good],
        },
    )
    .unwrap();
    // Fehlende oder leere Dateien sind ein Konfigurationsfehler, kein stilles Weglassen.
    let missing = dir.path().join("missing.pem");
    assert!(
        EgressProxy::new(
            ca(),
            &ProxyOptions {
                extra_ca_files: vec![missing]
            }
        )
        .is_err()
    );
    let empty = dir.path().join("empty.pem");
    std::fs::write(&empty, "kein Zertifikat").unwrap();
    assert!(
        EgressProxy::new(
            ca(),
            &ProxyOptions {
                extra_ca_files: vec![empty]
            }
        )
        .is_err()
    );
}
