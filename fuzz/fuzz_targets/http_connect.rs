//! QA-009: CONNECT-Parser und Tunnel-Aufbau des Egress-Proxys (PRX-003 AC5, PRX-005, PRX-007).
//!
//! Eingabe: erste Zeile = CONNECT-Authority (`host:port`, roh in Request-Zeile und `Host`),
//! der Rest geht nach `200` in den Tunnel: roh (z. B. ein `ClientHello` für Passthrough) oder,
//! wenn er mit `TLS\n` beginnt und das Ziel kein Passthrough ist, als Klartext in einer
//! TLS-Verbindung gegen die beton-CA.
//!
//! Invarianten: kein Hänger; ein Tunnel (`200`) entsteht nur zu einem Passthrough-Ziel oder
//! zu `host:port` mit passender Allow-Regel (nie zu privaten oder unbekannten Zielen); der
//! Passthrough-Upstream bekommt nur Bytes, die mit einem `ClientHello` für genau diesen Host
//! beginnen (prüft der Upstream selbst); kein echter Credential-Wert beim Client.

#![no_main]

use beton_fuzz::e2e;
use beton_proxy::{Host, Scheme};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if e2e::mentions_secret(data) {
        return;
    }
    let (authority, payload) = match data.iter().position(|b| *b == b'\n') {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let authority = authority.strip_suffix(b"\r").unwrap_or(authority);
    let p = e2e::proxy();
    // Ziel so lesen, wie hyper die Request-Zeile liest (`http::Uri` in Authority-Form).
    let target = http::Uri::try_from(authority).ok().and_then(|uri| {
        let a = uri.authority()?.clone();
        let host = Host::parse(a.host()).ok()?;
        Some((host, a.port_u16().unwrap_or(443)))
    });
    let (payload, tls) = match (payload.strip_prefix(b"TLS\n"), &target) {
        (Some(inner), Some((host @ Host::Domain(d), port)))
            if !p.passthrough.contains(host, *port) =>
        {
            (inner, Some((d.as_str(), false)))
        }
        _ => (payload, None),
    };
    let payload = e2e::expand(p, payload);
    let t = e2e::connect(p, authority, &payload, tls);
    if t.status == Some(200) {
        let Some((host, port)) = &target else {
            panic!("Tunnel, obwohl die Authority nicht lesbar ist: {authority:?}");
        };
        assert!(
            p.passthrough.contains(host, *port) || p.rules.may_connect(Scheme::Https, host, *port),
            "Tunnel zu nicht erlaubtem Ziel {host}:{port}"
        );
    }
    e2e::assert_no_secret(&t.head, "CONNECT-Antwort");
    e2e::assert_no_secret(&t.inner, "Tunnel");
});
