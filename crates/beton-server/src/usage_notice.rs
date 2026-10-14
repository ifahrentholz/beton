//! Inbox-Hinweis bei fast aufgebrauchtem Abo-Fenster (USE-004 AC3).
//!
//! Meldet eine Vendor-CLI über `usage.subscription`, dass ein Fenster zu mindestens 90 %
//! genutzt ist, schreibt der Server einmal je Anbieter, Fenster und Reset-Zeitpunkt
//! `usage.rate_limit_high` in die Session; die Inbox zeigt es als Hinweis (UX-001). Abschaltbar
//! über `usage.rate_limit_notice: false` (bis zur Einstellungsseite UX-006, M3).

use std::collections::HashSet;
use std::sync::Mutex;

use beton_core::event::{
    Actor, Event, EventPayload, SystemComponent, UsageRateLimitHigh, UsageSubscription,
};
use beton_core::id::SessionId;
use beton_core::usage::{RATE_LIMIT_NOTICE_PCT, window_name};

use crate::app::AppState;
use crate::problem::Problem;

/// `usage.*` aus der User-Konfiguration.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageConfig {
    /// Inbox-Hinweis ab 90 % eines Abo-Fensters (Default an).
    pub rate_limit_notice: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            rate_limit_notice: true,
        }
    }
}

/// Schon geschriebene Hinweise dieser Laufzeit (gegen Doppelungen bei gleichzeitigen Meldungen;
/// über Neustarts hinweg prüft der Store).
#[derive(Debug, Default)]
pub struct NoticeLog(Mutex<HashSet<(String, String, String)>>);

impl NoticeLog {
    fn claim(&self, key: (String, String, String)) -> bool {
        self.0.lock().map(|mut s| s.insert(key)).unwrap_or(false)
    }

    fn release(&self, key: &(String, String, String)) {
        if let Ok(mut s) = self.0.lock() {
            s.remove(key);
        }
    }
}

/// Fenster aus `written`, die die Schwelle erreichen (je Anbieter und Fenster das letzte).
pub fn high_windows(written: &[Event]) -> Vec<UsageSubscription> {
    let mut out: Vec<UsageSubscription> = Vec::new();
    for e in written {
        if let Some(EventPayload::UsageSubscription(u)) = e.payload()
            && u.used_pct >= RATE_LIMIT_NOTICE_PCT
        {
            let window = window_name(&u.window);
            out.retain(|o| !(o.vendor == u.vendor && o.window == window));
            out.push(UsageSubscription {
                window,
                ..u.clone()
            });
        }
    }
    out
}

/// Beobachtet gespeicherte Runner-Events.
pub struct UsageNoticeObserver(pub AppState);

impl crate::tunnel::EventObserver for UsageNoticeObserver {
    fn observe(&self, session: SessionId, written: &[Event]) {
        if !self.0.runtime.usage.rate_limit_notice {
            return;
        }
        let high = high_windows(written);
        if high.is_empty() {
            return;
        }
        let state = self.0.clone();
        tokio::spawn(async move {
            for u in high {
                if let Err(p) = notify(&state, session, &u).await {
                    tracing::warn!(%session, code = p.code.as_str(), "Hinweis zum Abo-Fenster nicht geschrieben");
                }
            }
        });
    }
}

/// Schreibt den Hinweis, falls es ihn für dieses Fenster und diese Periode noch nicht gibt.
pub async fn notify(
    state: &AppState,
    session: SessionId,
    u: &UsageSubscription,
) -> Result<(), Problem> {
    let key = (u.vendor.clone(), u.window.clone(), u.resets_at.to_string());
    if !state.runtime.usage_notices.claim(key.clone()) {
        return Ok(());
    }
    let org = state.local.org;
    let result = async {
        if state
            .store
            .rate_limit_notice_exists(org, &u.vendor, &u.window, u.resets_at)
            .await?
        {
            return Ok(());
        }
        crate::queue::append(
            &state.events(),
            org,
            session,
            Actor::System {
                component: SystemComponent::Server,
            },
            EventPayload::UsageRateLimitHigh(UsageRateLimitHigh {
                vendor: u.vendor.clone(),
                window: u.window.clone(),
                used_pct: u.used_pct,
                resets_at: u.resets_at,
                threshold_pct: RATE_LIMIT_NOTICE_PCT,
            }),
        )
        .await
        .map(|_| ())
    }
    .await;
    if result.is_err() {
        state.runtime.usage_notices.release(&key);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(vendor: &str, window: &str, used: f64) -> Event {
        Event::new(
            SessionId::new(),
            1,
            Actor::System {
                component: SystemComponent::Server,
            },
            EventPayload::UsageSubscription(UsageSubscription {
                vendor: vendor.into(),
                window: window.into(),
                used_pct: used,
                resets_at: beton_core::time::Timestamp::default(),
            }),
        )
    }

    #[test]
    fn use_004_ac3_threshold_is_90_percent_and_names_are_unified() {
        let high = high_windows(&[
            sub("anthropic", "five_hour", 89.9),
            sub("anthropic", "seven_day", 90.0),
            sub("openai", "300m", 95.0),
            sub("openai", "300m", 97.0),
        ]);
        let got: Vec<_> = high
            .iter()
            .map(|u| (u.vendor.as_str(), u.window.as_str(), u.used_pct))
            .collect();
        assert_eq!(got, [("anthropic", "7d", 90.0), ("openai", "5h", 97.0)]);
    }
}
