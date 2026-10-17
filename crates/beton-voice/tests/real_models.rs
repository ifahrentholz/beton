//! Nur lokal, nicht in der CI: echte Whisper-Modelle und Sprach-Fixtures.
//!
//! Die CI lädt keine Modelle herunter (ADR-0033), und Sprachaufnahmen mit passender Lizenz
//! liegen nicht im Repo. Deshalb sind diese Tests `#[ignore]` und brauchen:
//!
//! - `BETON_TEST_VOICE_MODELS`: Verzeichnis mit Modellen der gepinnten Liste unter ihrem
//!   Dateinamen (z. B. `~/.beton/models/whisper` nach `beton voice models pull small`);
//! - `BETON_TEST_VOICE_FIXTURES`: Verzeichnis mit `de.wav`, `en.wav`, `fr.wav` aus
//!   `scripts/voice_fixtures.sh` (Referenz-Transkripte: `tests/fixtures/*.txt`);
//! - optional `BETON_TEST_VOICE_MODEL` für die Spracherkennung (Default `small`).
//!
//! `cargo test -p beton-voice --release --test real_models -- --ignored`. Fehlt etwas,
//! scheitert der Test (kein stilles Überspringen).

#![allow(clippy::unwrap_used)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_voice::catalog::Catalog;
use beton_voice::engine::{Backend, Engine};
use beton_voice::service::{FixedSource, ServiceConfig, VoiceService};
use beton_voice::store::ModelStore;
use beton_voice::whisper::WhisperEngine;
use beton_voice::{Acceleration, Lang, LanguageSetting};

fn env_dir(var: &str) -> PathBuf {
    let dir = std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{var} ist nicht gesetzt (siehe Kopf von tests/real_models.rs)"));
    assert!(dir.is_dir(), "{var}={} ist kein Verzeichnis", dir.display());
    dir
}

/// Pfad eines installierten Modells der gepinnten Liste.
fn model(name: &str) -> PathBuf {
    let store = ModelStore::new(env_dir("BETON_TEST_VOICE_MODELS"));
    let catalog = Catalog::pinned();
    let entry = catalog.get(name).unwrap();
    assert!(
        store.is_installed(entry),
        "{} fehlt oder ist unvollständig (beton voice models pull {name})",
        store.path(entry).display()
    );
    store.path(entry)
}

fn fixture(lang: &str) -> (Vec<f32>, String) {
    let dir = env_dir("BETON_TEST_VOICE_FIXTURES");
    let wav = std::fs::read(dir.join(format!("{lang}.wav"))).unwrap();
    let reference = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{lang}.txt")),
    )
    .unwrap();
    (beton_voice::audio::read_wav(&wav).unwrap(), reference)
}

fn service(path: PathBuf, language: LanguageSetting) -> VoiceService {
    VoiceService::new(
        Arc::new(WhisperEngine::new()),
        Arc::new(FixedSource(path)),
        ServiceConfig {
            idle_unload: Duration::from_secs(600),
            max_concurrent: 1,
            language,
            languages: vec![Lang::De, Lang::En],
            acceleration: Acceleration::Auto,
        },
    )
}

fn detection_model() -> String {
    std::env::var("BETON_TEST_VOICE_MODEL").unwrap_or_else(|_| "small".into())
}

/// VOI-001 AC1: Die deutsche 10-s-Fixture wird mit `small` mit WER ≤ 15 % transkribiert.
#[tokio::test]
#[ignore = "nur lokal: echtes Modell small und Fixtures (BETON_TEST_VOICE_MODELS, BETON_TEST_VOICE_FIXTURES)"]
async fn voi_001_ac1_german_fixture_with_small_has_wer_at_most_15_percent() {
    let (pcm, reference) = fixture("de");
    let secs = beton_voice::audio::duration_secs(&pcm);
    assert!((9.0..=11.5).contains(&secs), "Fixture dauert {secs} s");
    let t = service(model("small"), LanguageSetting::Auto)
        .transcribe(pcm, |_| {})
        .await
        .unwrap();
    let wer = support::word_error_rate(&reference, &t.text);
    eprintln!("WER {wer:.3}: {}", t.text);
    assert!(wer <= 0.15, "WER {wer:.3} > 0,15: {}", t.text);
}

