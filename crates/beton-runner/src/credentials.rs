//! Credentials des Stufenmodells über den Tunnel (PRX-006, SEC-001, PRX-009).
//!
//! Werte holt der Daemon (`credential.resolve`: Secret-Store über die Proxy-Capability, Env,
//! Datei, Kommando), jede Nutzung und jeder Fehlgebrauch geht ins Audit-Log
//! (`credential.used`, `credential.misuse`). Werte liegen nur im Speicher des Runners, bis zu
//! ihrer `ttl`; danach wird neu geholt, und scheitert das, gibt es einen Fehler, nie den alten
//! Wert (PRX-006 AC5). Egress-Events gehen ins Session-Log.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use beton_core::event::EventPayload;
use beton_proxy::credentials::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

/// Frist für Credential-Aufrufe über den Tunnel; danach scheitert der Request (fail closed).
pub const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Ein System-Call über den Tunnel (`tool`, `args`) → Ergebnis oder Fehlertext.
pub type Calls =
    Arc<dyn Fn(&str, Value) -> BoxFuture<'static, Result<Value, String>> + Send + Sync>;

/// System-Calls über den Kanal des Runners zum Tunnel, mit [`CREDENTIAL_TIMEOUT`].
pub fn channel_calls(tx: mpsc::UnboundedSender<crate::mcp::SystemRequest>) -> Calls {
    Arc::new(move |tool: &str, args: Value| {
        let (reply, rx) = oneshot::channel();
        let sent = tx.send(crate::mcp::SystemRequest {
            tool: tool.into(),
            args,
            reply,
        });
        Box::pin(async move {
            sent.map_err(|_| "Runner beendet".to_owned())?;
            match tokio::time::timeout(CREDENTIAL_TIMEOUT, rx).await {
                Ok(Ok(Ok(v))) => Ok(v),
                Ok(Ok(Err(p))) => Err(format!(
                    "{}: {}",
                    p["code"].as_str().unwrap_or("failed"),
                    p["detail"].as_str().unwrap_or_default()
                )),
                Ok(Err(_)) => Err("keine Antwort des Daemons".into()),
                Err(_) => Err("Daemon antwortet nicht (Zeitüberschreitung)".into()),
            }
        })
    })
}

struct Cached {
    value: Zeroizing<Vec<u8>>,
    until: Option<tokio::time::Instant>,
}

/// Quelle, Audit und Event-Empfänger der Proxy-Instanzen einer Session.
pub struct TunnelCredentials {
    calls: Calls,
    events: mpsc::UnboundedSender<EventPayload>,
    cache: tokio::sync::Mutex<HashMap<String, Cached>>,
}

impl TunnelCredentials {
    pub fn new(calls: Calls, events: mpsc::UnboundedSender<EventPayload>) -> Self {
        Self {
            calls,
            events,
            cache: tokio::sync::Mutex::default(),
        }
    }
}

impl beton_proxy::CredentialSource for TunnelCredentials {
    fn value<'a>(
        &'a self,
        binding: &'a str,
    ) -> BoxFuture<'a, Result<beton_proxy::SecretBytes, String>> {
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            let now = tokio::time::Instant::now();
            if let Some(c) = cache.get(binding)
                && c.until.is_none_or(|u| u > now)
            {
                return Ok(beton_proxy::SecretBytes::new(c.value.to_vec()));
            }
            // Abgelaufen oder neu: nie den alten Wert weiterverwenden.
            cache.remove(binding);
            let v = (self.calls)("credential.resolve", json!({ "binding": binding })).await?;
            let encoded = Zeroizing::new(
                v["value_b64"]
                    .as_str()
                    .ok_or("Antwort ohne Wert")?
                    .to_owned(),
            );
            let value = Zeroizing::new(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| "Wert nicht lesbar")?,
            );
            let until = v["ttl_s"].as_u64().map(|s| now + Duration::from_secs(s));
            let out = beton_proxy::SecretBytes::new(value.to_vec());
            cache.insert(binding.to_owned(), Cached { value, until });
            Ok(out)
        })
    }
}

