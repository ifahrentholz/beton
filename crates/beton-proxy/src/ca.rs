//! beton-CA und Leaf-Zertifikate für TLS-MITM (PRX-002).
//!
//! - Die Root-CA (ECDSA P-256, 2 Jahre, CN `beton egress CA <installation-id>`) entsteht beim
//!   ersten Bedarf und liegt **nur im Secret-Store** unter [`CA_SECRET_REF`]; nie als
//!   Klartextdatei und nie in einem System-Trust-Store.
//! - Geladen wird sie ausschließlich über die Capability [`ProxyCap`] (SEC-001); jeder Abruf
//!   ist auditiert. Ist der Store gesperrt, gibt es keine CA und damit keine Proxy-Instanz
//!   (fail closed, `secrets_locked`).
//! - [`CaStore::rotate`] erzeugt eine neue CA und schreibt `proxy.ca_rotated` ins Audit-Log;
//!   laufende Proxys behalten ihre CA bis zum Neustart.
//! - [`LeafIssuer`] erzeugt pro Proxy-Instanz Leaf-Zertifikate (SAN = Host, 24 h, LRU-Cache);
//!   alle Leafs einer Instanz teilen einen Schlüssel (stabiler SPKI-Hash, PRX-011).

use std::fmt;
use std::sync::Arc;

use beton_secrets::{ProxyCap, Secrets, UseContext};
use beton_store::{LocalIdentity, Store};
use rustls::sign::CertifiedKey;
use serde::Serialize;
use time::OffsetDateTime;

use crate::normalize::Host;

/// Ort der CA im Secret-Store.
pub const CA_SECRET_REF: &str = "secret://org/beton/egress-ca";
/// Gültigkeit der CA.
pub const CA_VALIDITY: time::Duration = time::Duration::days(730);
/// Gültigkeit eines Leaf-Zertifikats.
pub const LEAF_VALIDITY: time::Duration = time::Duration::hours(24);
/// Größe des Leaf-Caches pro Instanz.
pub const LEAF_CACHE_SIZE: usize = 1024;

/// Fehler rund um die CA. Meldungen enthalten nie Schlüsselmaterial.
#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error(transparent)]
    Secrets(#[from] beton_secrets::Error),
    #[error("Datenbank: {0}")]
    Store(#[from] beton_store::Error),
    #[error("proxy_ca_corrupt: Die gespeicherte beton-CA ist unlesbar (beton proxy ca rotate)")]
    Corrupt,
    #[error("proxy_ca_expired: Die beton-CA ist abgelaufen (beton proxy ca rotate)")]
    Expired,
    #[error("proxy_ca_error: {0}")]
    Generate(String),
}

impl CaError {
    /// Maschinenlesbarer Code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Secrets(e) => e.code(),
            Self::Store(e) => e.code(),
            Self::Corrupt => "proxy_ca_corrupt",
            Self::Expired => "proxy_ca_expired",
            Self::Generate(_) => "proxy_ca_error",
        }
    }
}

/// Öffentliche Angaben zu einer CA (für CLI, API und Audit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaInfo {
    pub subject: String,
    /// SHA-256 über das DER-Zertifikat, hex.
    pub fingerprint_sha256: String,
    pub not_before: beton_core::time::Timestamp,
    pub not_after: beton_core::time::Timestamp,
}

/// Die beton-CA mit privatem Schlüssel (nur im Speicher des Proxys).
pub struct BetonCa {
    _private: (),
}

impl fmt::Debug for BetonCa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BetonCa").finish_non_exhaustive()
    }
}

impl BetonCa {
    /// Erzeugt eine neue CA für diese Installation.
    pub fn generate(_installation_id: &str, _now: OffsetDateTime) -> Result<Self, CaError> {
        Err(CaError::Generate("noch nicht umgesetzt".into()))
    }

    pub fn cert_der(&self) -> &[u8] {
        &[]
    }

    /// Zertifikat als PEM (für das CA-Bundle der Sandbox, PRX-008).
    pub fn cert_pem(&self) -> String {
        String::new()
    }

    pub fn info(&self) -> CaInfo {
        CaInfo {
            subject: String::new(),
            fingerprint_sha256: String::new(),
            not_before: beton_core::time::Timestamp::now(),
            not_after: beton_core::time::Timestamp::now(),
        }
    }

    /// Serialisierung für den Secret-Store.
    pub(crate) fn to_secret(&self) -> Result<beton_secrets::SecretValue, CaError> {
        Err(CaError::Generate("noch nicht umgesetzt".into()))
    }

    #[cfg(test)]
    pub(crate) fn key_pkcs8_der(&self) -> Vec<u8> {
        Vec::new()
    }
}

/// Lädt, erzeugt und rotiert die CA im Secret-Store.
#[derive(Debug, Clone)]
pub struct CaStore {
    _secrets: Secrets,
    _store: Store,
    _local: LocalIdentity,
}

impl CaStore {
    pub fn new(secrets: Secrets, store: Store, local: LocalIdentity) -> Self {
        Self {
            _secrets: secrets,
            _store: store,
            _local: local,
        }
    }

    /// Lädt die CA über die Proxy-Capability (auditiert) oder legt sie beim ersten Bedarf an.
    /// Fail closed: gesperrter Store, kaputte oder abgelaufene CA → Fehler.
    pub async fn load_or_create(
        &self,
        _cap: &ProxyCap,
        _ctx: UseContext,
    ) -> Result<BetonCa, CaError> {
        Err(CaError::Generate("noch nicht umgesetzt".into()))
    }

    /// Erzeugt eine neue CA (`beton proxy ca rotate`) und auditiert die Rotation.
    pub async fn rotate(&self) -> Result<CaInfo, CaError> {
        Err(CaError::Generate("noch nicht umgesetzt".into()))
    }
}

/// Leaf-Zertifikate einer Proxy-Instanz: ein Schlüssel, ein Zertifikat pro Host.
pub struct LeafIssuer {
    _ca: Arc<BetonCa>,
}

impl fmt::Debug for LeafIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafIssuer").finish_non_exhaustive()
    }
}

impl LeafIssuer {
    pub fn new(ca: Arc<BetonCa>) -> Result<Self, CaError> {
        Ok(Self { _ca: ca })
    }

    /// Zertifikat und Schlüssel für `host` (aus dem Cache, solange noch > 1 h gültig).
    pub fn certified_key(
        &self,
        _host: &Host,
        _now: OffsetDateTime,
    ) -> Result<Arc<CertifiedKey>, CaError> {
        Err(CaError::Generate("noch nicht umgesetzt".into()))
    }
}

#[cfg(test)]
#[path = "tests/ca_tests.rs"]
mod tests;
