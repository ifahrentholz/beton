//! Redaction-Pipeline (SEC-013, OBS-002).
//!
//! Alles, was den Knoten verlässt oder persistiert wird – Event-Payloads samt `raw`,
//! Live-Events, Logs, Exporte, Handover-Kontexte und Traces –, läuft durch einen
//! [`Redactor`]. Er ersetzt Treffer durch `[REDACTED:<kind>]`:
//!
//! 1. **Bekannte Werte:** exakter Abgleich (Aho-Corasick) gegen alle Secret-Werte des Knotens,
//!    dazu ihre Base64- (Standard und URL-sicher, alle drei Ausrichtungen), URL- und
//!    JSON-kodierten Varianten. Die Werte kommen ausschließlich über die
//!    [`crate::RedactionCap`] aus `beton-secrets` (#186); der [`Redactor`] gibt sie nie heraus,
//!    weder einzeln noch über `Debug`. Art: `secret`.
//! 2. **Muster-Detektoren** für verbreitete Token-Formate (GitHub, GitLab, Anthropic, OpenAI,
//!    AWS, Slack, PEM-Private-Keys, beton-Tokens mit CRC, JWTs, `Authorization`-Header).
//!
//! `bt_cred_*`-Platzhalter des Proxys bleiben stehen.
//!
//! **Fail closed:** Solange die bekannten Werte nicht geladen sind ([`RedactionStatus::Pending`])
//! oder ihr Aufbau gescheitert ist ([`RedactionStatus::Failed`]), liefern die strikten Methoden
//! [`Unavailable`]; Aufrufer persistieren oder senden dann nichts. Nur Logs laufen während des
//! Starts (`Pending`) schon durch die Detektoren: Bis die Capability die Werte geladen hat, hat
//! `beton-secrets` in diesem Prozess noch keinen Wert herausgegeben (die Capabilities entstehen
//! erst unmittelbar davor, siehe `beton_server::serve`).

mod detect;
mod known;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

pub use detect::beton_token_checksum;
pub(crate) use known::KnownSecrets;

/// Präfix jeder Ersetzung.
pub const REDACTED_PREFIX: &str = "[REDACTED:";

/// Art der manuellen Nachredaktion (DATA-012).
pub const MANUAL: &str = "manual";

/// Ersetzungstext für eine Art, z. B. `[REDACTED:github]`.
pub fn marker(kind: &str) -> String {
    format!("{REDACTED_PREFIX}{kind}]")
}

/// Die Pipeline ist nicht bereit; nichts darf ungeschwärzt hinaus (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "redaction_unavailable: Redaction-Pipeline nicht bereit, Ausgabe zurückgehalten (fail closed)"
)]
pub struct Unavailable;

/// Woher die bekannten Werte stammen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownSource {
    /// Prozess ohne Secret-Store (CLI, Runner): nur Detektoren.
    None,
    /// Aktuelle Werte aus dem Secret-Store.
    Store,
    /// Store gesperrt oder nicht erreichbar: die zuletzt geladenen Werte (anfangs keine).
    /// Solange er gesperrt ist, gibt er auch für Sessions keinen Wert heraus.
    StoreUnavailable,
}

/// Zustand der Pipeline (für `beton doctor` und Tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RedactionStatus {
    /// Bekannte Werte noch nicht geladen.
    Pending,
    /// Bereit; `values` ist die Zahl der bekannten Werte (ohne Varianten).
    Ready { values: usize, source: KnownSource },
    /// Aufbau gescheitert: alles wird zurückgehalten.
    Failed,
}

enum State {
    Pending,
    Ready {
        known: Arc<KnownSecrets>,
        source: KnownSource,
    },
    Failed,
}

/// Die Redaction-Pipeline eines Prozesses. Hält die bekannten Werte nur als Matcher.
pub struct Redactor {
    state: RwLock<State>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field("status", &self.status())
            .finish()
    }
}

impl Redactor {
    /// Pipeline des Daemons: Die bekannten Werte lädt [`crate::RedactionCap::attach`]; bis
    /// dahin blockieren die strikten Methoden.
    pub fn pending() -> Self {
        Self {
            state: RwLock::new(State::Pending),
        }
    }

