//! Tests des verschlüsselten Datei-Stores (SEC-003).

use std::path::PathBuf;

use super::file::{FileBackend, KdfParams};
use super::{Backend, BackendKind};
use crate::{Error, SecretRef, SecretValue};

/// Kleine Parameter, damit Tests schnell sind; die Spec-Parameter prüft ein eigener Test.
const FAST: KdfParams = KdfParams {
    m_kib: 64,
    t: 1,
    p: 1,
};

fn r(account: &str) -> SecretRef {
    SecretRef::user("openai", account, None).unwrap()
}

fn pw(s: &str) -> SecretValue {
    SecretValue::from(s)
}

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets").join("store.v1");
    Fixture { _dir: dir, path }
}

fn open(f: &Fixture) -> FileBackend {
    FileBackend::new(f.path.clone(), FAST)
}

fn get_text(b: &FileBackend, r: &SecretRef) -> Result<String, Error> {
    b.get(r)
        .map(|v| String::from_utf8(v.expose().to_vec()).unwrap())
}

#[test]
fn sec_003_round_trip_and_locked_until_unlocked() {
    let f = fixture();
    let b = open(&f);
    assert_eq!(b.kind(), BackendKind::File);
    assert!(b.locked());
    assert_eq!(b.get(&r("a")).unwrap_err().code(), "secrets_locked");
    assert_eq!(
        b.put(&r("a"), &pw("x")).unwrap_err().code(),
        "secrets_locked"
    );
    b.unlock(&pw("richtig gutes passwort")).unwrap();
    assert!(!b.locked());
    b.put(&r("a"), &pw("wert-a")).unwrap();
    b.put(&r("b"), &pw("wert-b")).unwrap();
    assert_eq!(get_text(&b, &r("a")).unwrap(), "wert-a");
    assert!(b.delete(&r("a")).unwrap());
    assert!(!b.delete(&r("a")).unwrap());
    assert_eq!(b.get(&r("a")).unwrap_err().code(), "not_found");

    // Neuer Prozess: gesperrt, bis die Passphrase kommt.
    let again = open(&f);
    assert!(again.locked());
    assert_eq!(again.get(&r("b")).unwrap_err().code(), "secrets_locked");
    again.unlock(&pw("richtig gutes passwort")).unwrap();
    assert_eq!(get_text(&again, &r("b")).unwrap(), "wert-b");
}

#[test]
fn sec_003_file_is_private_and_holds_no_plaintext() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("pass")).unwrap();
    b.put(&r("a"), &pw("MARKER-sk-0123456789")).unwrap();
    let bytes = std::fs::read(&f.path).unwrap();
    assert!(
        !bytes.windows(b"MARKER".len()).any(|w| w == b"MARKER"),
        "Klartext in der Datei"
    );
    assert!(
        !bytes.windows(b"openai".len()).any(|w| w == b"openai"),
        "auch die Refs sind verschlüsselt"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir = std::fs::metadata(f.path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir, 0o700);
    }
}

#[test]
fn sec_003_ac1_wrong_passphrase_is_locked_without_hint_and_file_unchanged() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("richtig")).unwrap();
    b.put(&r("a"), &pw("wert")).unwrap();
    let before = std::fs::read(&f.path).unwrap();

    let other = open(&f);
    let err = other.unlock(&pw("falsch")).unwrap_err();
    assert_eq!(err.code(), "secrets_locked");
    // Dieselbe Meldung wie „gesperrt“: kein Hinweis, welcher Teil falsch war.
    assert_eq!(err.to_string(), Error::Locked.to_string());
    assert!(other.locked());
    assert_eq!(other.get(&r("a")).unwrap_err().code(), "secrets_locked");
    assert_eq!(std::fs::read(&f.path).unwrap(), before, "Datei unverändert");
}

