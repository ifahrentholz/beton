//! OS-Keychain über das keyring-Ökosystem (SEC-002): macOS Keychain, Windows Credential
//! Manager, Linux Secret Service (D-Bus). Service-Name `beton`, Account = SecretRef.
//!
//! Fehler der Plattform werden ohne Nutzdaten abgebildet: Einige `keyring_core::Error`-Varianten
//! tragen die gelesenen Bytes (`BadEncoding`, `BadDataFormat`) und dürfen daher nie formatiert
//! werden.

use std::sync::Arc;

use keyring_core::{CredentialStore, Entry};

use super::{Backend, BackendKind};
use crate::error::{Error, Result};
use crate::{SecretRef, SecretValue};

/// Service-Name der Keychain-Einträge.
pub const SERVICE: &str = "beton";

/// Ob ein Keychain grundsätzlich nutzbar ist, ermittelt **ohne** Zugriff auf Einträge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(String),
}

/// Regel für [`KeychainBackend::availability`]: macOS und Windows haben immer einen Keychain;
/// Linux nur mit D-Bus-Session (Secret Service); andere Systeme nie.
pub fn availability_for(os: &str, dbus_session: Option<&str>) -> Availability {
    match os {
        "macos" | "windows" => Availability::Available,
        "linux" => match dbus_session {
            Some(addr) if !addr.trim().is_empty() => Availability::Available,
            _ => Availability::Unavailable("keine D-Bus-Session (Secret Service)".into()),
        },
        other => Availability::Unavailable(format!("kein Keychain für {other}")),
    }
}

pub struct KeychainBackend {
    store: Arc<CredentialStore>,
}

impl std::fmt::Debug for KeychainBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainBackend").finish_non_exhaustive()
    }
}

fn map_err(e: &keyring_core::Error) -> Error {
    use keyring_core::Error as K;
    match e {
        K::NoEntry => Error::NotFound("Keychain-Eintrag".into()),
        K::NoStorageAccess(_) => Error::Locked,
        K::PlatformFailure(p) => Error::Unavailable(format!("Keychain: {p}")),
        K::TooLong(attr, limit) => {
            Error::Invalid(format!("Keychain: {attr} länger als {limit} Zeichen"))
        }
        K::Invalid(attr, why) => Error::Invalid(format!("Keychain: {attr}: {why}")),
        // Diese Varianten tragen Nutzdaten (Bytes des Werts): nie formatieren.
        K::BadEncoding(_) | K::BadDataFormat(..) => {
            Error::Corrupt("Keychain-Eintrag hat ein unerwartetes Format".into())
        }
        _ => Error::Unavailable("Keychain: unerwarteter Fehler".into()),
    }
}

impl KeychainBackend {
    /// Keychain dieser Plattform; unter Linux scheitert das ohne Secret Service.
    pub fn open() -> Result<Self> {
        if let Availability::Unavailable(why) = Self::availability() {
            return Err(Error::Unavailable(format!(
                "Keychain nicht verfügbar: {why}"
            )));
        }
        let store = platform_store().map_err(|e| match map_err(&e) {
            Error::Unavailable(why) => Error::Unavailable(why),
            _ => Error::Unavailable("Keychain nicht verfügbar".into()),
        })?;
        Ok(Self { store })
    }

    /// Verfügbarkeit nach [`availability_for`] für das laufende System.
    pub fn availability() -> Availability {
        let dbus = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
        availability_for(std::env::consts::OS, dbus.as_deref())
    }

    fn entry(&self, r: &SecretRef) -> Result<Entry> {
        self.store
            .build(SERVICE, &r.to_string(), None)
            .map_err(|e| map_err(&e))
    }
}

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(windows)]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "linux")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(keyring_core::Error::NotSupportedByStore(
        "kein Keychain auf diesem System".into(),
    ))
}

impl Backend for KeychainBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Keychain
    }

    fn put(&self, r: &SecretRef, value: &SecretValue) -> Result<()> {
        self.entry(r)?
            .set_secret(value.expose())
            .map_err(|e| map_err(&e))
    }

    fn get(&self, r: &SecretRef) -> Result<SecretValue> {
        match self.entry(r)?.get_secret() {
            Ok(bytes) => Ok(SecretValue::from_bytes(bytes)),
            Err(keyring_core::Error::NoEntry) => Err(Error::NotFound(r.to_string())),
            Err(e) => Err(map_err(&e)),
        }
    }

    fn delete(&self, r: &SecretRef) -> Result<bool> {
        match self.entry(r)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(map_err(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_002_ac2_linux_without_dbus_has_no_keychain() {
        assert!(matches!(
            availability_for("linux", None),
            Availability::Unavailable(_)
        ));
        assert!(matches!(
            availability_for("linux", Some("")),
            Availability::Unavailable(_)
        ));
        assert_eq!(
            availability_for("linux", Some("unix:path=/run/user/1000/bus")),
            Availability::Available
        );
        assert_eq!(availability_for("macos", None), Availability::Available);
        assert_eq!(availability_for("windows", None), Availability::Available);
        assert!(matches!(
            availability_for("freebsd", None),
            Availability::Unavailable(_)
        ));
    }
}
