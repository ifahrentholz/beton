//! Protokoll `stream-json` (Claude Code, QA-002): Eingaben als JSON-Zeilen auf stdin,
//! Ausgaben als JSON-Zeilen auf stdout; Freigaben über `control_request`/`can_use_tool`,
//! Unterbrechungen über `control_request`/`interrupt`.

use std::io::{BufRead, Write};
use std::time::Duration;

use std::path::{Path, PathBuf};

use beton_harness::scenario::{Scenario, Step, Usage, resolve_echo};
use serde_json::{Value, json};

use crate::io::{Lines, Stop, fnv};
use crate::mcp::{Clients, ServerConfig, result_text};

/// Was ein Turn erlebt hat.
enum TurnEnd {
    Done,
    Interrupted,
    Failed(String),
    /// Login abgelaufen: Result mit `api_error_status: 401`.
    AuthFailed(String),
}

/// Fortsetzen, Abzweigen und Ablage der Sessions wie bei der echten CLI.
#[derive(Debug, Default)]
pub struct Resume {
    /// `--resume <id>`.
    pub session: Option<String>,
    /// `--fork-session`.
    pub fork: bool,
    /// Projektverzeichnis der Session-Dateien (`--persist`).
    pub persist: Option<PathBuf>,
    /// `--session-id` für eine neue Session.
    pub session_id: Option<String>,
}

/// Modell, Effort und Permission-Mode wie bei der echten CLI (HAR-017, HAR-027).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    model: Option<String>,
    effort: Option<String>,
    /// `--permission-mode`; ohne Flag gilt `permissions.defaultMode` aus
    /// `.claude/settings.json` im Arbeitsverzeichnis (wie 2.1.285), sonst `default`.
    mode: String,
    /// Fehlerinjektion: Modus, den die CLI meldet, egal was gesetzt ist.
    report: Option<String>,
    /// Modus vor `plan`; `ExitPlanMode` stellt ihn wieder her (wie 2.1.285, `prePlanMode`).
    pre_plan: String,
}

impl Settings {
    pub fn from_flags(
        model: Option<String>,
        effort: Option<String>,
        mode: Option<String>,
        report: Option<String>,
    ) -> Self {
        let mode = mode.unwrap_or_else(|| {
            std::fs::read_to_string(".claude/settings.json")
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v["permissions"]["defaultMode"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "default".into())
        });
        Self {
            model,
            effort,
            mode,
            report,
            pre_plan: "default".into(),
        }
    }

    fn reported(&self) -> &str {
        self.report.as_deref().unwrap_or(&self.mode)
    }
}

/// Projektverzeichnis wie bei Claude Code: `<config>/projects/<cwd>`, jedes Zeichen außer
/// ASCII-Buchstaben und -Ziffern wird zu `-` (wie `beton_harness_claude::project_dir`).
pub fn project_dir(config: &Path, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    config.join("projects").join(slug)
}

