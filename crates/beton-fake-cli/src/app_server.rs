//! Protokoll `app-server` (Codex, QA-002): JSON-RPC 2.0 über stdio wie `codex app-server`.
//!
//! Nachgebildet nach dem Schema von `codex app-server generate-json-schema` (codex-cli
//! 0.153.2) und einem Handshake mit der echten CLI: Antworten und Notifications tragen kein
//! `jsonrpc`-Feld; vor `initialize` beantwortet der Server alles mit `Not initialized`.
//!
//! Abbildung der Szenario-Schritte: `message_delta`/`message` → Item `agentMessage` mit
//! `item/agentMessage/delta`; `reasoning` → Item `reasoning` mit
//! `item/reasoning/summaryTextDelta`; `tool_call` mit `kind: shell` → `commandExecution`
//! (Freigabe über `item/commandExecution/requestApproval`), `kind: file_edit|file_write` →
//! `fileChange` (`item/fileChange/requestApproval`), sonst `mcpToolCall`; `usage` →
//! `thread/tokenUsage/updated`; `error`/`auth_expired` → `error`-Notification und
//! `turn/completed` mit Status `failed`; `hang` wartet auf `turn/interrupt`; `await_steer`
//! wartet auf `turn/steer` und setzt den Turn danach fort.

use std::io::{BufRead, Write};
use std::time::Duration;

use beton_harness::scenario::{Scenario, Step, Turn, Usage};
use serde_json::{Value, json};

use crate::io::{Lines, Stop, chunks, fnv};

const CONTEXT_WINDOW: u64 = 272_000;

pub struct AppServer<R, W> {
    io: Lines<R, W>,
    turns: Vec<Turn>,
    next_turn: usize,
    version: String,
    bad_handshake: bool,
    initialized: bool,
    seed: u64,
    thread_id: Option<String>,
    cwd: String,
    model: String,
    items: u32,
    requests: i64,
    total: Usage,
    /// Injizierte MCP-Server (HAR-009).
    mcp: crate::mcp::Clients,
}

/// Was ein Turn erlebt hat.
enum TurnEnd {
    Done,
    Interrupted,
    Failed { message: String, info: &'static str },
}

#[derive(Default)]
struct TurnState {
    turn_id: String,
    call: Option<Call>,
    allowed: bool,
}

#[derive(Clone)]
struct Call {
    id: String,
    item: Value,
}

impl<R: BufRead, W: Write> AppServer<R, W> {
    pub fn new(scenario: &Scenario, version: String, input: R, out: W) -> Self {
        Self {
            io: Lines::new(input, out, scenario.faults),
            turns: scenario.turns.clone(),
            next_turn: 0,
            version,
            bad_handshake: scenario.faults.bad_handshake,
            initialized: false,
            seed: fnv(&format!("{scenario:?}")),
            thread_id: None,
            cwd: "/".into(),
            model: "gpt-fake-codex".into(),
            items: 0,
            requests: 0,
            total: Usage::default(),
            mcp: crate::mcp::Clients::default(),
        }
    }

    fn respond(&mut self, id: &Value, result: Value) -> Result<(), Stop> {
        self.io.emit(&json!({"id": id, "result": result}))
    }

    fn respond_error(&mut self, id: &Value, code: i64, message: &str) -> Result<(), Stop> {
        self.io
            .emit(&json!({"error": {"code": code, "message": message}, "id": id}))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), Stop> {
        self.io.emit(&json!({"method": method, "params": params}))
    }

    fn thread(&self) -> Value {
        json!({
            "id": self.thread_id, "cliVersion": self.version, "createdAt": 0, "updatedAt": 0,
            "cwd": self.cwd, "ephemeral": false, "modelProvider": "openai", "preview": "",
            "projectId": null, "sessionId": self.thread_id, "source": "appServer",
            "status": {"type": "idle"}, "turns": [],
        })
    }

    fn thread_response(&self, params: &Value) -> Value {
        json!({
            "thread": self.thread(),
            "model": self.model,
            "modelProvider": "openai",
            "cwd": self.cwd,
            "approvalPolicy": params.get("approvalPolicy").cloned().unwrap_or(json!("untrusted")),
            "approvalsReviewer": "user",
            "sandbox": {"type": "workspaceWrite"},
        })
    }

