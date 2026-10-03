//! `tracing`-Layer, der jedes Event als eine JSON-Zeile schreibt (OBS-001).

use std::io::Write as _;

use serde_json::{Map, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::Component;

/// Felder aus dem Span-Kontext, die als Top-Level-Felder in jede Zeile übernommen werden.
const CONTEXT_KEYS: [&str; 4] = ["session_id", "runner_id", "request_id", "seq"];

/// Schreibt Events als JSON-Lines mit den Pflichtfeldern aus OBS-001.
pub struct JsonLayer<W> {
    component: Component,
    writer: W,
}

impl<W> JsonLayer<W> {
    pub fn new(component: Component, writer: W) -> Self {
        Self { component, writer }
    }
}

/// Am Span gespeicherte Felder.
struct SpanFields(Map<String, Value>);

impl<S, W> Layer<S> for JsonLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = Map::new();
        attrs.record(&mut JsonVisitor(&mut fields));
        span.extensions_mut().insert(SpanFields(fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        if let Some(SpanFields(fields)) = extensions.get_mut::<SpanFields>() {
            values.record(&mut JsonVisitor(fields));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut line = Map::new();
        let ts = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        line.insert("ts".into(), Value::String(ts));
        line.insert("level".into(), Value::String(meta.level().to_string()));
        line.insert("target".into(), Value::String(meta.target().to_owned()));
        line.insert("component".into(), self.component.as_str().into());
        line.insert("version".into(), crate::VERSION.into());

        // Span-Kontext: vom äußersten zum innersten Span, innere Werte gewinnen.
        if let Some(scope) = ctx.event_scope(event) {
            let mut span_name = None;
            for span in scope.from_root() {
                span_name = Some(span.name());
                if let Some(SpanFields(fields)) = span.extensions().get::<SpanFields>() {
                    for key in CONTEXT_KEYS {
                        if let Some(value) = fields.get(key) {
                            line.insert(key.into(), value.clone());
                        }
                    }
                }
            }
            if let Some(name) = span_name {
                line.insert("span".into(), name.into());
            }
        }

        let mut fields = Map::new();
        event.record(&mut JsonVisitor(&mut fields));
        if let Some(message) = fields.remove("message") {
            line.insert("message".into(), message);
        }
        if !fields.is_empty() {
            line.insert("fields".into(), Value::Object(fields));
        }

        let Ok(mut bytes) = serde_json::to_vec(&Value::Object(line)) else {
            return;
        };
        bytes.push(b'\n');
        // Eine Zeile = ein write_all, damit Zeilen nicht verschränkt werden.
        let _ = self.writer.make_writer_for(meta).write_all(&bytes);
    }
}

struct JsonVisitor<'a>(&'a mut Map<String, Value>);

impl Visit for JsonVisitor<'_> {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.0.insert(field.name().into(), value.to_string().into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
}
