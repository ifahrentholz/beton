//! Transkriptions-Dienst für Daemon bzw. zentralen Server (VOI-001, VOI-006).
//!
//! - Das Modell wird beim ersten Diktat geladen und nach `voice.idle_unload` ohne Nutzung
//!   wieder entladen (VOI-001 AC2).
//! - Höchstens `voice.max_concurrent` Transkriptionen laufen gleichzeitig; weitere warten in
//!   Ankunftsreihenfolge und bekommen `voice.status {state: "queued", position}` (VOI-001 AC3).
//! - Sprache: fest (`voice.language: de|en`, ohne Erkennung) oder die wahrscheinlichste aus
//!   `voice.languages` (VOI-006).
//! - Ohne installiertes Modell lädt der Dienst nichts herunter, sondern meldet
//!   [`VoiceError::ModelMissing`] samt Downloadgröße für „Modell laden (≈ X MB)“ (VOI-002 AC3).
//!
//! Audio bleibt im Arbeitsspeicher und wird nach der Transkription verworfen (VOI-004 AC2).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::audio::duration_secs;
use crate::catalog::Catalog;
use crate::engine::{Backend, Engine, EngineError, LoadedModel, pick_language};
use crate::settings::{Acceleration, Lang, LanguageSetting, VoiceSettings, default_max_concurrent};
use crate::store::ModelStore;

/// Zustand einer Transkriptions-Anfrage (`voice.status`, VOI-001 AC3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceState {
    /// Wartet auf einen freien Platz; `position` 1 ist als Nächstes dran.
    Queued,
    /// Wird transkribiert.
    Transcribing,
}

/// `voice.status {state, position?}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceStatus {
    pub state: VoiceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
}

impl VoiceStatus {
    pub fn queued(position: u32) -> Self {
        Self {
            state: VoiceState::Queued,
            position: Some(position),
        }
    }

    pub fn transcribing() -> Self {
        Self {
            state: VoiceState::Transcribing,
            position: None,
        }
    }
}

/// Ergebnis einer Transkription.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// Gewählte Sprache (`voice.final.lang`).
    pub lang: Lang,
    /// `false` bei fester Sprache (`voice.language: de|en`).
    pub detected: bool,
    pub backend: Backend,
    pub audio_secs: f64,
    pub processing_secs: f64,
}

impl Transcript {
    /// Real-Time-Factor: Rechenzeit ÷ Audiodauer (VOI-003).
    pub fn rtf(&self) -> f64 {
        if self.audio_secs > 0.0 {
            self.processing_secs / self.audio_secs
        } else {
            0.0
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    /// Kein Modell installiert; es wird nichts heruntergeladen (VOI-002 AC3).
    #[error(
        "Für Diktate fehlt ein Sprachmodell: Modell laden (≈ {}) mit `beton voice models pull {model}` oder eine Modelldatei importieren",
        crate::format_size(*download_size_bytes)
    )]
    ModelMissing {
        /// Vorgeschlagenes Modell (Default für diese Hardware).
        model: String,
        /// Downloadgröße in Bytes.
        download_size_bytes: u64,
    },
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("Transkription abgebrochen")]
    Aborted,
}

/// Woher der Dienst das Modell nimmt.
pub trait ModelSource: Send + Sync {
    fn resolve(&self) -> Result<PathBuf, VoiceError>;
}

/// Modelle aus dem Modellverzeichnis, ausgewählt per `voice.model` (VOI-002).
pub struct StoreSource {
    pub store: ModelStore,
    pub catalog: Catalog,
    /// `auto` oder ein Name der Liste.
    pub choice: String,
    /// Default-Modell für diese Hardware.
    pub default_model: String,
}

impl ModelSource for StoreSource {
    fn resolve(&self) -> Result<PathBuf, VoiceError> {
        let wanted = if self.choice == "auto" {
            self.default_model.as_str()
        } else {
            self.choice.as_str()
        };
        if let Some(e) = self.catalog.get(wanted)
            && self.store.is_installed(e)
        {
            return Ok(self.store.path(e));
        }
        if self.choice == "auto"
            && let Some(e) = self
                .catalog
                .entries()
                .iter()
                .find(|e| self.store.is_installed(e))
        {
            return Ok(self.store.path(e));
        }
        Err(VoiceError::ModelMissing {
            model: wanted.to_owned(),
            download_size_bytes: self.catalog.get(wanted).map_or(0, |e| e.size_bytes),
        })
    }
}

