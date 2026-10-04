//! Protokoll `stream-json` (Claude Code, QA-002): Eingaben als JSON-Zeilen auf stdin,
//! Ausgaben als JSON-Zeilen auf stdout; Freigaben über `control_request`/`can_use_tool`,
//! Unterbrechungen über `control_request`/`interrupt`.

use std::io::{BufRead, Write};
use std::time::Duration;

use beton_harness::scenario::{Scenario, Step, Usage};
use serde_json::{Value, json};

use crate::io::{Lines, Stop, fnv};

/// Was ein Turn erlebt hat.
enum TurnEnd {
    Done,
    Interrupted,
    Failed(String),
    /// Login abgelaufen: Result mit `api_error_status: 401`.
    AuthFailed(String),
}

pub struct Sim<R, W> {
    io: Lines<R, W>,
    partial: bool,
    session_id: String,
    model: String,
    ids: u32,
    initialized: bool,
}

impl<R: BufRead, W: Write> Sim<R, W> {
    pub fn new(
        scenario: &Scenario,
        partial: bool,
        resume: Option<String>,
        input: R,
        out: W,
    ) -> Self {
        let seed = fnv(&format!("{scenario:?}"));
        Self {
            io: Lines::new(input, out, scenario.faults),
            partial,
            // Wie die echte CLI: `--resume <id>` setzt die Session fort.
            session_id: resume.unwrap_or_else(|| {
                format!("00000000-0000-4000-8000-{:012x}", seed & 0xffff_ffff_ffff)
            }),
            model: "claude-fake".into(),
            ids: 0,
            initialized: false,
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.ids += 1;
        format!("{prefix}_fake{:08}", self.ids)
    }

    fn emit(&mut self, value: Value) -> Result<(), Stop> {
        self.io.emit(&value)
    }

    fn read(&mut self) -> Result<Value, Stop> {
        self.io.read()
    }

    /// Beantwortet Steuerbefehle des Adapters. Liefert `true` bei `interrupt`.
    fn control(&mut self, msg: &Value) -> Result<bool, Stop> {
        let request_id = msg["request_id"].clone();
        let subtype = msg["request"]["subtype"].as_str().unwrap_or_default();
        if subtype == "set_model"
            && let Some(m) = msg["request"]["model"].as_str()
        {
            self.model = m.to_owned();
        }
        self.emit(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": {}},
        }))?;
        Ok(subtype == "interrupt")
    }

    pub fn run(&mut self, scenario: &Scenario) -> Result<(), Stop> {
        let mut turns = scenario.turns.iter();
        loop {
            let msg = self.read()?;
            match msg["type"].as_str() {
                Some("control_request") => {
                    self.control(&msg)?;
                }
                Some("user") => {
                    let text = user_text(&msg);
                    if !self.initialized {
                        self.initialized = true;
                        self.emit(json!({
                            "type": "system", "subtype": "init",
                            "session_id": self.session_id, "model": self.model,
                            "tools": ["Bash", "Read", "Edit"], "mcp_servers": [],
                            "permissionMode": "default", "apiKeySource": "none",
                        }))?;
                    }
                    let Some(turn) = turns.next() else {
                        self.result(
                            TurnEnd::Failed("Szenario zu Ende".into()),
                            "",
                            &Usage::default(),
                        )?;
                        continue;
                    };
                    if let Some(expected) = &turn.expect_input
                        && *expected != text
                    {
                        let why = format!("erwartet `{expected}`, erhalten `{text}`");
                        self.result(TurnEnd::Failed(why), "", &Usage::default())?;
                        continue;
                    }
                    self.turn(&turn.emit)?;
                }
                _ => {} // Andere Nachrichten ignoriert die echte CLI ebenfalls.
            }
        }
    }

    fn turn(&mut self, steps: &[Step]) -> Result<(), Stop> {
        let mut state = TurnState::default();
        let end = self.steps(steps, &mut state)?;
        let usage = state.usage.clone();
        self.result(end, &state.last_text, &usage)
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

    fn assistant(&mut self, content: Value) -> Result<(), Stop> {
        let id = self.next_id("msg");
        self.assistant_with_id(id, content)
    }

    /// Wie die echte CLI: dieselbe ID wie im vorangehenden `message_start`.
    fn assistant_with_id(&mut self, id: String, content: Value) -> Result<(), Stop> {
        self.emit(json!({
            "type": "assistant",
            "message": {
                "id": id, "type": "message", "role": "assistant", "model": self.model,
                "content": [content], "stop_reason": null,
            },
            "parent_tool_use_id": null,
            "session_id": self.session_id,
        }))
    }

    fn step(&mut self, step: &Step, state: &mut TurnState) -> Result<TurnEnd, Stop> {
        if let Some(text) = &step.message_delta {
            let id = self.next_id("msg");
            if self.partial {
                self.emit(json!({
                    "type": "stream_event",
                    "event": {"type": "message_start",
                              "message": {"id": id, "type": "message", "role": "assistant",
                                          "model": self.model, "content": []}},
                    "parent_tool_use_id": null,
                    "session_id": self.session_id,
                }))?;
                let chunk = step.chunk.unwrap_or(usize::MAX);
                let chars: Vec<char> = text.chars().collect();
                for (i, piece) in chars.chunks(chunk.min(chars.len().max(1))).enumerate() {
                    if i > 0
                        && let Some(ms) = step.chunk_delay_ms
                    {
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                    let piece: String = piece.iter().collect();
                    self.emit(json!({
                        "type": "stream_event",
                        "event": {"type": "content_block_delta", "index": 0,
                                  "delta": {"type": "text_delta", "text": piece}},
                        "parent_tool_use_id": null,
                        "session_id": self.session_id,
                    }))?;
                }
            }
            state.last_text.clone_from(text);
            self.assistant_with_id(id, json!({"type": "text", "text": text}))?;
        } else if let Some(text) = &step.message {
            state.last_text.clone_from(text);
            self.assistant(json!({"type": "text", "text": text}))?;
        } else if let Some(text) = &step.reasoning {
            self.assistant(json!({"type": "thinking", "thinking": text, "signature": ""}))?;
        } else if let Some(call) = &step.tool_call {
            let id = self.next_id("toolu");
            self.assistant(
                json!({"type": "tool_use", "id": id, "name": call.name, "input": call.args}),
            )?;
            state.allowed = true;
            if step.gate {
                let request_id = self.next_id("req");
                self.emit(json!({
                    "type": "control_request",
                    "request_id": request_id,
                    "request": {"subtype": "can_use_tool", "tool_name": call.name,
                                "input": call.args, "tool_use_id": id},
                }))?;
                loop {
                    let msg = self.read()?;
                    match msg["type"].as_str() {
                        Some("control_response")
                            if msg["response"]["request_id"] == json!(request_id) =>
                        {
                            state.allowed =
                                msg["response"]["response"]["behavior"] == json!("allow");
                            break;
                        }
                        Some("control_request") => {
                            let interrupted = self.control(&msg)?;
                            if interrupted {
                                return Ok(TurnEnd::Interrupted);
                            }
                        }
                        _ => {}
                    }
                }
                if !state.allowed {
                    self.tool_result(&id, &json!("Permission denied"), true)?;
                }
            }
            state.call = Some(id);
        } else if let Some(on_gate) = &step.on_gate {
            let branch = if state.allowed {
                &on_gate.allow
            } else {
                &on_gate.deny
            };
            return self.steps(branch, state);
        } else if let Some(result) = &step.tool_result {
            if let Some(id) = state.call.take() {
                self.tool_result(&id, result, false)?;
            }
        } else if let Some(error) = &step.tool_error {
            if let Some(id) = state.call.take() {
                self.tool_result(&id, &json!(error), true)?;
            }
        } else if let Some(usage) = &step.usage {
            state.usage = usage.clone();
        } else if let Some(message) = &step.error {
            return Ok(TurnEnd::Failed(message.clone()));
        } else if let Some(code) = step.crash {
            return Err(Stop::Crash(u8::try_from(code.clamp(1, 255)).unwrap_or(1)));
        } else if let Some(hint) = &step.auth_expired {
            return Ok(TurnEnd::AuthFailed(format!("authentication_error: {hint}")));
        } else if let Some(write) = &step.write_file {
            // Wie ein Edit-Tool: Datei relativ zum Arbeitsverzeichnis (SES-017).
            let workdir = std::env::current_dir().unwrap_or_default();
            let _ = write.apply(&workdir);
        } else if step.hang {
            loop {
                let msg = self.read()?;
                if msg["type"] == json!("control_request") && self.control(&msg)? {
                    return Ok(TurnEnd::Interrupted);
                }
            }
        }
        Ok(TurnEnd::Done)
    }

    fn tool_result(&mut self, id: &str, content: &Value, is_error: bool) -> Result<(), Stop> {
        let content = match content {
            Value::String(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        };
        self.emit(json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error}
            ]},
            "parent_tool_use_id": null,
            "session_id": self.session_id,
        }))
    }

    fn result(&mut self, end: TurnEnd, text: &str, usage: &Usage) -> Result<(), Stop> {
        let mut api_status = Value::Null;
        let (subtype, is_error, result) = match end {
            TurnEnd::Done => ("success", false, text.to_owned()),
            TurnEnd::Interrupted => ("error_during_execution", true, "interrupted".to_owned()),
            TurnEnd::Failed(why) => ("error_during_execution", true, why),
            TurnEnd::AuthFailed(why) => {
                api_status = json!(401);
                ("error_during_execution", true, why)
            }
        };
        self.emit(json!({
            "type": "result", "subtype": subtype, "is_error": is_error,
            "duration_ms": 0, "duration_api_ms": 0, "num_turns": 1,
            "result": result, "session_id": self.session_id, "api_error_status": api_status,
            "total_cost_usd": usage.cost_usd.unwrap_or(0.0),
            "usage": {
                "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_read_tokens,
                "cache_creation_input_tokens": usage.cache_write_tokens,
            },
        }))
    }
}

#[derive(Debug, Default)]
struct TurnState {
    call: Option<String>,
    allowed: bool,
    last_text: String,
    usage: Usage,
}

fn user_text(msg: &Value) -> String {
    match &msg["message"]["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}
