//! Native Benachrichtigungen nur für nicht betrachtete Sessions (DESK-005, UX-006).
//!
//! Der [`Tracker`] macht aus Inbox und Event-Log Beobachtungen: neue offene Freigaben und
//! Fragen (aus der Inbox, damit eine sofort automatisch entschiedene Freigabe nie als
//! Rückfrage erscheint, #153), Turn-Ende, getrennte Runner, erschöpfte Budgets und fast
//! aufgebrauchte Kontingente (aus dem Log). Beim ersten Abruf merkt er sich nur den Stand,
//! damit ein App-Start keine alten Ereignisse meldet. Jede Beobachtung entsteht genau einmal.
//!
//! [`decide`] entscheidet, ob daraus eine Mitteilung wird: nicht, wenn die Session in einem
//! fokussierten Fenster sichtbar ist (AC1), nicht, wenn der Kanal Desktop für den Typ aus ist
//! oder Ruhezeiten gelten (UX-006 AC1, AC2). Mit `preview: false` enthält sie nur Session und
//! Ereignis, keinen Nachrichtentext (AC3). Ein Klick öffnet die Session am Ereignis (AC2).

use std::collections::{HashMap, HashSet};

use beton_core::notifications::{NotificationEvent, NotificationSettings};
use serde_json::Value;

use crate::status::{InboxItem, SessionInfo, display_title};

/// Etwas, worüber die App benachrichtigen kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub event: NotificationEvent,
    pub session_id: String,
    pub session_title: String,
    /// Das Ereignis im Log (für den Sprung beim Klick).
    pub seq: u64,
    /// Inhalt für die Vorschau (erste Zeile, Befehl, Frage …); nie ohne `preview` gezeigt.
    pub detail: Option<String>,
}

/// Eine Mitteilung des Betriebssystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub event: NotificationEvent,
    pub session_id: String,
    /// Session-Titel.
    pub title: String,
    pub body: String,
    pub sound: bool,
    /// Wartet auf dich (Freigabe, Frage): mit Schaltfläche „Öffnen“.
    pub waiting: bool,
    /// Route der Web-UI, die ein Klick öffnet.
    pub route: String,
}

/// Welche Sessions gerade in einem fokussierten Fenster zu sehen sind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Focus {
    pub visible: HashSet<String>,
}

impl Focus {
    pub fn sees(&self, session: &str) -> bool {
        self.visible.contains(session)
    }
}

/// Text ohne Vorschau: nur das Ereignis (DESK-005 AC3).
pub fn generic_text(event: NotificationEvent) -> &'static str {
    match event {
        NotificationEvent::ApprovalRequested | NotificationEvent::Question => "Wartet auf dich",
        NotificationEvent::TurnCompleted => "Turn beendet",
        NotificationEvent::AsyncDone => "Hintergrund-Agent fertig",
        NotificationEvent::Mention => "Du wurdest erwähnt",
        NotificationEvent::RunnerLost => "Runner getrennt",
        NotificationEvent::BudgetThreshold => "Budget-Schwelle erreicht",
        NotificationEvent::RateLimitHigh => "Kontingent fast aufgebraucht",
    }
}

fn preview_prefix(event: NotificationEvent) -> &'static str {
    match event {
        NotificationEvent::ApprovalRequested => "Freigabe nötig",
        NotificationEvent::Question => "Frage",
        NotificationEvent::TurnCompleted => "Fertig",
        NotificationEvent::RunnerLost => "Runner getrennt",
        NotificationEvent::BudgetThreshold => "Budget erschöpft",
        NotificationEvent::RateLimitHigh => "Kontingent",
        NotificationEvent::AsyncDone => "Hintergrund-Agent",
        NotificationEvent::Mention => "Erwähnung",
    }
}

