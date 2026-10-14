//! Namensauflösung und Verbindungsaufbau zum Upstream (PRX-007).
//!
//! Der Proxy löst Hostnamen selbst auf, prüft **alle** Adressen gegen die Private-IP-Sperre
//! und verbindet sich ausschließlich mit einer geprüften Adresse; es gibt keine zweite
//! Auflösung. Resolver und Dialer sind crate-interne Traits: Produktion nutzt immer
//! [`Net::system`]; Test-Doubles (Test-Resolver, Dialer auf lokale Test-Upstreams) gibt es nur
//! unter `#[cfg(test)]` und sind aus keiner Konfiguration erreichbar.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::Uri;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::Sleep;
use tokio_rustls::TlsConnector;

use crate::ip::{PrivateAllowList, canonical, destination_allowed};
use crate::normalize::Host;
use crate::proxy::{CONNECT_TIMEOUT, IDLE_TIMEOUT};

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Löst einen Hostnamen in Adressen auf.
pub(crate) trait Resolve: Send + Sync + 'static {
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>>;
}

/// Baut eine TCP-Verbindung zu einer bereits geprüften Adresse auf.
pub(crate) trait Dial: Send + Sync + 'static {
    fn dial(&self, addr: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>>;
}

/// Resolver und Dialer einer Proxy-Instanz.
#[derive(Clone)]
pub(crate) struct Net {
    pub(crate) resolver: Arc<dyn Resolve>,
    pub(crate) dialer: Arc<dyn Dial>,
}

impl Net {
    /// Produktion: System-Resolver (`getaddrinfo`) und direkte TCP-Verbindungen.
    pub(crate) fn system() -> Self {
        Self {
            resolver: Arc::new(SystemResolver),
            dialer: Arc::new(TcpDialer),
        }
    }
}

struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>> {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host, 0))
                .await?
                .map(|a| a.ip())
                .collect())
        })
    }
}

struct TcpDialer;

impl Dial for TcpDialer {
    fn dial(&self, addr: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>> {
        Box::pin(TcpStream::connect(addr))
    }
}

/// Fehler beim Verbindungsaufbau zum Upstream; bestimmt die Antwort an den Client.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error("ungültiger Host")]
    InvalidHost,
    #[error("DNS-Auflösung fehlgeschlagen: {0}")]
    Dns(io::Error),
    #[error("Ziel {0} ist privat bzw. nicht global und nicht freigegeben")]
    PrivateDestination(IpAddr),
    #[error("Verbindung fehlgeschlagen: {0}")]
    Connect(io::Error),
    #[error("Zeitüberschreitung beim Verbindungsaufbau")]
    Timeout,
    #[error("Upstream-TLS ungültig: {0}")]
    Tls(io::Error),
}

/// Sucht einen [`ConnectError`] in einer Fehlerkette (z. B. im Fehler des HTTP-Clients).
pub(crate) fn find_connect_error<'a>(
    err: &'a (dyn std::error::Error + 'static),
) -> Option<&'a ConnectError> {
    let mut current = Some(err);
    while let Some(e) = current {
        if let Some(c) = e.downcast_ref::<ConnectError>() {
            return Some(c);
        }
        current = e.source();
    }
    None
}

/// Verbindungsaufbau für den HTTP-Client einer Instanz: auflösen, **alle** Adressen prüfen,
/// nur geprüfte Adressen wählen, bei `https` TLS strikt validieren (webpki, SNI = Host).
#[derive(Clone)]
pub(crate) struct Connector {
    pub(crate) net: Net,
    pub(crate) tls: Arc<rustls::ClientConfig>,
    pub(crate) allow_private: Arc<PrivateAllowList>,
}

