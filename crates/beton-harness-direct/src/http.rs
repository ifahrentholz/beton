//! HTTP-Client des Direkt-API-Harness (HAR-010, HAR-011).
//!
//! Sicherheitsregeln (fail closed):
//! - Keine Weiterleitungen: Ein 3xx endet mit Fehler, statt den Key an ein anderes Ziel zu
//!   schicken.
//! - Kein impliziter Proxy aus der Umgebung (`HTTPS_PROXY` u. a.); der Egress-Proxy von
//!   beton kommt mit M2 (PRX-006) und wird dann ausdrücklich konfiguriert.
//! - Key-Header sind als sensibel markiert; Header, Bodies und Antworten werden nie geloggt.
//!   Fehlertexte des Anbieters laufen durch [`ApiKey::scrub`].
//!
//! Wiederholungen: bei 429, 5xx und Verbindungsfehlern vor dem ersten Byte des Stroms,
//! höchstens [`HttpOptions::max_attempts`] Versuche; `retry-after` wird respektiert, sonst
//! exponentielles Backoff.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use tokio::sync::watch;

use crate::config::{Provider, WireKind};
use crate::secret::ApiKey;
use crate::wire::sse::SseParser;
use crate::wire::{self, StreamError, StreamEvent};

/// Fristen und Wiederholungen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpOptions {
    /// Gesamtdauer eines Requests inklusive Strom (HAR-010: 300 s).
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    /// Versuche je Model-Request (HAR-010 AC3: nach 5 Fehlschlägen `turn.failed`).
    pub max_attempts: u32,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// Obergrenze für `retry-after`.
    pub retry_after_max: Duration,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(300),
            connect_timeout: Duration::from_secs(10),
            max_attempts: 5,
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            retry_after_max: Duration::from_secs(120),
        }
    }
}

/// Fehler eines Model-Requests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// 401/403: Key fehlt, ist falsch oder abgelaufen.
    #[error("HTTP {status}: {message}")]
    Auth { status: u16, message: String },
    #[error("HTTP {status}: {message}")]
    Http {
        status: u16,
        message: String,
        retryable: bool,
    },
    #[error("Weiterleitung (HTTP {status}) abgelehnt: beton folgt keinen Weiterleitungen")]
    Redirect { status: u16 },
    #[error("Verbindung: {0}")]
    Transport(String),
    #[error("Strom: {}: {}", .0.kind, .0.message)]
    Stream(StreamError),
    #[error("nach {attempts} Versuchen: {last}")]
    Exhausted { attempts: u32, last: Box<CallError> },
    #[error("abgebrochen")]
    Cancelled,
}

impl CallError {
    fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Http {
                retryable: true,
                ..
            } | Self::Transport(_)
        )
    }

    /// Fehlercode für `problem.code`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Auth { .. } => "provider_auth",
            Self::Http { .. } => "provider_error",
            Self::Redirect { .. } => "provider_redirect",
            Self::Transport(_) => "provider_unreachable",
            Self::Stream(_) => "provider_stream",
            Self::Exhausted { .. } => "provider_retries_exhausted",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_auth(&self) -> bool {
        match self {
            Self::Auth { .. } => true,
            Self::Exhausted { last, .. } => last.is_auth(),
            _ => false,
        }
    }
}

/// Empfänger der Ereignisse eines Model-Requests.
#[async_trait::async_trait]
pub trait StreamSink: Send {
    /// Eine Wiederholung steht an (vor der Wartezeit).
    async fn retry(&mut self, retry: &Retry);
    async fn event(&mut self, event: StreamEvent);
}

/// Eine Wiederholung, wie sie der Loop meldet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retry {
    pub attempt: u32,
    pub wait: Duration,
    pub reason: String,
}

/// Der Client.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    pub options: HttpOptions,
}

impl Client {
    pub fn new(options: HttpOptions) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(options.connect_timeout)
            .timeout(options.request_timeout)
            .user_agent(concat!("beton/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("HTTP-Client: {e}"))?;
        Ok(Self { http, options })
    }

    fn headers(provider: &Provider, key: Option<&ApiKey>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        match provider.wire {
            WireKind::Anthropic => {
                h.insert(
                    "anthropic-version",
                    HeaderValue::from_static(wire::anthropic::API_VERSION),
                );
                if let Some(k) = key
                    && let Ok(mut v) = HeaderValue::from_str(k.expose())
                {
                    v.set_sensitive(true);
                    h.insert("x-api-key", v);
                }
            }
            WireKind::Openai => {
                if let Some(k) = key
                    && let Ok(mut v) = HeaderValue::from_str(&format!("Bearer {}", k.expose()))
                {
                    v.set_sensitive(true);
                    h.insert("authorization", v);
                }
            }
        }
        h
    }

    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
        self.options
            .backoff_base
            .saturating_mul(factor)
            .min(self.options.backoff_max)
    }

