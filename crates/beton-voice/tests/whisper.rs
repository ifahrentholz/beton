//! Die echte whisper.cpp-Engine mit einem synthetischen Modell (ohne Download, ADR-0033):
//! Laden und Transkribieren (VOI-001), Backend-Wahl und CPU-Rückfall (VOI-003). Den
//! Speicher nach dem Entladen misst `tests/memory.rs` in einem eigenen Prozess.

#![allow(clippy::unwrap_used)]

mod support;

use std::sync::Arc;

use beton_voice::engine::{Backend, Engine};
use beton_voice::gpu::{GpuDevice, GpuProbe};
use beton_voice::whisper::WhisperEngine;
use beton_voice::{Acceleration, Lang};
use support::{Dims, LogBuffer, synthetic_model, tone};

#[test]
fn synthetic_model_loads_and_transcribes_on_cpu() {
    let model = synthetic_model(Dims::SMALL);
    let loaded = WhisperEngine::new()
        .load(&model.path, Acceleration::Cpu)
        .unwrap();
    assert_eq!(loaded.backend(), Backend::Cpu);
    let probs = loaded.detect_language(&tone(2)).unwrap();
    assert!(probs.iter().any(|(c, _)| c == "de"));
    assert!(probs.iter().any(|(c, _)| c == "fr"));
    loaded.transcribe(&tone(2), Lang::De).unwrap();
}

/// GPU-Probe, die eine fehlschlagende Initialisierung simuliert.
struct FailingGpu;

impl GpuProbe for FailingGpu {
    fn probe(&self) -> Result<Option<GpuDevice>, String> {
        Err("simulierter Fehler bei der Metal-Initialisierung".into())
    }
}

/// VOI-003 AC2: Schlägt die GPU-Initialisierung fehl (simuliert), läuft die Transkription
/// auf der CPU, und ein Warn-Log wird geschrieben.
#[test]
fn voi_003_ac2_gpu_init_failure_falls_back_to_cpu_with_warning() {
    let model = synthetic_model(Dims::SMALL);
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    let engine = WhisperEngine::with_probe(Arc::new(FailingGpu));
    tracing::subscriber::with_default(subscriber, || {
        let loaded = engine.load(&model.path, Acceleration::Auto).unwrap();
        assert_eq!(loaded.backend(), Backend::Cpu);
        loaded.transcribe(&tone(2), Lang::En).unwrap();
    });
    let text = logs.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("falling back to cpu"), "{text}");
    assert!(text.contains("simulierter Fehler"), "{text}");
    // `beton doctor` meldet dasselbe Backend.
    let (backend, note) = engine.select_backend(Acceleration::Auto);
    assert_eq!(backend, Backend::Cpu);
    assert!(note.unwrap().contains("simulierter Fehler"));
}

/// VOI-003: `acceleration: cpu` erzwingt die CPU auch mit GPU.
#[test]
fn voi_003_cpu_setting_forces_cpu() {
    let model = synthetic_model(Dims::SMALL);
    let loaded = WhisperEngine::new()
        .load(&model.path, Acceleration::Cpu)
        .unwrap();
    assert_eq!(loaded.backend(), Backend::Cpu);
    assert_eq!(
        WhisperEngine::new().select_backend(Acceleration::Cpu).0,
        Backend::Cpu
    );
}

/// VOI-003 AC1: Auf Apple Silicon rechnet die Engine mit Metal.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn voi_003_ac1_apple_silicon_loads_on_metal() {
    let model = synthetic_model(Dims::SMALL);
    let engine = WhisperEngine::new();
    assert_eq!(engine.select_backend(Acceleration::Auto).0, Backend::Metal);
    let loaded = engine.load(&model.path, Acceleration::Auto).unwrap();
    assert_eq!(loaded.backend(), Backend::Metal);
    loaded.transcribe(&tone(2), Lang::De).unwrap();
}

#[test]
fn word_error_rate_counts_edits_per_reference_word() {
    use support::word_error_rate as wer;
    assert_eq!(wer("Hallo Welt", "hallo, Welt!"), 0.0);
    assert_eq!(wer("eins zwei drei vier", "eins drei vier"), 0.25);
    assert_eq!(wer("eins zwei", "eins zwei drei"), 0.5);
    assert_eq!(wer("eins zwei", "eins drei"), 0.5);
}
