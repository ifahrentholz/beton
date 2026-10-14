//! SEC-002: Keychain-Backend gegen einen echten OS-Keychain.
//!
//! Läuft nur mit `BETON_TEST_KEYCHAIN=1`. Die macOS-CI legt dafür einen **temporären**
//! Keychain an und macht ihn zum Standard des Runners (`.github/workflows/ci.yml`, Job
//! „test (macos)“); `BETON_TEST_KEYCHAIN_PATH` nennt seine Datei. Lokal bleibt der Test aus,
//! damit er nie den Keychain des Entwicklers liest oder verändert.

#![allow(clippy::unwrap_used)] // Testcode: Panics sind hier die Fehlermeldung.

use beton_secrets::backend::{Backend, KeychainBackend};
use beton_secrets::{SecretRef, SecretValue};

fn enabled() -> bool {
    if std::env::var("BETON_TEST_KEYCHAIN").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("übersprungen: BETON_TEST_KEYCHAIN=1 fehlt (nur CI mit temporärem Keychain)");
    false
}

/// Fragt den temporären Keychain direkt über `security` ab (nur macOS).
fn in_temp_keychain(account: &str) -> Option<bool> {
    let path = std::env::var("BETON_TEST_KEYCHAIN_PATH").ok()?;
    let status = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "beton", "-a", account, &path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

#[test]
fn sec_002_keychain_round_trip_in_temporary_keychain() {
    if !enabled() {
        return;
    }
    let backend = KeychainBackend::open().unwrap();
    let account = format!("ci-{}", std::process::id());
    let r = SecretRef::user("openai", &account, None).unwrap();
    let account_name = r.to_string();
    backend
        .put(&r, &SecretValue::from("keychain-test-wert"))
        .unwrap();
    assert_eq!(backend.get(&r).unwrap().expose(), b"keychain-test-wert");
    if let Some(found) = in_temp_keychain(&account_name) {
        assert!(found, "Eintrag liegt im temporären Keychain");
    }
    backend.put(&r, &SecretValue::from("rotiert")).unwrap();
    assert_eq!(backend.get(&r).unwrap().expose(), b"rotiert");
    assert!(backend.delete(&r).unwrap());
    assert!(!backend.delete(&r).unwrap());
    assert_eq!(backend.get(&r).unwrap_err().code(), "not_found");
    if let Some(found) = in_temp_keychain(&account_name) {
        assert!(!found, "Eintrag ist entfernt");
    }
}
