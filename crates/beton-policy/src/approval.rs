//! Eine Approval-Card aus allen `ask`-Gründen (POL-010).
//!
//! - **Timeout:** kleinster `approval.timeout` aller beteiligten Regeln, sonst
//!   `async.approval.timeout` des Agents (`fallback`), sonst [`DEFAULT_TIMEOUT`] (24 h).
//! - **`on_timeout`:** strengster Wert aller beteiligten Regeln (`abort` > `deny` > `allow`);
//!   ohne Angabe gilt `deny` (fail closed). `allow` ist nur wirksam, wenn keine Regel einer
//!   autoritativeren Ebene als eine der `allow`-Regeln `allow_on_timeout: false` setzt.
//! - **Optionen:** `approve`, `approve_session` (nur wenn alle beteiligten Regeln
//!   `remember: session` erlauben – Default *(Annahme)* – und einen Fingerprint haben, also in
//!   Phase `tool_call`; Defaults einer Ebene sind nie merkbar), `deny`.

use std::time::Duration;

use crate::doc::{Remember, TimeoutAction};
use crate::engine::Reason;

/// Timeout ohne Angabe in Regeln und Agent.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// Wie die Card entschieden werden kann und was ohne Antwort geschieht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPlan {
    pub timeout: Duration,
    pub on_timeout: TimeoutAction,
    /// „Für diese Session erlauben“ wird angeboten.
    pub approve_session: bool,
}

impl ApprovalPlan {
    /// Optionen der Card in fester Reihenfolge.
    pub fn options(&self) -> Vec<&'static str> {
        if self.approve_session {
            vec!["approve", "approve_session", "deny"]
        } else {
            vec!["approve", "deny"]
        }
    }
}

fn strictness(a: TimeoutAction) -> u8 {
    match a {
        TimeoutAction::Allow => 0,
        TimeoutAction::Deny => 1,
        TimeoutAction::Abort => 2,
    }
}

fn rule_timeout(r: &Reason) -> Option<Duration> {
    let text = r.approval.as_ref()?.timeout.as_deref()?;
    let d = crate::cel::ext::parse_duration(text).ok()?;
    Duration::try_from(d).ok().filter(|d| !d.is_zero())
}

/// Plan der Card für die `ask`-Gründe einer Entscheidung.
pub fn plan(reasons: &[Reason], fallback: Option<Duration>) -> ApprovalPlan {
    let timeout = reasons
        .iter()
        .filter_map(rule_timeout)
        .min()
        .or(fallback.filter(|d| !d.is_zero()))
        .unwrap_or(DEFAULT_TIMEOUT);

    let action = |r: &Reason| {
        r.approval
            .as_ref()
            .and_then(|a| a.on_timeout)
            .unwrap_or(TimeoutAction::Deny)
    };
    let mut on_timeout = reasons
        .iter()
        .map(action)
        .max_by_key(|a| strictness(*a))
        .unwrap_or(TimeoutAction::Deny);
    if on_timeout == TimeoutAction::Allow {
        let vetoed = reasons.iter().any(|allowing| {
            reasons.iter().any(|v| {
                v.approval.as_ref().and_then(|a| a.allow_on_timeout) == Some(false)
                    && v.rule.scope < allowing.rule.scope
            })
        });
        if vetoed {
            on_timeout = TimeoutAction::Deny;
        }
    }

    let approve_session = !reasons.is_empty()
        && reasons.iter().all(|r| {
            r.fingerprint.is_some()
                && r.approval.as_ref().and_then(|a| a.remember) != Some(Remember::None)
        });

    ApprovalPlan {
        timeout,
        on_timeout,
        approve_session,
    }
}
