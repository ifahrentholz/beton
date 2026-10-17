//! Benachrichtigungs-Einstellungen eines Users (UX-006): Matrix Ereignistyp × Kanal,
//! Ruhezeiten und Vorschau.
//!
//! Gespeichert werden sie serverseitig je User (`GET`/`PUT /v1/settings/notifications`) und
//! gelten damit auf allen Geräten. Die Inbox bzw. das In-App-Signal ist kein Kanal dieser
//! Matrix: Sie zeigt immer alles. Web-Push (WEB-014) kommt mit M4 und ist hier noch kein Feld.
//! Ausgewertet werden die Einstellungen von der Desktop-App (DESK-005); das serverseitige
//! Routing (COL-010) folgt mit M4.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ereignistyp, für den eine Benachrichtigung entstehen kann.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    /// Freigabe angefordert (`approval.requested`, außer Fragen).
    ApprovalRequested,
    /// Frage des Agents (`approval.requested` mit `kind: question`).
    Question,
    /// Agent ist fertig (`turn.completed`).
    TurnCompleted,
    /// Hintergrund-Agent fertig oder fehlgeschlagen (ab M5).
    AsyncDone,
    /// Erwähnung in geteilter Session (ab M4).
    Mention,
    /// Runner oder Host getrennt (`runner.status` → `lost`).
    RunnerLost,
    /// Budget-Schwelle erreicht (`budget.exhausted`).
    BudgetThreshold,
    /// Subscription-Kontingent fast aufgebraucht (`usage.rate_limit_high`, USE-004).
    RateLimitHigh,
}

impl NotificationEvent {
    /// Alle Typen in der Reihenfolge der Einstellungsseite.
    pub const ALL: [Self; 8] = [
        Self::ApprovalRequested,
        Self::Question,
        Self::TurnCompleted,
        Self::AsyncDone,
        Self::Mention,
        Self::RunnerLost,
        Self::BudgetThreshold,
        Self::RateLimitHigh,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalRequested => "approval_requested",
            Self::Question => "question",
            Self::TurnCompleted => "turn_completed",
            Self::AsyncDone => "async_done",
            Self::Mention => "mention",
            Self::RunnerLost => "runner_lost",
            Self::BudgetThreshold => "budget_threshold",
            Self::RateLimitHigh => "rate_limit_high",
        }
    }
}

/// Kanäle eines Ereignistyps. Inbox/In-App ist immer an und deshalb kein Feld.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct NotificationChannels {
    /// Mitteilung des Betriebssystems (DESK-005).
    pub desktop: bool,
    /// Ton zur Mitteilung.
    pub sound: bool,
    /// Zählt im Dock- bzw. Tray-Badge (DESK-004).
    pub badge: bool,
}

impl NotificationChannels {
    const fn new(desktop: bool, sound: bool, badge: bool) -> Self {
        Self {
            desktop,
            sound,
            badge,
        }
    }
}

/// Kanäle je Ereignistyp. Defaults wie im Entwurf `settings-notifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationMatrix {
    pub approval_requested: NotificationChannels,
    pub question: NotificationChannels,
    pub turn_completed: NotificationChannels,
    pub async_done: NotificationChannels,
    pub mention: NotificationChannels,
    pub runner_lost: NotificationChannels,
    pub budget_threshold: NotificationChannels,
    pub rate_limit_high: NotificationChannels,
}

impl Default for NotificationMatrix {
    fn default() -> Self {
        Self {
            approval_requested: NotificationChannels::new(true, true, true),
            question: NotificationChannels::new(true, false, true),
            turn_completed: NotificationChannels::new(false, false, false),
            async_done: NotificationChannels::new(true, false, true),
            mention: NotificationChannels::new(true, false, true),
            runner_lost: NotificationChannels::new(true, false, false),
            budget_threshold: NotificationChannels::new(true, false, false),
            rate_limit_high: NotificationChannels::new(true, false, false),
        }
    }
}

