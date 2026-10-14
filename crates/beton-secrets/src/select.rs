//! Wahl des Backends beim Start des Daemons (SEC-002, SEC-003).
//!
//! `auto` (Default): OS-Keychain, wenn verfügbar, sonst die verschlüsselte Datei – nie
//! Klartext. `keychain` bzw. `file` erzwingen ein Backend. Der Datei-Store startet gesperrt;
//! `BETON_SECRETS_PASSPHRASE_FILE` entsperrt ihn ohne Rückfrage (Headless, CI).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::SecretValue;
use crate::backend::keychain::Availability;
use crate::backend::{Backend, FileBackend, KdfParams, KeychainBackend, NoBackend};
use crate::error::{Error, Result};

/// Env-Variable mit dem Pfad einer Datei, die die Passphrase des Datei-Stores enthält.
pub const PASSPHRASE_FILE_ENV: &str = "BETON_SECRETS_PASSPHRASE_FILE";

/// Konfiguration `secrets.backend`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BackendChoice {
    /// OS-Keychain, sonst verschlüsselte Datei.
    #[default]
    Auto,
    /// Nur der OS-Keychain; ohne Keychain sind Secrets nicht verfügbar (fail closed).
    Keychain,
    /// Nur die verschlüsselte Datei `~/.beton/secrets/store.v1`.
    File,
}

/// Ergebnis der Planung ohne Seiteneffekte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Keychain,
    /// Datei-Store; `reason` erklärt einen Fallback, z. B. „Keychain nicht verfügbar“.
    File {
        reason: Option<String>,
    },
    /// Kein Backend (fail closed), mit Grund.
    None {
        reason: String,
    },
}

/// Welches Backend für `choice` bei gegebener Keychain-Verfügbarkeit?
pub fn plan(choice: BackendChoice, keychain: &Availability) -> Plan {
    match (choice, keychain) {
        (BackendChoice::File, _) => Plan::File { reason: None },
        (BackendChoice::Keychain | BackendChoice::Auto, Availability::Available) => Plan::Keychain,
        (BackendChoice::Auto, Availability::Unavailable(why)) => Plan::File {
            reason: Some(format!("Keychain nicht verfügbar ({why}) → Datei-Store")),
        },
        (BackendChoice::Keychain, Availability::Unavailable(why)) => Plan::None {
            reason: format!("Keychain nicht verfügbar ({why}); secrets.backend ist keychain"),
        },
    }
}

/// Das gewählte Backend samt Hinweisen für Log und `beton doctor`.
pub struct Selected {
    pub backend: Arc<dyn Backend>,
    /// Hinweise (Fallback, gesperrt, Passphrase-Datei); enthalten nie Werte.
    pub notes: Vec<String>,
    /// Ergebnis eines Entsperrens über `BETON_SECRETS_PASSPHRASE_FILE` (für das Audit-Log):
    /// `Some(Ok)` entsperrt, `Some(Err(code))` gescheitert, `None` nicht versucht.
    pub passphrase_file: Option<Result<(), &'static str>>,
}

impl std::fmt::Debug for Selected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selected")
            .field("backend", &self.backend.kind())
            .field("notes", &self.notes)
            .finish()
    }
}

/// Liest die Passphrase aus einer Datei (ohne abschließenden Zeilenumbruch). Unter Unix muss
/// die Datei nur für den Eigentümer lesbar sein.
pub fn read_passphrase_file(path: &Path) -> Result<SecretValue> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(Error::Invalid(format!(
                "{}: Rechte {:o}; die Passphrase-Datei darf nur für den Eigentümer lesbar sein (chmod 600)",
                path.display(),
                mode & 0o777
            )));
        }
    }
    let mut bytes =
        std::fs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        bytes.pop();
    }
    Ok(SecretValue::from_bytes(bytes))
}

/// Baut das Backend für `beton serve`.
pub fn select(choice: BackendChoice, home: &Path, passphrase_file: Option<PathBuf>) -> Selected {
    select_with(
        choice,
        &KeychainBackend::availability(),
        home,
        passphrase_file,
        KdfParams::default(),
    )
}

