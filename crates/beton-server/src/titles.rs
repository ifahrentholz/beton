//! Automatische Session-Titel (SES-010).
//!
//! Nach dem ersten abgeschlossenen Turn einer Session ohne Titel erzeugt der Server einen
//! kurzen Titel (≤ 60 Zeichen) und schreibt `session.title_changed {source: generated}`.
//!
//! - `auto` (Default): Einmal-Aufruf über den Harness der Session (`harness.one_shot` an den
//!   Runner; bei Vendor-CLIs deren nicht-interaktiver Modus mit eigener Anmeldung, kleinstes
//!   Modell), bei Fehlschlag die Heuristik.
//! - `harness`: wie `auto`, ohne Heuristik.
//! - `direct`: Direkt-API-Harness mit `titles.provider`; den gibt es erst mit WP-25, bis dahin
//!   Heuristik und ein Hinweis im Log. Ein API-Key ist nie Voraussetzung (ADR-0034).
//! - `off`: nur Heuristik (erste Zeile der ersten Nachricht), kein Modellaufruf.
//!
//! Ein vorhandener Titel (vom User, aus Fork, Import oder Sub-Agent) wird nie überschrieben;
//! nach einer Umbenennung entsteht kein generierter Titel mehr. Der Verbrauch des Aufrufs
//! landet als `cost.delta` mit `purpose: "title"` im Log.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use beton_core::event::{
    Actor, CostDelta, Event, EventPayload, MessageRole, SessionTitleChanged, SystemComponent,
    TitleSource,
};
use beton_core::id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::problem::Problem;

/// Höchstlänge eines Titels in Zeichen.
pub const MAX_TITLE_CHARS: usize = 60;
/// Höchstlänge von `titles.instructions`.
pub const MAX_INSTRUCTIONS_CHARS: usize = 2_000;
/// So viel der ersten Nachrichten geht an das Modell.
const PROMPT_USER_CHARS: usize = 4_000;
const PROMPT_ASSISTANT_CHARS: usize = 1_000;
/// `purpose` des `cost.delta` einer Titelgenerierung (SES-010 AC3).
pub const COST_PURPOSE: &str = "title";
/// Kommando an den Runner (siehe `beton_runner::ONE_SHOT_CMD`).
pub const ONE_SHOT_CMD: &str = "harness.one_shot";

/// Grundanweisung an das Modell; `titles.instructions` kommt dazu.
pub const DEFAULT_INSTRUCTIONS: &str = "Du benennst Arbeitssitzungen eines Coding-Agents. \
Antworte ausschließlich mit einem kurzen, konkreten Titel für die Sitzung: höchstens 60 Zeichen, \
in der Sprache der Nachricht, ohne Anführungszeichen, ohne Punkt am Ende, ohne Erklärung.";

/// `titles.generator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleGenerator {
    /// Harness der Session, bei Fehlschlag Heuristik.
    #[default]
    Auto,
    /// Direkt-API-Harness mit `titles.provider` (nur ausdrücklich gewählt).
    Direct,
    /// Harness der Session, ohne Heuristik.
    Harness,
    /// Nur Heuristik, kein Modellaufruf.
    Off,
}

/// Einstellungen der Titelgenerierung (`titles.*` in der User-Konfiguration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitlesConfig {
    pub generator: TitleGenerator,
    /// Zusätzliche Anweisung (≤ 2 000 Zeichen).
    pub instructions: Option<String>,
    /// Kleinmodell des Direkt-API-Harness (`generator: direct`).
    pub provider: Option<String>,
    /// Höchstdauer des Einmal-Aufrufs.
    pub timeout: Duration,
}

impl Default for TitlesConfig {
    fn default() -> Self {
        Self {
            generator: TitleGenerator::Auto,
            instructions: None,
            provider: None,
            timeout: Duration::from_secs(60),
        }
    }
}

/// Sessions, für die in dieser Laufzeit schon ein Titel versucht wurde.
#[derive(Debug, Default)]
pub struct TitleJobs {
    attempted: Mutex<HashSet<SessionId>>,
}

impl TitleJobs {
    fn claim(&self, session: SessionId) -> bool {
        self.attempted
            .lock()
            .map(|mut s| s.insert(session))
            .unwrap_or(false)
    }
}

/// Kürzt auf höchstens `max` Zeichen, möglichst an einer Wortgrenze, mit `…`.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max - 1).collect();
    let at_word = match cut.rfind(char::is_whitespace) {
        Some(i) if cut[..i].chars().count() >= max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        at_word.trim_end_matches(|c: char| c.is_whitespace() || ",;:-–".contains(c))
    )
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Heuristik (SES-010): erste nicht leere Zeile der ersten User-Nachricht, ohne
/// Markdown-Präfixe, auf 60 Zeichen gekürzt.
pub fn heuristic(first_message: &str) -> Option<String> {
    let line = first_message
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .find(|l| !l.is_empty() && !l.starts_with("```"))?;
    let title = shorten(&collapse(line), MAX_TITLE_CHARS);
    (!title.is_empty()).then_some(title)
}