impl NotificationMatrix {
    pub fn get(&self, event: NotificationEvent) -> NotificationChannels {
        match event {
            NotificationEvent::ApprovalRequested => self.approval_requested,
            NotificationEvent::Question => self.question,
            NotificationEvent::TurnCompleted => self.turn_completed,
            NotificationEvent::AsyncDone => self.async_done,
            NotificationEvent::Mention => self.mention,
            NotificationEvent::RunnerLost => self.runner_lost,
            NotificationEvent::BudgetThreshold => self.budget_threshold,
            NotificationEvent::RateLimitHigh => self.rate_limit_high,
        }
    }

    pub fn get_mut(&mut self, event: NotificationEvent) -> &mut NotificationChannels {
        match event {
            NotificationEvent::ApprovalRequested => &mut self.approval_requested,
            NotificationEvent::Question => &mut self.question,
            NotificationEvent::TurnCompleted => &mut self.turn_completed,
            NotificationEvent::AsyncDone => &mut self.async_done,
            NotificationEvent::Mention => &mut self.mention,
            NotificationEvent::RunnerLost => &mut self.runner_lost,
            NotificationEvent::BudgetThreshold => &mut self.budget_threshold,
            NotificationEvent::RateLimitHigh => &mut self.rate_limit_high,
        }
    }
}

/// Ruhezeiten: täglich von `start` bis `end` (`HH:MM`, Ortszeit des Geräts; über Mitternacht
/// erlaubt). In dieser Zeit bleiben Mitteilung und Ton stumm, die Inbox sammelt weiter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, deny_unknown_fields)]
pub struct QuietHours {
    pub enabled: bool,
    pub start: String,
    pub end: String,
    /// Freigaben (`approval_requested`) trotzdem melden (UX-006 AC2).
    pub allow_approvals: bool,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "22:00".into(),
            end: "07:00".into(),
            allow_approvals: true,
        }
    }
}

/// Minuten seit Mitternacht aus `HH:MM` (00:00 bis 23:59).
pub fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u16 = h.parse().ok()?;
    let m: u16 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl QuietHours {
    /// Liegt `minute` (Minuten seit Mitternacht, Ortszeit) in den Ruhezeiten? `start == end`
    /// bedeutet den ganzen Tag.
    pub fn contains(&self, minute: u16) -> bool {
        if !self.enabled {
            return false;
        }
        let (Some(start), Some(end)) = (parse_hhmm(&self.start), parse_hhmm(&self.end)) else {
            return false;
        };
        if start == end {
            true
        } else if start < end {
            (start..end).contains(&minute)
        } else {
            minute >= start || minute < end
        }
    }
}

/// Benachrichtigungs-Einstellungen eines Users (UX-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationSettings {
    pub events: NotificationMatrix,
    pub quiet_hours: QuietHours,
    /// Session-Titel plus erste Zeile; `false`: nur Session und Ereignis (DESK-005 AC3).
    pub preview: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            events: NotificationMatrix::default(),
            quiet_hours: QuietHours::default(),
            preview: true,
        }
    }
}

/// Was die Einstellungen für ein Ereignis zu einer bestimmten Uhrzeit erlauben.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    /// Mitteilung des Betriebssystems zeigen.
    pub desktop: bool,
    /// Mit Ton.
    pub sound: bool,
}

