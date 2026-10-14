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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{Event, EventPayload, MessageRole};
use beton_core::id::{ApprovalId, OrgId, SessionId, TurnId};
use beton_store::{Store, ValueRedactor};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// Pfad der Traces relativ zu `OTEL_EXPORTER_OTLP_ENDPOINT`.
pub const TRACES_PATH: &str = "/v1/traces";

/// Höchstlänge eines Inhalts-Attributs (Prompt, Tool-Argumente) mit `capture_content`.
const CONTENT_LIMIT: usize = 4096;

/// Spans je Export.
const BATCH: usize = 512;

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
    ///
    /// Reihenfolge: `observability.otlp.endpoint` (Basis-URL), sonst
    /// `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` (vollständig), sonst `OTEL_EXPORTER_OTLP_ENDPOINT`
    /// (Basis-URL). Header aus `OTEL_EXPORTER_OTLP_[TRACES_]HEADERS`, ergänzt bzw. ersetzt durch
    /// `observability.otlp.headers`. Nur das Protokoll `http/json`.
    pub fn resolve(
        endpoint: Option<&str>,
        headers: &BTreeMap<String, String>,
        capture_content: bool,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, String> {
        let var = |k: &str| {
            env(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let endpoint = match endpoint.map(str::trim).filter(|e| !e.is_empty()) {
            Some(base) => with_traces_path(base),
            None => match var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
                Some(full) => full,
                None => match var("OTEL_EXPORTER_OTLP_ENDPOINT") {
                    Some(base) => with_traces_path(&base),
                    None => return Ok(None),
                },
            },
        };
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(format!(
                "OTLP-Endpoint muss mit http:// oder https:// beginnen: {endpoint}"
            ));
        }
        if let Some(protocol) =
            var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL").or_else(|| var("OTEL_EXPORTER_OTLP_PROTOCOL"))
            && protocol != "http/json"
        {
            return Err(format!(
                "OTLP-Protokoll `{protocol}` wird nicht unterstützt; beton sendet http/json"
            ));
        }
        let mut pairs: Vec<(String, String)> = var("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
            .or_else(|| var("OTEL_EXPORTER_OTLP_HEADERS"))
            .map(|h| parse_headers(&h))
            .unwrap_or_default();
        for (k, v) in headers {
            pairs.retain(|(key, _)| !key.eq_ignore_ascii_case(k));
            pairs.push((k.clone(), v.clone()));
        }
        Ok(Some(Self {
            endpoint,
            headers: pairs,
            capture_content,
            flush_interval: Duration::from_secs(2),
        }))
    }
}

fn with_traces_path(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with(TRACES_PATH) {
        base.to_owned()
    } else {
        format!("{base}{TRACES_PATH}")
    }
}

/// `k=v,k2=v2` mit Prozent-kodierten Werten (OTel-Spezifikation).
fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_owned(), percent_decode(v.trim())))
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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

/// Offene Spans einer Session.
#[derive(Default)]
struct Open {
    harness: Option<String>,
    /// Text der letzten Nutzereingabe (nur mit `capture_content`).
    prompt: Option<String>,
    turns: HashMap<TurnId, TurnOpen>,
    tools: HashMap<String, ToolOpen>,
    approvals: HashMap<ApprovalId, ApprovalOpen>,
}

struct TurnOpen {
    start_ns: u64,
    prompt: Option<String>,
    requests: i64,
    cost_micro: Option<i64>,
    input_tokens: u64,
    output_tokens: u64,
}

struct ToolOpen {
    start_ns: u64,
    turn: Option<TurnId>,
    name: String,
    kind: Option<String>,
    args: Option<String>,
}

struct ApprovalOpen {
    start_ns: u64,
    parent: Option<[u8; 8]>,
    kind: String,
}

/// Leitet Spans aus dem Event-Log ab und reicht sie an den Exporter.
pub struct SessionTracer {
    capture_content: bool,
    sink: mpsc::Sender<SpanData>,
    state: Mutex<HashMap<SessionId, Open>>,
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

