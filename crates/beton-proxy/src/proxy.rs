//! Proxy-Kern und Instanzmodell (PRX-001), TLS-MITM (PRX-002), Regelprüfung je Request bzw.
//! HTTP/2-Stream (PRX-003, PRX-004) und Private-IP-Schutz beim Verbindungsaufbau (PRX-007).

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use zeroize::Zeroizing;

use crate::ca::{BetonCa, CaInfo};
use crate::ip::PrivateAllowList;
use crate::net::Net;
use crate::rules::RuleSet;

/// Timeout für den Verbindungsaufbau zum Upstream (inkl. DNS und TLS).
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Leerlauf-Timeout für Client- und Upstream-Verbindungen.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Höchstzahl gleichzeitiger HTTP/2-Streams pro Client-Verbindung.
pub const MAX_CONCURRENT_STREAMS: u32 = 100;

/// Einstellungen des Proxys (`proxy.*` in der Konfiguration).
#[derive(Debug, Clone, Default)]
pub struct ProxyOptions {
    /// Zusätzliche vertrauenswürdige CAs für Upstream-TLS (PEM), z. B. eine Firmen-CA.
    pub extra_ca_files: Vec<PathBuf>,
}

/// Fehler beim Aufbau des Proxys oder einer Instanz.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("proxy_config: {0}")]
    Config(String),
    #[error("proxy_io: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ca(#[from] crate::ca::CaError),
}

/// Regeln und Freigaben einer Proxy-Instanz (eine pro Session-Stufe).
#[derive(Debug, Clone, Default)]
pub struct InstanceSpec {
    /// Kennung für Logs, z. B. `ses_…/tools`; nie ein Secret.
    pub label: String,
    pub rules: RuleSet,
    pub allow_private: PrivateAllowList,
}

/// Der Egress-Proxy eines Runners: eine CA, ein Upstream-Vertrauensanker, beliebig viele
/// Instanzen.
#[derive(Clone)]
pub struct EgressProxy {
    _ca: Arc<BetonCa>,
    _net: Net,
}

impl fmt::Debug for EgressProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EgressProxy").finish_non_exhaustive()
    }
}

impl EgressProxy {
    /// Proxy mit System-Resolver, direkten Verbindungen und System-Roots plus
    /// `extra_ca_files` für Upstream-TLS.
    pub fn new(ca: BetonCa, _options: &ProxyOptions) -> Result<Self, ProxyError> {
        Ok(Self {
            _ca: Arc::new(ca),
            _net: Net::system(),
        })
    }

    /// Nur für Tests: Test-Resolver, Test-Dialer und die Test-CA der lokalen Upstreams als
    /// einziger Vertrauensanker. Nicht aus Konfiguration erreichbar.
    #[cfg(test)]
    pub(crate) fn for_tests(
        ca: BetonCa,
        _upstream_roots: Vec<rustls::pki_types::CertificateDer<'static>>,
        net: Net,
    ) -> Self {
        Self {
            _ca: Arc::new(ca),
            _net: net,
        }
    }

    pub fn ca_info(&self) -> CaInfo {
        self._ca.info()
    }

    /// Startet eine Instanz auf `127.0.0.1:<zufälliger Port>` mit eigenem Token.
    pub async fn start(&self, _spec: InstanceSpec) -> Result<ProxyInstance, ProxyError> {
        Err(ProxyError::Config("noch nicht umgesetzt".into()))
    }
}

/// Auth-Token einer Instanz (256 bit, hex); `Debug` zeigt es nie.
pub struct ProxyToken(Zeroizing<String>);

impl ProxyToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProxyToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Eine laufende Proxy-Instanz; endet mit `Drop`.
pub struct ProxyInstance {
    addr: SocketAddr,
    token: ProxyToken,
}

impl fmt::Debug for ProxyInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyInstance")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl ProxyInstance {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &ProxyToken {
        &self.token
    }

    /// Proxy-URL für die Sandbox: `http://bt:<token>@127.0.0.1:<port>` (enthält das Token).
    pub fn proxy_url(&self) -> Zeroizing<String> {
        Zeroizing::new(String::new())
    }
}