/// VOI-006 AC1: Deutsche und englische Fixture werden ohne Vorgabe als `de` bzw. `en` erkannt.
#[tokio::test]
#[ignore = "nur lokal: echtes Modell und Fixtures (BETON_TEST_VOICE_MODELS, BETON_TEST_VOICE_FIXTURES)"]
async fn voi_006_ac1_german_and_english_fixtures_are_detected() {
    let svc = service(model(&detection_model()), LanguageSetting::Auto);
    for (lang, expected) in [("de", Lang::De), ("en", Lang::En)] {
        let (pcm, _) = fixture(lang);
        let t = svc.transcribe(pcm, |_| {}).await.unwrap();
        eprintln!("{lang}: {:?} {}", t.lang, t.text);
        assert!(t.detected);
        assert_eq!(t.lang, expected, "{}", t.text);
    }
}

/// VOI-006 AC2: Die französische Fixture wird als `de` oder `en` transkribiert, nie als `fr`.
#[tokio::test]
#[ignore = "nur lokal: echtes Modell und Fixtures (BETON_TEST_VOICE_MODELS, BETON_TEST_VOICE_FIXTURES)"]
async fn voi_006_ac2_french_fixture_is_transcribed_as_de_or_en() {
    let path = model(&detection_model());
    // Whisper selbst erkennt Französisch …
    let (pcm, _) = fixture("fr");
    let loaded = WhisperEngine::new()
        .load(&path, Acceleration::Auto)
        .unwrap();
    let probs = loaded.detect_language(&pcm).unwrap();
    let top = probs
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0
        .clone();
    assert_eq!(top, "fr", "Fixture taugt nicht als Gegenprobe");
    drop(loaded);
    // … beton wählt trotzdem de oder en.
    let t = service(path, LanguageSetting::Auto)
        .transcribe(pcm, |_| {})
        .await
        .unwrap();
    eprintln!("fr → {:?}: {}", t.lang, t.text);
    assert!(matches!(t.lang, Lang::De | Lang::En));
}

/// VOI-003 AC3: `large-v3-turbo` erreicht auf Apple M-Serie einen Real-Time-Factor ≤ 0,3 für
/// die 10-s-Fixture (Benchmark, kein PR-Gate). Gemessen wird nach einem Aufwärmlauf, also
/// ohne Laden des Modells; im Release-Build.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
#[ignore = "nur lokal: Benchmark mit large-v3-turbo und Fixtures (BETON_TEST_VOICE_MODELS, BETON_TEST_VOICE_FIXTURES)"]
async fn voi_003_ac3_large_v3_turbo_rtf_at_most_0_3_on_apple_silicon() {
    let (pcm, _) = fixture("de");
    let svc = service(model("large-v3-turbo"), LanguageSetting::Auto);
    let warm = svc.transcribe(pcm.clone(), |_| {}).await.unwrap();
    assert_eq!(warm.backend, Backend::Metal);
    let started = Instant::now();
    let t = svc.transcribe(pcm, |_| {}).await.unwrap();
    let rtf = started.elapsed().as_secs_f64() / t.audio_secs;
    eprintln!("RTF {rtf:.3} ({:.2} s Audio): {}", t.audio_secs, t.text);
    assert!(rtf <= 0.3, "RTF {rtf:.3} > 0,3");
}

/// Hilfstest: lädt das Modell mit der Engine direkt (prüft die Umgebung).
#[test]
#[ignore = "nur lokal: echtes Modell (BETON_TEST_VOICE_MODELS)"]
fn real_model_loads() {
    let loaded = WhisperEngine::new()
        .load(&model(&detection_model()), Acceleration::Auto)
        .unwrap();
    eprintln!("Backend {:?}", loaded.backend());
}