impl NotificationSettings {
    /// Prüft die Felder, die das Schema nicht ausdrücken kann.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("quiet_hours.start", &self.quiet_hours.start),
            ("quiet_hours.end", &self.quiet_hours.end),
        ] {
            if parse_hhmm(value).is_none() {
                return Err(format!(
                    "{field}: `{value}` ist keine Uhrzeit im Format HH:MM"
                ));
            }
        }
        Ok(())
    }

    /// Zustellung von `event` um `minute` (Minuten seit Mitternacht, Ortszeit). Während der
    /// Ruhezeiten bleibt alles stumm, außer Freigaben, wenn der User sie zulässt (UX-006 AC2).
    pub fn delivery(&self, event: NotificationEvent, minute: u16) -> Delivery {
        let ch = self.events.get(event);
        let quiet = self.quiet_hours.contains(minute)
            && !(event == NotificationEvent::ApprovalRequested && self.quiet_hours.allow_approvals);
        Delivery {
            desktop: ch.desktop && !quiet,
            sound: ch.desktop && ch.sound && !quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ux_006_defaults_follow_the_design() {
        let s = NotificationSettings::default();
        assert!(s.events.approval_requested.desktop && s.events.approval_requested.sound);
        assert!(
            !s.events.turn_completed.desktop,
            "Turn-Ende standardmäßig ohne Mitteilung"
        );
        assert!(s.events.question.badge && !s.events.runner_lost.badge);
        assert!(s.preview);
        assert!(!s.quiet_hours.enabled && s.quiet_hours.allow_approvals);
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn ux_006_partial_documents_fill_in_defaults_and_unknown_fields_fail() {
        let s: NotificationSettings = serde_json::from_value(serde_json::json!({
            "events": {"turn_completed": {"desktop": true, "sound": false, "badge": false}},
            "preview": false
        }))
        .unwrap();
        assert!(s.events.turn_completed.desktop);
        assert!(
            s.events.approval_requested.desktop,
            "fehlende Typen behalten den Default"
        );
        assert!(!s.preview);
        assert!(
            serde_json::from_value::<NotificationSettings>(serde_json::json!({"push": true}))
                .is_err()
        );
    }

    #[test]
    fn ux_006_ac1_disabled_desktop_channel_suppresses_the_notification() {
        let mut s = NotificationSettings::default();
        s.events.turn_completed.desktop = true;
        assert!(s.delivery(NotificationEvent::TurnCompleted, 600).desktop);
        s.events.turn_completed.desktop = false;
        s.events.turn_completed.sound = true;
        let d = s.delivery(NotificationEvent::TurnCompleted, 600);
        assert!(!d.desktop && !d.sound, "ohne Mitteilung auch kein Ton");
    }

    #[test]
    fn ux_006_ac2_quiet_hours_only_let_approvals_through_when_chosen() {
        let mut s = NotificationSettings::default();
        s.events.turn_completed.desktop = true;
        s.quiet_hours.enabled = true; // 22:00–07:00
        let night = 23 * 60;
        let morning = 6 * 60 + 59;
        let day = 12 * 60;
        for minute in [night, morning] {
            assert!(
                s.delivery(NotificationEvent::ApprovalRequested, minute)
                    .desktop
            );
            for other in NotificationEvent::ALL {
                if other != NotificationEvent::ApprovalRequested {
                    assert!(!s.delivery(other, minute).desktop, "{other:?} um {minute}");
                }
            }
        }
        assert!(s.delivery(NotificationEvent::TurnCompleted, day).desktop);
        assert!(
            !s.delivery(NotificationEvent::TurnCompleted, 7 * 60 - 1)
                .desktop
        );
        assert!(s.delivery(NotificationEvent::TurnCompleted, 7 * 60).desktop);
        s.quiet_hours.allow_approvals = false;
        assert!(
            !s.delivery(NotificationEvent::ApprovalRequested, night)
                .desktop
        );
        assert!(
            s.delivery(NotificationEvent::ApprovalRequested, day)
                .desktop
        );
    }

    #[test]
    fn ux_006_quiet_hours_within_one_day_and_invalid_times() {
        let q = QuietHours {
            enabled: true,
            start: "12:00".into(),
            end: "13:30".into(),
            allow_approvals: false,
        };
        assert!(q.contains(12 * 60) && q.contains(13 * 60 + 29) && !q.contains(13 * 60 + 30));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("7:00"), None);
        assert_eq!(parse_hhmm("07:00"), Some(420));
        let mut s = NotificationSettings::default();
        s.quiet_hours.end = "25:00".into();
        assert!(s.validate().unwrap_err().contains("quiet_hours.end"));
    }
}
