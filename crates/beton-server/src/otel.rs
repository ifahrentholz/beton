//! OpenTelemetry-Traces je Session (OBS-003).
//!
//! Export nur, wenn ein Endpoint konfiguriert ist: `observability.otlp.endpoint` in der
//! User-Konfiguration, sonst `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` bzw.
//! `OTEL_EXPORTER_OTLP_ENDPOINT`. Ohne Endpoint entsteht kein Tracer und keine Verbindung
//! (ADR-0025, ADR-0033; Registry `observability.otlp`).
//!
//! Die Spans entstehen aus dem Event-Log, nachdem Events gespeichert (und damit geschwärzt,
//! SEC-013) sind. Jede Session hat eine stabile Trace-ID (die 128 Bit ihrer ULID); Span-IDs
//! sind aus Session und Bezug (Turn, Tool-Call, Approval, Entscheidung) abgeleitet, sodass
//! Eltern auch über einen Neustart des Daemons hinweg stimmen. Hierarchie:
//!
//! - `turn` (je Turn eine Wurzel) mit `model.request` (je `cost.delta`), `tool.call` und
//!   `cost` (Summe des Turns) als Kindern,
//! - `policy.evaluate` unter dem Tool-Call, den es bewertet, sonst unter dem Turn,
//! - `approval.wait` unter dem Tool-Call bzw. Turn.
//!
//! Inhalte (Prompt, Tool-Argumente) nur mit `observability.capture_content: true`, und auch
//! dann geschwärzt. Übertragen wird OTLP/HTTP mit JSON-Kodierung (`<endpoint>/v1/traces`).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::Event;
use beton_core::id::{OrgId, SessionId};
use beton_store::{Store, ValueRedactor};
use serde_json::Value;
use tokio::sync::mpsc;

/// Pfad der Traces relativ zu `OTEL_EXPORTER_OTLP_ENDPOINT`.
pub const TRACES_PATH: &str = "/v1/traces";

/// Ein auflösbarer Export-Endpoint samt Optionen.
#[derive(Debug, Clone, PartialEq)]
pub struct OtelConfig {
    /// Vollständige URL für Traces, z. B. `http://127.0.0.1:4318/v1/traces`.
    pub endpoint: String,
    /// Zusätzliche Header, z. B. für die Anmeldung beim Collector (werden nie geloggt).
    pub headers: Vec<(String, String)>,
    /// Prompts und Tool-Argumente als Attribute (geschwärzt).
    pub capture_content: bool,
    /// Wie oft gesammelte Spans gesendet werden.
    pub flush_interval: Duration,
}

impl OtelConfig {
    /// Löst die Konfiguration auf. `Ok(None)` heißt: kein Endpoint, also kein Export.
    pub fn resolve(
        endpoint: Option<&str>,
        headers: &BTreeMap<String, String>,
        capture_content: bool,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, String> {
        let _ = (endpoint, headers, capture_content, env);
        Ok(None)
    }
}

/// Wert eines Span-Attributs.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Ein abgeschlossener Span.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanData {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent: Option<[u8; 8]>,
    pub name: &'static str,
    pub start_ns: u64,
    pub end_ns: u64,
    pub attributes: Vec<(&'static str, AttrValue)>,
    pub error: bool,
}

/// Leitet Spans aus dem Event-Log ab und reicht sie an den Exporter.
pub struct SessionTracer {
    capture_content: bool,
    sink: mpsc::Sender<SpanData>,
    state: Mutex<HashMap<SessionId, ()>>,
}

impl std::fmt::Debug for SessionTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTracer")
            .field("capture_content", &self.capture_content)
            .finish_non_exhaustive()
    }
}

impl SessionTracer {
    /// Tracer mit eigener Senke (Tests).
    pub fn with_sink(capture_content: bool, sink: mpsc::Sender<SpanData>) -> Self {
        Self {
            capture_content,
            sink,
            state: Mutex::default(),
        }
    }

    /// Tracer mit OTLP-Exporter; Attribute laufen vor dem Senden durch `redactor`.
    pub fn start(config: OtelConfig, redactor: Arc<dyn ValueRedactor>) -> Arc<Self> {
        let (tx, _rx) = mpsc::channel(4096);
        let _ = redactor;
        Arc::new(Self::with_sink(config.capture_content, tx))
    }

    /// Verarbeitet frisch gespeicherte Events einer Session.
    pub async fn observe(&self, store: &Store, org: OrgId, session: SessionId, events: &[Event]) {
        let _ = (store, org, session, events, &self.state, &self.sink);
    }
}

/// Stabile Trace-ID einer Session (OBS-003 AC2).
pub fn trace_id(session: SessionId) -> [u8; 16] {
    let _ = session;
    [0; 16]
}

/// Rumpf eines OTLP/HTTP-JSON-Exports (`ExportTraceServiceRequest`).
pub fn otlp_json(spans: &[SpanData]) -> Value {
    let _ = spans;
    Value::Null
}

#[cfg(test)]
mod tests;
