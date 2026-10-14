//! Backends für Secret-Werte: OS-Keychain (SEC-002), verschlüsselte Datei (SEC-003), dazu ein
//! Speicher-Backend für Tests und ein gesperrtes für „kein Backend“.
//!
//! Backends sind synchron; der [`crate::Secrets`]-Dienst ruft sie über `spawn_blocking` auf
//! (Keychain-Zugriffe können blockieren, z. B. für einen Freigabedialog von macOS).

pub mod file;
#[cfg(test)]
mod file_kill_tests;
#[cfg(test)]
mod file_tests;
pub mod keychain;
pub mod memory;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::{SecretRef, SecretValue};

pub use file::{FileBackend, KdfParams};
pub use keychain::KeychainBackend;
pub use memory::MemoryBackend;

/// Art des Backends (für Metadaten, `beton doctor` und die API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Keychain,
    File,
    /// Nur für Tests.
    Memory,
    /// Kein Backend konfiguriert: jeder Zugriff scheitert (fail closed).
    None,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::File => "file",
            Self::Memory => "memory",
            Self::None => "none",
        }
    }
}

/// Speicher für Secret-Werte, adressiert per [`SecretRef`].
pub trait Backend: Send + Sync + 'static {
    fn kind(&self) -> BackendKind;

    /// `true`, solange Werte nicht zugänglich sind (Datei-Store ohne Passphrase).
    fn locked(&self) -> bool {
        false
    }

    fn put(&self, r: &SecretRef, value: &SecretValue) -> Result<()>;

    /// `not_found`, wenn es keinen Wert gibt.
    fn get(&self, r: &SecretRef) -> Result<SecretValue>;

    /// `false`, wenn es keinen Wert gab.
    fn delete(&self, r: &SecretRef) -> Result<bool>;

    /// Entsperrt den Store (nur Datei-Store).
    fn unlock(&self, _passphrase: &SecretValue) -> Result<()> {
        Err(Error::Invalid(format!(
            "Das Backend {} braucht keine Passphrase",
            self.kind().as_str()
        )))
    }

    /// Wechselt die Passphrase (nur Datei-Store).
    fn change_passphrase(&self, _old: &SecretValue, _new: &SecretValue) -> Result<()> {
        Err(Error::Invalid(format!(
            "Das Backend {} hat keine Passphrase",
            self.kind().as_str()
        )))
    }
}

/// Kein Backend: alles scheitert mit `secrets_unavailable` (fail closed).
#[derive(Debug, Default)]
pub struct NoBackend;

impl Backend for NoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::None
    }

    fn locked(&self) -> bool {
        true
    }

    fn put(&self, _r: &SecretRef, _v: &SecretValue) -> Result<()> {
        Err(unavailable())
    }

    fn get(&self, _r: &SecretRef) -> Result<SecretValue> {
        Err(unavailable())
    }

    fn delete(&self, _r: &SecretRef) -> Result<bool> {
        Err(unavailable())
    }
}

fn unavailable() -> Error {
    Error::Unavailable("kein Secret-Backend konfiguriert".into())
}
