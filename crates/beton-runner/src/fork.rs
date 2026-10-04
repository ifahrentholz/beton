//! Start mit übernommenem Verlauf (HAR-018, HAR-019, SES-006, SES-007): Der Runner liest den
//! Fork-Plan des Servers und wählt je Capability `fork_history`, wie der Verlauf beim
//! Harness ankommt:
//!
//! - `native`: Vendor-Fork der Quell-Session (`--resume <id> --fork-session`),
//! - `rebuild`: native Session-Datei aus dem Event-Log rekonstruieren,
//! - `preamble`: Handover-Dokument im Workspace plus erste Nachricht.
//!
//! Scheitert der Rebuild, gilt automatisch `preamble` mit `fallback_reason` und Hinweis-Event
//! (HAR-019 AC3). Der Runner schreibt dafür `session.forked` (bzw. bei einem Import
//! `session.resumed`) als erstes eigenes Event, denn erst er kennt den tatsächlichen Modus.

use std::path::Path;

use beton_core::event::{
    EventPayload, HistoryMode, Notice, NoticeLevel, ResumeMode, SessionForked, SessionResumed,
};
use beton_core::id::SessionId;
use beton_harness::handover::{self, ForkPlan, HistoryChoice, PlanKind, Preamble};
use beton_harness::{Capabilities, HarnessAdapter, HarnessId, HostEnv, RebuildRequest};

/// Was der Start aus dem Plan übernimmt.
#[derive(Debug, Default)]
pub struct Prepared {
    /// Native Referenz für `SessionSpec::resume`.
    pub resume: Option<String>,
    /// Mit `resume`: als neue Session abzweigen.
    pub fork_session: bool,
    /// Kurzfassung für die erste Eingabe (Präambel).
    pub handover: Option<String>,
    /// Events vor `session.started` (`session.forked` bzw. `session.resumed`).
    pub first: Vec<EventPayload>,
    /// Events nach `session.started` (Hinweise).
    pub notices: Vec<EventPayload>,
}

fn notice(level: NoticeLevel, text: String) -> EventPayload {
    EventPayload::Notice(Notice { level, text })
}

/// Liest den Plan; ein unlesbarer Plan ist ein Startfehler (fail closed: die Session soll
/// nicht ohne den zugesagten Verlauf weiterlaufen).
pub fn load(path: &Path) -> Result<ForkPlan, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Fork-Plan lesen: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Fork-Plan ungültig: {e}"))
}

/// Bereitet den Start vor. `logged`: `session.forked` mit `preamble` steht schon im Log und
/// der erste Turn fehlt noch; dann wird nur die Präambel erneut gerendert (deterministisch).
#[allow(clippy::too_many_arguments)]
pub async fn prepare(
    plan: &ForkPlan,
    logged: bool,
    session: SessionId,
    target: &HarnessId,
    adapter: Option<&dyn HarnessAdapter>,
    caps: Option<&Capabilities>,
    env: &HostEnv,
    workdir: &Path,
    model: Option<String>,
) -> Prepared {
    let default_caps = Capabilities::minimal(
        beton_harness::Mode::Native,
        beton_harness::Transport::Native,
    );
    let caps = caps.unwrap_or(&default_caps);
    let mut out = Prepared::default();
    let mut fallback: Option<String> = None;
    let choice = if logged {
        HistoryChoice::Preamble
    } else {
        plan.choose(target.as_str(), caps)
    };
    let mode = match choice {
        HistoryChoice::Native { reference } => {
            out.resume = Some(reference);
            out.fork_session = true;
            HistoryMode::Native
        }
        HistoryChoice::Rebuild => {
            let request = RebuildRequest {
                workdir: workdir.to_path_buf(),
                history: plan.history.clone(),
                model,
            };
            let result = match adapter {
                Some(a) => a.rebuild_history(&request, env).await,
                None => Err(beton_harness::HarnessError::NotFound(target.to_string())),
            };
            match result {
                Ok(reference) => {
                    out.resume = Some(reference);
                    HistoryMode::Rebuild
                }
                Err(e) => {
                    fallback = Some(e.to_string());
                    HistoryMode::Preamble
                }
            }
        }
        HistoryChoice::Preamble => HistoryMode::Preamble,
    };
    if mode == HistoryMode::Preamble {
        let preamble = render(plan, session, target, caps);
        let path = workdir.join(handover::document_path(session));
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &preamble.document));
        if !logged {
            out.notices.push(notice(
                NoticeLevel::Info,
                format!(
                    "Übergabe an {} per Präambel: {} Turns, davon {} vollständig, ca. {} Tokens \
                     ({} % des Kontextfensters) in `{}`.",
                    handover::harness_label(target.as_str()),
                    preamble.turns,
                    preamble.full_turns,
                    preamble.tokens,
                    (preamble.tokens * 100).div_ceil(caps.context_window().max(1)),
                    handover::document_path(session).display(),
                ),
            ));
        }
        if let Err(e) = written {
            tracing::warn!(session_id = %session, "Handover-Dokument nicht geschrieben: {e}");
            out.notices.push(notice(
                NoticeLevel::Warn,
                "Das Handover-Dokument konnte nicht in den Workspace geschrieben werden; der \
                 Agent erhält nur die Kurzfassung."
                    .into(),
            ));
        }
        out.handover = Some(preamble.brief);
    }
    if logged {
        return out;
    }
    if let Some(reason) = &fallback {
        // HAR-019 AC3: Hinweis-Event; der Grund steht auch in `session.forked`.
        out.notices.push(notice(
            NoticeLevel::Warn,
            format!(
                "Der Verlauf ließ sich nicht nativ für {} rekonstruieren ({reason}). Die \
                 Session läuft mit einer Übergabe-Präambel weiter.",
                handover::harness_label(target.as_str())
            ),
        ));
    }
    out.first.push(match plan.kind {
        PlanKind::Fork => EventPayload::SessionForked(SessionForked {
            from_session: plan.from_session,
            at_seq: plan.at_seq,
            reason: plan.reason,
            harness: Some(target.to_string()),
            from_harness: Some(plan.from_harness.clone()),
            history_mode: Some(mode),
            fallback_reason: fallback,
        }),
        PlanKind::Resume => EventPayload::SessionResumed(SessionResumed {
            mode: if mode == HistoryMode::Preamble {
                ResumeMode::Handover
            } else {
                ResumeMode::Native
            },
        }),
    });
    out
}

/// Präambel des Plans für den Ziel-Harness (HAR-018).
pub fn render(
    plan: &ForkPlan,
    session: SessionId,
    target: &HarnessId,
    caps: &Capabilities,
) -> Preamble {
    handover::render(
        &plan.context(),
        target.as_str(),
        caps.context_window(),
        &handover::document_path(session).display().to_string(),
    )
}