/// Bereinigt die Antwort des Modells: erste Zeile, ohne Präfix, Anführungszeichen und Punkt.
pub fn sanitize(model_output: &str) -> Option<String> {
    let line = model_output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("Titel:")
        .or_else(|| line.strip_prefix("Title:"))
        .unwrap_or(line)
        .trim();
    let quotes: &[char] = &['"', '\'', '„', '“', '”', '«', '»', '‚', '‘', '’', '`', '*'];
    let line = line
        .trim_end_matches('.')
        .trim_matches(quotes)
        .trim()
        .trim_end_matches('.')
        .trim();
    let title = shorten(&collapse(line), MAX_TITLE_CHARS);
    (!title.is_empty()).then_some(title)
}

/// Anweisung an das Modell aus Grundanweisung und `titles.instructions`.
pub fn instructions(cfg: &TitlesConfig) -> String {
    match cfg
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(extra) => format!(
            "{DEFAULT_INSTRUCTIONS}\n\n{}",
            extra
                .chars()
                .take(MAX_INSTRUCTIONS_CHARS)
                .collect::<String>()
        ),
        None => DEFAULT_INSTRUCTIONS.to_owned(),
    }
}

fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Inhalt für das Modell: erste User-Nachricht und Anfang der ersten Antwort.
pub fn prompt(user: &str, assistant: Option<&str>) -> String {
    let mut p = format!(
        "Erste Nachricht der Sitzung:\n{}",
        user.chars().take(PROMPT_USER_CHARS).collect::<String>()
    );
    if let Some(a) = assistant.filter(|a| !a.trim().is_empty()) {
        p.push_str("\n\nBeginn der Antwort des Agents:\n");
        p.extend(a.chars().take(PROMPT_ASSISTANT_CHARS));
    }
    p
}

/// Beobachtet gespeicherte Runner-Events: nach `turn.completed` einer Session ohne Titel
/// startet die Generierung (einmal je Session und Laufzeit).
pub struct TitleObserver(pub AppState);

impl crate::tunnel::EventObserver for TitleObserver {
    fn observe(&self, session: SessionId, written: &[Event]) {
        if !written
            .iter()
            .any(|e| matches!(e.payload(), Some(EventPayload::TurnCompleted(_))))
        {
            return;
        }
        if !self.0.runtime.title_jobs.claim(session) {
            return;
        }
        let state = self.0.clone();
        tokio::spawn(async move {
            if let Err(p) = generate(&state, session).await {
                tracing::warn!(%session, code = p.code.as_str(), "Session-Titel nicht erzeugt");
            }
        });
    }
}

fn server() -> Actor {
    Actor::System {
        component: SystemComponent::Server,
    }
}

/// Erste User-Nachricht und erste Antwort (bis zum ersten Turn-Ende).
async fn first_messages(
    state: &AppState,
    session: SessionId,
) -> Result<(Option<String>, Option<String>), Problem> {
    let org = state.local.org;
    let mut after = 0;
    let (mut user, mut assistant) = (None, None);
    loop {
        let page = state.store.events(org, session, after, 200).await?;
        let Some(last) = page.last() else { break };
        after = last.seq;
        for e in &page {
            match e.payload() {
                Some(EventPayload::MessageCompleted(m)) => {
                    let text = text_of(&m.content);
                    if text.trim().is_empty() {
                        continue;
                    }
                    match m.role {
                        MessageRole::User if user.is_none() => user = Some(text),
                        MessageRole::Assistant if user.is_some() && assistant.is_none() => {
                            assistant = Some(text);
                        }
                        _ => {}
                    }
                }
                Some(EventPayload::TurnCompleted(_)) if user.is_some() => {
                    return Ok((user, assistant));
                }
                _ => {}
            }
        }
    }
    Ok((user, assistant))
}

/// Hat die Session (noch) keinen Titel?
async fn untitled(state: &AppState, session: SessionId) -> Result<bool, Problem> {
    let record = state.store.session(state.local.org, session).await?;
    Ok(record.title.trim().is_empty())
}

