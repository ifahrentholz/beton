//! Lokaler Mock-Server für Tests (Feature `mock`): HTTP/1.1 mit SSE auf `127.0.0.1`,
//! spricht das Anthropic- und das OpenAI-Wire-Format inklusive Tool-Calls und Streaming.
//! Kein Test braucht einen echten Anbieter, einen echten Key oder das Internet.
//!
//! Antworten werden je Pfad in Reihenfolge abgespielt ([`MockServer::push`]); jeder Request
//! wird mit Headern und Body aufgezeichnet ([`MockServer::requests`]).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_harness::scenario::{Scenario, Step};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::WireKind;
use crate::session::DENIED_PREFIX;

/// Ein SSE-Ereignis der Antwort.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub event: Option<String>,
    pub data: String,
    pub delay_ms: u64,
}

/// Eine vorbereitete Antwort.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Sse(Vec<Chunk>),
    Status {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
    Json(Value),
    /// Sendet `prefix` und hält die Verbindung offen, bis der Client sie schließt.
    Hang(Vec<Chunk>),
    /// Schließt die Verbindung ohne Antwort.
    Drop,
    /// Wählt nach dem letzten Tool-Ergebnis im Request: abgelehnt → `deny`, sonst `allow`.
    Branch {
        allow: Box<Reply>,
        deny: Box<Reply>,
    },
}

impl Reply {
    /// Fehlerantwort im Format der Anbieter.
    pub fn error(status: u16, message: &str) -> Self {
        Self::Status {
            status,
            headers: Vec::new(),
            body: json!({"type": "error", "error": {"type": "mock_error", "message": message}})
                .to_string(),
        }
    }
}

/// Ein aufgezeichneter Request.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct State {
    routes: Mutex<HashMap<String, VecDeque<Reply>>>,
    requests: Mutex<Vec<Recorded>>,
}

/// Der Server.
pub struct MockServer {
    pub addr: SocketAddr,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::fmt::Debug for MockServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl MockServer {
    /// Startet den Server auf einem freien Loopback-Port.
    pub async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State::default());
        let st = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    let _ = serve(sock, st).await;
                });
            }
        });
        Ok(Self { addr, state, task })
    }

    /// `http://127.0.0.1:<port>` plus `suffix` (z. B. `/v1` für OpenAI-kompatible Basis-URLs).
    pub fn base_url(&self, suffix: &str) -> String {
        format!("http://{}{suffix}", self.addr)
    }

    /// Hängt eine Antwort für `path` an (z. B. `/v1/messages`).
    pub fn push(&self, path: &str, reply: Reply) {
        if let Ok(mut r) = self.state.routes.lock() {
            r.entry(path.to_owned()).or_default().push_back(reply);
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state
            .requests
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Requests an `path`.
    pub fn requests_to(&self, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }
}

async fn serve(mut sock: TcpStream, st: Arc<State>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let recorded = Recorded {
        method,
        path: path.clone(),
        headers,
        body: body.clone(),
    };
    if let Ok(mut r) = st.requests.lock() {
        r.push(recorded);
    }
    let reply = st
        .routes
        .lock()
        .ok()
        .and_then(|mut r| r.get_mut(&path).and_then(VecDeque::pop_front));
    let mut reply = reply.unwrap_or_else(|| Reply::error(500, "mock: keine Antwort hinterlegt"));
    while let Reply::Branch { allow, deny } = reply {
        reply = if last_tool_result_denied(&body) {
            *deny
        } else {
            *allow
        };
    }
    respond(&mut sock, reply).await
}

/// Wurde das letzte Tool-Ergebnis im Request von beton abgelehnt?
fn last_tool_result_denied(body: &Value) -> bool {
    let msgs = body["messages"].as_array().cloned().unwrap_or_default();
    for m in msgs.iter().rev() {
        // OpenAI: `{"role": "tool", "content": "…"}`
        if m["role"] == "tool" {
            return m["content"]
                .as_str()
                .is_some_and(|c| c.starts_with(DENIED_PREFIX));
        }
        // Anthropic: `tool_result`-Blöcke in einer User-Nachricht.
        if let Some(blocks) = m["content"].as_array()
            && let Some(r) = blocks.iter().rev().find(|b| b["type"] == "tool_result")
        {
            return r["content"]
                .as_str()
                .is_some_and(|c| c.starts_with(DENIED_PREFIX));
        }
    }
    false
}

async fn write_chunks(sock: &mut TcpStream, chunks: &[Chunk]) -> std::io::Result<()> {
    for c in chunks {
        if c.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(c.delay_ms)).await;
        }
        let mut text = String::new();
        if let Some(e) = &c.event {
            text.push_str(&format!("event: {e}\n"));
        }
        text.push_str(&format!("data: {}\n\n", c.data));
        sock.write_all(text.as_bytes()).await?;
        sock.flush().await?;
    }
    Ok(())
}

