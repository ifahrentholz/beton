//! Echter Daemon über TCP und Unix-Socket (AUTH-001).

#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::path::Path;

use beton_server::{ServerConfig, StartError, start};
use beton_store::{Store, StoreOptions};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

async fn http<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, request: &str) -> (u16, String) {
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, text)
}

fn request(host: &str, token: Option<&str>) -> String {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    format!("GET /v1/info HTTP/1.1\r\nHost: {host}\r\n{auth}Connection: close\r\n\r\n")
}

fn config(dir: &Path) -> ServerConfig {
    let mut cfg = ServerConfig::local(dir.to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg
}

async fn store(dir: &Path) -> Store {
    Store::open(dir, StoreOptions::default()).await.unwrap()
}

fn token(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned()
}

#[tokio::test]
async fn auth_001_ac1_real_tcp_listener_on_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = start(config(dir.path()), store(dir.path()).await)
        .await
        .unwrap();
    let addr: SocketAddr = daemon.addrs[0];
    assert!(addr.ip().is_loopback());
    let host = format!("127.0.0.1:{}", addr.port());
    let connect = || tokio::net::TcpStream::connect(addr);

    let (status, _) = http(connect().await.unwrap(), &request(&host, None)).await;
    assert_eq!(status, 401);
    let (status, _) = http(connect().await.unwrap(), &request(&host, Some("falsch"))).await;
    assert_eq!(status, 401);
    let (status, body) = http(
        connect().await.unwrap(),
        &request(&host, Some(&token(dir.path()))),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("org_local"));
    daemon.shutdown().await;
}

#[tokio::test]
async fn auth_001_ac2_insecure_token_file_refuses_start() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = start(config(dir.path()), store(dir.path()).await)
        .await
        .unwrap();
    daemon.shutdown().await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.path().join("auth/local.token");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = start(config(dir.path()), store(dir.path()).await)
            .await
            .unwrap_err();
        assert!(matches!(err, StartError::Auth(_)), "{err}");
        assert!(err.to_string().contains("644"), "{err}");
    }
}

#[tokio::test]
async fn auth_001_ac4_non_loopback_listen_refuses_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.listen = vec!["0.0.0.0:0".parse().unwrap()];
    let err = start(cfg, store(dir.path()).await).await.unwrap_err();
    assert!(matches!(err, StartError::Config(_)), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn auth_001_unix_socket_is_private_and_needs_token() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let daemon = start(config(dir.path()), store(dir.path()).await)
        .await
        .unwrap();
    let socket = daemon.socket.clone().unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&socket), 0o600);
    assert_eq!(mode(socket.parent().unwrap()), 0o700);

    let connect = || tokio::net::UnixStream::connect(&socket);
    let (status, _) = http(connect().await.unwrap(), &request("localhost", None)).await;
    assert_eq!(status, 401);
    let (status, _) = http(
        connect().await.unwrap(),
        &request("localhost", Some(&token(dir.path()))),
    )
    .await;
    assert_eq!(status, 200);
    daemon.shutdown().await;
    assert!(!socket.exists(), "Socket wird beim Beenden entfernt");
}

/// AUTH-001 AC3: Ein zweiter OS-User kann weder Token-Datei noch Socket öffnen.
/// Läuft in CI (Linux) mit `BETON_TEST_OTHER_USER=<user>` und passwortlosem `sudo`.
#[cfg(unix)]
#[tokio::test]
async fn auth_001_ac3_other_os_user_cannot_open_token_or_socket() {
    use std::os::unix::fs::PermissionsExt;
    let Ok(other) = std::env::var("BETON_TEST_OTHER_USER") else {
        eprintln!("übersprungen: BETON_TEST_OTHER_USER nicht gesetzt (läuft in CI)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Elternverzeichnis bewusst offen: Nur die Rechte von beton selbst dürfen schützen.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let daemon = start(config(dir.path()), store(dir.path()).await)
        .await
        .unwrap();
    let token_file = dir.path().join("auth/local.token");
    let socket = daemon.socket.clone().unwrap();

    let as_other = |script: String| {
        std::process::Command::new("sudo")
            .args(["-n", "-u", &other, "sh", "-c", &script])
            .output()
            .unwrap()
    };
    let read = as_other(format!("cat '{}'", token_file.display()));
    assert!(!read.status.success(), "Token-Datei lesbar: {:?}", read);
    let connect = as_other(format!(
        "python3 -c \"import socket; s=socket.socket(socket.AF_UNIX); s.connect('{}')\"",
        socket.display()
    ));
    assert!(
        !connect.status.success(),
        "Socket erreichbar: {:?}",
        connect
    );
    let stderr = String::from_utf8_lossy(&connect.stderr);
    assert!(
        stderr.contains("Permission denied") || stderr.contains("PermissionError"),
        "{stderr}"
    );
    daemon.shutdown().await;
}
