//! Modell-Discovery mit Last-Known-Good-Cache (HAR-011 AC3).
//!
//! `GET /models` läuft nur im Daemon (Katalog), mit kurzem Timeout und höchstens alle
//! [`REFRESH_AFTER_OK`] bzw. [`RETRY_AFTER_FAILURE`]. Scheitert der Abruf, liefert der
//! Cache die zuletzt erfolgreiche Liste und markiert sie als veraltet.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Nach einem erfolgreichen Abruf erst nach dieser Zeit erneut fragen.
pub const REFRESH_AFTER_OK: Duration = Duration::from_secs(300);
/// Nach einem Fehlschlag erst nach dieser Zeit erneut fragen.
pub const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);
/// Timeout eines Abrufs (der Katalog soll nicht warten).
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
struct State {
    last_ok: Option<Vec<String>>,
    last_attempt: Option<Instant>,
    last_failed: bool,
}

/// Cache der Modell-Liste eines Providers.
#[derive(Debug, Default)]
pub struct ModelCache {
    state: Mutex<State>,
}

impl ModelCache {
    /// Ist ein neuer Abruf fällig?
    pub fn due(&self, now: Instant) -> bool {
        let Ok(s) = self.state.lock() else {
            return false;
        };
        match s.last_attempt {
            None => true,
            Some(t) => {
                let wait = if s.last_failed {
                    RETRY_AFTER_FAILURE
                } else {
                    REFRESH_AFTER_OK
                };
                now.duration_since(t) >= wait
            }
        }
    }

    /// Ergebnis eines Abrufs eintragen.
    pub fn record(&self, now: Instant, result: Result<Vec<String>, String>) {
        if let Ok(mut s) = self.state.lock() {
            s.last_attempt = Some(now);
            match result {
                Ok(models) => {
                    s.last_ok = Some(models);
                    s.last_failed = false;
                }
                Err(e) => {
                    tracing::debug!("Modell-Discovery fehlgeschlagen: {e}");
                    s.last_failed = true;
                }
            }
        }
    }

    /// Bekannte Modelle und ob die Liste veraltet ist.
    pub fn snapshot(&self) -> (Vec<String>, bool) {
        match self.state.lock() {
            Ok(s) => (s.last_ok.clone().unwrap_or_default(), s.last_failed),
            Err(_) => (Vec::new(), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_known_good_survives_a_failure_and_is_marked_stale() {
        let c = ModelCache::default();
        let t0 = Instant::now();
        assert!(c.due(t0));
        c.record(t0, Ok(vec!["a".into(), "b".into()]));
        assert_eq!(c.snapshot(), (vec!["a".into(), "b".into()], false));
        assert!(!c.due(t0 + Duration::from_secs(10)));
        assert!(c.due(t0 + REFRESH_AFTER_OK));
        c.record(t0 + REFRESH_AFTER_OK, Err("weg".into()));
        assert_eq!(c.snapshot(), (vec!["a".into(), "b".into()], true));
        assert!(!c.due(t0 + REFRESH_AFTER_OK + Duration::from_secs(1)));
        assert!(c.due(t0 + REFRESH_AFTER_OK + RETRY_AFTER_FAILURE));
    }
}
