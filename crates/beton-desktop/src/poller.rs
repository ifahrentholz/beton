//! Beobachtet den Daemon für Badge, Tray und Benachrichtigungen (DESK-004, DESK-005,
//! UX-001 AC5).
//!
//! Alle [`Poller::period`] (Standard 500 ms) liest die App über Loopback die offenen Einträge
//! der Inbox, die seit dem letzten Abruf geänderten Sessions (`updated_after`) und deren neue
//! Events. Ein neuer oder entschiedener Eintrag ändert den Badge deshalb binnen einer Periode
//! (Ziel: 1 s, UX-001 AC5; 2 s, DESK-004 AC1). Die Einstellungen (UX-006) liest sie alle paar
//! Sekunden neu, damit Änderungen auf einem anderen Gerät ohne Neustart gelten.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::notifications::NotificationSettings;
use beton_sdk::Client;
use tokio::sync::mpsc;

use crate::notify::{Focus, Notification, Tracker, decide};
use crate::status::{InboxItem, SessionInfo, TrayModel, badge, working};

/// Was der Poller meldet; die App setzt es in Badge, Tray und Mitteilungen um.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Neuer Zählerstand für Dock und Tray.
    Badge(u32),
    /// Neuer Inhalt des Tray-Menüs.
    Tray(TrayModel),
    /// Eine Mitteilung des Betriebssystems.
    Notify(Notification),
    /// Daemon erreichbar bzw. nicht (für Tray-Status und Startbildschirm).
    Reachable(bool),
}

/// Geteilter Stand für Fokus und den letzten Session-Überblick.
#[derive(Debug, Default)]
pub struct Shared {
    pub focus: Mutex<Focus>,
    pub sessions: Mutex<BTreeMap<String, SessionInfo>>,
}

impl Shared {
    /// Sessions, die gerade arbeiten (für den Beenden-Dialog).
    pub fn working(&self) -> Vec<String> {
        self.sessions
            .lock()
            .map(|s| working(&s))
            .unwrap_or_default()
    }
}

/// Ortszeit in Minuten seit Mitternacht (für Ruhezeiten).
pub fn local_minute() -> u16 {
    let now = jiff::Zoned::now();
    u16::try_from(i32::from(now.hour()) * 60 + i32::from(now.minute())).unwrap_or(0)
}

pub struct Poller {
    pub client: Client,
    pub shared: Arc<Shared>,
    pub period: Duration,
    /// Wie oft die Einstellungen neu gelesen werden (in Perioden).
    pub settings_every: u32,
    /// Uhr für Ruhezeiten (Tests setzen eine feste).
    pub clock: fn() -> u16,
}

impl Poller {
    pub fn new(client: Client, shared: Arc<Shared>) -> Self {
        Self {
            client,
            shared,
            period: Duration::from_millis(500),
            settings_every: 6,
            clock: local_minute,
        }
    }

    /// Läuft, bis der Empfänger weg ist.
    pub async fn run(self, tx: mpsc::Sender<Update>) {
        let mut state = State::default();
        let mut tick = 0u32;
        loop {
            if tick.is_multiple_of(self.settings_every.max(1))
                && let Ok(s) = self.client.notification_settings().await
            {
                state.settings = s;
            }
            tick = tick.wrapping_add(1);
            let ok = self.poll(&mut state, &tx).await.is_ok();
            if state.reachable != Some(ok) {
                state.reachable = Some(ok);
                if tx.send(Update::Reachable(ok)).await.is_err() {
                    return;
                }
            }
            if tx.is_closed() {
                return;
            }
            tokio::time::sleep(self.period).await;
        }
    }

    async fn poll(&self, st: &mut State, tx: &mpsc::Sender<Update>) -> Result<(), ()> {
        let inbox = self.client.inbox(Some("open"), 200).await.map_err(|_| ())?;
        let open: Vec<InboxItem> =
            serde_json::from_value(inbox["items"].clone()).unwrap_or_default();
        let mut observations = st.tracker.inbox(&open);

        let page = self
            .client
            .sessions_changed(st.cursor.as_deref())
            .await
            .map_err(|_| ())?;
        let changed: Vec<SessionInfo> = page
            .items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        for s in &changed {
            if st
                .cursor
                .as_deref()
                .is_none_or(|c| s.changed_at.as_str() > c)
            {
                st.cursor = Some(s.changed_at.clone());
            }
            match st.tracker.after_seq(&s.id) {
                None => st.tracker.prime(&s.id, s.head_seq),
                Some(after) if s.head_seq > after => {
                    let mut after = after;
                    while let Ok(page) = self.client.events(&s.id, after, 200).await {
                        let n = page.items.len();
                        observations.extend(st.tracker.events(s, &page.items));
                        after = st.tracker.after_seq(&s.id).unwrap_or(after);
                        if n < 200 {
                            break;
                        }
                    }
                }
                Some(_) => {}
            }
        }
        let sessions = {
            let Ok(mut all) = self.shared.sessions.lock() else {
                return Err(());
            };
            for s in changed {
                all.insert(s.id.clone(), s);
            }
            all.clone()
        };

        let focus = self
            .shared
            .focus
            .lock()
            .map(|f| f.clone())
            .unwrap_or_default();
        let minute = (self.clock)();
        for o in &observations {
            if let Some(n) = decide(o, &focus, &st.settings, minute)
                && tx.send(Update::Notify(n)).await.is_err()
            {
                return Err(());
            }
        }

        let count = badge(&open, &sessions, &st.settings);
        if st.badge != Some(count) {
            st.badge = Some(count);
            let _ = tx.send(Update::Badge(count)).await;
        }
        let tray = TrayModel::build(&open, &sessions);
        if st.tray.as_ref() != Some(&tray) {
            st.tray = Some(tray.clone());
            let _ = tx.send(Update::Tray(tray)).await;
        }
        Ok(())
    }
}

#[derive(Default)]
struct State {
    tracker: Tracker,
    settings: NotificationSettings,
    cursor: Option<String>,
    badge: Option<u32>,
    tray: Option<TrayModel>,
    reachable: Option<bool>,
}
