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

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beton_core::id::PrincipalId;
use beton_core::time::Timestamp;
use beton_secrets::{NewSecret, ProxyCap, SecretKind, SecretRef, SecretValue, Secrets, UseContext};
use beton_store::{LocalIdentity, NewAuditEntry, Store};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, SerialNumber,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::{CertifiedKey, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use zeroize::{Zeroize as _, Zeroizing};

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
    cn: String,
    cert_der: Vec<u8>,
    key: KeyPair,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

impl fmt::Debug for BetonCa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BetonCa")
            .field("subject", &self.cn)
            .field("not_after", &self.not_after)
            .finish_non_exhaustive()
    }
}

impl Drop for BetonCa {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/// Gespeicherte Form im Secret-Store (JSON, Schlüssel als PKCS#8-DER in Base64; keine
/// PEM-Marker).
#[derive(Serialize, Deserialize)]
struct Stored {
    v: u32,
    cn: String,
    not_before: i64,
    not_after: i64,
    cert: String,
    key: String,
}

fn gen_err(e: impl fmt::Display) -> CaError {
    CaError::Generate(e.to_string())
}

/// Zufällige, positive Seriennummer (16 Byte).
fn serial() -> Result<SerialNumber, CaError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(gen_err)?;
    bytes[0] &= 0x7f;
    bytes[0] |= 0x01;
    Ok(SerialNumber::from_slice(&bytes))
}

/// Parameter der CA; aus CN und Gültigkeit reproduzierbar (für den Issuer der Leafs).
fn ca_params(cn: &str, not_before: OffsetDateTime, not_after: OffsetDateTime) -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "beton");
    dn.push(DnType::CommonName, cn);
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = not_before;
    params.not_after = not_after;
    params
}

fn unix(t: OffsetDateTime) -> i64 {
    t.unix_timestamp()
}

fn from_unix(t: i64) -> Result<OffsetDateTime, CaError> {
    OffsetDateTime::from_unix_timestamp(t).map_err(|_| CaError::Corrupt)
}

