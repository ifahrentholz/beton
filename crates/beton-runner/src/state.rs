//! Lebenszyklus eines Runners (RUN-003 AC1).
//!
//! ```text
//! requested → provisioning → starting → connected → busy ⇄ idle → draining → terminated
//!                  │             │           │
//!                  └──── failed ─┴── lost ───┘   (lost → reconnecting → connected | failed)
//! ```

use beton_core::event::RunnerStatus;
use beton_core::id::RunnerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunnerState {
    Requested,
    Provisioning,
    Starting,
    Connected,
    Busy,
    Idle,
    Draining,
    Terminated,
    Failed,
    Lost,
    Reconnecting,
}

impl RunnerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Provisioning => "provisioning",
            Self::Starting => "starting",
            Self::Connected => "connected",
            Self::Busy => "busy",
            Self::Idle => "idle",
            Self::Draining => "draining",
            Self::Terminated => "terminated",
            Self::Failed => "failed",
            Self::Lost => "lost",
            Self::Reconnecting => "reconnecting",
        }
    }

    /// Erlaubte Übergänge laut Zustandsdiagramm.
    pub fn can_go(self, to: Self) -> bool {
        use RunnerState::*;
        // Abbrechen und Fehlschlagen ist aus jedem aktiven Zustand möglich.
        if matches!(to, Failed | Draining) && !matches!(self, Terminated | Failed | Draining) {
            return true;
        }
        matches!(
            (self, to),
            (Requested, Provisioning)
                | (Provisioning, Starting)
                | (Starting, Connected)
                | (Connected, Busy | Idle | Lost)
                | (Busy, Idle | Lost)
                | (Idle, Busy | Lost)
                | (Lost, Reconnecting)
                | (Reconnecting, Connected)
                | (Draining, Terminated)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültiger Runner-Übergang {from:?} → {to:?}")]
pub struct InvalidTransition {
    pub from: RunnerState,
    pub to: RunnerState,
}

/// Zustand mit Ereignis je Übergang.
#[derive(Debug, Clone)]
pub struct Lifecycle {
    pub runner_id: RunnerId,
    state: RunnerState,
}

impl Lifecycle {
    pub fn new(runner_id: RunnerId) -> Self {
        Self {
            runner_id,
            state: RunnerState::Requested,
        }
    }

    pub fn state(&self) -> RunnerState {
        self.state
    }

    /// Wechselt den Zustand und liefert das `runner.status`-Event (`from`, `to`, `reason`).
    pub fn go(
        &mut self,
        to: RunnerState,
        reason: Option<String>,
    ) -> Result<RunnerStatus, InvalidTransition> {
        if !self.state.can_go(to) {
            return Err(InvalidTransition {
                from: self.state,
                to,
            });
        }
        let from = std::mem::replace(&mut self.state, to);
        Ok(RunnerStatus {
            runner_id: self.runner_id,
            from: from.as_str().into(),
            to: to.as_str().into(),
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RunnerState::*;
    use super::*;

    #[test]
    fn run_003_ac1_transitions_emit_events_and_reject_invalid_ones() {
        let mut l = Lifecycle::new(RunnerId::new());
        for to in [
            Provisioning,
            Starting,
            Connected,
            Idle,
            Busy,
            Idle,
            Lost,
            Reconnecting,
            Connected,
            Draining,
            Terminated,
        ] {
            let e = l.go(to, Some("test".into())).unwrap();
            assert_eq!(e.to, to.as_str());
            assert_eq!(e.runner_id, l.runner_id);
        }
        assert_eq!(
            l.go(Busy, None),
            Err(InvalidTransition {
                from: Terminated,
                to: Busy
            })
        );
        let mut l = Lifecycle::new(RunnerId::new());
        assert!(
            l.go(Connected, None).is_err(),
            "requested → connected überspringt Schritte"
        );
        l.go(Provisioning, None).unwrap();
        assert_eq!(
            l.go(Failed, Some("Exit 1".into())).unwrap().from,
            "provisioning"
        );
        assert!(l.go(Idle, None).is_err());
    }
}
