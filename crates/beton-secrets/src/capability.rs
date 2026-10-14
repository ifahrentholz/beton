//! Capability-Sperre für Secret-Werte (SEC-001 AC2).
//!
//! `Secrets::get_for_use` ist `pub(crate)`. Von außen kommt man an einen Wert nur über einen
//! der drei Capability-Typen [`ProxyCap`], [`DirectApiCap`] und [`GitProviderCap`]. Sie haben
//! private Felder, sind weder `Clone` noch `Copy` und entstehen ausschließlich in
//! [`crate::Secrets::new`]. Die Komposition im Daemon reicht sie an Proxy, Direkt-API-Harness
//! und Git-Provider-Client weiter; jedes andere Modul hat keine und kann deshalb keinen Wert
//! abrufen – das prüft der Compiler. Den Zweck im Audit-Eintrag bestimmt der Typ, nicht der
//! Aufrufer.
//!
//! Der erlaubte Weg kompiliert:
//!
//! ```
//! # async fn demo(secrets: &beton_secrets::Secrets, caps: &beton_secrets::UseCapabilities) {
//! use beton_secrets::{SecretRef, UseContext};
//! let r = SecretRef::user("openai", "default", None).unwrap();
//! let ctx = UseContext { session_id: None, host_id: "hst_local".into() };
//! let _ = caps.proxy.get_for_use(secrets, &r, ctx).await;
//! # }
//! ```
//!
//! SEC-001 AC2: Ein Modul ohne Capability kann `get_for_use` nicht aufrufen (privat) …
//!
//! ```compile_fail,E0624
//! # async fn demo(secrets: &beton_secrets::Secrets) {
//! use beton_secrets::{Purpose, SecretRef, UseContext};
//! let r = SecretRef::user("openai", "default", None).unwrap();
//! let ctx = UseContext { session_id: None, host_id: "hst_local".into() };
//! let _ = secrets.get_for_use(&r, Purpose::Proxy, ctx).await;
//! # }
//! ```
//!
//! … und sich auch keine Capability bauen (privates Feld) …
//!
//! ```compile_fail,E0451
//! let _cap = beton_secrets::ProxyCap { _private: () };
//! ```
//!
//! … oder eine vorhandene kopieren.
//!
//! ```compile_fail,E0599
//! # fn demo(caps: &beton_secrets::UseCapabilities) {
//! let _copy = caps.proxy.clone();
//! # }
//! ```
//!
//! ## Vierter Konsument: die Redaction-Pipeline (SEC-013, #186)
//!
//! [`RedactionCap`] erlaubt genau eines: alle bekannten Werte in einen Matcher der
//! [`Redactor`]-Pipeline zu laden. Sie liefert nie einen Wert, weder einzeln noch als Liste;
//! der Matcher hat keine Methode, die Werte herausgibt. `attach` verbraucht die Capability
//! (eine Pipeline je Dienst), auditiert jeden Aufbau genau einmal (`secrets.redaction_loaded`,
//! nicht je Treffer) und hält den Matcher bei Anlegen, Rotieren, Löschen und Entsperren
//! aktuell.
//!
//! ```
//! # async fn demo(secrets: &beton_secrets::Secrets, caps: beton_secrets::UseCapabilities) {
//! use std::sync::Arc;
//! let redactor = Arc::new(beton_secrets::Redactor::pending());
//! caps.redaction.attach(secrets, redactor.clone()).await.unwrap();
//! # }
//! ```
//!
//! Auch sie lässt sich weder bauen …
//!
//! ```compile_fail,E0451
//! let _cap = beton_secrets::RedactionCap { _private: () };
//! ```
//!
//! … noch kopieren …
//!
//! ```compile_fail,E0599
//! # fn demo(caps: &beton_secrets::UseCapabilities) {
//! let _copy = caps.redaction.clone();
//! # }
//! ```
//!
//! … noch für `get_for_use` nutzen.
//!
//! ```compile_fail,E0599
//! # async fn demo(secrets: &beton_secrets::Secrets, caps: &beton_secrets::UseCapabilities) {
//! use beton_secrets::{SecretRef, UseContext};
//! let r = SecretRef::user("openai", "default", None).unwrap();
//! let ctx = UseContext { session_id: None, host_id: "hst_local".into() };
//! let _ = caps.redaction.get_for_use(secrets, &r, ctx).await;
//! # }
//! ```

use std::sync::Arc;

use crate::error::Result;
use crate::redact::Redactor;
use crate::service::{Purpose, Secrets, UseContext};
use crate::{SecretRef, SecretValue};

macro_rules! capability {
    ($(#[$m:meta])* $name:ident, $purpose:expr) => {
        $(#[$m])*
        #[derive(Debug)]
        pub struct $name {
            _private: (),
        }

        impl $name {
            /// Gibt den Wert heraus und auditiert den Abruf mit Zweck, Session und Host.
            pub async fn get_for_use(
                &self,
                secrets: &Secrets,
                r: &SecretRef,
                ctx: UseContext,
            ) -> Result<SecretValue> {
                secrets.get_for_use(r, $purpose, ctx).await
            }
        }
    };
}

capability!(
    /// Recht des Egress-Proxys, Credentials zu injizieren (PRX-006).
    ProxyCap,
    Purpose::Proxy
);
capability!(
    /// Recht des Direkt-API-Harness, seinen API-Key zu lesen (HAR-011).
    DirectApiCap,
    Purpose::DirectApi
);
capability!(
    /// Recht des Git-Provider-Clients, Provider-Tokens zu lesen (GIT-002).
    GitProviderCap,
    Purpose::GitProvider
);

/// Recht der Redaction-Pipeline, alle bekannten Werte als Matcher zu laden (SEC-013, #186).
#[derive(Debug)]
pub struct RedactionCap {
    _private: (),
}

impl RedactionCap {
    /// Verbindet die Pipeline mit dem Dienst: lädt alle Werte, baut den Matcher, auditiert
    /// den Aufbau und hält ihn bei Änderungen aktuell. Ist der Store gesperrt, gilt die
    /// Pipeline mit den zuletzt bekannten Werten (anfangs keinen) als bereit; gibt er auch
    /// keinen Wert heraus. Scheitert der Aufbau oder das Audit, bleibt die Pipeline
    /// blockiert (fail closed) und der Fehler kommt zurück.
    pub async fn attach(self, secrets: &Secrets, redactor: Arc<Redactor>) -> Result<()> {
        secrets.attach_redaction(redactor).await
    }
}

/// Die Capabilities; entstehen nur zusammen mit einem [`Secrets`]-Dienst.
#[derive(Debug)]
pub struct UseCapabilities {
    pub proxy: ProxyCap,
    pub direct_api: DirectApiCap,
    pub git_provider: GitProviderCap,
    pub redaction: RedactionCap,
}

impl UseCapabilities {
    pub(crate) fn new() -> Self {
        Self {
            proxy: ProxyCap { _private: () },
            direct_api: DirectApiCap { _private: () },
            git_provider: GitProviderCap { _private: () },
            redaction: RedactionCap { _private: () },
        }
    }
}