/// Erste nicht leere Zeile, höchstens `max` Zeichen.
pub fn first_line(s: &str, max: usize) -> Option<String> {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().count() <= max {
        return Some(line.to_owned());
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    Some(out)
}

/// Mitteilung für `obs` oder `None` (siehe Moduldoku). `minute`: Minuten seit Mitternacht in
/// Ortszeit (für Ruhezeiten).
pub fn decide(
    obs: &Observation,
    focus: &Focus,
    settings: &NotificationSettings,
    minute: u16,
) -> Option<Notification> {
    if focus.sees(&obs.session_id) {
        return None;
    }
    let delivery = settings.delivery(obs.event, minute);
    if !delivery.desktop {
        return None;
    }
    let body = match (&obs.detail, settings.preview) {
        (Some(d), true) => first_line(d, 120)
            .map(|line| format!("{}: {line}", preview_prefix(obs.event)))
            .unwrap_or_else(|| generic_text(obs.event).to_owned()),
        _ => generic_text(obs.event).to_owned(),
    };
    Some(Notification {
        event: obs.event,
        session_id: obs.session_id.clone(),
        title: display_title(&obs.session_title),
        body,
        sound: delivery.sound,
        waiting: matches!(
            obs.event,
            NotificationEvent::ApprovalRequested | NotificationEvent::Question
        ),
        route: if obs.seq > 0 {
            format!("/s/{}?seq={}", obs.session_id, obs.seq)
        } else {
            format!("/s/{}", obs.session_id)
        },
    })
}

/// Text einer Nachricht (`message.completed.content`).
fn text_of(content: &Value) -> String {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Merkt sich, was schon gemeldet ist.
#[derive(Debug, Default)]
pub struct Tracker {
    inbox_primed: bool,
    seen_items: HashSet<String>,
    last_seq: HashMap<String, u64>,
    last_text: HashMap<String, String>,
}

impl Tracker {
    /// Neue offene Freigaben und Fragen; der erste Aufruf merkt sich nur den Stand.
    pub fn inbox(&mut self, open: &[InboxItem]) -> Vec<Observation> {
        let mut out = Vec::new();
        for item in open.iter().filter(|i| i.is_open()) {
            if !self.seen_items.insert(item.id.clone()) || !self.inbox_primed {
                continue;
            }
            out.push(Observation {
                event: if item.kind == "question" {
                    NotificationEvent::Question
                } else {
                    NotificationEvent::ApprovalRequested
                },
                session_id: item.session_id.clone(),
                session_title: item.session_title.clone(),
                seq: item.created_seq,
                detail: Some(item.summary.clone()).filter(|s| !s.trim().is_empty()),
            });
        }
        self.inbox_primed = true;
        out
    }

    /// Ab welcher `seq` die Events einer Session zu lesen sind; `None`, wenn die Session neu ist
    /// (dann [`Tracker::prime`]).
    pub fn after_seq(&self, session: &str) -> Option<u64> {
        self.last_seq.get(session).copied()
    }

    /// Bekannter Stand einer Session ohne Meldung (App-Start).
    pub fn prime(&mut self, session: &str, head_seq: u64) {
        self.last_seq.insert(session.to_owned(), head_seq);
    }

    /// Beobachtungen aus neuen dauerhaften Events einer Session.
    pub fn events(&mut self, session: &SessionInfo, events: &[Value]) -> Vec<Observation> {
        let mut out = Vec::new();
        let last = self.last_seq.get(&session.id).copied().unwrap_or(0);
        let mut max = last;
        for e in events {
            let seq = e["seq"].as_u64().unwrap_or(0);
            if seq <= last {
                continue;
            }
            max = max.max(seq);
            let payload = &e["payload"];
            let obs = |event, detail: Option<String>| Observation {
                event,
                session_id: session.id.clone(),
                session_title: session.title.clone(),
                seq,
                detail,
            };
            match e["type"].as_str().unwrap_or_default() {
                "message.completed" if payload["role"] == "assistant" => {
                    let text = text_of(&payload["content"]);
                    if !text.trim().is_empty() {
                        self.last_text.insert(session.id.clone(), text);
                    }
                }
                "turn.completed" => {
                    out.push(obs(
                        NotificationEvent::TurnCompleted,
                        self.last_text.remove(&session.id),
                    ));
                }
                "runner.status" if payload["to"] == "lost" => {
                    out.push(obs(
                        NotificationEvent::RunnerLost,
                        payload["reason"].as_str().map(str::to_owned),
                    ));
                }
                "budget.exhausted" => {
                    out.push(obs(
                        NotificationEvent::BudgetThreshold,
                        payload["scope"].as_str().map(str::to_owned),
                    ));
                }
                "usage.rate_limit_high" => {
                    let detail = format!(
                        "{} {} zu {:.0} % genutzt",
                        payload["vendor"].as_str().unwrap_or_default(),
                        payload["window"].as_str().unwrap_or_default(),
                        payload["used_pct"].as_f64().unwrap_or_default()
                    );
                    out.push(obs(NotificationEvent::RateLimitHigh, Some(detail)));
                }
                _ => {}
            }
        }
        self.last_seq.insert(session.id.clone(), max);
        out
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obs(event: NotificationEvent) -> Observation {
        Observation {
            event,
            session_id: "ses_a".into(),
            session_title: "Review: Rate-Limiter".into(),
            seq: 7,
            detail: Some(
                "Zwei Anmerkungen: Der Bucket-Speicher wächst unbegrenzt\nzweite Zeile".into(),
            ),
        }
    }

    fn on() -> NotificationSettings {
        let mut s = NotificationSettings::default();
        s.events.turn_completed.desktop = true;
        s
    }

    #[test]
    fn desk_005_ac1_no_notification_when_the_session_is_visible_in_a_focused_window() {
        let s = on();
        let visible = Focus {
            visible: HashSet::from(["ses_a".to_owned()]),
        };
        assert_eq!(
            decide(&obs(NotificationEvent::TurnCompleted), &visible, &s, 600),
            None
        );
        let other = Focus {
            visible: HashSet::from(["ses_b".to_owned()]),
        };
        let n = decide(&obs(NotificationEvent::TurnCompleted), &other, &s, 600).unwrap();
        assert_eq!(n.title, "Review: Rate-Limiter");
        assert_eq!(
            n.body,
            "Fertig: Zwei Anmerkungen: Der Bucket-Speicher wächst unbegrenzt"
        );
        assert!(
            decide(
                &obs(NotificationEvent::TurnCompleted),
                &Focus::default(),
                &s,
                600
            )
            .is_some()
        );
    }

    #[test]
    fn desk_005_ac1_each_turn_end_is_observed_exactly_once() {
        let mut t = Tracker::default();
        let s = crate::status::SessionInfo {
            id: "ses_a".into(),
            title: "A".into(),
            status: "idle".into(),
            harness: "fake".into(),
            archived: false,
            unread: true,
            head_seq: 0,
            changed_at: String::new(),
        };
        t.prime(&s.id, 1);
        let events = vec![
            json!({"seq": 1, "type": "turn.completed", "payload": {}}),
            json!({"seq": 2, "type": "message.completed", "payload": {"role": "assistant", "content": [{"type": "text", "text": "Alles grün"}]}}),
            json!({"seq": 3, "type": "turn.completed", "payload": {}}),
        ];
        let seen = t.events(&s, &events);
        assert_eq!(seen.len(), 1, "seq 1 war schon bekannt");
        assert_eq!(seen[0].seq, 3);
        assert_eq!(seen[0].detail.as_deref(), Some("Alles grün"));
        assert!(t.events(&s, &events).is_empty(), "kein zweites Mal");
        assert_eq!(t.after_seq("ses_a"), Some(3));
    }

    #[test]
    fn desk_005_ac2_approval_notification_opens_the_session_at_the_approval() {
        let mut t = Tracker::default();
        let item = InboxItem {
            id: "inb_1".into(),
            session_id: "ses_a".into(),
            session_title: "Rate-Limiter für die Login-API".into(),
            kind: "approval".into(),
            status: "open".into(),
            summary: "git push -u origin beton/rate-limiter-7f3k".into(),
            created_seq: 41,
        };
        assert!(
            t.inbox(&[]).is_empty(),
            "erster Abruf merkt sich nur den Stand"
        );
        let seen = t.inbox(std::slice::from_ref(&item));
        assert_eq!(seen.len(), 1);
        let n = decide(
            &seen[0],
            &Focus::default(),
            &NotificationSettings::default(),
            600,
        )
        .unwrap();
        assert!(n.waiting && n.sound);
        assert_eq!(n.route, "/s/ses_a?seq=41");
        assert_eq!(
            n.body,
            "Freigabe nötig: git push -u origin beton/rate-limiter-7f3k"
        );
        assert!(t.inbox(&[item]).is_empty(), "dieselbe Freigabe nur einmal");
    }

    #[test]
    fn desk_005_ac3_without_preview_the_notification_has_no_message_text() {
        let mut s = on();
        s.preview = false;
        for event in [
            NotificationEvent::ApprovalRequested,
            NotificationEvent::Question,
            NotificationEvent::TurnCompleted,
            NotificationEvent::RunnerLost,
            NotificationEvent::BudgetThreshold,
            NotificationEvent::RateLimitHigh,
        ] {
            let n = decide(&obs(event), &Focus::default(), &s, 600).unwrap();
            assert_eq!(n.body, generic_text(event), "{event:?}");
            assert!(!n.body.contains("Bucket"), "{event:?}");
            assert!(!n.title.contains("Bucket"));
        }
    }

    #[test]
    fn ux_006_ac1_desktop_channel_off_suppresses_only_the_os_notification() {
        let mut s = on();
        s.events.turn_completed.desktop = false;
        assert_eq!(
            decide(
                &obs(NotificationEvent::TurnCompleted),
                &Focus::default(),
                &s,
                600
            ),
            None
        );
        // Inbox und In-App-Signal hängen nicht an diesem Kanal: Die Freigabe bleibt im Tracker
        // sichtbar und zählt im Badge (siehe status::badge).
        assert!(
            decide(
                &obs(NotificationEvent::ApprovalRequested),
                &Focus::default(),
                &s,
                600
            )
            .is_some()
        );
    }

    #[test]
    fn ux_006_ac2_quiet_hours_let_only_approvals_through() {
        let mut s = on();
        s.quiet_hours.enabled = true;
        let night = 23 * 60;
        assert!(
            decide(
                &obs(NotificationEvent::ApprovalRequested),
                &Focus::default(),
                &s,
                night
            )
            .is_some()
        );
        assert!(
            decide(
                &obs(NotificationEvent::Question),
                &Focus::default(),
                &s,
                night
            )
            .is_none()
        );
        assert!(
            decide(
                &obs(NotificationEvent::TurnCompleted),
                &Focus::default(),
                &s,
                night
            )
            .is_none()
        );
        s.quiet_hours.allow_approvals = false;
        assert!(
            decide(
                &obs(NotificationEvent::ApprovalRequested),
                &Focus::default(),
                &s,
                night
            )
            .is_none()
        );
    }

    #[test]
    fn desk_005_runner_lost_budget_and_rate_limit_come_from_the_log() {
        let mut t = Tracker::default();
        let s = crate::status::SessionInfo {
            id: "ses_a".into(),
            title: "Nächtliches Dependency-Update".into(),
            status: "idle".into(),
            harness: "fake".into(),
            archived: false,
            unread: false,
            head_seq: 0,
            changed_at: String::new(),
        };
        t.prime(&s.id, 0);
        let seen = t.events(
            &s,
            &[
                json!({"seq": 1, "type": "runner.status", "payload": {"from": "running", "to": "lost", "reason": "Host build-02 antwortet nicht mehr"}}),
                json!({"seq": 2, "type": "runner.status", "payload": {"from": "lost", "to": "running"}}),
                json!({"seq": 3, "type": "budget.exhausted", "payload": {"scope": "session", "action": "pause"}}),
                json!({"seq": 4, "type": "usage.rate_limit_high", "payload": {"vendor": "claude", "window": "5h", "used_pct": 91.6, "threshold_pct": 90.0}}),
            ],
        );
        let kinds: Vec<_> = seen.iter().map(|o| o.event).collect();
        assert_eq!(
            kinds,
            vec![
                NotificationEvent::RunnerLost,
                NotificationEvent::BudgetThreshold,
                NotificationEvent::RateLimitHigh
            ]
        );
        assert_eq!(seen[2].detail.as_deref(), Some("claude 5h zu 92 % genutzt"));
    }
}