/// Einmal-Aufruf über den Runner der Session.
async fn via_harness(
    state: &AppState,
    session: SessionId,
    cfg: &TitlesConfig,
    user: &str,
    assistant: Option<&str>,
) -> Result<beton_harness::OneShotReply, Problem> {
    let args = json!({
        "instructions": instructions(cfg),
        "prompt": prompt(user, assistant),
        "timeout_ms": u64::try_from(cfg.timeout.as_millis()).unwrap_or(u64::MAX),
    });
    let result = state
        .runtime
        .runners
        .deliver(
            session,
            ONE_SHOT_CMD,
            args,
            cfg.timeout + Duration::from_secs(5),
        )
        .await?;
    serde_json::from_value(result).map_err(|e| Problem::internal(&e))
}

/// Erzeugt den Titel einer Session (SES-010), falls sie noch keinen hat.
pub async fn generate(state: &AppState, session: SessionId) -> Result<(), Problem> {
    if !untitled(state, session).await? {
        return Ok(());
    }
    let (Some(user), assistant) = first_messages(state, session).await? else {
        return Ok(());
    };
    let cfg = state.runtime.titles.clone();
    let events = state.events();
    let org = state.local.org;
    let mut generated = None;
    match cfg.generator {
        TitleGenerator::Off => {}
        TitleGenerator::Direct => {
            // Der Direkt-API-Harness folgt mit WP-25; bis dahin nur die Heuristik.
            tracing::warn!(%session, "titles.generator=direct: Direkt-API-Harness nicht verfügbar, Heuristik");
        }
        TitleGenerator::Auto | TitleGenerator::Harness => {
            match via_harness(state, session, &cfg, &user, assistant.as_deref()).await {
                Ok(reply) => {
                    if let Some(cost) = reply.cost {
                        crate::queue::append(
                            &events,
                            org,
                            session,
                            server(),
                            EventPayload::CostDelta(CostDelta {
                                purpose: Some(COST_PURPOSE.into()),
                                ..cost
                            }),
                        )
                        .await?;
                    }
                    generated = sanitize(&reply.text);
                }
                Err(p) => {
                    tracing::info!(%session, code = p.code.as_str(), "Titel über den Harness nicht möglich");
                }
            }
        }
    }
    let title = match generated {
        Some(t) => t,
        None if cfg.generator == TitleGenerator::Harness => return Ok(()),
        None => match heuristic(&user) {
            Some(t) => t,
            None => return Ok(()),
        },
    };
    // Inzwischen umbenannt? Dann bleibt der Titel des Users (SES-010 AC2).
    if !untitled(state, session).await? {
        return Ok(());
    }
    crate::queue::append(
        &events,
        org,
        session,
        server(),
        EventPayload::SessionTitleChanged(SessionTitleChanged {
            title,
            source: TitleSource::Generated,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ses_010_ac1_heuristic_takes_first_line_and_shortens() {
        assert_eq!(
            heuristic("Die Login-Route braucht einen Rate-Limiter.\nMit Tests."),
            Some("Die Login-Route braucht einen Rate-Limiter.".into())
        );
        assert_eq!(
            heuristic("\n\n## Bug   im   Parser\n"),
            Some("Bug im Parser".into())
        );
        assert_eq!(heuristic("   \n "), None);
        let long = "Bitte implementiere einen Token-Bucket-Rate-Limiter für alle Routen unter /api/auth mit Redis";
        let t = heuristic(long).unwrap();
        assert!(t.chars().count() <= MAX_TITLE_CHARS, "{t}");
        assert!(t.ends_with('…'), "{t}");
        assert!(long.starts_with(t.trim_end_matches('…')), "{t}");
    }

    #[test]
    fn ses_010_model_output_is_sanitized() {
        assert_eq!(
            sanitize("„Rate-Limiter für Login-Route“.\n\nErklärung …"),
            Some("Rate-Limiter für Login-Route".into())
        );
        assert_eq!(
            sanitize("Titel: Parser reparieren"),
            Some("Parser reparieren".into())
        );
        assert_eq!(sanitize("  \n"), None);
        assert!(sanitize(&"x".repeat(200)).unwrap().chars().count() <= MAX_TITLE_CHARS);
    }

    #[test]
    fn ses_010_instructions_are_appended_and_capped() {
        let cfg = TitlesConfig {
            instructions: Some("Immer auf Englisch.".into()),
            ..TitlesConfig::default()
        };
        let i = instructions(&cfg);
        assert!(i.starts_with(DEFAULT_INSTRUCTIONS));
        assert!(i.ends_with("Immer auf Englisch."));
        let cfg = TitlesConfig {
            instructions: Some("y".repeat(5_000)),
            ..TitlesConfig::default()
        };
        assert_eq!(
            instructions(&cfg).len(),
            DEFAULT_INSTRUCTIONS.len() + 2 + MAX_INSTRUCTIONS_CHARS
        );
        assert_eq!(TitlesConfig::default().generator, TitleGenerator::Auto);
    }
}
