//! `voice:` in der Konfiguration (VOI-001, VOI-002, VOI-003, VOI-006).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;

/// Sprache eines Diktats; die Erkennung ist auf diese Sprachen beschränkt (VOI-006).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    De,
    En,
}

impl Lang {
    /// Whisper-Sprachcode.
    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "de" => Some(Lang::De),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

/// `voice.language`: automatische Erkennung oder feste Sprache (VOI-006).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LanguageSetting {
    /// Whisper erkennt die Sprache, beton wählt die wahrscheinlichste aus `voice.languages`.
    #[default]
    Auto,
    /// Immer Deutsch, ohne Erkennung.
    De,
    /// Immer Englisch, ohne Erkennung.
    En,
}

impl LanguageSetting {
    /// Feste Sprache, falls gesetzt.
    pub fn fixed(self) -> Option<Lang> {
        match self {
            LanguageSetting::Auto => None,
            LanguageSetting::De => Some(Lang::De),
            LanguageSetting::En => Some(Lang::En),
        }
    }
}

/// `voice.acceleration` (VOI-003): `auto` wählt das beste Backend und fällt bei
/// Initialisierungsfehlern auf die CPU zurück; `cpu` erzwingt die CPU.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Acceleration {
    #[default]
    Auto,
    Cpu,
}

/// `voice:` in `config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct VoiceSettings {
    /// Modell: `auto` (Default: `large-v3-turbo` auf Apple Silicon bzw. mit GPU, sonst `small`;
    /// ist es nicht installiert, das erste installierte der Liste) oder ein Name aus
    /// `beton voice models list`.
    pub model: String,
    /// `auto` (Default), `de` oder `en`.
    pub language: LanguageSetting,
    /// Sprachen der automatischen Erkennung (Default `[de, en]`).
    pub languages: Vec<Lang>,
    /// Modell nach so langer Pause entladen, z. B. `10m` (Default), `90s` oder `1h`.
    #[schemars(regex(pattern = r"^[0-9]+(s|m|h)$"))]
    pub idle_unload: String,
    /// Gleichzeitige Transkriptionen; weitere warten mit `voice.status {state: queued}`.
    /// Default: 1 lokal, 2 im zentralen Betrieb.
    pub max_concurrent: Option<u32>,
    /// `auto` (Default) oder `cpu`.
    pub acceleration: Acceleration,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            model: "auto".into(),
            language: LanguageSetting::Auto,
            languages: vec![Lang::De, Lang::En],
            idle_unload: "10m".into(),
            max_concurrent: None,
            acceleration: Acceleration::Auto,
        }
    }
}

/// Default für `max_concurrent` (VOI-001).
pub fn default_max_concurrent(central: bool) -> usize {
    if central { 2 } else { 1 }
}

impl VoiceSettings {
    /// Prüft die Werte; der Fehler nennt Schlüssel und Grund.
    pub fn validate(&self) -> Result<(), String> {
        if self.model != "auto" && Catalog::pinned().get(&self.model).is_none() {
            return Err(format!(
                "voice.model „{}“ ist kein Modell der mitgelieferten Liste",
                self.model
            ));
        }
        if self.languages.is_empty() {
            return Err("voice.languages darf nicht leer sein".into());
        }
        if let Some(lang) = self.language.fixed()
            && !self.languages.contains(&lang)
        {
            return Err(format!(
                "voice.language {} fehlt in voice.languages",
                lang.code()
            ));
        }
        self.idle_unload_duration()?;
        if self.max_concurrent == Some(0) {
            return Err("voice.max_concurrent muss mindestens 1 sein".into());
        }
        Ok(())
    }

    /// `idle_unload` als Dauer.
    pub fn idle_unload_duration(&self) -> Result<Duration, String> {
        parse_duration(&self.idle_unload).ok_or_else(|| {
            format!(
                "voice.idle_unload „{}“: erwartet z. B. 10m, 90s oder 1h",
                self.idle_unload
            )
        })
    }
}

fn parse_duration(text: &str) -> Option<Duration> {
    let unit = text.chars().last()?;
    let number: u64 = text[..text.len() - unit.len_utf8()].parse().ok()?;
    let secs = match unit {
        's' => number,
        'm' => number.checked_mul(60)?,
        'h' => number.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec() {
        let s = VoiceSettings::default();
        assert_eq!(s.idle_unload_duration().unwrap(), Duration::from_secs(600));
        assert_eq!(s.languages, vec![Lang::De, Lang::En]);
        assert_eq!(s.language, LanguageSetting::Auto);
        assert_eq!(default_max_concurrent(false), 1);
        assert_eq!(default_max_concurrent(true), 2);
        s.validate().unwrap();
    }

    #[test]
    fn invalid_values_name_the_key() {
        let bad = |f: fn(&mut VoiceSettings)| {
            let mut s = VoiceSettings::default();
            f(&mut s);
            s.validate().unwrap_err()
        };
        assert!(bad(|s| s.idle_unload = "10".into()).contains("voice.idle_unload"));
        assert!(bad(|s| s.idle_unload = "5d".into()).contains("voice.idle_unload"));
        assert!(bad(|s| s.model = "tiny".into()).contains("voice.model"));
        assert!(bad(|s| s.languages.clear()).contains("voice.languages"));
        assert!(
            bad(|s| {
                s.languages = vec![Lang::De];
                s.language = LanguageSetting::En;
            })
            .contains("voice.language")
        );
        assert!(bad(|s| s.max_concurrent = Some(0)).contains("max_concurrent"));
    }

    #[test]
    fn parses_yaml() {
        let s: VoiceSettings = serde_json::from_value(serde_json::json!({
            "language": "en", "idle_unload": "90s", "max_concurrent": 2, "acceleration": "cpu"
        }))
        .unwrap();
        assert_eq!(s.language.fixed(), Some(Lang::En));
        assert_eq!(s.idle_unload_duration().unwrap(), Duration::from_secs(90));
        assert_eq!(s.acceleration, Acceleration::Cpu);
        assert!(
            serde_json::from_value::<VoiceSettings>(serde_json::json!({"language": "fr"})).is_err()
        );
    }
}
