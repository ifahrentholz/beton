//! `loop_detection` (POL-018): erkennt festgefahrene Agents.
//!
//! Entzuckerung zu zwei Regeln in Phase `tool_call` mit der Aktion `action` (`ask`, `deny`
//! oder `notify`):
//!
//! - `<id>/repeat`: `tool.repeat_count >= identical_threshold` (identische Calls – Klasse,
//!   Name, Argumente – unter den letzten 10 Calls einschließlich des aktuellen);
//! - `<id>/errors`: `tool.consecutive_errors >= consecutive_errors` (fehlgeschlagene Calls in
//!   Folge).
//!
//! Fehlen die Zähler (z. B. `beton policy eval` ohne Session), greift die Regel nicht: Die
//! Erkennung ist eine Heuristik gegen Endlosschleifen, keine Schutzgrenze. Das Fenster ist
//! das von `tool.repeat_count` (10 Calls, POL-004); `window` muss deshalb 10 sein.

use super::{ParamError, Params, Sugar, SugarRule};
use crate::Phase;
use crate::doc::{Action, Channel, NotifyDoc};

const PARAMS: [&str; 4] = [
    "window",
    "identical_threshold",
    "consecutive_errors",
    "action",
];

/// Fenster von `tool.repeat_count` (Runner und Policy-Tests).
pub(crate) const WINDOW: i64 = 10;

pub(super) fn desugar(params: &Params) -> Result<Sugar, ParamError> {
    params.only(&PARAMS)?;
    let window = params.int("window")?.unwrap_or(WINDOW);
    if window != WINDOW {
        return Err(ParamError::new(
            "window",
            format!("`window` muss {WINDOW} sein (Fenster von tool.repeat_count)"),
        ));
    }
    let identical = params.int("identical_threshold")?.unwrap_or(3);
    if identical < 2 {
        return Err(ParamError::new(
            "identical_threshold",
            "`identical_threshold` muss mindestens 2 sein",
        ));
    }
    let errors = params.int("consecutive_errors")?.unwrap_or(5);
    if errors < 1 {
        return Err(ParamError::new(
            "consecutive_errors",
            "`consecutive_errors` muss mindestens 1 sein",
        ));
    }
    let action = match params.text("action")?.as_deref() {
        None | Some("ask") => Action::Ask,
        Some("deny") => Action::Deny,
        Some("notify") => Action::Notify,
        Some(other) => {
            return Err(ParamError::new(
                "action",
                format!("`action: {other}` ist ungültig (ask, deny, notify)"),
            ));
        }
    };
    let notify = |message: &str| {
        (action == Action::Notify).then(|| NotifyDoc {
            channels: vec![Channel::Inbox],
            message: Some(message.to_owned()),
        })
    };
    Ok(Sugar {
        rules: vec![
            SugarRule {
                part: Some("repeat"),
                phase: Phase::ToolCall,
                when: format!("has(tool.repeat_count) && tool.repeat_count >= {identical}"),
                action,
                reason: format!(
                    "Derselbe Tool-Call mindestens {identical}× unter den letzten {WINDOW} Calls \
                     (loop_detection)"
                ),
                kinds: None,
                notify: notify(&format!(
                    "Derselbe Tool-Call {{{{ tool.repeat_count }}}}× unter den letzten {WINDOW} \
                     Calls (loop_detection)"
                )),
            },
            SugarRule {
                part: Some("errors"),
                phase: Phase::ToolCall,
                when: format!(
                    "has(tool.consecutive_errors) && tool.consecutive_errors >= {errors}"
                ),
                action,
                reason: format!(
                    "Mindestens {errors} fehlgeschlagene Tool-Calls in Folge (loop_detection)"
                ),
                kinds: None,
                notify: notify(
                    "{{ tool.consecutive_errors }} fehlgeschlagene Tool-Calls in Folge \
                     (loop_detection)",
                ),
            },
        ],
        defaults: Vec::new(),
    })
}