const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";

async fn respond(sock: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    match reply {
        Reply::Sse(chunks) => {
            sock.write_all(SSE_HEAD.as_bytes()).await?;
            write_chunks(sock, &chunks).await?;
            sock.shutdown().await
        }
        Reply::Hang(chunks) => {
            sock.write_all(SSE_HEAD.as_bytes()).await?;
            write_chunks(sock, &chunks).await?;
            // Offen halten, bis der Client geht (Interrupt) – höchstens eine Minute.
            let mut tmp = [0u8; 64];
            let _ = tokio::time::timeout(Duration::from_secs(60), async {
                while let Ok(n) = sock.read(&mut tmp).await {
                    if n == 0 {
                        break;
                    }
                }
            })
            .await;
            Ok(())
        }
        Reply::Drop => Ok(()),
        Reply::Json(v) => {
            let body = v.to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).await?;
            sock.write_all(body.as_bytes()).await?;
            sock.shutdown().await
        }
        Reply::Status {
            status,
            headers,
            body,
        } => {
            let mut head = format!(
                "HTTP/1.1 {status} Mock\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            sock.write_all(head.as_bytes()).await?;
            sock.write_all(body.as_bytes()).await?;
            sock.shutdown().await
        }
        Reply::Branch { .. } => Ok(()),
    }
}

// ---------------------------------------------------------------------- Antworten bauen

/// Bausteine einer Modell-Antwort.
#[derive(Debug, Clone, PartialEq)]
pub enum Say {
    /// Text, in Stücken von `chunk` Zeichen gestreamt (mindestens 1).
    Text(String, usize),
    Thinking(String),
    Tool {
        id: String,
        name: String,
        input: Value,
    },
    Usage {
        input: u64,
        output: u64,
    },
    /// Expliziter Stop-Grund (sonst `tool_use` bei Tool-Calls, sonst `end_turn`).
    Stop(String),
    /// Pause vor dem nächsten Baustein.
    Delay(u64),
}

fn chunks_of(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

fn ev(event: Option<&str>, data: Value, delay_ms: u64) -> Chunk {
    Chunk {
        event: event.map(str::to_owned),
        data: data.to_string(),
        delay_ms,
    }
}

/// Antwort im Anthropic-Format.
pub fn anthropic(id: &str, items: &[Say]) -> Reply {
    Reply::Sse(anthropic_chunks(id, items))
}

fn usage_of(items: &[Say]) -> (u64, u64) {
    items
        .iter()
        .rev()
        .find_map(|s| match s {
            Say::Usage { input, output } => Some((*input, *output)),
            _ => None,
        })
        .unwrap_or((0, 0))
}

fn stop_of(items: &[Say], tool: &str, end: &str) -> String {
    items
        .iter()
        .find_map(|s| match s {
            Say::Stop(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            if items.iter().any(|s| matches!(s, Say::Tool { .. })) {
                tool.to_owned()
            } else {
                end.to_owned()
            }
        })
}

fn anthropic_chunks(id: &str, items: &[Say]) -> Vec<Chunk> {
    let (input, output) = usage_of(items);
    let mut out = vec![ev(
        Some("message_start"),
        json!({"type": "message_start", "message": {"id": id, "type": "message", "role": "assistant", "content": [], "usage": {"input_tokens": input, "output_tokens": 1}}}),
        0,
    )];
    let mut delay = 0;
    let mut index = 0;
    for s in items {
        match s {
            Say::Delay(ms) => delay = *ms,
            Say::Text(text, size) => {
                out.push(ev(Some("content_block_start"), json!({"type": "content_block_start", "index": index, "content_block": {"type": "text", "text": ""}}), std::mem::take(&mut delay)));
                for part in chunks_of(text, *size) {
                    out.push(ev(Some("content_block_delta"), json!({"type": "content_block_delta", "index": index, "delta": {"type": "text_delta", "text": part}}), 0));
                }
                out.push(ev(
                    Some("content_block_stop"),
                    json!({"type": "content_block_stop", "index": index}),
                    0,
                ));
                index += 1;
            }
            Say::Thinking(text) => {
                out.push(ev(Some("content_block_start"), json!({"type": "content_block_start", "index": index, "content_block": {"type": "thinking", "thinking": ""}}), std::mem::take(&mut delay)));
                out.push(ev(Some("content_block_delta"), json!({"type": "content_block_delta", "index": index, "delta": {"type": "thinking_delta", "thinking": text}}), 0));
                out.push(ev(Some("content_block_delta"), json!({"type": "content_block_delta", "index": index, "delta": {"type": "signature_delta", "signature": "sig-mock"}}), 0));
                out.push(ev(
                    Some("content_block_stop"),
                    json!({"type": "content_block_stop", "index": index}),
                    0,
                ));
                index += 1;
            }
            Say::Tool { id, name, input } => {
                let args = input.to_string();
                let mid = args.len() / 2;
                let (a, b) = args.split_at(
                    args.char_indices()
                        .map(|(i, _)| i)
                        .find(|i| *i >= mid)
                        .unwrap_or(0),
                );
                out.push(ev(Some("content_block_start"), json!({"type": "content_block_start", "index": index, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}), std::mem::take(&mut delay)));
                for part in [a, b] {
                    out.push(ev(Some("content_block_delta"), json!({"type": "content_block_delta", "index": index, "delta": {"type": "input_json_delta", "partial_json": part}}), 0));
                }
                out.push(ev(
                    Some("content_block_stop"),
                    json!({"type": "content_block_stop", "index": index}),
                    0,
                ));
                index += 1;
            }
            Say::Usage { .. } | Say::Stop(_) => {}
        }
    }
    let stop = stop_of(items, "tool_use", "end_turn");
    out.push(ev(Some("message_delta"), json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null}, "usage": {"output_tokens": output}}), 0));
    out.push(ev(Some("message_stop"), json!({"type": "message_stop"}), 0));
    out
}