    pub fn run(&mut self) -> Result<(), Stop> {
        loop {
            let msg = self.io.read()?;
            let Some(method) = msg["method"].as_str().map(str::to_owned) else {
                continue; // Antworten außerhalb eines Turns erwartet niemand.
            };
            let Some(id) = msg.get("id").cloned() else {
                continue; // Notifications wie `initialized`.
            };
            self.request(&id, &method, &msg["params"])?;
        }
    }

    fn request(&mut self, id: &Value, method: &str, params: &Value) -> Result<(), Stop> {
        if method == "initialize" {
            if params["clientInfo"]["name"].as_str().is_none() {
                return self.respond_error(
                    id,
                    -32600,
                    "Invalid request: missing field `clientInfo`",
                );
            }
            self.initialized = true;
            if self.bad_handshake {
                // Ein Server mit anderem Protokoll-Schema (HAR-006 AC4).
                return self.respond(
                    id,
                    json!({"protocolVersion": "v3", "serverInfo": {"name": "codex"}}),
                );
            }
            let name = params["clientInfo"]["name"].as_str().unwrap_or("client");
            let client_version = params["clientInfo"]["version"].as_str().unwrap_or("0");
            let user_agent = format!(
                "{name}/{} (beton-fake-cli) unknown ({name}; {client_version})",
                self.version
            );
            return self.respond(
                id,
                json!({
                    "userAgent": user_agent,
                    "codexHome": "/beton-fake/codex-home",
                    "platformFamily": "unix",
                    "platformOs": "fake",
                }),
            );
        }
        if !self.initialized {
            return self.respond_error(id, -32600, "Not initialized");
        }
        match method {
            "thread/start" | "thread/resume" => {
                self.thread_id = Some(match params["threadId"].as_str() {
                    Some(t) if method == "thread/resume" => t.to_owned(),
                    _ => format!(
                        "019a0000-0000-7000-8000-{:012x}",
                        self.seed & 0xffff_ffff_ffff
                    ),
                });
                if let Some(cwd) = params["cwd"].as_str() {
                    self.cwd = cwd.to_owned();
                }
                if let Some(model) = params["model"].as_str() {
                    self.model = model.to_owned();
                }
                let response = self.thread_response(params);
                self.respond(id, response)?;
                let thread = self.thread();
                self.notify("thread/started", json!({"thread": thread}))?;
                // Wie Codex: MCP-Server aus `config.mcp_servers` starten und melden.
                let configs = crate::mcp::from_codex_config(&params["config"]);
                if configs.is_empty() {
                    return Ok(());
                }
                self.mcp = crate::mcp::Clients::connect(&configs);
                let thread_id = self.thread_id.clone();
                for name in self.mcp.connected() {
                    self.notify("mcpServer/startupStatus/updated", json!({"threadId": thread_id, "name": name, "status": "ready", "error": null}))?;
                }
                for (name, error) in self.mcp.failed.clone() {
                    self.notify("mcpServer/startupStatus/updated", json!({"threadId": thread_id, "name": name, "status": "failed", "error": error}))?;
                }
                Ok(())
            }
            "skills/extraRoots/set" => self.respond(id, json!({})),
            "turn/start" => self.turn(id, params),
            "turn/interrupt" => self.respond(id, json!({})),
            _ => self.respond_error(id, -32601, &format!("Method not found: {method}")),
        }
    }

