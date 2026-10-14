//! Secret-Store von beton (SEC-001 … SEC-004, ADR-0024).
//!
//! - [`SecretRef`] adressiert Secrets (`secret://user/openai/default`), [`SecretValue`] hält
//!   einen Wert im Speicher (kein Klartext in `Debug`, kein `Serialize`, genullt beim Drop).
//! - [`Secrets`] verwaltet Metadaten (Datenbank) und Werte (Backend: OS-Keychain oder
//!   verschlüsselte Datei) und schreibt jede Änderung und Nutzung ins hash-verkettete
//!   Audit-Log (SEC-012, `beton-store`).
//! - Werte gibt es nur über die Capability-Typen in [`capability`] (SEC-001 AC2).
//!
//! Fail closed: Ist der Store gesperrt, fehlt ein Backend oder scheitert das Audit, gibt es
//! keinen Wert (`secrets_locked`, `secrets_unavailable`).

pub mod backend;
pub mod capability;
pub mod error;
pub mod secret_ref;
mod service;
#[cfg(test)]
mod service_tests;
pub mod value;

pub use capability::{DirectApiCap, GitProviderCap, ProxyCap, UseCapabilities};
pub use error::{Error, Result};
pub use secret_ref::{SecretRef, SecretScope};
pub use service::{NewSecret, Purpose, SecretKind, SecretMeta, Secrets, StoreStatus, UseContext};
pub use value::{SecretValue, mask};