/// Antwort im OpenAI-Format.
pub fn openai(id: &str, items: &[Say]) -> Reply {
    Reply::Sse(openai_chunks(id, items))
}

fn openai_chunks(id: &str, items: &[Say]) -> Vec<Chunk> {
    let chunk = |delta: Value, finish: Value, delay: u64| {
        ev(
            None,
            json!({"id": id, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}),
            delay,
        )
    };
    let mut out = vec![chunk(
        json!({"role": "assistant", "content": ""}),
        Value::Null,
        0,
    )];
    let mut delay = 0;
    let mut tool_index = 0;
    for s in items {
        match s {
            Say::Delay(ms) => delay = *ms,
            Say::Text(text, size) => {
                for part in chunks_of(text, *size) {
                    out.push(chunk(
                        json!({"content": part}),
                        Value::Null,
                        std::mem::take(&mut delay),
                    ));
                }
            }
            Say::Thinking(text) => {
                out.push(chunk(
                    json!({"reasoning_content": text}),
                    Value::Null,
                    std::mem::take(&mut delay),
                ));
            }
            Say::Tool { id, name, input } => {
                let args = input.to_string();
                let mid = args.len() / 2;
                let (a, b) = args.split_at(
                    args.char_indices()
                        .map(|(i, _)| i)
                        .find(|i| *i >= mid)
                        .unwrap_or(0),
                );
                out.push(chunk(json!({"tool_calls": [{"index": tool_index, "id": id, "type": "function", "function": {"name": name, "arguments": a}}]}), Value::Null, std::mem::take(&mut delay)));
                out.push(chunk(
                    json!({"tool_calls": [{"index": tool_index, "function": {"arguments": b}}]}),
                    Value::Null,
                    0,
                ));
                tool_index += 1;
            }
            Say::Usage { .. } | Say::Stop(_) => {}
        }
    }
    let stop = stop_of(items, "tool_calls", "stop");
    out.push(chunk(json!({}), Value::String(stop), 0));
    let (input, output) = usage_of(items);
    out.push(ev(
        None,
        json!({"id": id, "object": "chat.completion.chunk", "choices": [], "usage": {"prompt_tokens": input, "completion_tokens": output, "total_tokens": input + output}}),
        0,
    ));
    out.push(Chunk {
        event: None,
        data: "[DONE]".into(),
        delay_ms: 0,
    });
    out
}

/// Antwort im Format der Wire-Familie.
pub fn reply(wire: WireKind, id: &str, items: &[Say]) -> Reply {
    match wire {
        WireKind::Anthropic => anthropic(id, items),
        WireKind::Openai => openai(id, items),
    }
}

/// Pfad des Model-Requests bei Basis-URL `MockServer::base_url(base_suffix(wire))`.
pub fn messages_path(wire: WireKind) -> &'static str {
    match wire {
        WireKind::Anthropic => "/v1/messages",
        WireKind::Openai => "/v1/chat/completions",
    }
}

