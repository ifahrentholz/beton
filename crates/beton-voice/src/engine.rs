//! Schnittstelle zur Transkriptions-Engine (VOI-001). Die echte Engine ist whisper.cpp
//! ([`crate::whisper::WhisperEngine`]); Tests der Ablauf-Logik nutzen eine Fake-Engine.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::{Acceleration, Lang};

/// Backend, auf dem ein Modell rechnet (VOI-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cpu,
    Metal,
    Cuda,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Metal => "metal",
            Backend::Cuda => "cuda",
        }
    }

    pub fn is_gpu(self) -> bool {
        self != Backend::Cpu
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Modell {path} lässt sich nicht laden: {reason}")]
    Load { path: String, reason: String },
    #[error("Transkription fehlgeschlagen: {0}")]
    Transcribe(String),
}

/// Lädt Modelle.
pub trait Engine: Send + Sync {
    /// Lädt ein Modell. Bei `Acceleration::Auto` wählt die Engine das beste Backend und fällt
    /// bei Initialisierungsfehlern auf die CPU zurück (mit Warn-Log, VOI-003 AC2).
    fn load(&self, path: &Path, accel: Acceleration) -> Result<Box<dyn LoadedModel>, EngineError>;
}

/// Ein geladenes Modell; mehrere Transkriptionen dürfen es gleichzeitig nutzen.
pub trait LoadedModel: Send + Sync {
    fn backend(&self) -> Backend;

    /// Wahrscheinlichkeit je Whisper-Sprachcode für den Anfang der Aufnahme (VOI-006).
    fn detect_language(&self, pcm: &[f32]) -> Result<Vec<(String, f32)>, EngineError>;

    /// Transkribiert in der angegebenen Sprache, ohne eigene Erkennung.
    fn transcribe(&self, pcm: &[f32], lang: Lang) -> Result<String, EngineError>;
}

/// Wählt die wahrscheinlichste der erlaubten Sprachen (VOI-006 AC2): Erkennt Whisper eine
/// andere Sprache (z. B. Französisch), gewinnt trotzdem eine der erlaubten.
pub fn pick_language(probs: &[(String, f32)], allowed: &[Lang]) -> Lang {
    let mut best: Option<(Lang, f32)> = None;
    for &lang in allowed {
        let p = probs
            .iter()
            .find(|(code, _)| code == lang.code())
            .map_or(0.0, |(_, p)| *p);
        if best.is_none_or(|(_, bp)| p > bp) {
            best = Some((lang, p));
        }
    }
    best.map_or(Lang::En, |(l, _)| l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probs(list: &[(&str, f32)]) -> Vec<(String, f32)> {
        list.iter().map(|(c, p)| ((*c).to_owned(), *p)).collect()
    }

    /// VOI-006 AC2: Französisch ist am wahrscheinlichsten, gewählt wird trotzdem `de` oder
    /// `en` – nie `fr`.
    #[test]
    fn voi_006_ac2_restriction_never_picks_a_language_outside_the_list() {
        let p = probs(&[("fr", 0.9), ("en", 0.04), ("de", 0.06)]);
        assert_eq!(pick_language(&p, &[Lang::De, Lang::En]), Lang::De);
        let p = probs(&[("fr", 0.9), ("en", 0.07), ("de", 0.03)]);
        assert_eq!(pick_language(&p, &[Lang::De, Lang::En]), Lang::En);
        // Nur eine erlaubte Sprache: immer diese.
        assert_eq!(pick_language(&p, &[Lang::De]), Lang::De);
        // Keine Angaben: erste erlaubte.
        assert_eq!(pick_language(&[], &[Lang::En, Lang::De]), Lang::En);
    }

    #[test]
    fn voi_006_ac1_most_likely_allowed_language_wins() {
        let p = probs(&[("de", 0.8), ("en", 0.1)]);
        assert_eq!(pick_language(&p, &[Lang::De, Lang::En]), Lang::De);
        let p = probs(&[("de", 0.2), ("en", 0.7)]);
        assert_eq!(pick_language(&p, &[Lang::De, Lang::En]), Lang::En);
    }
}
