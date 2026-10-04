//! Architekturtest (API-005 AC1): `beton-cli` greift nur über `beton-sdk` auf Server zu.
//! Direkte HTTP- oder WebSocket-Clients sind weder als Abhängigkeit noch im Code erlaubt.

#![allow(clippy::unwrap_used)]

use std::path::Path;

/// Crates, die HTTP- oder WebSocket-Clients mitbringen.
const CLIENT_CRATES: [&str; 8] = [
    "reqwest",
    "hyper",
    "hyper-util",
    "ureq",
    "isahc",
    "surf",
    "tokio-tungstenite",
    "tungstenite",
];

#[test]
fn api_005_ac1_cli_has_no_direct_http_clients() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let mut section = String::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_owned();
            continue;
        }
        if !section.contains("dependencies") || section.contains("dev-dependencies") {
            continue;
        }
        let name = line.split(['=', '.']).next().unwrap_or_default().trim();
        assert!(
            !CLIENT_CRATES.contains(&name),
            "beton-cli hängt direkt von {name} ab; Server-Zugriffe gehören nach beton-sdk"
        );
    }
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in [
                "reqwest::",
                "hyper::",
                "tungstenite::",
                "TcpStream::connect",
            ] {
                assert!(
                    !text.contains(needle),
                    "{} nutzt {needle}; Netzzugriffe gehören nach beton-sdk",
                    path.display()
                );
            }
        }
    }
}
