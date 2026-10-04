//! Probe lokaler Modell-Server (HAR-016): Ollama und LM Studio, nur Loopback, 500 ms.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Höchstdauer je Probe.
pub const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Antwortet unter `addr` ein Server der Art `id` (`ollama` oder `lmstudio`)?
/// Andere Adressen als Loopback werden nie kontaktiert (ADR-0033).
pub async fn local_server(id: &str, addr: SocketAddr) -> bool {
    if !addr.ip().is_loopback() {
        return false;
    }
    let path = if id == "ollama" { "/" } else { "/v1/models" };
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(addr).await.ok()?;
        let request = format!("GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut buf = Vec::new();
        let _ = (&mut stream).take(4096).read_to_end(&mut buf).await;
        Some(String::from_utf8_lossy(&buf).into_owned())
    };
    let Ok(Some(response)) = tokio::time::timeout(LOCAL_PROBE_TIMEOUT, attempt).await else {
        return false;
    };
    let ok_status = response.lines().next().is_some_and(|l| l.contains(" 200"));
    match id {
        "ollama" => ok_status && response.contains("Ollama"),
        _ => ok_status && response.contains("\"data\""),
    }
}