/// Nutzer-Texte einer Session-Datei (ohne reine Tool-Results).
fn user_texts(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l["type"] == "user")
        .map(|l| match &l["message"]["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        })
        .filter(|t| !t.is_empty())
        .collect()
}

pub struct Sim<R, W> {
    io: Lines<R, W>,
    /// Nutzer-Nachrichten des nativen Verlaufs (für `echo_history`).
    history: Vec<String>,
    /// Datei der laufenden Session, wenn Sessions abgelegt werden.
    file: Option<PathBuf>,
    partial: bool,
    session_id: String,
    model: String,
    ids: u32,
    initialized: bool,
    mcp_configs: Vec<ServerConfig>,
    mcp: Clients,
    /// Belegter Kontext nach dem letzten `result` und bekanntes Kontextfenster (SES-011).
    context: u64,
    window: Option<u64>,
    settings: Settings,
}

impl<R: BufRead, W: Write> Sim<R, W> {
    pub fn new(
        scenario: &Scenario,
        partial: bool,
        resume: Resume,
        settings: Settings,
        mcp_configs: Vec<ServerConfig>,
        input: R,
        out: W,
    ) -> Self {
        let seed = fnv(&format!("{scenario:?}"));
        let uuid = |n: u64| format!("00000000-0000-4000-8000-{:012x}", n & 0xffff_ffff_ffff);
        // Wie die echte CLI: `--resume <id>` setzt die Session fort, mit `--fork-session` unter
        // neuer ID.
        let session_id = match (&resume.session, &resume.session_id) {
            (Some(id), _) if resume.fork => uuid(fnv(&format!("fork:{id}"))),
            (Some(id), _) => id.clone(),
            (None, Some(id)) => id.clone(),
            (None, None) => uuid(seed),
        };
        let mut lines: Vec<Value> = Vec::new();
        if let (Some(dir), Some(id)) = (&resume.persist, &resume.session) {
            let text = std::fs::read_to_string(dir.join(format!("{id}.jsonl"))).unwrap_or_default();
            lines = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
        }
        let file = resume
            .persist
            .as_ref()
            .map(|dir| dir.join(format!("{session_id}.jsonl")));
        if let Some(f) = &file
            && resume.fork
        {
            let copied: String = lines.iter().map(|l| format!("{l}\n")).collect();
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(f, copied);
        }
        Self {
            io: Lines::new(input, out, scenario.faults),
            history: user_texts(&lines),
            file,
            partial,
            session_id,
            model: settings
                .model
                .clone()
                .unwrap_or_else(|| "claude-fake".into()),
            ids: 0,
            initialized: false,
            mcp_configs,
            mcp: Clients::default(),
            context: 0,
            window: None,
            settings,
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
        let request = &msg["request"];
        let subtype = request["subtype"].as_str().unwrap_or_default();
        let mut response = json!({});
        let mut status = false;
        match subtype {
            "initialize" => {
                // Wie 2.1.285: der wirksame Modus steht in der Initialize-Antwort.
                response = json!({"current_permission_mode": self.settings.reported()});
            }
            "set_model" => {
                if let Some(m) = request["model"].as_str() {
                    self.model = m.to_owned();
                }
            }
            "apply_flag_settings" => {
                if let Some(e) = request["settings"]["effortLevel"].as_str() {
                    self.settings.effort = Some(e.to_owned());
                }
            }
            "set_permission_mode" => {
                if let Some(m) = request["mode"].as_str() {
                    if m == "plan" && self.settings.mode != "plan" {
                        self.settings.pre_plan = self.settings.mode.clone();
                    }
                    m.clone_into(&mut self.settings.mode);
                    response = json!({"mode": m});
                    status = true;
                }
            }
            _ => {}
        }
        self.emit(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response},
        }))?;
        if status {
            self.emit(json!({
                "type": "system", "subtype": "status", "status": null,
                "permissionMode": self.settings.reported(), "session_id": self.session_id,
            }))?;
        }
        Ok(subtype == "interrupt")
    }

    pub fn run(&mut self, scenario: &Scenario) -> Result<(), Stop> {
        let mut cursor = beton_harness::scenario::TurnCursor::default();
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
                        // Wie die echte CLI: injizierte MCP-Server starten, Tools melden.
                        self.mcp = Clients::connect(&self.mcp_configs);
                        let mut tools = vec![json!("Bash"), json!("Read"), json!("Edit")];
                        tools.extend(self.mcp.claude_tool_names().into_iter().map(Value::String));
                        self.emit(json!({
                            "type": "system", "subtype": "init",
                            "session_id": self.session_id, "model": self.model,
                            "tools": tools, "mcp_servers": self.mcp.status(),
                            "permissionMode": self.settings.reported(), "apiKeySource": "none",
                        }))?;
                    }
                    // Wie die echte CLI: `/compact` fasst den Verlauf zusammen, meldet
                    // `compact_boundary` und ein `result`, ohne einen Szenario-Turn zu verbrauchen.
                    if text == "/compact" {
                        let before = self.context;
                        self.emit(json!({
                            "type": "system", "subtype": "compact_boundary",
                            "session_id": self.session_id,
                            "compact_metadata": {"trigger": "manual", "pre_tokens": before},
                        }))?;
                        let after = (before / 10).max(1);
                        let usage = Usage {
                            input_tokens: before,
                            output_tokens: after,
                            context_window: self.window,
                            ..Usage::default()
                        };
                        self.result(TurnEnd::Done, "", &usage)?;
                        self.context = after;
                        continue;
                    }
                    let Some(turn) = cursor.pick(scenario, &text) else {
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
                    crate::io::record_context("user", &text);
                    let settings = beton_harness::scenario::settings_text(
                        &self.model,
                        self.settings.effort.as_deref(),
                        self.settings.reported(),
                    );
                    let steps = beton_harness::scenario::resolve_settings(
                        &resolve_echo(&turn.emit, &text, &self.history),
                        &settings,
                    );
                    self.record("user", json!(text));
                    self.history.push(text);
                    let last = self.turn(&steps)?;
                    if !last.is_empty() {
                        self.record("assistant", json!([{"type": "text", "text": last}]));
                    }
                }
                _ => {} // Andere Nachrichten ignoriert die echte CLI ebenfalls.
            }
        }
    }

    /// Liefert den letzten Text des Agents.
    fn turn(&mut self, steps: &[Step]) -> Result<String, Stop> {
        let mut state = TurnState::default();
        let end = self.steps(steps, &mut state)?;
        let usage = state.usage.clone();
        self.result(end, &state.last_text, &usage)?;
        Ok(state.last_text)
    }

    /// Hängt einen Eintrag an die Session-Datei (nur mit `--persist`).
    fn record(&self, kind: &str, content: Value) {
        let Some(file) = &self.file else { return };
        let line = json!({
            "type": kind,
            "sessionId": self.session_id,
            "message": {"role": kind, "content": content},
        });
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = writeln!(f, "{line}");
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
            // Wie die CLI: ein freigegebenes `ExitPlanMode` verlässt `plan` (HAR-027).
            if call.name == "ExitPlanMode" && state.allowed && self.settings.mode == "plan" {
                self.settings.mode = self.settings.pre_plan.clone();
                self.emit(json!({
                    "type": "system", "subtype": "status", "status": null,
                    "permissionMode": self.settings.reported(), "session_id": self.session_id,
                }))?;
            }
            state.call = Some(id);
        } else if let Some(call) = &step.mcp_call {
            let id = self.next_id("toolu");
            let name = format!("mcp__{}__{}", call.server, call.tool);
            let args = self.mcp.resolve(&call.args);
            self.assistant(json!({"type": "tool_use", "id": id, "name": name, "input": args}))?;
            match self.mcp.call(&call.server, &call.tool, &args) {
                Ok(result) => {
                    let is_error = result["isError"] == true;
                    self.tool_result(&id, &json!(result_text(&result)), is_error)?;
                }
                Err(e) => self.tool_result(&id, &json!(e), true)?,
            }
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
        let mut msg = json!({
            "type": "result", "subtype": subtype, "is_error": is_error,
            "duration_ms": 0, "duration_api_ms": 0, "num_turns": 1,
            "result": result, "session_id": self.session_id, "api_error_status": api_status,
            "total_cost_usd": usage.cost_usd.unwrap_or(0.0),
            "usage": {
                "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_read_tokens,
                "cache_creation_input_tokens": usage.cache_write_tokens,
            },
        });
        // Wie die echte CLI: Kontextfenster je Modell in `modelUsage` (SES-011).
        self.window = usage.context_window.or(self.window);
        self.context = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
        // Wie die echte CLI: `modelUsage` je Modell, das den Turn bearbeitet hat (HAR-017 AC1).
        msg["modelUsage"] = json!({ self.model.clone(): {} });
        if let Some(window) = self.window {
            msg["modelUsage"][&self.model]["contextWindow"] = json!(window);
        }
        self.emit(msg)
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