/// Wie [`select`], mit vorgegebener Verfügbarkeit und KDF-Parametern (Tests).
pub fn select_with(
    choice: BackendChoice,
    keychain: &Availability,
    home: &Path,
    passphrase_file: Option<PathBuf>,
    params: KdfParams,
) -> Selected {
    let mut notes = Vec::new();
    let file_reason = match plan(choice, keychain) {
        Plan::Keychain => match KeychainBackend::open() {
            Ok(k) => {
                return Selected {
                    backend: Arc::new(k),
                    notes,
                    passphrase_file: None,
                };
            }
            Err(e) if choice == BackendChoice::Auto => {
                Some(format!("Keychain nicht verfügbar ({e}) → Datei-Store"))
            }
            Err(e) => {
                notes.push(format!("Secrets nicht verfügbar: {e}"));
                return Selected {
                    backend: Arc::new(NoBackend),
                    notes,
                    passphrase_file: None,
                };
            }
        },
        Plan::File { reason } => reason,
        Plan::None { reason } => {
            notes.push(format!("Secrets nicht verfügbar: {reason}"));
            return Selected {
                backend: Arc::new(NoBackend),
                notes,
                passphrase_file: None,
            };
        }
    };
    notes.extend(file_reason);
    let file = FileBackend::new(FileBackend::default_path(home), params);
    let mut outcome = None;
    if let Some(path) = passphrase_file {
        match read_passphrase_file(&path).and_then(|p| file.unlock(&p)) {
            Ok(()) => {
                notes.push(format!("Datei-Store über {PASSPHRASE_FILE_ENV} entsperrt"));
                outcome = Some(Ok(()));
            }
            Err(e) => {
                notes.push(format!(
                    "Datei-Store bleibt gesperrt ({PASSPHRASE_FILE_ENV}): {}",
                    e.code()
                ));
                outcome = Some(Err(e.code()));
            }
        }
    } else if file.locked() {
        notes.push("Datei-Store gesperrt: beton secrets unlock".into());
    }
    Selected {
        backend: Arc::new(file),
        notes,
        passphrase_file: outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;

    const FAST: KdfParams = KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    };

    fn unavailable() -> Availability {
        Availability::Unavailable("keine D-Bus-Session (Secret Service)".into())
    }

    #[test]
    fn sec_002_ac2_auto_without_keychain_falls_back_to_the_file_store() {
        assert_eq!(
            plan(BackendChoice::Auto, &unavailable()),
            Plan::File {
                reason: Some(
                    "Keychain nicht verfügbar (keine D-Bus-Session (Secret Service)) → Datei-Store"
                        .into()
                )
            }
        );
        assert_eq!(
            plan(BackendChoice::Auto, &Availability::Available),
            Plan::Keychain
        );
        assert_eq!(
            plan(BackendChoice::File, &Availability::Available),
            Plan::File { reason: None }
        );
        // Ausdrücklich Keychain, aber keiner da: kein stiller Wechsel, sondern fail closed.
        assert!(matches!(
            plan(BackendChoice::Keychain, &unavailable()),
            Plan::None { .. }
        ));
        let home = tempfile::tempdir().unwrap();
        let s = select_with(BackendChoice::Auto, &unavailable(), home.path(), None, FAST);
        assert_eq!(s.backend.kind(), BackendKind::File);
        assert!(
            s.notes[0].contains("Keychain nicht verfügbar"),
            "{:?}",
            s.notes
        );
        assert!(s.backend.locked());
        let s = select_with(
            BackendChoice::Keychain,
            &unavailable(),
            home.path(),
            None,
            FAST,
        );
        assert_eq!(s.backend.kind(), BackendKind::None);
    }

    #[test]
    fn sec_003_passphrase_file_unlocks_headless() {
        let home = tempfile::tempdir().unwrap();
        let pass = home.path().join("pass");
        std::fs::write(&pass, "headless-passphrase\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o644)).unwrap();
            let s = select_with(
                BackendChoice::File,
                &unavailable(),
                home.path(),
                Some(pass.clone()),
                FAST,
            );
            assert!(
                s.backend.locked(),
                "zu offene Passphrase-Datei wird abgelehnt"
            );
            std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let s = select_with(
            BackendChoice::File,
            &unavailable(),
            home.path(),
            Some(pass.clone()),
            FAST,
        );
        assert!(!s.backend.locked(), "{:?}", s.notes);
        // Dieselbe Datei entsperrt den Store auch beim nächsten Start (ohne Zeilenumbruch).
        let again = FileBackend::new(FileBackend::default_path(home.path()), FAST);
        again
            .unlock(&SecretValue::from("headless-passphrase"))
            .unwrap();
        // Falsche Passphrase: gesperrt, Hinweis nur mit Code.
        std::fs::write(&pass, "falsch").unwrap();
        let s = select_with(
            BackendChoice::File,
            &unavailable(),
            home.path(),
            Some(pass),
            FAST,
        );
        assert!(s.backend.locked());
        assert!(
            s.notes.iter().any(|n| n.contains("secrets_locked")),
            "{:?}",
            s.notes
        );
        assert!(!s.notes.iter().any(|n| n.contains("falsch")));
    }
}
