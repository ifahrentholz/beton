//! whisper.cpp als Engine (VOI-001, VOI-003, VOI-006).

use std::path::Path;
use std::sync::{Arc, Once};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::engine::{Backend, Engine, EngineError, LoadedModel};
use crate::gpu::{GgmlProbe, GpuProbe, gpu_expected};
use crate::settings::{Acceleration, Lang};

/// Threads für whisper.cpp: alle Kerne, höchstens 8 (darüber bringt whisper.cpp kaum mehr).
fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(8)
}

/// whisper.cpp und ggml loggen sonst direkt nach stderr; so landen ihre Meldungen in
/// `tracing` (OBS-001).
fn install_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(whisper_rs::install_logging_hooks);
}

/// Die whisper.cpp-Engine.
#[derive(Clone)]
pub struct WhisperEngine {
    probe: Arc<dyn GpuProbe>,
}

impl Default for WhisperEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WhisperEngine {
    pub fn new() -> Self {
        Self::with_probe(Arc::new(GgmlProbe))
    }

    /// Mit eigener GPU-Probe, z. B. einer, die eine fehlschlagende Initialisierung simuliert
    /// (VOI-003 AC2).
    pub fn with_probe(probe: Arc<dyn GpuProbe>) -> Self {
        install_logging();
        Self { probe }
    }

    /// Backend, das [`Engine::load`] für `accel` wählen würde, ohne ein Modell zu laden
    /// (`beton doctor`). Bei Fehlschlag der GPU: CPU samt Grund.
    pub fn select_backend(&self, accel: Acceleration) -> (Backend, Option<String>) {
        if accel == Acceleration::Cpu {
            return (Backend::Cpu, None);
        }
        match self.probe.probe() {
            Ok(Some(dev)) => (dev.backend, Some(dev.description)),
            Ok(None) if gpu_expected() => (Backend::Cpu, Some("kein GPU-Gerät gefunden".into())),
            Ok(None) => (Backend::Cpu, None),
            Err(e) => (Backend::Cpu, Some(e)),
        }
    }
}

fn load_ctx(path: &Path, gpu: bool) -> Result<WhisperContext, EngineError> {
    let mut params = WhisperContextParameters::default();
    params.use_gpu(gpu).flash_attn(gpu);
    WhisperContext::new_with_params(path, params).map_err(|e| EngineError::Load {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

impl Engine for WhisperEngine {
    fn load(&self, path: &Path, accel: Acceleration) -> Result<Box<dyn LoadedModel>, EngineError> {
        let mut backend = Backend::Cpu;
        if accel == Acceleration::Auto {
            match self.probe.probe() {
                Ok(Some(dev)) => backend = dev.backend,
                Ok(None) => {
                    if gpu_expected() {
                        tracing::warn!("no gpu device found, falling back to cpu");
                    }
                }
                Err(error) => {
                    // VOI-003 AC2: Rückfall auf die CPU mit Warn-Log.
                    tracing::warn!(%error, "metal init failed, falling back to cpu");
                }
            }
        }
        let ctx = match load_ctx(path, backend.is_gpu()) {
            Ok(ctx) => ctx,
            Err(error) if backend.is_gpu() => {
                tracing::warn!(%error, backend = backend.as_str(), "gpu init failed, falling back to cpu");
                backend = Backend::Cpu;
                load_ctx(path, false)?
            }
            Err(e) => return Err(e),
        };
        tracing::debug!(backend = backend.as_str(), "whisper model loaded");
        Ok(Box::new(WhisperModel { ctx, backend }))
    }
}

struct WhisperModel {
    ctx: WhisperContext,
    backend: Backend,
}

fn failed(e: impl std::fmt::Display) -> EngineError {
    EngineError::Transcribe(e.to_string())
}

impl LoadedModel for WhisperModel {
    fn backend(&self) -> Backend {
        self.backend
    }

    fn detect_language(&self, pcm: &[f32]) -> Result<Vec<(String, f32)>, EngineError> {
        if !self.ctx.is_multilingual() {
            return Ok(vec![("en".into(), 1.0)]);
        }
        let mut state = self.ctx.create_state().map_err(failed)?;
        state.pcm_to_mel(pcm, threads()).map_err(failed)?;
        let (_, probs) = state.lang_detect(0, threads()).map_err(failed)?;
        Ok(probs
            .into_iter()
            .enumerate()
            .filter_map(|(id, p)| {
                let id = i32::try_from(id).ok()?;
                whisper_rs::get_lang_str(id).map(|code| (code.to_owned(), p))
            })
            .collect())
    }

    fn transcribe(&self, pcm: &[f32], lang: Lang) -> Result<String, EngineError> {
        if pcm.is_empty() {
            return Ok(String::new());
        }
        let mut state = self.ctx.create_state().map_err(failed)?;
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_n_threads(i32::try_from(threads()).unwrap_or(4));
        params.set_language(Some(lang.code()));
        // VOI-006 AC3: Die Sprache steht fest, whisper.cpp erkennt nichts selbst.
        params.set_detect_language(false);
        params.set_translate(false);
        params.set_no_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        state.full(params, pcm).map_err(failed)?;
        let mut text = String::new();
        for segment in state.as_iter() {
            let part = segment.to_str_lossy().map_err(failed)?;
            text.push_str(&part);
        }
        Ok(text.trim().to_owned())
    }
}
