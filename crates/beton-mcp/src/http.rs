//! Weiterleitung an HTTP-MCP-Server (Streamable HTTP, AGT-006).
//!
//! Der Harness spricht stdio mit dem Relay; der Hub sendet jede Nachricht per `POST` an die
//! konfigurierte `url` (mit `headers`, `Mcp-Session-Id` und `MCP-Protocol-Version`) und
//! leitet Antworten aus JSON oder Server-Sent Events zurück. Verbindungen entstehen nur zu
//! konfigurierten Servern (ADR-0033). Header-Werte erscheinen nie in Logs oder Events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use beton_core::event::EventPayload;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc};

use crate::config::{ConfiguredServer, Endpoint};
use crate::hub::{Filter, MAX_LINE, failed, read_line, writer};
use crate::protocol::{self, Kind, codes};

struct State {
    client: reqwest::Client,
    server: ConfiguredServer,
    url: String,
    headers: Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
    session: Mutex<Option<String>>,
    protocol_version: Mutex<Option<String>>,
    filter: Filter,
    initialized: AtomicBool,
    events: mpsc::UnboundedSender<EventPayload>,
    out: mpsc::UnboundedSender<Value>,
}

pub(crate) async fn proxy_http<R, W>(
    events: mpsc::UnboundedSender<EventPayload>,
    server: ConfiguredServer,
    mut r: R,
    w: W,
) where
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let out = writer(w);
    let Ok(Endpoint::Http { url, headers }) = server.endpoint() else {
        return;
    };
    let mut parsed = Vec::new();
    for (k, v) in headers {
        match (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            (Ok(name), Ok(mut value)) => {
                value.set_sensitive(true);
                parsed.push((name, value));
            }
            _ => {
                let msg = format!("ungültiger Header „{k}“");
                failed(&events, &server.name, msg.clone());
                crate::hub::refuse_rest(r, out, msg).await;
                return;
            }
        }
    }
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("HTTP-Client: {}", e.without_url());
            failed(&events, &server.name, msg.clone());
            crate::hub::refuse_rest(r, out, msg).await;
            return;
        }
    };
    let state = Arc::new(State {
        client,
        url: url.to_owned(),
        headers: parsed,
        server: server.clone(),
        session: Mutex::new(None),
        protocol_version: Mutex::new(None),
        filter: Filter::default(),
        initialized: AtomicBool::new(false),
        events,
        out,
    });
    while let Ok(Some(line)) = read_line(&mut r, MAX_LINE).await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Err(reply) = state.filter.outbound(&state.server, &msg) {
            let _ = state.out.send(reply);
            continue;
        }
        if msg["method"] == "initialize" {
            // Synchron, damit Folgeanfragen die Session-ID tragen.
            post(state.clone(), msg).await;
        } else {
            tokio::spawn(post(state.clone(), msg));
        }
    }
    // Sitzung beim Server beenden (best effort).
    let session = state.session.lock().await.clone();
    if let Some(sid) = session {
        let mut req = state
            .client
            .delete(&state.url)
            .header("Mcp-Session-Id", sid);
        for (k, v) in &state.headers {
            req = req.header(k, v);
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), req.send()).await;
    }
}

async fn post(state: Arc<State>, msg: Value) {
    let is_request = protocol::kind(&msg) == Kind::Request;
    let is_init = msg["method"] == "initialize";
    let id = msg["id"].clone();
    let fail = |detail: String| {
        if is_init {
            failed(&state.events, &state.server.name, detail.clone());
        }
        if is_request {
            let _ = state.out.send(protocol::error(
                &id,
                codes::SERVER_UNAVAILABLE,
                format!("MCP-Server {}: {detail}", state.server.name),
                None,
            ));
        }
    };
    let mut req = state
        .client
        .post(&state.url)
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    for (k, v) in &state.headers {
        req = req.header(k, v);
    }
    if let Some(sid) = state.session.lock().await.clone() {
        req = req.header("Mcp-Session-Id", sid);
    }
    if let Some(v) = state.protocol_version.lock().await.clone() {
        req = req.header("MCP-Protocol-Version", v);
    }
    let resp = match req
        .body(serde_json::to_vec(&msg).unwrap_or_default())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return fail(format!("HTTP: {}", e.without_url())),
    };
    let status = resp.status();
    if !status.is_success() {
        return fail(format!("HTTP-Status {status}"));
    }
    if is_init
        && let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
    {
        *state.session.lock().await = Some(sid.to_owned());
    }
    let event_stream = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/event-stream"));
    if status == reqwest::StatusCode::ACCEPTED || !is_request {
        return;
    }
    let mut answered = false;
    if event_stream {
        let mut resp = resp;
        let mut buf = String::new();
        loop {
            let chunk = match resp.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => {
                    if !answered {
                        fail(format!("HTTP: {}", e.without_url()));
                    }
                    return;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            buf = buf.replace("\r\n", "\n");
            while let Some(end) = buf.find("\n\n") {
                let event: String = buf.drain(..end + 2).collect();
                if let Some(data) = sse_data(&event)
                    && let Ok(v) = serde_json::from_str::<Value>(&data)
                {
                    answered |= forward(&state, v, &id).await;
                }
            }
            if answered {
                break;
            }
        }
    } else {
        match resp.json::<Value>().await {
            Ok(Value::Array(batch)) => {
                for v in batch {
                    answered |= forward(&state, v, &id).await;
                }
            }
            Ok(v) => answered = forward(&state, v, &id).await,
            Err(e) => return fail(format!("ungültige Antwort: {}", e.without_url())),
        }
    }
    if !answered {
        fail("keine Antwort".into());
    }
}

/// Leitet eine Server-Nachricht an den Harness; `true`, wenn sie die Anfrage `id` beantwortet.
async fn forward(state: &State, mut v: Value, id: &Value) -> bool {
    let answers = protocol::kind(&v) == Kind::Response && v["id"] == *id;
    if state.filter.inbound(&state.server, &mut v).as_deref() == Some("initialize") {
        state.initialized.store(true, Ordering::SeqCst);
        if let Some(pv) = v["result"]["protocolVersion"].as_str() {
            *state.protocol_version.lock().await = Some(pv.to_owned());
        }
    }
    let _ = state.out.send(v);
    answers
}

/// `data:`-Zeilen eines SSE-Events, zusammengefügt.
fn sse_data(event: &str) -> Option<String> {
    let lines: Vec<&str> = event
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_join_data_lines() {
        assert_eq!(
            sse_data("event: message\ndata: {\"a\":\ndata: 1}\n\n").as_deref(),
            Some("{\"a\":\n1}")
        );
        assert_eq!(sse_data(": ping\n\n"), None);
    }
}
