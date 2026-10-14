//! Was die Sandbox vom Proxy bekommt (PRX-008) und der Test-Zugang für Netz-Tests anderer
//! Crates (Feature `test-net`).

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use time::OffsetDateTime;

use super::support::*;
use crate::{BetonCa, EgressProxy, InstanceSpec, PrivateAllowList, ProxyOptions, RuleSet};

#[test]
fn prx_008_ca_bundle_holds_the_beton_ca_and_the_system_roots() {
    let ca = BetonCa::generate("nod_x", OffsetDateTime::now_utc()).unwrap();
    let der = ca.cert_der().to_vec();
    let proxy = EgressProxy::new(ca, &ProxyOptions::default()).unwrap();
    let pem = proxy.ca_bundle_pem();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(certs.iter().any(|c| c.as_ref() == der.as_slice()));
    let native = rustls_native_certs::load_native_certs().certs;
    if !native.is_empty() {
        assert!(certs.len() > 1, "System-Roots fehlen im Bundle");
        assert!(certs.iter().any(|c| c.as_ref() == native[0].as_ref()));
    }
    // Kein privater Schlüssel im Bundle.
    assert!(!pem.contains("PRIVATE KEY"));
}

#[tokio::test]
async fn prx_008_test_hosts_map_names_to_local_upstreams_only() {
    let pki = TestPki::new();
    let up = Upstream::tls(pki.valid(&["registry.npmjs.org"])).await;
    let ca = BetonCa::generate("nod_x", OffsetDateTime::now_utc()).unwrap();
    let beton = CertificateDer::from(ca.cert_der().to_vec());
    let proxy = EgressProxy::with_test_hosts(
        ca,
        vec![pki.ca_der.clone()],
        &[("registry.npmjs.org", up.addr)],
    );
    let a = proxy
        .start(InstanceSpec {
            label: "test".into(),
            rules: RuleSet::compile(&["GET registry.npmjs.org/** other.test/**"]).unwrap(),
            allow_private: PrivateAllowList::default(),
            ..InstanceSpec::default()
        })
        .await
        .unwrap();
    let (stream, raw) = connect(&a, "registry.npmjs.org:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    let tls = connector(std::slice::from_ref(&beton), &[b"http/1.1"])
        .connect(
            rustls::pki_types::ServerName::try_from("registry.npmjs.org").unwrap(),
            stream,
        )
        .await
        .unwrap();
    let mut s = h1(tls).await;
    let got = collect(
        s.send_request(origin("GET", "registry.npmjs.org", "/x"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(up.count(), 1);
    // Ein nicht eingetragener Name ist NXDOMAIN, nichts geht ins echte Netz.
    let (stream, raw) = connect(&a, "other.test:443", Some(&auth(&a))).await;
    assert_eq!(raw.status, 200, "{raw:?}");
    let tls = connector(std::slice::from_ref(&beton), &[b"http/1.1"])
        .connect(
            rustls::pki_types::ServerName::try_from("other.test").unwrap(),
            stream,
        )
        .await
        .unwrap();
    let mut s = h1(tls).await;
    let got = collect(
        s.send_request(origin("GET", "other.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 502, "dns_error");
}