impl beton_proxy::CredentialAudit for TunnelCredentials {
    fn used<'a>(&'a self, binding: &'a str, host: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            (self.calls)(
                "credential.used",
                json!({ "binding": binding, "host": host }),
            )
            .await
            .map(|_| ())
        })
    }

    fn misuse<'a>(
        &'a self,
        binding: Option<&'a str>,
        host: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let args = json!({ "binding": binding, "host": host, "reason": reason });
            if let Err(e) = (self.calls)("credential.misuse", args).await {
                tracing::warn!(reason, "credential.misuse nicht auditiert: {e}");
            }
        })
    }
}

impl beton_proxy::EgressEvents for TunnelCredentials {
    fn blocked(&self, event: beton_core::event::EgressBlocked) {
        let _ = self.events.send(EventPayload::EgressBlocked(event));
    }

    fn summary(&self, summary: beton_core::event::EgressSummary) {
        let _ = self.events.send(EventPayload::EgressSummary(summary));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Mutex;

    use beton_proxy::CredentialSource as _;

    use super::*;

    /// Daemon-Attrappe: liefert der Reihe nach die Antworten und zählt die Aufrufe.
    fn daemon(answers: Vec<Result<Value, String>>) -> (Calls, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(answers.into_iter()));
        let l = log.clone();
        let calls: Calls = Arc::new(move |tool: &str, _args| {
            l.lock().unwrap().push(tool.to_owned());
            let next = answers
                .lock()
                .unwrap()
                .next()
                .unwrap_or(Err("keine Antwort".into()));
            Box::pin(async move { next })
        });
        (calls, log)
    }

    fn answer(v: &str, ttl: Option<u64>) -> Result<Value, String> {
        Ok(json!({
            "value_b64": base64::engine::general_purpose::STANDARD.encode(v),
            "ttl_s": ttl,
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn prx_006_ac5_command_values_expire_and_are_never_reused_after_a_failure() {
        let (calls, log) = daemon(vec![
            answer("FIXTURE-1", Some(60)),
            answer("FIXTURE-2", Some(60)),
            Err("credential_source_error: Kommando endete mit 1".into()),
            answer("FIXTURE-3", Some(60)),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        let c = TunnelCredentials::new(calls, tx);
        assert_eq!(c.value("vault").await.unwrap().expose(), b"FIXTURE-1");
        tokio::time::sleep(Duration::from_secs(59)).await;
        assert_eq!(c.value("vault").await.unwrap().expose(), b"FIXTURE-1");
        assert_eq!(
            log.lock().unwrap().len(),
            1,
            "innerhalb der ttl kein neuer Aufruf"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(c.value("vault").await.unwrap().expose(), b"FIXTURE-2");
        tokio::time::sleep(Duration::from_secs(61)).await;
        // Scheitert die Quelle nach Ablauf, gibt es keinen (veralteten) Wert …
        let err = c.value("vault").await.unwrap_err();
        assert!(err.contains("credential_source_error"), "{err}");
        assert!(!err.contains("FIXTURE"));
        // … und der nächste Request versucht es erneut.
        assert_eq!(c.value("vault").await.unwrap().expose(), b"FIXTURE-3");
        assert_eq!(log.lock().unwrap().len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn prx_006_values_without_ttl_stay_for_the_session() {
        let (calls, log) = daemon(vec![answer("FIXTURE-ENV", None)]);
        let (tx, _rx) = mpsc::unbounded_channel();
        let c = TunnelCredentials::new(calls, tx);
        for _ in 0..3 {
            assert_eq!(c.value("gh").await.unwrap().expose(), b"FIXTURE-ENV");
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        assert_eq!(log.lock().unwrap().as_slice(), ["credential.resolve"]);
    }
}
