//! Protokoll `acp` (Agent Client Protocol, QA-002): deterministischer ACP-Test-Agent für
//! HAR-007, JSON-RPC 2.0 über stdio, Protokollversion 1.
//!
//! Ablauf: `initialize` → `session/new` bzw. `session/load` → `session/prompt` (Antwort mit
//! `stopReason` am Turn-Ende) mit `session/update`-Notifications dazwischen;
//! `session/cancel` bricht ab. Freigaben laufen über `session/request_permission`.
//!
//! Abbildung der Szenario-Schritte: `message_delta`/`message` → `agent_message_chunk`;
//! `reasoning` → `agent_thought_chunk`; `tool_call` → `tool_call` (Status `pending`), mit
//! `gate` zuvor `session/request_permission`; `tool_result`/`tool_error` →
//! `tool_call_update` (`completed`/`failed`); `error` → Fehlerantwort auf `session/prompt`;
//! `auth_expired` → Fehler `-32000` (`auth_required`); `usage` meldet ACP nicht. Mit
//! `capabilities: { resume: none|cold }` meldet der Agent `loadSession: false`.

use std::io::{BufRead, Write};
use std::time::Duration;

use beton_harness::scenario::{Scenario, Step, Turn};
use serde_json::{Value, json};

use crate::io::{Lines, Stop, chunks, fnv};

/// Fehlercode von ACP für fehlende Anmeldung.
const AUTH_REQUIRED: i64 = -32000;

pub struct Agent<R, W> {
    io: Lines<R, W>,
    turns: Vec<Turn>,
    next_turn: usize,
    version: String,
    bad_handshake: bool,
    load_session: bool,
    seed: u64,
    session_id: Option<String>,
    calls: u32,
    requests: i64,
    /// Injizierte MCP-Server (HAR-009).
    mcp: crate::mcp::Clients,
}

enum TurnEnd {
    Done,
    Cancelled,
    Error { code: i64, message: String },
}

#[derive(Default)]
struct TurnState {
    call: Option<String>,
    allowed: bool,
}

/// ACP-`kind` zur kanonischen Klasse (Umkehrung der Abbildung im Adapter).
fn acp_kind(kind: &str) -> &'static str {
    match kind {
        "shell" => "execute",
        "file_read" => "read",
        "file_edit" | "file_write" => "edit",
        "search" => "search",
        "web_fetch" => "fetch",
        _ => "other",
    }
}

impl<R: BufRead, W: Write> Agent<R, W> {
    pub fn new(scenario: &Scenario, version: String, input: R, out: W) -> Self {
        let resume = scenario
            .capabilities
            .get("resume")
            .and_then(Value::as_str)
            .unwrap_or("warm");
        Self {
            io: Lines::new(input, out, scenario.faults),
            turns: scenario.turns.clone(),
            next_turn: 0,
            version,
            bad_handshake: scenario.faults.bad_handshake,
            load_session: !matches!(resume, "none" | "cold"),
            seed: fnv(&format!("{scenario:?}")),
            session_id: None,
            calls: 0,
            requests: 0,
            mcp: crate::mcp::Clients::default(),
        }
    }