    /// Tracer mit OTLP-Exporter; Attribute laufen vor dem Senden durch `redactor` (fail
    /// closed: ist die Pipeline nicht bereit, wird der Stapel verworfen). Braucht eine
    /// laufende Tokio-Laufzeit.
    pub fn start(config: OtelConfig, redactor: Arc<dyn ValueRedactor>) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(4096);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let capture_content = config.capture_content;
        tokio::spawn(export_loop(rx, config, client, redactor));
        Arc::new(Self::with_sink(capture_content, tx))
    }

    /// Verarbeitet frisch gespeicherte Events einer Session.
    pub async fn observe(&self, store: &Store, org: OrgId, session: SessionId, events: &[Event]) {
        for e in events {
            let payload = match e.payload() {
                Some(p) => Cow::Borrowed(p),
                None => match store.resolve_payload(org, e).await {
                    Ok(p) => Cow::Owned(p),
                    Err(_) => continue,
                },
            };
            let spans = {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                let open = state.entry(session).or_default();
                self.apply(session, open, e, &payload)
            };
            for span in spans {
                if self.sink.try_send(span).is_err() {
                    tracing::debug!(%session, "Span verworfen (Exporter ausgelastet)");
                }
            }
        }
    }

    fn apply(
        &self,
        session: SessionId,
        open: &mut Open,
        e: &Event,
        payload: &EventPayload,
    ) -> Vec<SpanData> {
        let ts = nanos(e);
        let trace = trace_id(session);
        let span = |name: &'static str, key: &str| SpanData {
            trace_id: trace,
            span_id: span_id(session, name, key),
            parent: None,
            name,
            start_ns: ts,
            end_ns: ts,
            attributes: Vec::new(),
            error: false,
        };
        let turn_span =
            |turn: Option<TurnId>| turn.map(|t| span_id(session, "turn", &t.to_string()));
        let mut out = Vec::new();
        match payload {
            EventPayload::SessionCreated(c) => open.harness = Some(c.harness.clone()),
            EventPayload::MessageCompleted(m) if m.role == MessageRole::User => {
                if self.capture_content {
                    let text: Vec<&str> = m
                        .content
                        .iter()
                        .filter_map(|c| c.get("text").and_then(Value::as_str))
                        .collect();
                    open.prompt = Some(truncate(&text.join("\n")));
                }
            }
            EventPayload::TurnStarted(t) => {
                open.turns.insert(
                    t.turn_id,
                    TurnOpen {
                        start_ns: ts,
                        prompt: open.prompt.take(),
                        requests: 0,
                        cost_micro: None,
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                );
            }
            EventPayload::TurnCompleted(_)
            | EventPayload::TurnFailed(_)
            | EventPayload::TurnInterrupted(_) => {
                let (turn_id, status, stop) = match payload {
                    EventPayload::TurnCompleted(t) => {
                        (t.turn_id, "completed", Some(&t.stop_reason))
                    }
                    EventPayload::TurnFailed(t) => (t.turn_id, "failed", None),
                    EventPayload::TurnInterrupted(t) => (t.turn_id, "interrupted", None),
                    _ => return out,
                };
                let key = turn_id.to_string();
                let opened = open.turns.remove(&turn_id);
                let mut s = span("turn", &key);
                s.start_ns = opened.as_ref().map_or(ts, |o| o.start_ns);
                s.error = status == "failed";
                s.attributes
                    .push(("beton.session.id", str_attr(session.to_string())));
                s.attributes.push(("beton.turn.id", str_attr(key.clone())));
                if let Some(h) = &open.harness {
                    s.attributes.push(("beton.harness", str_attr(h.clone())));
                }
                s.attributes
                    .push(("beton.turn.status", str_attr(status.to_owned())));
                if let Some(stop) = stop {
                    s.attributes
                        .push(("beton.turn.stop_reason", str_attr(stop.clone())));
                }
                if let Some(o) = &opened {
                    if let Some(p) = &o.prompt {
                        s.attributes.push(("gen_ai.prompt", str_attr(p.clone())));
                    }
                    if o.requests > 0 {
                        let mut c = span("cost", &key);
                        c.parent = Some(s.span_id);
                        c.attributes
                            .push(("beton.cost.requests", AttrValue::Int(o.requests)));
                        c.attributes.push((
                            "gen_ai.usage.input_tokens",
                            AttrValue::Int(int(o.input_tokens)),
                        ));
                        c.attributes.push((
                            "gen_ai.usage.output_tokens",
                            AttrValue::Int(int(o.output_tokens)),
                        ));
                        if let Some(micro) = o.cost_micro {
                            c.attributes
                                .push(("beton.cost.usd", AttrValue::Float(micro as f64 / 1e6)));
                        }
                        out.push(c);
                    }
                }
                out.push(s);
            }
            EventPayload::ToolCallRequested(r) => {
                open.tools.insert(
                    r.call_id.clone(),
                    ToolOpen {
                        start_ns: ts,
                        turn: e.turn_id,
                        name: r.tool.clone(),
                        kind: r.kind.clone(),
                        args: self.capture_content.then(|| truncate(&r.args.to_string())),
                    },
                );
            }
            EventPayload::ToolCallCompleted(c) => {
                let opened = open.tools.remove(&c.call_id);
                let mut s = span("tool.call", &c.call_id);
                s.start_ns = opened
                    .as_ref()
                    .map_or(ts.saturating_sub(c.duration_ms * 1_000_000), |o| o.start_ns);
                s.parent = turn_span(opened.as_ref().and_then(|o| o.turn).or(e.turn_id));
                let status = enum_str(&c.status);
                s.error = status != "ok";
                if let Some(o) = &opened {
                    s.attributes
                        .push(("beton.tool.name", str_attr(o.name.clone())));
                    s.attributes
                        .push(("gen_ai.tool.name", str_attr(o.name.clone())));
                    if let Some(kind) = &o.kind {
                        s.attributes
                            .push(("beton.tool.kind", str_attr(kind.clone())));
                    }
                    if let Some(args) = &o.args {
                        s.attributes
                            .push(("beton.tool.args", str_attr(args.clone())));
                    }
                }
                s.attributes
                    .push(("beton.tool.call_id", str_attr(c.call_id.clone())));
                s.attributes.push(("beton.tool.status", str_attr(status)));
                s.attributes
                    .push(("beton.tool.duration_ms", AttrValue::Int(int(c.duration_ms))));
                out.push(s);
            }
            EventPayload::PolicyDecision(d) => {
                let mut s = span("policy.evaluate", &d.decision_id);
                s.start_ns = ts.saturating_sub(d.eval_us * 1000);
                s.parent = match d.subject_ref.get("call_id").and_then(Value::as_str) {
                    Some(call) => Some(span_id(session, "tool.call", call)),
                    None => turn_span(e.turn_id),
                };
                let rules: Vec<&str> = d
                    .matched
                    .iter()
                    .filter_map(|m| m.get("rule_id").and_then(Value::as_str))
                    .collect();
                s.attributes
                    .push(("beton.policy.phase", str_attr(d.phase.clone())));
                s.attributes
                    .push(("beton.policy.decision", str_attr(enum_str(&d.outcome))));
                s.attributes
                    .push(("beton.policy.rule_ids", str_attr(rules.join(","))));
                s.attributes
                    .push(("beton.policy.enforcement", str_attr(d.enforcement.clone())));
                s.attributes
                    .push(("beton.policy.decision_id", str_attr(d.decision_id.clone())));
                s.error = !d.errors.is_empty();
                out.push(s);
            }
            EventPayload::ApprovalRequested(a) => {
                let parent = match a.subject.get("call_id").and_then(Value::as_str) {
                    Some(call) => Some(span_id(session, "tool.call", call)),
                    None => turn_span(e.turn_id),
                };
                open.approvals.insert(
                    a.approval_id,
                    ApprovalOpen {
                        start_ns: ts,
                        parent,
                        kind: enum_str(&a.kind),
                    },
                );
            }
            EventPayload::ApprovalResolved(r) => {
                let opened = open.approvals.remove(&r.approval_id);
                let mut s = span("approval.wait", &r.approval_id.to_string());
                if let Some(o) = &opened {
                    s.start_ns = o.start_ns;
                    s.parent = o.parent;
                    s.attributes
                        .push(("beton.approval.kind", str_attr(o.kind.clone())));
                }
                s.attributes
                    .push(("beton.approval.id", str_attr(r.approval_id.to_string())));
                s.attributes
                    .push(("beton.approval.decision", str_attr(enum_str(&r.decision))));
                s.attributes
                    .push(("beton.approval.via", str_attr(enum_str(&r.via))));
                out.push(s);
            }
            EventPayload::CostDelta(c) => {
                let mut s = span("model.request", &e.id.to_string());
                s.parent = turn_span(e.turn_id);
                s.attributes
                    .push(("gen_ai.system", str_attr(c.harness.clone())));
                s.attributes
                    .push(("gen_ai.request.model", str_attr(c.model.clone())));
                s.attributes.push((
                    "gen_ai.usage.input_tokens",
                    AttrValue::Int(int(c.input_tokens)),
                ));
                s.attributes.push((
                    "gen_ai.usage.output_tokens",
                    AttrValue::Int(int(c.output_tokens)),
                ));
                s.attributes.push((
                    "gen_ai.usage.cache_read_tokens",
                    AttrValue::Int(int(c.cache_read_tokens)),
                ));
                s.attributes.push((
                    "gen_ai.usage.cache_write_tokens",
                    AttrValue::Int(int(c.cache_write_tokens)),
                ));
                if let Some(micro) = c.cost_micro {
                    s.attributes
                        .push(("beton.cost.usd", AttrValue::Float(micro as f64 / 1e6)));
                }
                if let Some(turn) = e.turn_id.and_then(|t| open.turns.get_mut(&t)) {
                    turn.requests += 1;
                    turn.input_tokens += c.input_tokens;
                    turn.output_tokens += c.output_tokens;
                    if let Some(micro) = c.cost_micro {
                        turn.cost_micro = Some(turn.cost_micro.unwrap_or(0) + micro);
                    }
                }
                out.push(s);
            }
            _ => {}
        }
        out
    }
}

