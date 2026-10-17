//! Kontextfüllstand für die Statuszeile (USE-008 AC3, SES-011): dieselbe Logik wie die Web-UI
//! (`apps/web/src/lib/context.ts`, `contextOf` und `contextLevel`).

use beton_core::event::{ContextSource, Event, EventPayload};

/// Letzter gemeldeter Kontext einer Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextState {
    pub used: u64,
    /// `None`: Fenster unbekannt (`window_tokens: 0`).
    pub window: Option<u64>,
    pub source: ContextSource,
}

/// Kontext aus einem `context.usage`-Event; andere Events liefern `None`.
pub fn context_of(event: &Event) -> Option<ContextState> {
    match event.payload()? {
        EventPayload::ContextUsage(c) => Some(ContextState {
            used: c.used_tokens,
            window: (c.window_tokens > 0).then_some(c.window_tokens),
            source: c.source,
        }),
        _ => None,
    }
}

/// Stufe der Anzeige: Schwellen 70 / 85 / 95 %; ohne Fenster nur die absoluten Tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextLevel {
    Plain,
    Ok,
    Warn,
    High,
    Full,
}

impl ContextLevel {
    /// Text neben dem Wert: „wird knapp“, „fast voll“, „voll“.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plain | Self::Ok => "",
            Self::Warn => "wird knapp",
            Self::High => "fast voll",
            Self::Full => "voll",
        }
    }
}

/// Prozent (gerundet wie `Math.round`) und Stufe.
pub fn context_level(used: u64, window: Option<u64>) -> (Option<u64>, ContextLevel) {
    let Some(window) = window.filter(|w| *w > 0) else {
        return (None, ContextLevel::Plain);
    };
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let pct = ((used as f64 / window as f64) * 100.0).round() as u64;
    let level = match pct {
        95.. => ContextLevel::Full,
        85.. => ContextLevel::High,
        70.. => ContextLevel::Warn,
        _ => ContextLevel::Ok,
    };
    (Some(pct), level)
}

/// Text der Statuszeile, z. B. `Kontext 36 %`, `Kontext 88 % · fast voll` oder ohne Fenster
/// `150 k Tokens`.
pub fn status_text(state: &ContextState) -> (String, ContextLevel) {
    let (pct, level) = context_level(state.used, state.window);
    let text = match pct {
        Some(p) => {
            let label = level.label();
            if label.is_empty() {
                format!("Kontext {p} %")
            } else {
                format!("Kontext {p} % · {label}")
            }
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        None => format!("{} k Tokens", (state.used as f64 / 1000.0).round() as u64),
    };
    (text, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(used: u64, window: u64) -> ContextState {
        ContextState {
            used,
            window: (window > 0).then_some(window),
            source: ContextSource::Harness,
        }
    }

    #[test]
    fn use_008_ac3_levels_match_the_web_ui_thresholds() {
        assert_eq!(
            context_level(36_000, Some(100_000)),
            (Some(36), ContextLevel::Ok)
        );
        assert_eq!(
            context_level(69_400, Some(100_000)),
            (Some(69), ContextLevel::Ok)
        );
        assert_eq!(
            context_level(69_500, Some(100_000)),
            (Some(70), ContextLevel::Warn)
        );
        assert_eq!(
            context_level(85_000, Some(100_000)),
            (Some(85), ContextLevel::High)
        );
        assert_eq!(
            context_level(95_000, Some(100_000)),
            (Some(95), ContextLevel::Full)
        );
        assert_eq!(
            context_level(120_000, Some(100_000)),
            (Some(120), ContextLevel::Full)
        );
        assert_eq!(context_level(150_000, None), (None, ContextLevel::Plain));
    }

    #[test]
    fn use_008_ac3_status_text_shows_percent_label_or_only_tokens() {
        assert_eq!(status_text(&state(36_000, 100_000)).0, "Kontext 36 %");
        assert_eq!(
            status_text(&state(72_000, 100_000)).0,
            "Kontext 72 % · wird knapp"
        );
        assert_eq!(
            status_text(&state(88_000, 100_000)).0,
            "Kontext 88 % · fast voll"
        );
        assert_eq!(
            status_text(&state(99_000, 100_000)).0,
            "Kontext 99 % · voll"
        );
        // Ohne bekanntes Fenster (`window_tokens: 0`) nur die Tokenzahl.
        assert_eq!(status_text(&state(150_000, 0)).0, "150 k Tokens");
    }
}
