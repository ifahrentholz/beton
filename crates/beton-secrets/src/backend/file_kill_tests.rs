//! SEC-003 AC4: Kill-Test. Ein Kindprozess schreibt einen neuen Wert in den Datei-Store und
//! wird mitten im Schreiben hart beendet (SIGKILL bzw. TerminateProcess). Danach lässt sich
//! die Datei immer entsperren und enthält entweder den alten oder den neuen Wert.
//!
//! Damit der Kill sicher *im* Schreibvorgang landet, hält der Test-Build an einer gewählten
//! Stelle von `write_atomic` an (`file::kill_hook`): mitten in der Temp-Datei (Phase 1), nach
//! `fsync` vor `rename` (Phase 2), nach `rename` vor dem `fsync` des Verzeichnisses (Phase 3).
//! Dazu kommen Runden mit Kill zu zufälligen Zeitpunkten.
//!
//! Der Kindprozess ist dieselbe Test-Binary mit dem ignorierten Test `kill_child_writer`.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::Backend;
use super::file::{FileBackend, KdfParams, PHASE_ENV};
use crate::{SecretRef, SecretValue};

const PATH_ENV: &str = "BETON_SECRETS_KILL_PATH";
const START_ENV: &str = "BETON_SECRETS_KILL_START";
const PASS: &str = "kill-test-passphrase";
const FAST: KdfParams = KdfParams {
    m_kib: 64,
    t: 1,
    p: 1,
};

fn key() -> SecretRef {
    SecretRef::user("openai", "default", None).unwrap()
}

fn value(i: u64) -> String {
    format!("{i:020}-{}", "x".repeat(64 * 1024))
}

fn number(v: &SecretValue) -> u64 {
    std::str::from_utf8(&v.expose()[..20])
        .unwrap()
        .parse()
        .unwrap()
}

/// Kindprozess: schreibt neue Werte, bis er getötet wird.
#[test]
#[ignore = "wird nur als Kindprozess von sec_003_ac4_kill_leaves_old_or_new_file gestartet"]
fn kill_child_writer() {
    let (Some(path), Some(start)) = (std::env::var_os(PATH_ENV), std::env::var_os(START_ENV))
    else {
        return;
    };
    let start: u64 = start.to_str().unwrap().parse().unwrap();
    let backend = FileBackend::new(PathBuf::from(path), FAST);
    backend.unlock(&SecretValue::from(PASS)).unwrap();
    println!("bereit");
    let mut i = start;
    loop {
        i += 1;
        backend.put(&key(), &SecretValue::from(value(i))).unwrap();
        println!("ACK {i}");
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Test: eigener Prozess für den Kill-Test"
)]
fn spawn(path: &Path, start: u64, phase: Option<u8>) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "backend::file_kill_tests::kill_child_writer",
        "--ignored",
        "--nocapture",
    ])
    .env(PATH_ENV, path)
    .env(START_ENV, start.to_string())
    .env_remove(PHASE_ENV)
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    if let Some(p) = phase {
        cmd.env(PHASE_ENV, p.to_string());
    }
    cmd.spawn().unwrap()
}

fn read_value(path: &Path, round: &str) -> u64 {
    let reader = FileBackend::new(path.to_path_buf(), FAST);
    reader
        .unlock(&SecretValue::from(PASS))
        .unwrap_or_else(|e| panic!("{round}: Datei kaputt: {e}"));
    number(&reader.get(&key()).unwrap())
}

#[test]
fn sec_003_ac4_kill_leaves_old_or_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets").join("store.v1");
    let setup = FileBackend::new(path.clone(), FAST);
    setup.unlock(&SecretValue::from(PASS)).unwrap();
    setup.put(&key(), &SecretValue::from(value(0))).unwrap();
    let mut current = 0;

    // Kill an festen Stellen des Schreibvorgangs: vor `rename` bleibt der alte Wert, danach
    // gilt der neue.
    for (phase, expect_new) in [(1u8, false), (2, false), (3, true)] {
        let mut child = spawn(&path, current, Some(phase));
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let reached = lines
            .by_ref()
            .map_while(Result::ok)
            .any(|l| l.trim() == format!("PHASE {phase}"));
        assert!(reached, "Phase {phase} nicht erreicht");
        child.kill().unwrap();
        child.wait().unwrap();
        let found = read_value(&path, &format!("Phase {phase}"));
        let expected = if expect_new { current + 1 } else { current };
        assert_eq!(found, expected, "Phase {phase}");
        current = found;
    }

    // Kill zu pseudozufälligen Zeitpunkten.
    let rounds: u32 = std::env::var("BETON_SECRETS_KILL_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let mut seed: u64 = 0x5eed;
    for round in 0..rounds {
        let mut child = spawn(&path, current, None);
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        for line in lines.by_ref().map_while(Result::ok) {
            if line.trim() == "bereit" {
                break;
            }
        }
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        std::thread::sleep(Duration::from_micros((seed >> 33) % 20_000));
        child.kill().unwrap();
        child.wait().unwrap();
        let mut acked = current;
        for line in lines.map_while(Result::ok) {
            if let Some(n) = line.trim().strip_prefix("ACK ") {
                acked = n.parse().unwrap();
            }
        }
        let found = read_value(&path, &format!("Runde {round}"));
        assert!(
            found == acked || found == acked + 1,
            "Runde {round}: gefunden {found}, bestätigt {acked}"
        );
        current = found;
    }
}