impl Connector {
    async fn connect(self, uri: Uri) -> Result<UpstreamIo, ConnectError> {
        let https = uri.scheme_str() == Some("https");
        let host = Host::parse(uri.host().ok_or(ConnectError::InvalidHost)?)
            .map_err(|_| ConnectError::InvalidHost)?;
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let addrs = match &host {
            Host::Ip(ip) => vec![*ip],
            Host::Domain(name) => self
                .net
                .resolver
                .resolve(name)
                .await
                .map_err(ConnectError::Dns)?,
        };
        if addrs.is_empty() {
            return Err(ConnectError::Dns(io::Error::new(
                io::ErrorKind::NotFound,
                "keine Adresse",
            )));
        }
        // Erst alle Adressen prüfen, dann verbinden: Eine einzige private Adresse in der
        // Antwort sperrt das Ziel (DNS-Rebinding, gemischte Antworten).
        let addrs: Vec<SocketAddr> = addrs
            .into_iter()
            .map(|ip| SocketAddr::new(canonical(ip), port))
            .collect();
        if let Some(bad) = addrs
            .iter()
            .find(|a| !destination_allowed(**a, &self.allow_private))
        {
            return Err(ConnectError::PrivateDestination(bad.ip()));
        }
        let mut last = None;
        let mut tcp = None;
        for addr in addrs {
            match self.net.dialer.dial(addr).await {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        let tcp = tcp.ok_or_else(|| {
            ConnectError::Connect(
                last.unwrap_or_else(|| io::Error::other("keine Verbindung möglich")),
            )
        })?;
        let _ = tcp.set_nodelay(true);
        let io = Idle::new(tcp, IDLE_TIMEOUT);
        if !https {
            return Ok(UpstreamIo {
                io: TokioIo::new(MaybeTls::Plain(io)),
                h2: false,
            });
        }
        let name = match host {
            Host::Domain(d) => ServerName::try_from(d).map_err(|_| ConnectError::InvalidHost)?,
            Host::Ip(ip) => ServerName::IpAddress(ip.into()),
        };
        let tls = TlsConnector::from(self.tls.clone())
            .connect(name, io)
            .await
            .map_err(ConnectError::Tls)?;
        let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
        Ok(UpstreamIo {
            io: TokioIo::new(MaybeTls::Tls(Box::new(tls))),
            h2,
        })
    }
}

impl tower_service::Service<Uri> for Connector {
    type Response = UpstreamIo;
    type Error = ConnectError;
    type Future = BoxFuture<'static, Result<UpstreamIo, ConnectError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            tokio::time::timeout(CONNECT_TIMEOUT, this.connect(uri))
                .await
                .unwrap_or(Err(ConnectError::Timeout))
        })
    }
}

/// Stream mit Leerlauf-Timeout: Ohne Lese- oder Schreibfortschritt endet er nach `timeout`.
pub(crate) struct Idle<S> {
    inner: S,
    timeout: Duration,
    sleep: Pin<Box<Sleep>>,
}

impl<S> Idle<S> {
    pub(crate) fn new(inner: S, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            sleep: Box::pin(tokio::time::sleep(timeout)),
        }
    }

    fn progress(&mut self) {
        let deadline = tokio::time::Instant::now() + self.timeout;
        self.sleep.as_mut().reset(deadline);
    }

    fn pending<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        match self.sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Leerlauf-Timeout",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Idle<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(r) => {
                self.progress();
                Poll::Ready(r)
            }
            Poll::Pending => self.pending(cx),
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Idle<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(r) => {
                self.progress();
                Poll::Ready(r)
            }
            Poll::Pending => self.pending(cx),
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(r) => {
                self.progress();
                Poll::Ready(r)
            }
            Poll::Pending => self.pending(cx),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Upstream-Verbindung: Klartext oder TLS.
pub(crate) enum MaybeTls {
    Plain(Idle<TcpStream>),
    Tls(Box<tokio_rustls::client::TlsStream<Idle<TcpStream>>>),
}

impl AsyncRead for MaybeTls {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Plain(s) => s.is_write_vectored(),
            Self::Tls(s) => s.is_write_vectored(),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Verbindung, wie sie der HTTP-Client von hyper braucht; meldet ausgehandeltes HTTP/2.
pub(crate) struct UpstreamIo {
    io: TokioIo<MaybeTls>,
    h2: bool,
}

impl Connection for UpstreamIo {
    fn connected(&self) -> Connected {
        let connected = Connected::new();
        if self.h2 {
            connected.negotiated_h2()
        } else {
            connected
        }
    }
}

impl hyper::rt::Read for UpstreamIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl hyper::rt::Write for UpstreamIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write_vectored(cx, bufs)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn prx_001_idle_connections_time_out() {
        let (a, mut b) = tokio::io::duplex(64);
        let mut idle = Idle::new(a, Duration::from_secs(120));
        b.write_all(b"x").await.unwrap();
        let mut buf = [0u8; 1];
        idle.read_exact(&mut buf).await.unwrap();
        // 119 s Ruhe sind in Ordnung, Aktivität setzt die Frist zurück …
        tokio::time::advance(Duration::from_secs(119)).await;
        b.write_all(b"y").await.unwrap();
        idle.read_exact(&mut buf).await.unwrap();
        // … nach 120 s ohne Fortschritt ist Schluss.
        let err = idle.read_exact(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
