//! `beton-voice`: lokale Spracheingabe mit whisper.cpp (VOI, ADR-0023, ADR-0033).
//!
//! - [`service::VoiceService`] transkribiert im Daemon bzw. zentralen Server: Modell lazy
//!   laden, nach `voice.idle_unload` entladen, gleichzeitige Diktate begrenzen (VOI-001),
//!   Sprache DE/EN erkennen (VOI-006).
//! - [`store::ModelStore`], [`catalog::Catalog`] und [`download`] verwalten Modelle: nur
//!   gepinnte URLs und SHA-256, Download nur mit [`download::Consent`], Import lokaler Dateien
//!   ohne Netzwerk (VOI-002).
//! - [`whisper::WhisperEngine`] wählt Metal bzw. CPU und fällt bei GPU-Fehlern mit Warn-Log auf
//!   die CPU zurück (VOI-003).
//!
//! Netzwerk nutzt nur [`download`], ausgelöst vom User (VOI-008). Audio wird nie gespeichert.

pub mod audio;
pub mod catalog;
pub mod download;
pub mod engine;
pub mod gpu;
pub mod service;
pub mod settings;
pub mod store;
pub mod whisper;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

pub use engine::Backend;
pub use settings::{Acceleration, Lang, LanguageSetting, VoiceSettings};

use crate::service::ModelSource as _;

/// Größe für Menschen wie in den Einstellungen und am Mikrofon-Button: `488 MB`, `1,6 GB`
/// (dezimal, VOI-002 AC3).
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9).replace('.', ",")
    } else {
        format!("{} MB", (bytes as f64 / 1e6).round())
    }
}

/// Default-Modell für diesen Rechner (VOI-002).
pub fn default_model() -> &'static str {
    catalog::Catalog::default_model(false)
}

/// Was `beton doctor` über die Spracheingabe meldet (VOI-003).
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    /// Gewähltes Backend.
    pub backend: Backend,
    /// GPU-Beschreibung (z. B. `Apple M3 Pro`) bzw. Grund für den CPU-Rückfall.
    pub gpu_note: Option<String>,
    /// Die GPU war erwartet, ließ sich aber nicht nutzen.
    pub gpu_fallback: bool,
    /// Installiertes Modell, mit dem gemessen wurde.
    pub model: Option<String>,
    /// Real-Time-Factor der Messung (Rechenzeit ÷ Audiodauer).
    pub rtf: Option<f64>,
    /// Fehler beim Messen.
    pub error: Option<String>,
}

/// Dauer des Testsignals für die RTF-Messung.
const BENCH_SECS: usize = 5;

/// Synthetisches Testsignal (Töne mit Hüllkurve), 16 kHz mono; kein gespeichertes Audio.
pub fn bench_signal(secs: usize) -> Vec<f32> {
    let n = secs * audio::SAMPLE_RATE as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / audio::SAMPLE_RATE as f32;
            let env = (t * std::f32::consts::PI * 2.0).sin().abs();
            let f = 180.0 + 40.0 * (t * 3.0).sin();
            0.2 * env
                * ((2.0 * std::f32::consts::PI * f * t).sin()
                    + 0.5 * (2.0 * std::f32::consts::PI * 2.0 * f * t).sin())
        })
        .collect()
}

/// Backend und RTF für `beton doctor`: misst mit dem installierten Modell (nach `voice.model`)
/// an einem 5-s-Testsignal; ohne Modell nur das Backend. Lädt nichts herunter.
pub fn diagnose(home: &Path, settings: &VoiceSettings) -> Diagnosis {
    let engine = whisper::WhisperEngine::new();
    let (backend, gpu_note) = engine.select_backend(settings.acceleration);
    let gpu_fallback = settings.acceleration == Acceleration::Auto
        && gpu::gpu_expected()
        && backend == Backend::Cpu;
    let mut d = Diagnosis {
        backend,
        gpu_note,
        gpu_fallback,
        model: None,
        rtf: None,
        error: None,
    };
    let catalog = catalog::Catalog::pinned();
    let source = service::StoreSource {
        store: store::ModelStore::in_home(home),
        catalog: catalog.clone(),
        choice: settings.model.clone(),
        default_model: default_model().to_owned(),
    };
    let Ok(path) = source.resolve() else {
        return d;
    };
    d.model = catalog
        .entries()
        .iter()
        .find(|e| path.ends_with(&e.file))
        .map(|e| e.name.clone());
    let engine: Arc<dyn engine::Engine> = Arc::new(engine);
    match engine.load(&path, settings.acceleration) {
        Ok(model) => {
            d.backend = model.backend();
            let pcm = bench_signal(BENCH_SECS);
            let started = Instant::now();
            match model.transcribe(&pcm, Lang::En) {
                Ok(_) => {
                    d.rtf = Some(started.elapsed().as_secs_f64() / BENCH_SECS as f64);
                }
                Err(e) => d.error = Some(e.to_string()),
            }
        }
        Err(e) => d.error = Some(e.to_string()),
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VOI-002 AC3: Beschriftung „Modell laden (≈ X MB)“ mit den Größen der Liste.
    #[test]
    fn voi_002_ac3_sizes_for_the_download_label() {
        assert_eq!(format_size(487_601_967), "488 MB");
        assert_eq!(format_size(574_041_195), "574 MB");
        assert_eq!(format_size(1_624_555_275), "1,6 GB");
    }
}