/// Suffix der Basis-URL, damit [`messages_path`] passt.
pub fn base_suffix(wire: WireKind) -> &'static str {
    match wire {
        WireKind::Anthropic => "",
        WireKind::Openai => "/v1",
    }
}

// ---------------------------------------------------------------- Szenarien (HAR-026)

struct Ids {
    n: u32,
}

impl Ids {
    fn next(&mut self, prefix: &str) -> String {
        self.n += 1;
        format!("{prefix}_mock{:012}", self.n)
    }
}

/// Übersetzt die Schritte eines Szenarios in eine Antwort (ohne Gate-Verzweigung).
fn steps_reply(wire: WireKind, steps: &[Step], ids: &mut Ids) -> Reply {
    let mut items = Vec::new();
    for s in steps {
        if let Some(ms) = s.delay_ms {
            items.push(Say::Delay(ms));
        }
        if let Some(t) = &s.message_delta {
            items.push(Say::Text(t.clone(), s.chunk.unwrap_or(4)));
        } else if let Some(t) = &s.message {
            items.push(Say::Text(t.clone(), usize::MAX));
        } else if let Some(t) = &s.reasoning {
            items.push(Say::Thinking(t.clone()));
        } else if let Some(u) = &s.usage {
            items.push(Say::Usage {
                input: u.input_tokens,
                output: u.output_tokens,
            });
        } else if let Some(e) = &s.error {
            return Reply::error(400, e);
        } else if let Some(e) = &s.auth_expired {
            return Reply::error(401, e);
        } else if s.hang {
            let id = ids.next("msg");
            let mut chunks = match wire {
                WireKind::Anthropic => anthropic_chunks(&id, &items),
                WireKind::Openai => openai_chunks(&id, &items),
            };
            // Ohne Abschluss: nur Start und Deltas.
            let keep = chunks.len().saturating_sub(match wire {
                WireKind::Anthropic => 2,
                WireKind::Openai => 3,
            });
            chunks.truncate(keep);
            return Reply::Hang(chunks);
        } else if s.crash.is_some() {
            return Reply::Drop;
        }
    }
    reply(wire, &ids.next("msg"), &items)
}

/// Übersetzt ein Szenario (HAR-026) in Antworten des Mock-Servers, je Model-Request eine:
/// Ein Turn mit `tool_call` liefert erst den Call und danach (`on_gate`) eine Verzweigung
/// nach der Gate-Entscheidung. `tool_result`-Schritte liefert die Tool-Ausführung, nicht das
/// Modell.
pub fn from_scenario(wire: WireKind, scenario: &Scenario) -> Vec<Reply> {
    let mut ids = Ids { n: 0 };
    let mut out = Vec::new();
    for turn in &scenario.turns {
        let mut before: Vec<Step> = Vec::new();
        let mut rest = turn.emit.iter();
        let mut pushed = false;
        while let Some(step) = rest.next() {
            if let Some(call) = &step.tool_call {
                let mut items: Vec<Say> = Vec::new();
                for s in &before {
                    if let Some(t) = &s.message_delta {
                        items.push(Say::Text(t.clone(), s.chunk.unwrap_or(4)));
                    } else if let Some(t) = &s.message {
                        items.push(Say::Text(t.clone(), usize::MAX));
                    } else if let Some(u) = &s.usage {
                        items.push(Say::Usage {
                            input: u.input_tokens,
                            output: u.output_tokens,
                        });
                    }
                }
                items.push(Say::Tool {
                    id: ids.next("toolu"),
                    name: call.name.clone(),
                    input: call.args.clone(),
                });
                out.push(reply(wire, &ids.next("msg"), &items));
                let after: Vec<Step> = rest.clone().cloned().collect();
                let branch = after.iter().find_map(|s| s.on_gate.clone());
                let next = match branch {
                    Some(g) => Reply::Branch {
                        allow: Box::new(steps_reply(wire, &g.allow, &mut ids)),
                        deny: Box::new(steps_reply(wire, &g.deny, &mut ids)),
                    },
                    None => steps_reply(
                        wire,
                        &after
                            .into_iter()
                            .filter(|s| s.tool_result.is_none())
                            .collect::<Vec<_>>(),
                        &mut ids,
                    ),
                };
                out.push(next);
                pushed = true;
                break;
            }
            before.push(step.clone());
        }
        if !pushed {
            out.push(steps_reply(wire, &before, &mut ids));
        }
    }
    out
}
