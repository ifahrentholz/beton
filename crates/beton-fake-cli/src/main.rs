//! Deterministische Fake-Vendor-CLI für Tests (QA-002).
//!
//! ```text
//! beton-fake-cli --protocol stream-json --scenario <datei.yaml> [Claude-Flags …]
//! beton-fake-cli --version
//! ```
//!
//! Spielt Szenarien im Format des Fake-Harness (HAR-026, `beton_harness::scenario`) über
//! das stream-json-Protokoll von Claude Code ab: Eingaben kommen als JSON-Zeilen auf stdin,
//! Ausgaben gehen als JSON-Zeilen auf stdout. Freigaben laufen über
//! `control_request`/`can_use_tool`, Unterbrechungen über `control_request`/`interrupt`.
//! Alle IDs sind aus dem Szenario abgeleitet, damit gleiche Eingaben byte-gleiche Ausgaben
//! liefern. Unbekannte Flags der echten CLI werden ignoriert.
//!
//! Die Treue zum echten Protokoll sichern die Golden-Transcripts des Claude-Adapters
//! (HAR-025); `app-server` (Codex) und `acp` folgen in M1.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use beton_harness::scenario::{Faults, Scenario, Step, Usage};
use serde_json::{Value, json};

const DEFAULT_VERSION: &str = "2.1.0";

#[derive(Debug, Default)]
struct Args {
    /// `auth status`: Login-Status wie die echte CLI (HAR-016).
    auth_status: bool,
    protocol: Option<String>,
    scenario: Option<PathBuf>,
    version: bool,
    partial: bool,
    faults: Faults,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.peekable();
    let number = |v: Option<String>, flag: &str| {
        v.and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{flag} braucht eine Zahl"))
    };
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--protocol" => out.protocol = args.next(),
            "--scenario" => out.scenario = args.next().map(PathBuf::from),
            "--version" | "-v" => out.version = true,
            "--include-partial-messages" => out.partial = true,
            "--crash-after" => out.faults.crash_after = Some(number(args.next(), &arg)?),
            "--hang-after" => out.faults.hang_after = Some(number(args.next(), &arg)?),
            "--malformed-line" => out.faults.malformed_line = Some(number(args.next(), &arg)?),
            a if !a.starts_with('-') => positional.push(arg),
            _ => {} // Flags der echten CLI (-p, --output-format, --model …) ignorieren.
        }
    }
    out.auth_status = positional.first().map(String::as_str) == Some("auth")
        && positional.get(1).map(String::as_str) == Some("status");
    if out.scenario.is_none() {
        out.scenario = std::env::var_os("BETON_FAKE_SCENARIO").map(PathBuf::from);
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("beton-fake-cli: {e}");
            return ExitCode::from(2);
        }
    };
    let scenario = match &args.scenario {
        Some(path) => match Scenario::load(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("beton-fake-cli: {e}");
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    if args.auth_status {
        // Wie `claude auth status --json`, samt Kontodaten, die beton verwerfen muss.
        let logged_in = std::env::var("BETON_FAKE_AUTH").as_deref() != Ok("logged_out");
        println!(
            "{}",
            serde_json::json!({
                "loggedIn": logged_in,
                "email": "fake-user@example.invalid",
                "orgName": "Fake Org",
            })
        );
        return ExitCode::SUCCESS;
    }
    if args.version {
        let version = scenario
            .as_ref()
            .and_then(|s| s.version.clone())
            .or_else(|| std::env::var("BETON_FAKE_VERSION").ok())
            .unwrap_or_else(|| DEFAULT_VERSION.into());
        println!("{version} (Claude Code)");
        return ExitCode::SUCCESS;
    }
    match args.protocol.as_deref() {
        Some("stream-json") => {}
        Some(p @ ("app-server" | "acp")) => {
            eprintln!("beton-fake-cli: Protokoll {p} folgt in M1");
            return ExitCode::from(2);
        }
        other => {
            eprintln!("beton-fake-cli: --protocol stream-json erwartet, erhalten {other:?}");
            return ExitCode::from(2);
        }
    }
    let Some(mut scenario) = scenario else {
        eprintln!("beton-fake-cli: --scenario <datei> fehlt");
        return ExitCode::from(2);
    };
    // Flags überschreiben die Fehlerinjektion des Szenarios.
    let f = args.faults;
    scenario.faults.crash_after = f.crash_after.or(scenario.faults.crash_after);
    scenario.faults.hang_after = f.hang_after.or(scenario.faults.hang_after);
    scenario.faults.malformed_line = f.malformed_line.or(scenario.faults.malformed_line);

    let stdin = io::stdin();
    let mut sim = Sim::new(&scenario, args.partial, stdin.lock(), io::stdout().lock());
    match sim.run(&scenario) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Stop::Crash(code)) => {
            eprintln!("beton-fake-cli: Absturz (Fehlerinjektion)");
            ExitCode::from(code)
        }
        Err(Stop::Io(e)) => {
            eprintln!("beton-fake-cli: {e}");
            ExitCode::from(1)
        }
        Err(Stop::Eof) => ExitCode::SUCCESS,
    }
}

enum Stop {
    Crash(u8),
    Eof,
    Io(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Was ein Turn erlebt hat.
enum TurnEnd {
    Done,
    Interrupted,
    Failed(String),
    /// Login abgelaufen: Result mit `api_error_status: 401`.
    AuthFailed(String),
}

struct Sim<R, W> {
    input: R,
    out: W,
    faults: Faults,
    lines: u32,
    partial: bool,
    session_id: String,
    model: String,
    ids: u32,
    initialized: bool,
}

/// FNV-1a: stabiler Hash ohne Abhängigkeit, für deterministische IDs.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl<R: BufRead, W: Write> Sim<R, W> {
    fn new(scenario: &Scenario, partial: bool, input: R, out: W) -> Self {
        let seed = fnv(&format!("{scenario:?}"));
        Self {
            input,
            out,
            faults: scenario.faults,
            lines: 0,
            partial,
            session_id: format!("00000000-0000-4000-8000-{:012x}", seed & 0xffff_ffff_ffff),
            model: "claude-fake".into(),
            ids: 0,
            initialized: false,
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.ids += 1;
        format!("{prefix}_fake{:08}", self.ids)
    }

    /// Schreibt eine Zeile und wendet die Fehlerinjektion an.
    fn emit(&mut self, value: Value) -> Result<(), Stop> {
        self.lines += 1;
        if self.faults.malformed_line == Some(self.lines) {
            writeln!(self.out, "{{\"type\":\"assistant\",\"message\":")?;
        } else {
            writeln!(self.out, "{value}")?;
        }
        self.out.flush()?;
        if self.faults.crash_after == Some(self.lines) {
            return Err(Stop::Crash(1));
        }
        if self.faults.hang_after == Some(self.lines) {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Value, Stop> {
        loop {
            let mut line = String::new();
            if self.input.read_line(&mut line)? == 0 {
                return Err(Stop::Eof);
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(v) => return Ok(v),
                Err(e) => eprintln!("beton-fake-cli: ungültige Eingabe ignoriert: {e}"),
            }
        }
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

    fn run(&mut self, scenario: &Scenario) -> Result<(), Stop> {
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
            if self.partial {
                let chunk = step.chunk.unwrap_or(usize::MAX);
                let chars: Vec<char> = text.chars().collect();
                for piece in chars.chunks(chunk.min(chars.len().max(1))) {
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
            self.assistant(json!({"type": "text", "text": text}))?;
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