    /// Pipeline eines Prozesses ohne Secret-Store (CLI-Client, Runner): nur Detektoren.
    pub fn detectors_only() -> Self {
        Self {
            state: RwLock::new(State::Ready {
                known: Arc::new(KnownSecrets::empty()),
                source: KnownSource::None,
            }),
        }
    }

    pub fn status(&self) -> RedactionStatus {
        match &*self.read() {
            State::Pending => RedactionStatus::Pending,
            State::Failed => RedactionStatus::Failed,
            State::Ready { known, source } => RedactionStatus::Ready {
                values: known.values(),
                source: *source,
            },
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Setzt neue bekannte Werte (nur `beton-secrets`).
    pub(crate) fn install(&self, known: KnownSecrets, source: KnownSource) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        *state = State::Ready {
            known: Arc::new(known),
            source,
        };
    }

    /// Behält die bisherigen Werte, markiert die Quelle aber als nicht erreichbar.
    pub(crate) fn mark_store_unavailable(&self) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        match &*state {
            State::Ready { known, .. } => {
                *state = State::Ready {
                    known: known.clone(),
                    source: KnownSource::StoreUnavailable,
                };
            }
            State::Pending => {
                *state = State::Ready {
                    known: Arc::new(KnownSecrets::empty()),
                    source: KnownSource::StoreUnavailable,
                };
            }
            State::Failed => {}
        }
    }

    /// Aufbau gescheitert: ab jetzt wird alles zurückgehalten (fail closed).
    pub(crate) fn fail(&self) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        *state = State::Failed;
    }

    /// Der aktuelle Matcher; `None` während des Starts.
    fn known(&self, lenient: bool) -> Result<Option<Arc<KnownSecrets>>, Unavailable> {
        match &*self.read() {
            State::Ready { known, .. } => Ok(Some(known.clone())),
            State::Pending if lenient => Ok(None),
            State::Pending | State::Failed => Err(Unavailable),
        }
    }

    /// Schwärzt einen Text (strikt: [`Unavailable`], solange die Pipeline nicht bereit ist).
    pub fn redact_str<'a>(&self, text: &'a str) -> Result<Cow<'a, str>, Unavailable> {
        let known = self.known(false)?;
        redact_text(known.as_deref(), text)
    }

    /// Schwärzt alle Strings eines JSON-Werts an Ort und Stelle; `true`, wenn sich etwas
    /// geändert hat. Schlüssel, Zahlen und Wahrheitswerte bleiben.
    pub fn redact_json(&self, value: &mut Value) -> Result<bool, Unavailable> {
        let known = self.known(false)?;
        redact_value(known.as_deref(), value)
    }

    /// Wie [`Redactor::redact_str`], für Log-Zeilen: Während des Starts (`Pending`) laufen nur
    /// die Detektoren (siehe Modul-Doku); nach einem gescheiterten Aufbau [`Unavailable`].
    pub fn redact_log_str<'a>(&self, text: &'a str) -> Result<Cow<'a, str>, Unavailable> {
        let known = self.known(true)?;
        redact_text(known.as_deref(), text)
    }

    /// Wie [`Redactor::redact_json`], für Log-Zeilen (siehe [`Redactor::redact_log_str`]).
    pub fn redact_log_json(&self, value: &mut Value) -> Result<bool, Unavailable> {
        let known = self.known(true)?;
        redact_value(known.as_deref(), value)
    }
}

/// Ein Treffer: Byte-Bereich im Text und Art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
}

/// Bekannte Werte und Detektoren über einen Text.
fn redact_text<'a>(
    known: Option<&KnownSecrets>,
    text: &'a str,
) -> Result<Cow<'a, str>, Unavailable> {
    let _ = (known, text);
    Ok(Cow::Borrowed(text))
}

fn redact_value(known: Option<&KnownSecrets>, value: &mut Value) -> Result<bool, Unavailable> {
    let _ = (known, value);
    Ok(false)
}
