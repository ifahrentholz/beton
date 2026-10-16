//! Runner-Token im Speicher des Runners und seine Erneuerung über den Tunnel (AUTH-011, #243).
//!
//! Das Token kommt über stdin (nie Env oder argv) und gilt `token_ttl` lang (Default 15 min).
//! Ein verbundener Runner erneuert es gleich nach jeder Anmeldung und danach nach einem
//! Drittel der Gültigkeit (`system.call` [`TOKEN_RENEW_TOOL`]); der Server rotiert es dabei.
//! Spätere Anmeldungen (Reconnect, Vorab-Verbindung) nutzen das jeweils neueste Token.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_proto::tunnel::{TOKEN_RENEW_TOOL, TunnelUp};
use serde_json::Value;
use zeroize::Zeroizing;

/// Das aktuelle Runner-Token. Geteilt zwischen allen Verbindungen des Runners; `Debug` zeigt
/// den Wert nie.
#[derive(Clone)]
pub struct RunnerToken(Arc<Mutex<Zeroizing<String>>>);

impl RunnerToken {
    pub fn new(token: String) -> Self {
        Self(Arc::new(Mutex::new(Zeroizing::new(token))))
    }

    /// Wert für `Authorization: Bearer …`.
    pub fn bearer(&self) -> Zeroizing<String> {
        let t = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = Zeroizing::new(String::with_capacity(7 + t.len()));
        out.push_str("Bearer ");
        out.push_str(&t);
        out
    }

    /// Ersetzt das Token (nach einer Erneuerung); das alte wird genullt.
    pub fn replace(&self, token: String) {
        let mut t = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *t = Zeroizing::new(token);
    }
}

impl std::fmt::Debug for RunnerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunnerToken(***)")
    }
}

/// Nach einer gescheiterten Erneuerung wieder versuchen nach …
const RETRY: Duration = Duration::from_secs(30);

/// Zustand der Erneuerung je Verbindung.
#[derive(Debug)]
pub(crate) struct Renewal {
    due: tokio::time::Instant,
    pending: Option<String>,
    next: u64,
}

impl Default for Renewal {
    fn default() -> Self {
        Self {
            due: tokio::time::Instant::now(),
            pending: None,
            next: 0,
        }
    }
}

impl Renewal {
    /// Nach dem (Wieder-)Verbinden: gleich erneuern, die Restlaufzeit ist unbekannt.
    pub(crate) fn reset(&mut self) {
        self.due = tokio::time::Instant::now();
        self.pending = None;
    }

    /// Zeitpunkt der nächsten Erneuerung; `None`, solange eine Antwort aussteht.
    pub(crate) fn due(&self) -> Option<tokio::time::Instant> {
        self.pending.is_none().then_some(self.due)
    }

    /// Nächste Anfrage `system.call` an den Server.
    pub(crate) fn request(&mut self) -> TunnelUp {
        self.next += 1;
        let call_id = format!("renew_{}", self.next);
        self.pending = Some(call_id.clone());
        TunnelUp::SystemCall {
            call_id,
            tool: TOKEN_RENEW_TOOL.into(),
            args: Value::Null,
        }
    }

    /// Verarbeitet ein `system.result`; `true`, wenn es zur Erneuerung gehörte. Mit neuem
    /// Token wird es übernommen und die nächste Erneuerung nach einem Drittel der Gültigkeit
    /// geplant; sonst ein neuer Versuch nach [`RETRY`] (das alte Token gilt bis zum Ablauf).
    pub(crate) fn on_result(
        &mut self,
        call_id: &str,
        result: &mut Option<Value>,
        token: &RunnerToken,
    ) -> bool {
        if self.pending.as_deref() != Some(call_id) {
            return false;
        }
        self.pending = None;
        let now = tokio::time::Instant::now();
        let renewed = result.as_mut().and_then(|r| {
            let ms = r.get("expires_in_ms")?.as_u64()?;
            match r.as_object_mut()?.remove("token")? {
                Value::String(t) if t.starts_with("bt_run_") => Some((t, ms)),
                Value::String(t) => {
                    drop(Zeroizing::new(t));
                    None
                }
                _ => None,
            }
        });
        match renewed {
            Some((t, ms)) => {
                token.replace(t);
                self.due = now + Duration::from_millis(ms) / 3;
            }
            None => {
                tracing::warn!("Runner-Token nicht erneuert; neuer Versuch später");
                self.due = now + RETRY;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// #243: `Debug` zeigt das Runner-Token nie.
    #[test]
    fn auth_011_debug_hides_the_runner_token() {
        let t = RunnerToken::new("bt_run_geheim0123456789".into());
        assert_eq!(format!("{t:?}"), "RunnerToken(***)");
        assert_eq!(&*t.bearer(), "Bearer bt_run_geheim0123456789");
    }

    /// #243 (AUTH-011 AC2): Das erneuerte Token gilt für spätere Anmeldungen; die nächste
    /// Erneuerung folgt nach einem Drittel der Gültigkeit.
    #[tokio::test]
    async fn auth_011_ac2_renewal_replaces_the_token() {
        let token = RunnerToken::new("bt_run_alt".into());
        let mut r = Renewal::default();
        assert!(r.due().is_some(), "gleich nach dem Verbinden");
        let TunnelUp::SystemCall { call_id, tool, .. } = r.request() else {
            panic!("system.call erwartet");
        };
        assert_eq!(tool, TOKEN_RENEW_TOOL);
        assert!(r.due().is_none(), "Antwort steht aus");
        assert!(!r.on_result("sys_1", &mut None, &token), "fremdes Ergebnis");
        let before = tokio::time::Instant::now();
        let mut result = Some(json!({"token": "bt_run_neu", "expires_in_ms": 900_000}));
        assert!(r.on_result(&call_id, &mut result, &token));
        assert_eq!(&*token.bearer(), "Bearer bt_run_neu");
        assert!(result.unwrap().get("token").is_none(), "Wert entnommen");
        let due = r.due().unwrap();
        assert!(
            due >= before + Duration::from_secs(299) && due <= before + Duration::from_secs(301)
        );
        // Abgelehnt: altes Token bleibt, neuer Versuch später.
        let TunnelUp::SystemCall { call_id, .. } = r.request() else {
            panic!()
        };
        assert!(r.on_result(&call_id, &mut None, &token));
        assert_eq!(&*token.bearer(), "Bearer bt_run_neu");
        assert!(r.due().unwrap() >= before + RETRY);
    }
}
