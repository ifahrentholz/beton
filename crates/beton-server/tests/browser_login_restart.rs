//! AUTH-004 AC4: Eine Browser-Anmeldung überlebt einen Neustart des Daemons. Strikt TDD.

#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;

use beton_server::{ServerConfig, start};
use beton_store::{Store, StoreOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Minimaler HTTP/1.1-Client: Status, Header-Block (Kleinbuchstaben) und Body der Antwort.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, String, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Length: 0\r\n",
        addr.port()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.split(' ').nth(1).unwrap().parse().unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (status, head.to_lowercase(), body.to_owned())
}

async fn daemon(dir: &std::path::Path) -> (beton_server::Daemon, String) {
    let store = Store::open(dir, StoreOptions::default()).await.unwrap();
    let mut cfg = ServerConfig::local(dir.to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let d = start(cfg, store).await.unwrap();
    let token = std::fs::read_to_string(dir.join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    (d, token)
}

#[tokio::test]
async fn auth_004_ac4_browser_login_survives_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (d, token) = daemon(dir.path()).await;
    let addr = d.addrs[0];
    let bearer = format!("Bearer {token}");
    let (status, head, body) = http(
        addr,
        "POST",
        "/v1/auth/local/codes",
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, 201, "{head}");
    let code = serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, head, _) =
        http(addr, "GET", &format!("/auth/local/redeem?code={code}"), &[]).await;
    assert_eq!(status, 303, "{head}");
    let cookie = head
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: "))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let (status, _, _) = http(addr, "GET", "/v1/me", &[("cookie", &cookie)]).await;
    assert_eq!(status, 200);

    // Neustart mit demselben Datenverzeichnis.
    d.shutdown().await;
    let (d, _) = daemon(dir.path()).await;
    let addr = d.addrs[0];
    let (status, _, _) = http(addr, "GET", "/v1/me", &[("cookie", &cookie)]).await;
    assert_eq!(status, 200, "Cookie gilt nach dem Neustart weiter");
    let (status, _, _) = http(addr, "GET", "/v1/me", &[("cookie", "beton_session=00ff")]).await;
    assert_eq!(status, 401);
    d.shutdown().await;
}