/// Eine feste Modelldatei (Tests, Benchmarks).
pub struct FixedSource(pub PathBuf);

impl ModelSource for FixedSource {
    fn resolve(&self) -> Result<PathBuf, VoiceError> {
        Ok(self.0.clone())
    }
}

/// Wirksame Einstellungen des Dienstes.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceConfig {
    pub idle_unload: Duration,
    pub max_concurrent: usize,
    pub language: LanguageSetting,
    pub languages: Vec<Lang>,
    pub acceleration: Acceleration,
}

impl ServiceConfig {
    /// Aus `voice:` der Konfiguration; `central` wählt den Default für `max_concurrent`.
    pub fn from_settings(s: &VoiceSettings, central: bool) -> Result<Self, String> {
        s.validate()?;
        Ok(Self {
            idle_unload: s.idle_unload_duration()?,
            max_concurrent: s.max_concurrent.map_or_else(
                || default_max_concurrent(central),
                |n| usize::try_from(n).unwrap_or(usize::MAX),
            ),
            language: s.language,
            languages: s.languages.clone(),
            acceleration: s.acceleration,
        })
    }
}

#[derive(Default)]
struct Queue {
    running: usize,
    waiting: VecDeque<u64>,
    next_ticket: u64,
}

#[derive(Default)]
struct Slot {
    loaded: Option<Arc<dyn LoadedModel>>,
    generation: u64,
    active: usize,
    last_used: Option<Instant>,
}

struct Inner {
    engine: Arc<dyn Engine>,
    source: Arc<dyn ModelSource>,
    cfg: ServiceConfig,
    queue: Mutex<Queue>,
    changed: tokio::sync::Notify,
    slot: tokio::sync::Mutex<Slot>,
    last: Mutex<Option<(Backend, f64)>>,
}

/// Der Transkriptions-Dienst.
#[derive(Clone)]
pub struct VoiceService {
    inner: Arc<Inner>,
}

/// Belegter Platz; gibt ihn beim Drop frei.
struct Running(Arc<Inner>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut q) = self.0.queue.lock() {
            q.running = q.running.saturating_sub(1);
        }
        self.0.changed.notify_waiters();
    }
}

/// Wartender Platz; entfernt das Ticket, wenn die Anfrage vorher abgebrochen wird.
struct Waiting<'a> {
    inner: &'a Inner,
    ticket: u64,
    done: bool,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if let Ok(mut q) = self.inner.queue.lock() {
            q.waiting.retain(|t| *t != self.ticket);
        }
        self.inner.changed.notify_waiters();
    }
}

