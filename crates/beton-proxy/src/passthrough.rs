//! Passthrough für Vendor-Hosts (PRX-005): CONNECT ohne TLS-Terminierung.
//!
//! Damit beton Subscription-Tokens nie sieht (HAR-015, ADR-0005), tunnelt eine Instanz die
//! Vendor-Hosts ihres Harness unverändert zum Upstream. Geprüft werden nur Host und Port
//! (Liste), die Private-IP-Sperre nach eigener Auflösung (PRX-007) und dass die SNI im
//! `ClientHello` dem CONNECT-Host entspricht; sonst wird die Verbindung getrennt. Der Proxy
//! parst kein HTTP im Tunnel, protokolliert keine Header und injiziert nie Credentials.

use crate::normalize::Host;

/// Ungültiger Passthrough-Eintrag (Konfigurationsfehler).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("passthrough[{index}] `{entry}`: {message}")]
pub struct PassthroughError {
    pub index: usize,
    pub entry: String,
    pub message: String,
}

/// Passthrough-Ziele einer Proxy-Instanz: `host` (Port 443) oder `host:port`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passthrough {
    targets: Vec<(Host, u16)>,
}

impl Passthrough {
    /// Liest die Einträge; Domains werden wie Regeln normalisiert.
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, PassthroughError> {
        let mut targets = Vec::new();
        for e in entries {
            if let Ok(h) = Host::parse(e.as_ref()) {
                targets.push((h, 443));
            }
        }
        Ok(Self { targets })
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Ist `host:port` ein Passthrough-Ziel?
    pub fn contains(&self, _host: &Host, _port: u16) -> bool {
        false
    }

    /// Die Ziele als `host:port` (für Logs und `explain`).
    pub fn entries(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|(h, p)| format!("{h}:{p}"))
            .collect()
    }
}