impl BetonCa {
    /// Erzeugt eine neue CA für diese Installation.
    pub fn generate(installation_id: &str, now: OffsetDateTime) -> Result<Self, CaError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(gen_err)?;
        let cn = format!("beton egress CA {installation_id}");
        // Sekundengenau, damit die gespeicherte Form exakt dieselben Parameter ergibt.
        let not_before = from_unix(unix(now))?;
        let not_after = not_before + CA_VALIDITY;
        let mut params = ca_params(&cn, not_before, not_after);
        params.serial_number = Some(serial()?);
        let cert = params.self_signed(&key).map_err(gen_err)?;
        Ok(Self {
            cn,
            cert_der: cert.der().to_vec(),
            key,
            not_before,
            not_after,
        })
    }

    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    /// Zertifikat als PEM (für das CA-Bundle der Sandbox, PRX-008).
    pub fn cert_pem(&self) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.cert_der);
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for chunk in b64.as_bytes().chunks(64) {
            pem.push_str(&String::from_utf8_lossy(chunk));
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        pem
    }

    pub fn info(&self) -> CaInfo {
        CaInfo {
            subject: self.cn.clone(),
            fingerprint_sha256: hex::encode(Sha256::digest(&self.cert_der)),
            not_before: self.not_before.into(),
            not_after: self.not_after.into(),
        }
    }

    pub fn not_after(&self) -> OffsetDateTime {
        self.not_after
    }

    /// Issuer für Leaf-Zertifikate.
    fn issuer(&self) -> Issuer<'static, &KeyPair> {
        Issuer::new(
            ca_params(&self.cn, self.not_before, self.not_after),
            &self.key,
        )
    }

    /// Serialisierung für den Secret-Store.
    pub(crate) fn to_secret(&self) -> Result<SecretValue, CaError> {
        let key = Zeroizing::new(
            base64::engine::general_purpose::STANDARD.encode(self.key.serialized_der()),
        );
        let mut stored = Stored {
            v: 1,
            cn: self.cn.clone(),
            not_before: unix(self.not_before),
            not_after: unix(self.not_after),
            cert: base64::engine::general_purpose::STANDARD.encode(&self.cert_der),
            key: key.to_string(),
        };
        let json = serde_json::to_vec(&stored).map_err(gen_err);
        stored.key.zeroize();
        Ok(SecretValue::from_bytes(json?))
    }

    /// Liest die gespeicherte Form; jede Unstimmigkeit ist `proxy_ca_corrupt`.
    fn from_secret(value: &SecretValue) -> Result<Self, CaError> {
        let mut stored: Stored =
            serde_json::from_slice(value.expose()).map_err(|_| CaError::Corrupt)?;
        let key_der = base64::engine::general_purpose::STANDARD
            .decode(stored.key.as_bytes())
            .map(Zeroizing::new);
        stored.key.zeroize();
        let key_der = key_der.map_err(|_| CaError::Corrupt)?;
        if stored.v != 1 {
            return Err(CaError::Corrupt);
        }
        let key = KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(key_der.as_slice()),
            &PKCS_ECDSA_P256_SHA256,
        )
        .map_err(|_| CaError::Corrupt)?;
        let cert_der = base64::engine::general_purpose::STANDARD
            .decode(stored.cert.as_bytes())
            .map_err(|_| CaError::Corrupt)?;
        // Zertifikat und Schlüssel müssen zusammenpassen.
        if !cert_der
            .windows(key.public_key_raw().len())
            .any(|w| w == key.public_key_raw())
        {
            return Err(CaError::Corrupt);
        }
        Ok(Self {
            cn: stored.cn,
            cert_der,
            key,
            not_before: from_unix(stored.not_before)?,
            not_after: from_unix(stored.not_after)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn key_pkcs8_der(&self) -> Vec<u8> {
        self.key.serialized_der().to_vec()
    }
}

fn ca_ref() -> SecretRef {
    // Konstante, gültige Ref.
    CA_SECRET_REF
        .parse()
        .unwrap_or_else(|_| unreachable!("CA_SECRET_REF ist gültig"))
}

/// Lädt, erzeugt und rotiert die CA im Secret-Store.
#[derive(Debug, Clone)]
pub struct CaStore {
    secrets: Secrets,
    store: Store,
    local: LocalIdentity,
}

impl CaStore {
    pub fn new(secrets: Secrets, store: Store, local: LocalIdentity) -> Self {
        Self {
            secrets,
            store,
            local,
        }
    }

    /// Lädt die CA über die Proxy-Capability (auditiert) oder legt sie beim ersten Bedarf an.
    /// Fail closed: gesperrter Store, kaputte oder abgelaufene CA → Fehler.
    pub async fn load_or_create(
        &self,
        cap: &ProxyCap,
        ctx: UseContext,
    ) -> Result<BetonCa, CaError> {
        let r = ca_ref();
        match cap.get_for_use(&self.secrets, &r, ctx.clone()).await {
            Ok(value) => return Self::usable(BetonCa::from_secret(&value)?),
            Err(beton_secrets::Error::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
        let ca = BetonCa::generate(&self.local.node.to_string(), OffsetDateTime::now_utc())?;
        let created = self
            .secrets
            .put(NewSecret {
                secret_ref: r.clone(),
                kind: SecretKind::Token,
                value: ca.to_secret()?,
            })
            .await;
        match created {
            Ok(_) => {
                self.audit("proxy.ca_created", &ca).await?;
                Ok(ca)
            }
            // Gleichzeitig von einer anderen Session angelegt: deren CA gilt.
            Err(beton_secrets::Error::Exists(_)) => {
                let value = cap.get_for_use(&self.secrets, &r, ctx).await?;
                Self::usable(BetonCa::from_secret(&value)?)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn usable(ca: BetonCa) -> Result<BetonCa, CaError> {
        if ca.not_after <= OffsetDateTime::now_utc() {
            return Err(CaError::Expired);
        }
        Ok(ca)
    }

    /// Erzeugt eine neue CA (`beton proxy ca rotate`) und auditiert die Rotation. Laufende
    /// Proxys behalten ihre CA bis zum Neustart.
    pub async fn rotate(&self) -> Result<CaInfo, CaError> {
        let r = ca_ref();
        let ca = BetonCa::generate(&self.local.node.to_string(), OffsetDateTime::now_utc())?;
        match self.secrets.rotate(&r, ca.to_secret()?).await {
            Ok(_) => {}
            Err(beton_secrets::Error::NotFound(_)) => {
                self.secrets
                    .put(NewSecret {
                        secret_ref: r,
                        kind: SecretKind::Token,
                        value: ca.to_secret()?,
                    })
                    .await?;
            }
            Err(e) => return Err(e.into()),
        }
        self.audit("proxy.ca_rotated", &ca).await?;
        Ok(ca.info())
    }

    async fn audit(&self, action: &str, ca: &BetonCa) -> Result<(), CaError> {
        let info = ca.info();
        self.store
            .append_audit(
                self.local.org,
                NewAuditEntry {
                    at: Timestamp::now(),
                    actor: PrincipalId::User(self.local.user),
                    action: action.into(),
                    target_kind: "proxy_ca".into(),
                    target_id: CA_SECRET_REF.into(),
                    session_id: None,
                    host_id: None,
                    details: serde_json::json!({
                        "subject": info.subject,
                        "fingerprint_sha256": info.fingerprint_sha256,
                        "not_after": info.not_after,
                    }),
                },
            )
            .await?;
        Ok(())
    }
}

/// Mindestrestlaufzeit eines Leafs aus dem Cache.
const LEAF_MIN_REMAINING: time::Duration = time::Duration::hours(1);
/// Rückdatierung gegen Uhrenabweichung.
const LEAF_BACKDATE: time::Duration = time::Duration::minutes(5);

struct CacheEntry {
    key: Arc<CertifiedKey>,
    not_after: OffsetDateTime,
    used: u64,
}

#[derive(Default)]
struct LeafCache {
    entries: HashMap<Host, CacheEntry>,
    tick: u64,
}

/// Leaf-Zertifikate einer Proxy-Instanz: ein Schlüssel, ein Zertifikat pro Host.
pub struct LeafIssuer {
    ca: Arc<BetonCa>,
    key: KeyPair,
    signing: Arc<dyn SigningKey>,
    cache: Mutex<LeafCache>,
}

impl fmt::Debug for LeafIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafIssuer").finish_non_exhaustive()
    }
}

impl Drop for LeafIssuer {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl LeafIssuer {
    pub fn new(ca: Arc<BetonCa>) -> Result<Self, CaError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(gen_err)?;
        let der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialized_der().to_vec()));
        let signing = rustls::crypto::ring::sign::any_ecdsa_type(&der).map_err(gen_err)?;
        Ok(Self {
            ca,
            key,
            signing,
            cache: Mutex::new(LeafCache::default()),
        })
    }

    /// Zertifikat und Schlüssel für `host` (aus dem Cache, solange noch > 1 h gültig).
    pub fn certified_key(
        &self,
        host: &Host,
        now: OffsetDateTime,
    ) -> Result<Arc<CertifiedKey>, CaError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| gen_err("Leaf-Cache vergiftet"))?;
        cache.tick += 1;
        let tick = cache.tick;
        if let Some(entry) = cache.entries.get_mut(host)
            && entry.not_after - now > LEAF_MIN_REMAINING
        {
            entry.used = tick;
            return Ok(entry.key.clone());
        }
        let (key, not_after) = self.issue(host, now)?;
        if cache.entries.len() >= LEAF_CACHE_SIZE && !cache.entries.contains_key(host) {
            let oldest = cache
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(h, _)| h.clone());
            if let Some(oldest) = oldest {
                cache.entries.remove(&oldest);
            }
        }
        cache.entries.insert(
            host.clone(),
            CacheEntry {
                key: key.clone(),
                not_after,
                used: tick,
            },
        );
        Ok(key)
    }

    fn issue(
        &self,
        host: &Host,
        now: OffsetDateTime,
    ) -> Result<(Arc<CertifiedKey>, OffsetDateTime), CaError> {
        let name = match host {
            Host::Domain(d) => d.clone(),
            Host::Ip(ip) => ip.to_string(),
        };
        let not_after = (now + LEAF_VALIDITY).min(self.ca.not_after);
        if not_after <= now {
            return Err(CaError::Expired);
        }
        let mut params = CertificateParams::new(vec![name.clone()]).map_err(gen_err)?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = now - LEAF_BACKDATE;
        params.not_after = not_after;
        params.serial_number = Some(serial()?);
        params.use_authority_key_identifier_extension = true;
        let cert = params
            .signed_by(&self.key, &self.ca.issuer())
            .map_err(gen_err)?;
        let chain = vec![
            CertificateDer::from(cert.der().to_vec()),
            CertificateDer::from(self.ca.cert_der.clone()),
        ];
        Ok((
            Arc::new(CertifiedKey::new(chain, self.signing.clone())),
            not_after,
        ))
    }
}

#[cfg(test)]
#[path = "tests/ca_tests.rs"]
mod tests;
