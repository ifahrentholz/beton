//! VOI-001 AC2 mit der echten whisper.cpp-Engine und einem synthetischen Modell: Nach
//! `idle_unload` ist der Modellspeicher wieder frei. Eigenes Test-Binary, damit kein anderer
//! Test im selben Prozess den RSS verändert.

#![allow(clippy::unwrap_used)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_voice::service::{FixedSource, ServiceConfig, VoiceService};
use beton_voice::whisper::WhisperEngine;
use beton_voice::{Acceleration, Lang, LanguageSetting};
use support::{Dims, synthetic_model, tone};

fn rss() -> u64 {
    memory_stats::memory_stats().unwrap().physical_mem as u64
}

/// VOI-001 AC2: Nach `idle_unload` ist der Modellspeicher freigegeben – der RSS sinkt um
/// mindestens 80 % der Modellgröße.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voi_001_ac2_idle_unload_releases_model_memory() {
    let model = synthetic_model(Dims::LARGE);
    let svc = VoiceService::new(
        Arc::new(WhisperEngine::new()),
        Arc::new(FixedSource(model.path.clone())),
        ServiceConfig {
            idle_unload: Duration::from_millis(500),
            max_concurrent: 1,
            language: LanguageSetting::Auto,
            languages: vec![Lang::De, Lang::En],
            acceleration: Acceleration::Cpu,
        },
    );
    svc.transcribe(tone(2), |_| {}).await.unwrap();
    assert!(svc.is_loaded().await);
    let loaded = rss();
    let deadline = Instant::now() + Duration::from_secs(10);
    while svc.is_loaded().await {
        assert!(Instant::now() < deadline, "Modell wurde nicht entladen");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Freigabe läuft in spawn_blocking; kurz Zeit lassen.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = rss();
    let freed = loaded.saturating_sub(after);
    eprintln!(
        "RSS geladen {loaded}, entladen {after}, Modell {}",
        model.weight_bytes
    );
    let need = model.weight_bytes * 8 / 10;
    assert!(
        freed >= need,
        "RSS {loaded} → {after}: {freed} Bytes frei, nötig ≥ {need} (Modell {} Bytes)",
        model.weight_bytes
    );
}
