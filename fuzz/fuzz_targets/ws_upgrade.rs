//! QA-009: WebSocket-Upgrade-Verarbeitung des Proxys (PRX-004).
//!
//! Einen eigenen WS-Frame-Decoder gibt es in beton nicht: Der Proxy prüft das Upgrade
//! (`is_upgrade`), entfernt Hop-by-Hop-Header in beide Richtungen (`strip_hop_by_hop`) und
//! kopiert nach `101` die Bytes unverändert (`copy_bidirectional`); die Frames des
//! Daemon-WebSockets dekodiert tungstenite (über axum), die Nachrichten darin
//! `ws_client_msg`. Gefuzzt wird hier die eigene Schicht über hyper, die über das Upgrade
//! entscheidet; den Weg mit echtem `101` deckt `http_request` ab (Upstream mit Echo).
//!
//! Eingabe (Zeilen): erste Zeile `1.0`, `1.1` oder `2` (Version), danach `name: wert`.
//!
//! Invarianten:
//! - `is_upgrade` nur bei HTTP/1.1 mit `Upgrade` und Token `upgrade` in `Connection`;
//! - nach `strip_hop_by_hop` stehen weder `Proxy-Authorization`/`Proxy-Authenticate`
//!   (das Proxy-Token verlässt den Proxy nie) noch `Transfer-Encoding`, `Trailer`,
//!   `Keep-Alive`, `Proxy-Connection` im Request; `TE` höchstens als `trailers`;
//!   `Connection` genau `upgrade` bei Upgrade, sonst keins; `Upgrade` nur bei Upgrade;
//! - in `Connection` genannte Header sind weg (außer `Upgrade` bei Upgrade);
//! - alle übrigen Header bleiben unverändert in ihrer Reihenfolge;
//! - die Funktion ist idempotent.

#![no_main]

use http::header::{CONNECTION, TE, UPGRADE};
use http::{HeaderMap, HeaderName, HeaderValue, Request, Version};
use libfuzzer_sys::fuzz_target;

const HOP: [&str; 8] = [
    "connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "proxy-connection",
    "keep-alive",
];

fn tokens(headers: &HeaderMap, name: HeaderName) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .collect()
}

fn check(headers: &HeaderMap, upgrade: bool) {
    let listed: Vec<HeaderName> = tokens(headers, CONNECTION)
        .iter()
        .filter_map(|t| HeaderName::from_bytes(t.as_bytes()).ok())
        .collect();
    let mut after = headers.clone();
    beton_proxy::fuzzing::strip_hop_by_hop(&mut after, upgrade);
    for name in HOP {
        if name == "connection" || name == "te" {
            continue;
        }
        assert!(!after.contains_key(name), "`{name}` nach strip_hop_by_hop");
    }
    let te: Vec<&HeaderValue> = after.get_all(TE).iter().collect();
    assert!(
        te.is_empty() || (te.len() == 1 && te[0] == "trailers"),
        "TE nach strip_hop_by_hop: {te:?}"
    );
    let conn: Vec<&HeaderValue> = after.get_all(CONNECTION).iter().collect();
    if upgrade {
        assert!(
            conn.len() == 1 && conn[0] == "upgrade",
            "Connection: {conn:?}"
        );
    } else {
        assert!(conn.is_empty(), "Connection ohne Upgrade: {conn:?}");
        assert!(!after.contains_key(UPGRADE), "Upgrade ohne Upgrade-Request");
    }
    for name in &listed {
        if *name == UPGRADE && upgrade || *name == CONNECTION || *name == TE {
            continue;
        }
        assert!(
            !after.contains_key(name),
            "in Connection genannter Header `{name}` nach strip_hop_by_hop"
        );
    }
    for name in headers.keys() {
        let hop = HOP.contains(&name.as_str()) || listed.contains(name) || *name == UPGRADE;
        if !hop {
            let before: Vec<&HeaderValue> = headers.get_all(name).iter().collect();
            let kept: Vec<&HeaderValue> = after.get_all(name).iter().collect();
            assert_eq!(before, kept, "Ende-zu-Ende-Header `{name}` verändert");
        }
    }
    let mut twice = after.clone();
    beton_proxy::fuzzing::strip_hop_by_hop(&mut twice, upgrade);
    assert_eq!(after, twice, "strip_hop_by_hop nicht idempotent");
}

fuzz_target!(|data: &[u8]| {
    let lines = beton_fuzz::lines(data);
    let Some((version, rest)) = lines.split_first() else {
        return;
    };
    let version = match version.trim() {
        "1.0" => Version::HTTP_10,
        "2" => Version::HTTP_2,
        _ => Version::HTTP_11,
    };
    let mut builder = Request::builder().version(version).uri("/ws");
    for line in rest {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.trim().as_bytes()),
            HeaderValue::from_bytes(value.trim_start().as_bytes()),
        ) {
            builder = builder.header(n, v);
        }
    }
    let Ok(req) = builder.body(()) else {
        return;
    };
    let upgrade = beton_proxy::fuzzing::is_upgrade(&req);
    let expected = version == Version::HTTP_11
        && req.headers().contains_key(UPGRADE)
        && tokens(req.headers(), CONNECTION)
            .iter()
            .any(|t| t == "upgrade");
    assert_eq!(upgrade, expected, "is_upgrade");
    // Request-Richtung und Antwort-Richtung (`101` bzw. ohne Upgrade).
    check(req.headers(), upgrade);
    check(req.headers(), !upgrade);
});