    fn turn(&mut self, id: &Value, params: &Value) -> Result<(), Stop> {
        if self.thread_id.is_none() || params["threadId"].as_str() != self.thread_id.as_deref() {
            return self.respond_error(id, -32600, "thread not found");
        }
        if let Some(model) = params["model"].as_str() {
            self.model = model.to_owned();
        }
        let index = self.next_turn;
        self.next_turn += 1;
        let mut state = TurnState {
            turn_id: format!("turn-{}", index + 1),
            ..TurnState::default()
        };
        let turn = json!({"id": state.turn_id, "items": [], "status": "inProgress", "error": null});
        self.respond(id, json!({"turn": turn}))?;
        let thread_id = self.thread_id.clone();
        self.notify("turn/started", json!({"threadId": thread_id, "turn": turn}))?;
        let end = match self.turns.get(index).cloned() {
            None => TurnEnd::Failed {
                message: "Szenario zu Ende".into(),
                info: "other",
            },
            Some(t) => {
                let text = input_text(params);
                match &t.expect_input {
                    Some(expected) if *expected != text => TurnEnd::Failed {
                        message: format!("erwartet `{expected}`, erhalten `{text}`"),
                        info: "badRequest",
                    },
                    _ => self.steps(
                        &beton_harness::scenario::resolve_echo(&t.emit, &text, &[]),
                        &mut state,
                    )?,
                }
            }
        };
        let (status, error) = match end {
            TurnEnd::Done => ("completed", Value::Null),
            TurnEnd::Interrupted => ("interrupted", Value::Null),
            TurnEnd::Failed { message, info } => {
                let error =
                    json!({"message": message, "codexErrorInfo": info, "additionalDetails": null});
                self.notify(
                    "error",
                    json!({"error": error, "willRetry": false, "threadId": thread_id, "turnId": state.turn_id}),
                )?;
                ("failed", error)
            }
        };
        self.notify(
            "turn/completed",
            json!({"threadId": thread_id, "turn": {
                "id": state.turn_id, "items": [], "status": status, "error": error,
                "durationMs": 0,
            }}),
        )
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

    fn next_item(&mut self, prefix: &str) -> String {
        self.items += 1;
        format!("{prefix}_fake{:08}", self.items)
    }

    fn item(&mut self, method: &str, state: &TurnState, item: Value) -> Result<(), Stop> {
        let thread_id = self.thread_id.clone();
        let mut params = json!({"item": item, "threadId": thread_id, "turnId": state.turn_id});
        if method == "item/started" {
            params["startedAtMs"] = json!(0);
        } else {
            params["completedAtMs"] = json!(0);
        }
        self.notify(method, params)
    }

    fn delta(
        &mut self,
        method: &str,
        state: &TurnState,
        item: &str,
        extra: Value,
    ) -> Result<(), Stop> {
        let thread_id = self.thread_id.clone();
        let mut params = json!({"itemId": item, "threadId": thread_id, "turnId": state.turn_id});
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        self.notify(method, params)
    }

    fn step(&mut self, step: &Step, state: &mut TurnState) -> Result<TurnEnd, Stop> {
        if let Some(text) = step.message_delta.as_ref().or(step.message.as_ref()) {
            let id = self.next_item("msg");
            self.item(
                "item/started",
                state,
                json!({"type": "agentMessage", "id": id, "text": ""}),
            )?;
            if step.message_delta.is_some() {
                for (i, piece) in chunks(text, step.chunk).into_iter().enumerate() {
                    if i > 0
                        && let Some(ms) = step.chunk_delay_ms
                    {
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                    self.delta(
                        "item/agentMessage/delta",
                        state,
                        &id,
                        json!({"delta": piece}),
                    )?;
                }
            }
            self.item(
                "item/completed",
                state,
                json!({"type": "agentMessage", "id": id, "text": text, "phase": "final_answer"}),
            )?;
        } else if let Some(text) = &step.reasoning {
            let id = self.next_item("rs");
            self.item(
                "item/started",
                state,
                json!({"type": "reasoning", "id": id, "summary": [], "content": []}),
            )?;
            self.delta(
                "item/reasoning/summaryTextDelta",
                state,
                &id,
                json!({"delta": text, "summaryIndex": 0}),
            )?;
            self.item(
                "item/completed",
                state,
                json!({"type": "reasoning", "id": id, "summary": [text], "content": []}),
            )?;
        } else if let Some(call) = &step.tool_call {
            let id = self.next_item("call");
            let item = match call.kind.as_str() {
                "shell" => json!({
                    "type": "commandExecution", "id": id,
                    "command": command_text(&call.args), "cwd": self.cwd,
                    "commandActions": [], "status": "inProgress",
                }),
                "file_edit" | "file_write" => json!({
                    "type": "fileChange", "id": id,
                    "changes": [{
                        "path": call.args["path"].as_str().unwrap_or("datei.txt"),
                        "kind": {"type": if call.kind == "file_write" { "add" } else { "update" }, "move_path": null},
                        "diff": call.args["diff"].as_str().unwrap_or(""),
                    }],
                    "status": "inProgress",
                }),
                _ => json!({
                    "type": "mcpToolCall", "id": id, "server": "fake", "tool": call.name,
                    "arguments": call.args, "status": "inProgress",
                }),
            };
            self.item("item/started", state, item.clone())?;
            state.allowed = true;
            if step.gate && item["type"] != "mcpToolCall" {
                match self.approval(&id, &item, state)? {
                    Some(allowed) => state.allowed = allowed,
                    None => return Ok(TurnEnd::Interrupted),
                }
                if !state.allowed {
                    let mut done = item.clone();
                    done["status"] = json!("declined");
                    self.item("item/completed", state, done)?;
                    state.call = None;
                    return Ok(TurnEnd::Done);
                }
            }
            state.call = Some(Call { id, item });
        } else if let Some(call) = &step.mcp_call {
            let id = self.next_item("mcp");
            let mut item = json!({
                "type": "mcpToolCall", "id": id, "server": call.server, "tool": call.tool,
                "arguments": call.args, "status": "inProgress",
            });
            self.item("item/started", state, item.clone())?;
            match self.mcp.call(&call.server, &call.tool, &call.args) {
                Ok(result) => {
                    item["status"] = json!(if result["isError"] == true {
                        "failed"
                    } else {
                        "completed"
                    });
                    item["result"] = result;
                }
                Err(e) => {
                    item["status"] = json!("failed");
                    item["error"] = json!({"message": e});
                }
            }
            item["durationMs"] = json!(0);
            self.item("item/completed", state, item)?;
        } else if let Some(on_gate) = &step.on_gate {
            let branch = if state.allowed {
                &on_gate.allow
            } else {
                &on_gate.deny
            };
            return self.steps(branch, state);
        } else if let Some(result) = &step.tool_result {
            self.complete(state, result, false)?;
        } else if let Some(error) = &step.tool_error {
            self.complete(state, &json!(error), true)?;
        } else if let Some(usage) = &step.usage {
            self.usage(state, usage)?;
        } else if let Some(message) = &step.error {
            return Ok(TurnEnd::Failed {
                message: message.clone(),
                info: "other",
            });
        } else if let Some(code) = step.crash {
            return Err(Stop::Crash(u8::try_from(code.clamp(1, 255)).unwrap_or(1)));
        } else if let Some(hint) = &step.auth_expired {
            return Ok(TurnEnd::Failed {
                message: format!("unauthorized: {hint}"),
                info: "unauthorized",
            });
        } else if let Some(write) = &step.write_file {
            // Wie ein Edit-Tool: Datei relativ zum Arbeitsverzeichnis (SES-017).
            let workdir = std::env::current_dir().unwrap_or_default();
            let _ = write.apply(&workdir);
        } else if step.hang {
            loop {
                let msg = self.io.read()?;
                if self.interrupt_request(&msg)? {
                    return Ok(TurnEnd::Interrupted);
                }
            }
        } else if let Some(expected) = &step.await_steer {
            return self.await_steer(expected, state);
        }
        Ok(TurnEnd::Done)
    }

    /// Wartet auf `turn/steer` für den laufenden Turn (SES-004 AC3). Wie die echte CLI
    /// prüft der Fake `expectedTurnId` und legt die Eingabe als Item `userMessage` an.
    fn await_steer(&mut self, expected: &str, state: &TurnState) -> Result<TurnEnd, Stop> {
        loop {
            let msg = self.io.read()?;
            if msg["method"] != "turn/steer" {
                if self.interrupt_request(&msg)? {
                    return Ok(TurnEnd::Interrupted);
                }
                continue;
            }
            let Some(id) = msg.get("id").cloned() else {
                continue;
            };
            let params = &msg["params"];
            if params["expectedTurnId"].as_str() != Some(state.turn_id.as_str()) {
                self.respond_error(&id, -32600, "expectedTurnId passt nicht zum aktiven Turn")?;
                continue;
            }
            let text = input_text(params);
            self.respond(&id, json!({"turnId": state.turn_id}))?;
            let item_id = self.next_item("user");
            let item = json!({"type": "userMessage", "id": item_id, "content": params["input"]});
            self.item("item/started", state, item.clone())?;
            self.item("item/completed", state, item)?;
            if !expected.is_empty() && text != expected {
                return Ok(TurnEnd::Failed {
                    message: format!("Steer: erwartet `{expected}`, erhalten `{text}`"),
                    info: "badRequest",
                });
            }
            return Ok(TurnEnd::Done);
        }
    }

    /// Beantwortet `turn/interrupt`; `true`, wenn es eine war.
    fn interrupt_request(&mut self, msg: &Value) -> Result<bool, Stop> {
        if msg["method"] == "turn/interrupt" {
            if let Some(id) = msg.get("id").cloned() {
                self.respond(&id, json!({}))?;
            }
            return Ok(true);
        }
        if let (Some(method), Some(id)) = (msg["method"].as_str(), msg.get("id").cloned()) {
            let method = method.to_owned();
            self.respond_error(&id, -32600, &format!("turn in progress: {method}"))?;
        }
        Ok(false)
    }

    /// Fragt beim Client nach. `None` bei Interrupt.
    fn approval(
        &mut self,
        id: &str,
        item: &Value,
        state: &TurnState,
    ) -> Result<Option<bool>, Stop> {
        let request_id = self.requests;
        self.requests += 1;
        let thread_id = self.thread_id.clone();
        let (method, params) = if item["type"] == "commandExecution" {
            (
                "item/commandExecution/requestApproval",
                json!({"itemId": id, "threadId": thread_id, "turnId": state.turn_id,
                       "command": item["command"], "cwd": item["cwd"], "reason": null,
                       "startedAtMs": 0}),
            )
        } else {
            (
                "item/fileChange/requestApproval",
                json!({"itemId": id, "threadId": thread_id, "turnId": state.turn_id,
                       "reason": null, "grantRoot": null, "startedAtMs": 0}),
            )
        };
        self.io
            .emit(&json!({"id": request_id, "method": method, "params": params}))?;
        loop {
            let msg = self.io.read()?;
            if msg.get("method").is_none() && msg["id"] == json!(request_id) {
                let decision = msg["result"]["decision"].as_str().unwrap_or("decline");
                let thread_id = self.thread_id.clone();
                self.notify(
                    "serverRequest/resolved",
                    json!({"threadId": thread_id, "requestId": request_id}),
                )?;
                return Ok(match decision {
                    "accept" | "acceptForSession" => Some(true),
                    "cancel" => None,
                    _ => Some(false),
                });
            }
            if self.interrupt_request(&msg)? {
                return Ok(None);
            }
        }
    }

    fn complete(
        &mut self,
        state: &mut TurnState,
        result: &Value,
        failed: bool,
    ) -> Result<(), Stop> {
        let Some(call) = state.call.take() else {
            return Ok(());
        };
        let text = match result {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut item = call.item.clone();
        match item["type"].as_str() {
            Some("commandExecution") => {
                self.delta(
                    "item/commandExecution/outputDelta",
                    state,
                    &call.id,
                    json!({"delta": format!("{text}\n")}),
                )?;
                item["status"] = json!(if failed { "failed" } else { "completed" });
                item["exitCode"] = json!(i32::from(failed));
                item["aggregatedOutput"] = json!(format!("{text}\n"));
                item["durationMs"] = json!(0);
            }
            Some("fileChange") => {
                item["status"] = json!(if failed { "failed" } else { "completed" });
            }
            _ => {
                item["status"] = json!(if failed { "failed" } else { "completed" });
                if failed {
                    item["error"] = json!({"message": text});
                } else {
                    item["result"] = json!({"content": [{"type": "text", "text": text}]});
                }
            }
        }
        self.item("item/completed", state, item)
    }

    fn usage(&mut self, state: &TurnState, usage: &Usage) -> Result<(), Stop> {
        let total_in = usage.input_tokens + usage.cache_read_tokens;
        self.total.input_tokens += total_in;
        self.total.cache_read_tokens += usage.cache_read_tokens;
        self.total.output_tokens += usage.output_tokens;
        let breakdown = |input: u64, cached: u64, output: u64| {
            json!({
                "inputTokens": input, "cachedInputTokens": cached, "cacheWriteInputTokens": 0,
                "outputTokens": output, "reasoningOutputTokens": 0, "totalTokens": input + output,
            })
        };
        let thread_id = self.thread_id.clone();
        let params = json!({
            "threadId": thread_id, "turnId": state.turn_id,
            "tokenUsage": {
                "total": breakdown(self.total.input_tokens, self.total.cache_read_tokens, self.total.output_tokens),
                "last": breakdown(total_in, usage.cache_read_tokens, usage.output_tokens),
                "modelContextWindow": CONTEXT_WINDOW,
            },
        });
        self.notify("thread/tokenUsage/updated", params)
    }
}

fn input_text(params: &Value) -> String {
    params["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn command_text(args: &Value) -> String {
    match &args["command"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => args.to_string(),
    }
}