#[test]
fn sec_003_ac2_any_flipped_bit_is_an_integrity_error() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("pass")).unwrap();
    b.put(&r("a"), &pw("wert")).unwrap();
    let good = std::fs::read(&f.path).unwrap();
    for pos in 0..good.len() {
        let mut bad = good.clone();
        bad[pos] ^= 1 << (pos % 8);
        std::fs::write(&f.path, &bad).unwrap();
        // Beim Entsperren …
        let err = open(&f).unlock(&pw("pass")).unwrap_err();
        assert_eq!(err.code(), "secrets_corrupt", "Byte {pos}: {err}");
        // … und beim Lesen mit bereits entsperrtem Schlüssel.
        let err = b.get(&r("a")).unwrap_err();
        assert_eq!(err.code(), "secrets_corrupt", "Byte {pos}: {err}");
    }
    std::fs::write(&f.path, &good).unwrap();
    assert_eq!(get_text(&b, &r("a")).unwrap(), "wert");
    // Abgeschnittene Datei ebenso.
    std::fs::write(&f.path, &good[..good.len() - 1]).unwrap();
    assert_eq!(b.get(&r("a")).unwrap_err().code(), "secrets_corrupt");
}

#[test]
fn sec_003_ac3_passphrase_change_reencrypts_and_old_one_stops_working() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("alt")).unwrap();
    b.put(&r("a"), &pw("wert")).unwrap();
    let before = std::fs::read(&f.path).unwrap();

    assert_eq!(
        b.change_passphrase(&pw("falsch"), &pw("neu"))
            .unwrap_err()
            .code(),
        "secrets_locked"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);

    b.change_passphrase(&pw("alt"), &pw("neu")).unwrap();
    let after = std::fs::read(&f.path).unwrap();
    assert_ne!(before, after);
    // Neuer Salt: der Header unterscheidet sich schon vor dem Chiffrat.
    assert_ne!(before[..38], after[..38]);
    assert_eq!(get_text(&b, &r("a")).unwrap(), "wert");

    assert_eq!(
        open(&f).unlock(&pw("alt")).unwrap_err().code(),
        "secrets_locked"
    );
    let again = open(&f);
    again.unlock(&pw("neu")).unwrap();
    assert_eq!(get_text(&again, &r("a")).unwrap(), "wert");
}

#[test]
fn sec_003_each_write_uses_a_new_nonce() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("pass")).unwrap();
    b.put(&r("a"), &pw("wert")).unwrap();
    let first = std::fs::read(&f.path).unwrap();
    b.put(&r("a"), &pw("wert")).unwrap();
    let second = std::fs::read(&f.path).unwrap();
    assert_eq!(first.len(), second.len());
    assert_eq!(
        first[..54],
        second[..54],
        "Header (Parameter, Salt, KCV) gleich"
    );
    assert_ne!(first[54..66], second[54..66], "Nonce neu");
    assert_ne!(first[66..], second[66..]);
}

#[test]
fn sec_003_default_parameters_match_the_spec_and_are_in_the_header() {
    assert_eq!(
        KdfParams::default(),
        KdfParams {
            m_kib: 64 * 1024,
            t: 3,
            p: 1
        }
    );
    let f = fixture();
    let b = FileBackend::new(f.path.clone(), KdfParams::default());
    b.unlock(&pw("pass")).unwrap();
    let bytes = std::fs::read(&f.path).unwrap();
    assert_eq!(&bytes[..8], b"BETONSV1");
    assert_eq!(bytes[8], 1, "Formatversion");
    assert_eq!(bytes[9], 1, "Argon2id");
    assert_eq!(u32::from_be_bytes(bytes[10..14].try_into().unwrap()), 65536);
    assert_eq!(u32::from_be_bytes(bytes[14..18].try_into().unwrap()), 3);
    assert_eq!(u32::from_be_bytes(bytes[18..22].try_into().unwrap()), 1);
    // Ein anderer Prozess liest die Parameter aus dem Header, nicht aus seiner Konfiguration.
    let reader = FileBackend::new(f.path.clone(), FAST);
    reader.unlock(&pw("pass")).unwrap();
}

#[test]
fn sec_003_absurd_parameters_in_the_header_are_rejected() {
    let f = fixture();
    let b = open(&f);
    b.unlock(&pw("pass")).unwrap();
    let mut bytes = std::fs::read(&f.path).unwrap();
    // m = 4 TiB: der Header muss abgelehnt werden, bevor Argon2 Speicher anfordert.
    bytes[10..14].copy_from_slice(&u32::MAX.to_be_bytes());
    let n = bytes.len();
    let sum = super::file::checksum(&bytes[..n - 32]);
    bytes[n - 32..].copy_from_slice(&sum);
    std::fs::write(&f.path, &bytes).unwrap();
    assert_eq!(
        open(&f).unlock(&pw("pass")).unwrap_err().code(),
        "secrets_corrupt"
    );
}
