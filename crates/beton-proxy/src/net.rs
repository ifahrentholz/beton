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

use tokio::net::TcpStream;

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