fn str_attr(s: String) -> AttrValue {
    AttrValue::Str(s)
}

fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Serde-Name eines Enums (`ok`, `ask`, …).
fn enum_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn truncate(text: &str) -> String {
    if text.len() <= CONTENT_LIMIT {
        return text.to_owned();
    }
    let mut end = CONTENT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn nanos(e: &Event) -> u64 {
    u64::try_from(e.ts.as_offset_date_time().unix_timestamp_nanos()).unwrap_or(0)
}

/// Stabile Trace-ID einer Session (OBS-003 AC2): die 128 Bit ihrer ULID.
pub fn trace_id(session: SessionId) -> [u8; 16] {
    let bytes = session.ulid().to_bytes();
    if bytes != [0; 16] {
        return bytes;
    }
    let digest = Sha256::digest(session.to_string().as_bytes());
    let mut out = [0; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

/// Span-ID aus Session, Art und Bezug: dieselbe Eingabe ergibt dieselbe ID.
fn span_id(session: SessionId, kind: &str, key: &str) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(session.to_string().as_bytes());
    h.update([0]);
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(key.as_bytes());
    let digest = h.finalize();
    let mut out = [0; 8];
    out.copy_from_slice(&digest[..8]);
    if out == [0; 8] {
        out[7] = 1;
    }
    out
}

/// Rumpf eines OTLP/HTTP-JSON-Exports (`ExportTraceServiceRequest`).
pub fn otlp_json(spans: &[SpanData]) -> Value {
    let version = env!("CARGO_PKG_VERSION");
    let spans: Vec<Value> = spans
        .iter()
        .map(|s| {
            let mut span = json!({
                "traceId": hex::encode(s.trace_id),
                "spanId": hex::encode(s.span_id),
                "name": s.name,
                "kind": 1,
                "startTimeUnixNano": s.start_ns.to_string(),
                "endTimeUnixNano": s.end_ns.max(s.start_ns).to_string(),
                "attributes": s.attributes.iter().map(|(k, v)| json!({
                    "key": k,
                    "value": match v {
                        AttrValue::Str(x) => json!({ "stringValue": x }),
                        AttrValue::Int(x) => json!({ "intValue": x.to_string() }),
                        AttrValue::Float(x) => json!({ "doubleValue": x }),
                        AttrValue::Bool(x) => json!({ "boolValue": x }),
                    },
                })).collect::<Vec<_>>(),
                "status": { "code": if s.error { 2 } else { 1 } },
            });
            if let Some(parent) = s.parent {
                span["parentSpanId"] = hex::encode(parent).into();
            }
            span
        })
        .collect();
    json!({
        "resourceSpans": [{
            "resource": { "attributes": [
                { "key": "service.name", "value": { "stringValue": "beton" } },
                { "key": "service.version", "value": { "stringValue": version } },
            ] },
            "scopeSpans": [{
                "scope": { "name": "beton-server", "version": version },
                "spans": spans,
            }],
        }],
    })
}

/// Schwärzt alle String-Attribute; `Err`, wenn die Pipeline nicht bereit ist.
fn redact_spans(
    mut spans: Vec<SpanData>,
    redactor: &dyn ValueRedactor,
) -> Result<Vec<SpanData>, beton_store::RedactionUnavailable> {
    for span in &mut spans {
        for (_, value) in &mut span.attributes {
            if let AttrValue::Str(s) = value {
                let mut v = Value::String(std::mem::take(s));
                redactor.redact(&mut v)?;
                if let Value::String(red) = v {
                    *s = red;
                }
            }
        }
    }
    Ok(spans)
}

async fn export_loop(
    mut rx: mpsc::Receiver<SpanData>,
    config: OtelConfig,
    client: reqwest::Client,
    redactor: Arc<dyn ValueRedactor>,
) {
    let mut batch = Vec::new();
    let mut tick = tokio::time::interval(config.flush_interval);
    loop {
        tokio::select! {
            span = rx.recv() => match span {
                Some(span) => {
                    batch.push(span);
                    if batch.len() >= BATCH {
                        export(std::mem::take(&mut batch), &config, &client, redactor.as_ref()).await;
                    }
                }
                None => {
                    if !batch.is_empty() {
                        export(batch, &config, &client, redactor.as_ref()).await;
                    }
                    return;
                }
            },
            _ = tick.tick() => {
                if !batch.is_empty() {
                    export(std::mem::take(&mut batch), &config, &client, redactor.as_ref()).await;
                }
            }
        }
    }
}

async fn export(
    spans: Vec<SpanData>,
    config: &OtelConfig,
    client: &reqwest::Client,
    redactor: &dyn ValueRedactor,
) {
    let count = spans.len();
    let Ok(spans) = redact_spans(spans, redactor) else {
        tracing::warn!(
            count,
            "Spans verworfen: Redaction nicht bereit (fail closed)"
        );
        return;
    };
    let mut req = client.post(&config.endpoint).json(&otlp_json(&spans));
    for (k, v) in &config.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    match req.send().await {
        Ok(res) if res.status().is_success() => {}
        Ok(res) => tracing::warn!(status = %res.status(), count, "OTLP-Export abgelehnt"),
        Err(e) => tracing::warn!(count, "OTLP-Export gescheitert: {}", e.without_url()),
    }
}

#[cfg(test)]
mod tests;