impl VoiceService {
    pub fn new(engine: Arc<dyn Engine>, source: Arc<dyn ModelSource>, cfg: ServiceConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                engine,
                source,
                cfg,
                queue: Mutex::default(),
                changed: tokio::sync::Notify::new(),
                slot: tokio::sync::Mutex::default(),
                last: Mutex::default(),
            }),
        }
    }

    /// Ist gerade ein Modell geladen?
    pub async fn is_loaded(&self) -> bool {
        self.inner.slot.lock().await.loaded.is_some()
    }

    /// Backend und Real-Time-Factor der letzten Transkription (VOI-003).
    pub fn last_measurement(&self) -> Option<(Backend, f64)> {
        self.inner.last.lock().ok().and_then(|l| *l)
    }

    /// Wartet auf einen Platz (VOI-001 AC3) und meldet die Position, solange gewartet wird.
    async fn acquire(&self, on_status: &mut (dyn FnMut(VoiceStatus) + Send)) -> Running {
        let inner = &self.inner;
        let ticket = {
            let mut q = inner
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if q.running < inner.cfg.max_concurrent && q.waiting.is_empty() {
                q.running += 1;
                return Running(inner.clone());
            }
            let t = q.next_ticket;
            q.next_ticket += 1;
            q.waiting.push_back(t);
            t
        };
        let mut waiting = Waiting {
            inner,
            ticket,
            done: false,
        };
        let mut reported = None;
        loop {
            let notified = inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut q = inner
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let pos = q.waiting.iter().position(|t| *t == ticket).unwrap_or(0);
                if pos == 0 && q.running < inner.cfg.max_concurrent {
                    q.waiting.pop_front();
                    q.running += 1;
                    waiting.done = true;
                    return Running(inner.clone());
                }
                let position = u32::try_from(pos + 1).unwrap_or(u32::MAX);
                if reported != Some(position) {
                    reported = Some(position);
                    on_status(VoiceStatus::queued(position));
                }
            }
            notified.await;
        }
    }

    async fn model(&self) -> Result<(Arc<dyn LoadedModel>, u64), VoiceError> {
        let path = self.inner.source.resolve()?;
        let mut slot = self.inner.slot.lock().await;
        if slot.loaded.is_none() {
            let engine = self.inner.engine.clone();
            let accel = self.inner.cfg.acceleration;
            let model = tokio::task::spawn_blocking(move || engine.load(&path, accel))
                .await
                .map_err(|_| VoiceError::Aborted)??;
            slot.loaded = Some(Arc::from(model));
            slot.generation += 1;
            tracing::info!(generation = slot.generation, "voice model loaded");
        }
        slot.active += 1;
        let model = slot.loaded.clone().ok_or(VoiceError::Aborted)?;
        Ok((model, slot.generation))
    }

    async fn release(&self, generation: u64) {
        let mut slot = self.inner.slot.lock().await;
        slot.active = slot.active.saturating_sub(1);
        slot.last_used = Some(Instant::now());
        let idle = self.inner.cfg.idle_unload;
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            tokio::time::sleep(idle).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let model = {
                let mut slot = inner.slot.lock().await;
                let idle_long_enough = slot.last_used.is_some_and(|t| t.elapsed() >= idle);
                if slot.generation != generation || slot.active > 0 || !idle_long_enough {
                    return;
                }
                slot.loaded.take()
            };
            if let Some(model) = model {
                // Freigeben außerhalb des Locks; whisper.cpp gibt dabei den Modellspeicher frei.
                let _ = tokio::task::spawn_blocking(move || drop(model)).await;
                tracing::info!("voice model unloaded after idle timeout");
            }
        });
    }

    /// Transkribiert 16-kHz-Mono-Audio (`f32`). `on_status` bekommt `queued` (mit Position)
    /// und `transcribing`.
    pub async fn transcribe(
        &self,
        pcm: Vec<f32>,
        mut on_status: impl FnMut(VoiceStatus) + Send,
    ) -> Result<Transcript, VoiceError> {
        // Ohne Modell sofort melden, statt zu warten oder etwas zu laden (VOI-002 AC3).
        self.inner.source.resolve()?;
        let _running = self.acquire(&mut on_status).await;
        on_status(VoiceStatus::transcribing());
        let (model, generation) = self.model().await?;
        let cfg = self.inner.cfg.clone();
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            let (lang, detected) = match cfg.language.fixed() {
                // VOI-006 AC3: feste Sprache, keine Erkennung.
                Some(lang) => (lang, false),
                None => {
                    let probs = model.detect_language(&pcm)?;
                    (pick_language(&probs, &cfg.languages), true)
                }
            };
            let text = model.transcribe(&pcm, lang)?;
            Ok::<_, EngineError>(Transcript {
                text,
                lang,
                detected,
                backend: model.backend(),
                audio_secs: duration_secs(&pcm),
                processing_secs: 0.0,
            })
        })
        .await;
        self.release(generation).await;
        let mut transcript = result.map_err(|_| VoiceError::Aborted)??;
        transcript.processing_secs = started.elapsed().as_secs_f64();
        if let Ok(mut last) = self.inner.last.lock() {
            *last = Some((transcript.backend, transcript.rtf()));
        }
        Ok(transcript)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Fake-Engine: zählt Lade- und Erkennungsaufrufe, liefert feste Wahrscheinlichkeiten und
    /// blockiert Transkriptionen optional, bis ein Signal kommt.
    /// Tor, das Transkriptionen aufhält, bis es geöffnet wird.
    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        cv: std::sync::Condvar,
    }

    impl Gate {
        fn wait(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.cv.wait(open).unwrap();
            }
        }
        fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.cv.notify_all();
        }
    }

    #[derive(Default)]
    struct Fake {
        loads: AtomicUsize,
        detections: Arc<AtomicUsize>,
        probs: Vec<(String, f32)>,
        gate: Option<Arc<Gate>>,
    }

    struct FakeModel {
        detections: Arc<AtomicUsize>,
        probs: Vec<(String, f32)>,
        gate: Option<Arc<Gate>>,
    }

    impl Engine for Fake {
        fn load(
            &self,
            _: &std::path::Path,
            _: Acceleration,
        ) -> Result<Box<dyn LoadedModel>, EngineError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeModel {
                detections: self.detections.clone(),
                probs: self.probs.clone(),
                gate: self.gate.clone(),
            }))
        }
    }

    impl LoadedModel for FakeModel {
        fn backend(&self) -> Backend {
            Backend::Cpu
        }
        fn detect_language(&self, _: &[f32]) -> Result<Vec<(String, f32)>, EngineError> {
            self.detections.fetch_add(1, Ordering::SeqCst);
            Ok(self.probs.clone())
        }
        fn transcribe(&self, _: &[f32], lang: Lang) -> Result<String, EngineError> {
            if let Some(g) = &self.gate {
                g.wait();
            }
            Ok(format!("text-{}", lang.code()))
        }
    }

    fn cfg(max: usize) -> ServiceConfig {
        ServiceConfig {
            idle_unload: Duration::from_secs(600),
            max_concurrent: max,
            language: LanguageSetting::Auto,
            languages: vec![Lang::De, Lang::En],
            acceleration: Acceleration::Cpu,
        }
    }

    fn service(engine: Fake, cfg: ServiceConfig) -> (VoiceService, Arc<Fake>) {
        let engine = Arc::new(engine);
        let svc = VoiceService::new(
            engine.clone(),
            Arc::new(FixedSource(PathBuf::from("/nicht/da.bin"))),
            cfg,
        );
        (svc, engine)
    }

    fn probs(list: &[(&str, f32)]) -> Vec<(String, f32)> {
        list.iter().map(|(c, p)| ((*c).to_owned(), *p)).collect()
    }

    /// VOI-001 AC3: Bei zwei laufenden Transkriptionen (max_concurrent 2) erhält der dritte
    /// Client `voice.status {state: queued, position: 1}`; die ersten beiden warten nie.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn voi_001_ac3_third_client_is_queued_with_position() {
        let gate = Arc::new(Gate::default());
        let (svc, _) = service(
            Fake {
                gate: Some(gate.clone()),
                ..Fake::default()
            },
            cfg(2),
        );
        let statuses: Arc<Mutex<Vec<(usize, VoiceStatus)>>> = Arc::default();
        let mut handles = Vec::new();
        for client in 0..2 {
            let (svc, st) = (svc.clone(), statuses.clone());
            handles.push(tokio::spawn(async move {
                svc.transcribe(vec![0.0; 16_000], move |s| {
                    st.lock().unwrap().push((client, s))
                })
                .await
            }));
        }
        // Beide laufen (transcribing gemeldet), bevor der dritte kommt.
        while statuses.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (svc3, st3) = (svc.clone(), statuses.clone());
        let third = tokio::spawn(async move {
            svc3.transcribe(vec![0.0; 16_000], move |s| st3.lock().unwrap().push((2, s)))
                .await
        });
        while !statuses.lock().unwrap().iter().any(|(c, _)| *c == 2) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            statuses
                .lock()
                .unwrap()
                .iter()
                .find(|(c, _)| *c == 2)
                .unwrap()
                .1,
            VoiceStatus::queued(1)
        );
        assert_eq!(
            serde_json::to_value(VoiceStatus::queued(1)).unwrap(),
            serde_json::json!({"state": "queued", "position": 1})
        );
        // Freigeben: danach laufen alle durch, der dritte als Letzter.
        gate.open();
        for h in handles {
            h.await.unwrap().unwrap();
        }
        third.await.unwrap().unwrap();
        let st = statuses.lock().unwrap().clone();
        for c in 0..2 {
            assert!(
                st.iter()
                    .filter(|(x, _)| *x == c)
                    .all(|(_, s)| s.state == VoiceState::Transcribing),
                "{st:?}"
            );
        }
        assert_eq!(
            st.iter()
                .filter(|(x, _)| *x == 2)
                .map(|(_, s)| *s)
                .collect::<Vec<_>>(),
            vec![VoiceStatus::queued(1), VoiceStatus::transcribing()]
        );
    }

    /// VOI-001: Das Modell wird erst beim ersten Diktat geladen und dann wiederverwendet.
    #[tokio::test]
    async fn voi_001_model_is_loaded_lazily_and_reused() {
        let (svc, engine) = service(Fake::default(), cfg(1));
        assert!(!svc.is_loaded().await);
        assert_eq!(engine.loads.load(Ordering::SeqCst), 0);
        svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        assert!(svc.is_loaded().await);
        assert_eq!(engine.loads.load(Ordering::SeqCst), 1);
    }

    /// VOI-001 AC2 (Ablauf): Nach `idle_unload` ohne Nutzung ist das Modell entladen; das
    /// nächste Diktat lädt es neu. Den Speicher misst `tests/whisper.rs`.
    #[tokio::test]
    async fn voi_001_ac2_model_is_unloaded_after_idle_timeout() {
        let mut c = cfg(1);
        c.idle_unload = Duration::from_millis(100);
        let (svc, engine) = service(Fake::default(), c);
        svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        assert!(svc.is_loaded().await);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!svc.is_loaded().await);
        svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        assert_eq!(engine.loads.load(Ordering::SeqCst), 2);
    }

    /// VOI-006 AC2 (Ablauf): Whisper hält Französisch für am wahrscheinlichsten; transkribiert
    /// wird trotzdem in `de` oder `en`.
    #[tokio::test]
    async fn voi_006_ac2_service_restricts_detection_to_de_and_en() {
        let (svc, engine) = service(
            Fake {
                probs: probs(&[("fr", 0.95), ("en", 0.03), ("de", 0.02)]),
                ..Fake::default()
            },
            cfg(1),
        );
        let t = svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        assert_eq!(t.lang, Lang::En);
        assert!(t.detected);
        assert_eq!(t.text, "text-en");
        assert_eq!(engine.detections.load(Ordering::SeqCst), 1);
    }

    /// VOI-006 AC3: Mit `voice.language: en` findet keine Erkennung statt.
    #[tokio::test]
    async fn voi_006_ac3_fixed_language_skips_detection() {
        let mut c = cfg(1);
        c.language = LanguageSetting::En;
        let (svc, engine) = service(
            Fake {
                probs: probs(&[("de", 0.99)]),
                ..Fake::default()
            },
            c,
        );
        let t = svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap();
        assert_eq!(t.lang, Lang::En);
        assert!(!t.detected);
        assert_eq!(engine.detections.load(Ordering::SeqCst), 0);
    }

    /// VOI-002 AC3: Ohne Modell lädt der Dienst nichts (weder Download noch Engine) und meldet
    /// das vorgeschlagene Modell mit Downloadgröße für „Modell laden (≈ X MB)“.
    #[tokio::test]
    async fn voi_002_ac3_without_model_nothing_is_downloaded_and_size_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(Fake::default());
        let source = StoreSource {
            store: ModelStore::new(dir.path()),
            catalog: Catalog::pinned(),
            choice: "auto".into(),
            default_model: "small".into(),
        };
        let svc = VoiceService::new(engine.clone(), Arc::new(source), cfg(1));
        let err = svc.transcribe(vec![0.0; 1600], |_| {}).await.unwrap_err();
        match &err {
            VoiceError::ModelMissing {
                model,
                download_size_bytes,
            } => {
                assert_eq!(model, "small");
                assert_eq!(*download_size_bytes, 487_601_967);
            }
            other => panic!("{other}"),
        }
        assert!(err.to_string().contains("Modell laden (≈ 488 MB)"), "{err}");
        assert_eq!(engine.loads.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn store_source_prefers_choice_then_any_installed() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::pinned();
        let store = ModelStore::new(dir.path());
        // Ein „installiertes“ Modell: richtige Größe genügt für den Status.
        let e = catalog.get("small-q5_1").unwrap();
        let f = std::fs::File::create(store.path(e)).unwrap();
        f.set_len(e.size_bytes).unwrap();
        let src = |choice: &str| StoreSource {
            store: store.clone(),
            catalog: catalog.clone(),
            choice: choice.into(),
            default_model: "large-v3-turbo".into(),
        };
        assert_eq!(src("auto").resolve().unwrap(), store.path(e));
        assert!(matches!(
            src("medium").resolve(),
            Err(VoiceError::ModelMissing { model, .. }) if model == "medium"
        ));
        assert_eq!(src("small-q5_1").resolve().unwrap(), store.path(e));
    }

    #[test]
    fn config_from_settings_uses_mode_defaults() {
        let s = VoiceSettings::default();
        assert_eq!(
            ServiceConfig::from_settings(&s, false)
                .unwrap()
                .max_concurrent,
            1
        );
        assert_eq!(
            ServiceConfig::from_settings(&s, true)
                .unwrap()
                .max_concurrent,
            2
        );
        let s = VoiceSettings {
            max_concurrent: Some(3),
            ..VoiceSettings::default()
        };
        assert_eq!(
            ServiceConfig::from_settings(&s, true)
                .unwrap()
                .max_concurrent,
            3
        );
    }
}