    /// Fehler aus einer Antwort ohne Erfolg.
    async fn failure(
        resp: reqwest::Response,
        key: Option<&ApiKey>,
    ) -> (CallError, Option<Duration>) {
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s >= 0.0)
            .map(Duration::from_secs_f64);
        if (300..400).contains(&status) {
            return (CallError::Redirect { status }, None);
        }
        let body = resp.text().await.unwrap_or_default();
        let mut message = wire::error_message(&body);
        if let Some(k) = key {
            message = k.scrub(&message);
        }
        let err = match status {
            401 | 403 => CallError::Auth { status, message },
            429 | 500..=599 => CallError::Http {
                status,
                message,
                retryable: true,
            },
            _ => CallError::Http {
                status,
                message,
                retryable: false,
            },
        };
        (err, retry_after)
    }

    /// Sendet einen Model-Request und liefert die Ereignisse des Stroms an `sink`.
    pub async fn stream(
        &self,
        provider: &Provider,
        key: Option<&ApiKey>,
        body: &Value,
        cancel: &mut watch::Receiver<bool>,
        sink: &mut dyn StreamSink,
    ) -> Result<(), CallError> {
        let url = provider.endpoint(provider.messages_path());
        let mut attempt = 0;
        let resp = loop {
            attempt += 1;
            let send = self
                .http
                .post(url.clone())
                .headers(Self::headers(provider, key))
                .json(body)
                .send();
            let result = tokio::select! {
                r = send => r,
                _ = cancelled(cancel) => return Err(CallError::Cancelled),
            };
            let (err, retry_after) = match result {
                Ok(resp) if resp.status().is_success() => break resp,
                Ok(resp) => Self::failure(resp, key).await,
                Err(e) => (CallError::Transport(transport_message(&e, key)), None),
            };
            tracing::debug!(provider = %provider.name, attempt, code = err.code(), "Model-Request fehlgeschlagen");
            if !err.retryable() {
                return Err(err);
            }
            if attempt >= self.options.max_attempts {
                return Err(CallError::Exhausted {
                    attempts: attempt,
                    last: Box::new(err),
                });
            }
            let wait = retry_after
                .map(|d| d.min(self.options.retry_after_max))
                .unwrap_or_else(|| self.backoff(attempt));
            sink.retry(&Retry {
                attempt,
                wait,
                reason: err.to_string(),
            })
            .await;
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                _ = cancelled(cancel) => return Err(CallError::Cancelled),
            }
        };
        let mut resp = resp;
        let mut sse = SseParser::default();
        let mut parser = wire::parser(provider.wire);
        loop {
            let chunk = tokio::select! {
                c = resp.chunk() => c,
                _ = cancelled(cancel) => return Err(CallError::Cancelled),
            };
            let chunk = match chunk {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => return Err(CallError::Transport(transport_message(&e, key))),
            };
            let events = sse.push(&chunk).map_err(|e| {
                CallError::Stream(StreamError {
                    kind: "too_large".into(),
                    message: e.to_string(),
                })
            })?;
            for ev in events {
                for out in parser
                    .on_event(ev.event.as_deref(), &ev.data)
                    .map_err(|e| CallError::Stream(scrubbed(e, key)))?
                {
                    sink.event(out).await;
                }
            }
        }
        if let Some(ev) = sse.finish() {
            for out in parser
                .on_event(ev.event.as_deref(), &ev.data)
                .map_err(|e| CallError::Stream(scrubbed(e, key)))?
            {
                sink.event(out).await;
            }
        }
        for out in parser.finish() {
            sink.event(out).await;
        }
        Ok(())
    }

    /// `GET {base_url}/models` (Anthropic: `/v1/models`): IDs der angebotenen Modelle.
    pub async fn list_models(
        &self,
        provider: &Provider,
        key: Option<&ApiKey>,
        timeout: Duration,
    ) -> Result<Vec<String>, CallError> {
        let url = provider.endpoint(provider.models_path());
        let send = self
            .http
            .get(url)
            .headers(Self::headers(provider, key))
            .timeout(timeout)
            .send();
        let resp = send
            .await
            .map_err(|e| CallError::Transport(transport_message(&e, key)))?;
        if !resp.status().is_success() {
            return Err(Self::failure(resp, key).await.0);
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| CallError::Transport(transport_message(&e, key)))?;
        let mut ids: Vec<String> = v["data"]
            .as_array()
            .or_else(|| v["models"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()))
            .map(str::to_owned)
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }
}

async fn cancelled(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

fn transport_message(e: &reqwest::Error, key: Option<&ApiKey>) -> String {
    // Ohne URL: Sie enthält nichts Geheimes, aber auch keinen Mehrwert für das Modell.
    let text = if e.is_timeout() {
        "Zeitüberschreitung".to_owned()
    } else if e.is_connect() {
        "Verbindung fehlgeschlagen".to_owned()
    } else {
        e.without_url_ref().to_string()
    };
    match key {
        Some(k) => k.scrub(&text),
        None => text,
    }
}

trait WithoutUrl {
    fn without_url_ref(&self) -> String;
}

impl WithoutUrl for reqwest::Error {
    fn without_url_ref(&self) -> String {
        let mut s = self.to_string();
        if let Some(url) = self.url() {
            s = s.replace(url.as_str(), "<url>");
        }
        s
    }
}

fn scrubbed(e: StreamError, key: Option<&ApiKey>) -> StreamError {
    match key {
        Some(k) => StreamError {
            kind: e.kind,
            message: k.scrub(&e.message),
        },
        None => e,
    }
}
