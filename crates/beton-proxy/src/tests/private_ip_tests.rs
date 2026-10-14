//! Private-IP- und DNS-Rebinding-Schutz über das Netz (PRX-007).

use std::net::SocketAddr;

use super::support::*;

#[tokio::test]
async fn prx_007_ac1_metadata_endpoint_via_dns_is_rejected() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["metadata.test"])).await;
    h.resolver
        .set("metadata.test", vec![vec![ip("169.254.169.254")]]);
    h.dialer
        .map(SocketAddr::new(ip("169.254.169.254"), 443), up.addr);
    let a = h.instance(&["* metadata.test/**"]).await;
    let mut s = h1(tunnel(&h, &a, "metadata.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "metadata.test", "/latest/meta-data/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 403, "private_destination");
    assert!(h.dialer.dialed().is_empty(), "{:?}", h.dialer.dialed());
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_007_ac2_dns_rebinding_never_reaches_loopback() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["rebind.test"])).await;
    h.resolver.set(
        "rebind.test",
        vec![vec![ip("93.184.216.34")], vec![ip("127.0.0.1")]],
    );
    h.dialer
        .map(SocketAddr::new(ip("93.184.216.34"), 443), up.addr);
    h.dialer.map(SocketAddr::new(ip("127.0.0.1"), 443), up.addr);
    let a = h.instance(&["* rebind.test/**"]).await;
    let mut statuses = Vec::new();
    for _ in 0..6 {
        // `/close`: Der Upstream schließt nach jeder Antwort, also neue Auflösung.
        let mut s = h1(tunnel(&h, &a, "rebind.test", 443, &[b"http/1.1"]).await).await;
        let got = collect(
            s.send_request(origin("GET", "rebind.test", "/close"))
                .await
                .unwrap(),
        )
        .await;
        if got.status != 200 {
            assert_denied(&got, 403, "private_destination");
        }
        statuses.push(got.status.as_u16());
    }
    assert!(
        statuses.contains(&200) && statuses.contains(&403),
        "{statuses:?}"
    );
    let dialed = h.dialer.dialed();
    assert!(!dialed.is_empty());
    assert!(
        dialed.iter().all(|a| a.ip() == ip("93.184.216.34")),
        "Verbindung zu einer gesperrten Adresse: {dialed:?}"
    );
}

#[tokio::test]
async fn prx_007_all_resolved_addresses_are_checked() {
    let h = Harness::new();
    let up = Upstream::tls(h.pki.valid(&["mixed.test"])).await;
    h.resolver.set(
        "mixed.test",
        vec![vec![ip("93.184.216.34"), ip("10.0.0.1")]],
    );
    h.dialer
        .map(SocketAddr::new(ip("93.184.216.34"), 443), up.addr);
    let a = h.instance(&["* mixed.test"]).await;
    let mut s = h1(tunnel(&h, &a, "mixed.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "mixed.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 403, "private_destination");
    assert!(h.dialer.dialed().is_empty());
}

#[tokio::test]
async fn prx_007_ac3_ip_literals_of_private_targets_are_rejected() {
    let h = Harness::new();
    let up = Upstream::plain().await;
    for target in ["10.0.0.1:80", "127.0.0.1:80", "[::ffff:10.0.0.1]:80"] {
        h.dialer.map(target.parse().unwrap(), up.addr);
    }
    let a = h
        .instance(&[
            "* http://[::ffff:10.0.0.1]",
            "* http://127.0.0.1",
            "* [::ffff:10.0.0.1]",
            "* 10.0.0.1",
        ])
        .await;
    for url in [
        "http://[::ffff:10.0.0.1]/",
        "http://2130706433/",
        "http://0x7f.1/",
    ] {
        let got = plain(&a, "GET", url, Some(&auth(&a))).await;
        assert_denied(&got, 403, "private_destination");
    }
    for authority in ["[::ffff:10.0.0.1]:443", "10.0.0.1:443", "167772161:443"] {
        let (_s, raw) = connect(&a, authority, Some(&auth(&a))).await;
        assert_eq!(raw.status, 403, "{authority}: {raw:?}");
        assert_eq!(raw.json()["error"], "private_destination", "{authority}");
    }
    assert!(h.dialer.dialed().is_empty(), "{:?}", h.dialer.dialed());
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn prx_007_ac4_allow_private_permits_exactly_one_target() {
    let h = Harness::new();
    let up = Upstream::plain().await;
    h.dialer.map("10.0.0.5:5432".parse().unwrap(), up.addr);
    h.dialer.map("10.0.0.5:22".parse().unwrap(), up.addr);
    let a = h
        .instance_with(
            &["* http://10.0.0.5:5432", "* http://10.0.0.5:22"],
            &["10.0.0.5:5432"],
        )
        .await;
    let got = plain(&a, "GET", "http://10.0.0.5:5432/", Some(&auth(&a))).await;
    assert_eq!(got.status, 200, "{}", got.body);
    let got = plain(&a, "GET", "http://10.0.0.5:22/", Some(&auth(&a))).await;
    assert_denied(&got, 403, "private_destination");
    assert_eq!(h.dialer.dialed(), vec!["10.0.0.5:5432".parse().unwrap()]);
}

#[tokio::test]
async fn prx_007_dns_failure_is_denied() {
    let h = Harness::new();
    let a = h.instance(&["* nxdomain.test"]).await;
    let mut s = h1(tunnel(&h, &a, "nxdomain.test", 443, &[b"http/1.1"]).await).await;
    let got = collect(
        s.send_request(origin("GET", "nxdomain.test", "/"))
            .await
            .unwrap(),
    )
    .await;
    assert_denied(&got, 502, "dns_error");
    assert!(h.dialer.dialed().is_empty());
}
