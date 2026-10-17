//! `beton voice models …` und der Check `voice` in `beton doctor` (VOI-002, VOI-003). Kein Test
//! lädt etwas herunter (ADR-0033); Downloads prüfen die Unit-Tests von `beton-voice` gegen einen
//! Mock-Server auf Loopback.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use beton_cli::doctor::{CheckStatus, DoctorReport};
use beton_voice::store::ModelInfo;
use common::{beton, run, stderr, stdout};

fn model_files(home: &Path) -> Vec<String> {
    let dir = home.join("models/whisper");
    let mut out = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(&dir).unwrap().display().to_string());
            }
        }
    }
    out
}

/// VOI-002 AC4: `beton voice models list --json` liefert Name, Größe, installiert ja/nein und
/// Prüfsumme.
#[test]
fn voi_002_ac4_models_list_json_has_name_size_installed_and_checksum() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args(["voice", "models", "list", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    // Gegen das veröffentlichte Schema (deny_unknown_fields).
    let models: Vec<ModelInfo> = serde_json::from_str(&stdout(&out)).unwrap();
    let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
    for name in ["small", "medium", "large-v3-turbo", "large-v3-turbo-q5_0"] {
        assert!(names.contains(&name), "{names:?}");
    }
    assert!(
        models
            .iter()
            .all(|m| !m.installed && m.size_bytes > 0 && m.sha256.len() == 64)
    );
    let raw: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    for m in raw.as_array().unwrap() {
        for key in ["name", "size_bytes", "installed", "sha256"] {
            assert!(m.get(key).is_some(), "{key} fehlt in {m}");
        }
    }

    // Eine vollständig vorhandene Datei gilt als installiert.
    let small = models.iter().find(|m| m.name == "small-q5_1").unwrap();
    let dir = home.path().join("models/whisper");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join(&small.file))
        .unwrap()
        .set_len(small.size_bytes)
        .unwrap();
    let out = run(beton(home.path()).args(["voice", "models", "list", "--json"]));
    let models: Vec<ModelInfo> = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        models
            .iter()
            .find(|m| m.name == "small-q5_1")
            .unwrap()
            .installed
    );
    assert!(!models.iter().find(|m| m.name == "small").unwrap().installed);
}

/// VOI-002 AC3: Ohne Zustimmung (kein `--yes`, kein Terminal) findet kein Download statt; die
/// Meldung nennt Größe und beide Wege.
#[test]
fn voi_002_ac3_pull_without_consent_downloads_nothing() {
    let home = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let out = run(beton(home.path()).args(["voice", "models", "pull", "small"]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("Zustimmung"), "{err}");
    assert!(err.contains("≈ 488 MB"), "{err}");
    assert!(err.contains("--yes") && err.contains("import"), "{err}");
    assert!(
        model_files(home.path()).is_empty(),
        "{:?}",
        model_files(home.path())
    );
    assert!(started.elapsed() < Duration::from_secs(20));
}

/// VOI-002 AC5 / VOI-008 AC3: Nur Namen der gepinnten Liste; eine URL ist kein Modell.
#[test]
fn voi_008_ac3_pull_accepts_only_names_from_the_pinned_list() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args([
        "voice",
        "models",
        "pull",
        "https://example.invalid/ggml-small.bin",
        "--yes",
    ]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("unbekanntes Modell"),
        "{}",
        stderr(&out)
    );
    assert!(model_files(home.path()).is_empty());
}

/// VOI-002 AC5: Eine Datei ohne passenden Eintrag in der gepinnten Liste wird abgelehnt; im
/// Modellverzeichnis bleibt nichts liegen. Den Erfolgsfall prüft `beton-voice` (store.rs).
#[test]
fn voi_002_ac5_import_rejects_files_outside_the_pinned_list() {
    let home = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let file = src.path().join("ggml-small.bin");
    std::fs::write(&file, b"kein echtes Modell").unwrap();
    let out = run(beton(home.path())
        .args(["voice", "models", "import"])
        .arg(&file));
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("passt zu keinem Modell"),
        "{}",
        stderr(&out)
    );
    assert!(
        model_files(home.path()).is_empty(),
        "{:?}",
        model_files(home.path())
    );
    assert!(file.exists());
}

#[test]
fn voi_002_remove_of_a_missing_model_is_a_usage_error() {
    let home = tempfile::tempdir().unwrap();
    let out = run(beton(home.path()).args(["voice", "models", "remove", "small"]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("nicht installiert"),
        "{}",
        stderr(&out)
    );
}

fn doctor_voice(home: &Path) -> beton_cli::doctor::Check {
    let out = run(beton(home).args(["doctor", "--json"]));
    let report: DoctorReport = serde_json::from_str(&stdout(&out)).unwrap();
    report.checks.into_iter().find(|c| c.id == "voice").unwrap()
}

/// VOI-003 AC1: Auf Apple Silicon meldet `beton doctor` `voice.backend = metal`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn voi_003_ac1_doctor_reports_metal_on_apple_silicon() {
    let home = tempfile::tempdir().unwrap();
    let c = doctor_voice(home.path());
    assert_eq!(c.details.as_ref().unwrap()["backend"], "metal", "{c:?}");
    assert_eq!(c.status, CheckStatus::Ok, "{c:?}");
}

/// VOI-003: `voice.acceleration: cpu` in der Konfiguration erzwingt die CPU, ohne Warnung.
#[test]
fn voi_003_doctor_honours_acceleration_cpu() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.yaml"),
        "voice:\n  acceleration: cpu\n  language: de\n",
    )
    .unwrap();
    let c = doctor_voice(home.path());
    assert_eq!(c.details.as_ref().unwrap()["backend"], "cpu", "{c:?}");
    assert_eq!(c.status, CheckStatus::Ok, "{c:?}");
    assert!(c.message.contains("kein Modell installiert"), "{c:?}");
}

/// `voice.language` außerhalb von `voice.languages` ist ein Konfigurationsfehler.
#[test]
fn voi_006_invalid_voice_settings_are_reported() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.yaml"),
        "voice:\n  language: en\n  languages: [de]\n",
    )
    .unwrap();
    let out = run(beton(home.path()).args(["doctor", "--json"]));
    let report: DoctorReport = serde_json::from_str(&stdout(&out)).unwrap();
    let config = report.checks.iter().find(|c| c.id == "config").unwrap();
    assert_eq!(config.status, CheckStatus::Fail);
    assert!(config.message.contains("voice"), "{}", config.message);
}