    fn respond(&mut self, id: &Value, result: Value) -> Result<(), Stop> {
        self.io
            .emit(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn respond_error(&mut self, id: &Value, code: i64, message: &str) -> Result<(), Stop> {
        self.io
            .emit(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    }

    fn update(&mut self, update: Value) -> Result<(), Stop> {
        let session_id = self.session_id.clone();
        self.io.emit(&json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": {"sessionId": session_id, "update": update},
        }))
    }

    pub fn run(&mut self) -> Result<(), Stop> {
        loop {
            let msg = self.io.read()?;
            let (Some(method), Some(id)) = (
                msg["method"].as_str().map(str::to_owned),
                msg.get("id").cloned(),
            ) else {
                continue; // Notifications (`session/cancel` außerhalb eines Turns) und Antworten.
            };
            self.request(&id, &method, &msg["params"])?;
        }
    }

    fn request(&mut self, id: &Value, method: &str, params: &Value) -> Result<(), Stop> {
        match method {
            "initialize" => {
                let version = if self.bad_handshake { 99 } else { 1 };
                let info = json!({"name": "beton-fake-acp", "version": self.version});
                self.respond(
                    id,
                    json!({
                        "protocolVersion": version,
                        "agentCapabilities": {
                            "loadSession": self.load_session,
                            "promptCapabilities": {"image": false, "audio": false, "embeddedContext": false},
                            "mcpCapabilities": {"http": false, "sse": false},
                        },
                        "authMethods": [],
                        "agentInfo": info,
                    }),
                )
            }
            "session/new" => {
                let session = format!("sess_fake{:012x}", self.seed & 0xffff_ffff_ffff);
                self.session_id = Some(session.clone());
                self.mcp =
                    crate::mcp::Clients::connect(&crate::mcp::from_acp(&params["mcpServers"]));
                self.respond(id, json!({"sessionId": session}))
            }
            "session/load" if self.load_session => {
                self.session_id = params["sessionId"].as_str().map(str::to_owned);
                self.mcp =
                    crate::mcp::Clients::connect(&crate::mcp::from_acp(&params["mcpServers"]));
                self.respond(id, Value::Null)
            }
            "session/prompt" => self.prompt(id, params),
            "session/set_mode" | "authenticate" => self.respond(id, json!({})),
            _ => self.respond_error(id, -32601, &format!("Method not found: {method}")),
        }
    }

    fn prompt(&mut self, id: &Value, params: &Value) -> Result<(), Stop> {
        if self.session_id.is_none() || params["sessionId"].as_str() != self.session_id.as_deref() {
            return self.respond_error(id, -32602, "unknown session");
        }
        let index = self.next_turn;
        self.next_turn += 1;
        let end = match self.turns.get(index).cloned() {
            None => TurnEnd::Error {
                code: -32603,
                message: "Szenario zu Ende".into(),
            },
            Some(turn) => {
                let text = prompt_text(params);
                match &turn.expect_input {
                    Some(expected) if *expected != text => TurnEnd::Error {
                        code: -32602,
                        message: format!("erwartet `{expected}`, erhalten `{text}`"),
                    },
                    _ => self.steps(&turn.emit, &mut TurnState::default())?,
                }
            }
        };
        match end {
            TurnEnd::Done => self.respond(id, json!({"stopReason": "end_turn"})),
            TurnEnd::Cancelled => self.respond(id, json!({"stopReason": "cancelled"})),
            TurnEnd::Error { code, message } => self.respond_error(id, code, &message),
        }
    }

    fn steps(&mut self, steps: &[Step], state: &mut TurnState) -> Result<TurnEnd, Stop> {
        for step in steps {
            if let Some(ms) = step.delay_ms {
                std::thread::sleep(Duration::from_millis(ms));
            }
            match self.step(step, state)? {
                TurnEnd::Done => {}
                other => return Ok(other),
            }
        }
        Ok(TurnEnd::Done)
    }

    fn step(&mut self, step: &Step, state: &mut TurnState) -> Result<TurnEnd, Stop> {
        if let Some(text) = step.message_delta.as_ref().or(step.message.as_ref()) {
            let chunk = if step.message_delta.is_some() {
                step.chunk
            } else {
                None
            };
            for (i, piece) in chunks(text, chunk).into_iter().enumerate() {
                if i > 0
                    && let Some(ms) = step.chunk_delay_ms
                {
                    std::thread::sleep(Duration::from_millis(ms));
                }
                self.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": piece}}))?;
            }
        } else if let Some(text) = &step.reasoning {
            self.update(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": text}}))?;
        } else if let Some(call) = &step.tool_call {
            self.calls += 1;
            let id = format!("call_fake{:08}", self.calls);
            let tool = json!({
                "toolCallId": id, "title": call.name, "kind": acp_kind(&call.kind),
                "status": "pending", "rawInput": call.args, "locations": [],
            });
            let mut announce = tool.clone();
            announce["sessionUpdate"] = json!("tool_call");
            self.update(announce)?;
            state.allowed = true;
            if step.gate {
                match self.permission(&tool)? {
                    Some(allowed) => state.allowed = allowed,
                    None => return Ok(TurnEnd::Cancelled),
                }
            }
            if state.allowed {
                self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": "in_progress"}))?;
                state.call = Some(id);
            } else {
                self.update(json!({
                    "sessionUpdate": "tool_call_update", "toolCallId": id, "status": "failed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "Permission denied"}}],
                }))?;
                state.call = None;
            }
        } else if let Some(call) = &step.mcp_call {
            self.calls += 1;
            let id = format!("call_fake{:08}", self.calls);
            self.update(json!({
                "sessionUpdate": "tool_call", "toolCallId": id,
                "title": format!("{}/{}", call.server, call.tool), "kind": "other",
                "status": "pending", "rawInput": call.args, "locations": [],
            }))?;
            self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": "in_progress"}))?;
            state.call = Some(id);
            match self.mcp.call(&call.server, &call.tool, &call.args) {
                Ok(result) => {
                    let status = if result["isError"] == true {
                        "failed"
                    } else {
                        "completed"
                    };
                    self.complete(state, &result, status)?;
                }
                Err(e) => self.complete(state, &json!(e), "failed")?,
            }
        } else if let Some(on_gate) = &step.on_gate {
            let branch = if state.allowed {
                &on_gate.allow
            } else {
                &on_gate.deny
            };
            return self.steps(branch, state);
        } else if let Some(result) = &step.tool_result {
            self.complete(state, result, "completed")?;
        } else if let Some(error) = &step.tool_error {
            self.complete(state, &json!(error), "failed")?;
        } else if step.usage.is_some() {
            // ACP meldet keinen Token-Verbrauch.
        } else if let Some(message) = &step.error {
            return Ok(TurnEnd::Error {
                code: -32603,
                message: message.clone(),
            });
        } else if let Some(code) = step.crash {
            return Err(Stop::Crash(u8::try_from(code.clamp(1, 255)).unwrap_or(1)));
        } else if let Some(hint) = &step.auth_expired {
            return Ok(TurnEnd::Error {
                code: AUTH_REQUIRED,
                message: format!("Authentication required: {hint}"),
            });
        } else if let Some(write) = &step.write_file {
            // Wie ein Edit-Tool: Datei relativ zum Arbeitsverzeichnis (SES-017).
            let workdir = std::env::current_dir().unwrap_or_default();
            let _ = write.apply(&workdir);
        } else if step.hang {
            loop {
                let msg = self.io.read()?;
                if msg["method"] == "session/cancel" {
                    return Ok(TurnEnd::Cancelled);
                }
            }
        }
        Ok(TurnEnd::Done)
    }

    /// `session/request_permission`; `None`, wenn der Client abbricht.
    fn permission(&mut self, tool: &Value) -> Result<Option<bool>, Stop> {
        let request_id = self.requests;
        self.requests += 1;
        let session_id = self.session_id.clone();
        self.io.emit(&json!({
            "jsonrpc": "2.0", "id": request_id, "method": "session/request_permission",
            "params": {
                "sessionId": session_id,
                "toolCall": tool,
                "options": [
                    {"optionId": "allow-once", "name": "Erlauben", "kind": "allow_once"},
                    {"optionId": "allow-always", "name": "Immer erlauben", "kind": "allow_always"},
                    {"optionId": "reject-once", "name": "Ablehnen", "kind": "reject_once"},
                ],
            },
        }))?;
        let mut cancelled = false;
        loop {
            let msg = self.io.read()?;
            if msg["method"] == "session/cancel" {
                // Der Client beantwortet die offene Anfrage danach mit `cancelled`.
                cancelled = true;
                continue;
            }
            if msg.get("method").is_none() && msg["id"] == json!(request_id) {
                let outcome = &msg["result"]["outcome"];
                if cancelled || outcome["outcome"] == "cancelled" {
                    return Ok(None);
                }
                let option = outcome["optionId"].as_str().unwrap_or_default();
                return Ok(Some(
                    outcome["outcome"] == "selected" && option.starts_with("allow"),
                ));
            }
        }
    }

    fn complete(
        &mut self,
        state: &mut TurnState,
        result: &Value,
        status: &str,
    ) -> Result<(), Stop> {
        let Some(id) = state.call.take() else {
            return Ok(());
        };
        let text = match result {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        self.update(json!({
            "sessionUpdate": "tool_call_update", "toolCallId": id, "status": status,
            "content": [{"type": "content", "content": {"type": "text", "text": text}}],
            "rawOutput": result,
        }))
    }
}

fn prompt_text(params: &Value) -> String {
    params["prompt"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}
