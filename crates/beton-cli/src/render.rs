//! Darstellung von Session-Events im Terminal (CLI-002, CLI-003, API-006).
//!
//! Interaktiv geht alles nach stdout (das Terminal ist die Oberfläche). Im Skript-Modus
//! landet auf stdout nur das Ergebnis (`text`/`json`) bzw. jedes Event als NDJSON
//! (`stream-json`); Fortschritt geht nach stderr.

use std::collections::HashSet;
use std::io::Write as _;

use serde_json::Value;

use crate::cli::OutputFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Interactive,
    Script(OutputFormat),
}

#[derive(Debug)]
pub struct Renderer {
    style: Style,
    /// Fortschritt auf stderr im Skript-Modus (aus mit `-q`).
    progress: bool,
    /// Nachricht, deren Deltas gerade ausgegeben werden.
    streaming: Option<String>,
    streamed: HashSet<String>,
    /// Text der letzten Assistenten-Nachricht.
    pub last_answer: String,
    /// Noch im Replay: eigene Eingaben aus dem Verlauf zeigen; live hat der Nutzer sie
    /// gerade selbst getippt.
    pub replaying: bool,
}

/// Text einer `message.completed`-Nachricht (Blöcke vom Typ `text`).
pub fn message_text(payload: &Value) -> String {
    payload["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| match b {
                    Value::String(s) => Some(s.clone()),
                    b if b["type"] == "text" => b["text"].as_str().map(str::to_owned),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Kurzform eines Tool-Aufrufs, z. B. `Bash: git push origin main`.
pub fn tool_summary(tool: &str, args: &Value) -> String {
    let detail = ["command", "file_path", "path", "url", "pattern", "query"]
        .iter()
        .find_map(|k| args[*k].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let s = args.to_string();
            if s == "null" || s == "{}" {
                String::new()
            } else {
                s
            }
        });
    let detail: String = detail
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    if detail.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool}: {detail}")
    }
}

/// Kurzform einer Freigabe-Anfrage.
pub fn approval_summary(payload: &Value) -> String {
    let subject = &payload["subject"];
    match subject["tool"].as_str() {
        Some(tool) => tool_summary(tool, &subject["args"]),
        None => payload["kind"].as_str().unwrap_or("Freigabe").to_owned(),
    }
}

impl Renderer {
    pub fn new(style: Style, progress: bool) -> Self {
        Self {
            style,
            progress,
            streaming: None,
            streamed: HashSet::new(),
            last_answer: String::new(),
            replaying: true,
        }
    }

    fn out(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    /// Zeile für den Nutzer: interaktiv auf stdout, im Skript-Modus auf stderr.
    fn line(&mut self, text: &str) {
        self.end_stream();
        match self.style {
            Style::Interactive => self.out(&format!("{text}\n")),
            Style::Script(_) => {
                if self.progress {
                    eprintln!("{text}");
                }
            }
        }
    }

    /// Fehler: immer auf stderr.
    fn error(&mut self, text: &str) {
        self.end_stream();
        eprintln!("{text}");
    }

    fn end_stream(&mut self) {
        if self.streaming.take().is_some() && self.style == Style::Interactive {
            self.out("\n");
        }
    }

    pub fn event(&mut self, e: &Value) {
        if self.style == Style::Script(OutputFormat::StreamJson) {
            self.out(&format!("{e}\n"));
        }
        let p = &e["payload"];
        match e["type"].as_str().unwrap_or_default() {
            "message.delta" => {
                let id = p["message_id"].as_str().unwrap_or_default().to_owned();
                if self.style != Style::Interactive {
                    return;
                }
                if self.streaming.as_deref() != Some(id.as_str()) {
                    self.end_stream();
                    self.streaming = Some(id.clone());
                }
                self.streamed.insert(id);
                let text = p["text"].as_str().unwrap_or_default().to_owned();
                self.out(&text);
            }
            "message.completed" => {
                let id = p["message_id"].as_str().unwrap_or_default();
                let text = message_text(p);
                match p["role"].as_str() {
                    Some("assistant") => {
                        self.last_answer.clone_from(&text);
                        if self.style == Style::Interactive {
                            if self.streamed.contains(id) {
                                self.end_stream();
                            } else if !text.is_empty() {
                                self.line(&text);
                            }
                        }
                    }
                    Some("user") if self.style == Style::Interactive && self.replaying => {
                        self.line(&format!("› {text}"));
                    }
                    _ => {}
                }
            }
            "tool.call.requested" => {
                let summary = tool_summary(p["tool"].as_str().unwrap_or("tool"), &p["args"]);
                self.line(&format!("▸ {summary}"));
            }
            "tool.call.completed" => {
                let secs = p["duration_ms"].as_u64().unwrap_or(0) as f64 / 1000.0;
                let mark = if p["status"] == "ok" { "✓" } else { "✗" };
                self.line(&format!(
                    "  {mark} {} ({secs:.1} s)",
                    p["status"].as_str().unwrap_or("")
                ));
            }
            "approval.resolved" => {
                let decision = p["decision"].as_str().unwrap_or_default();
                let text = if decision.starts_with("allow") {
                    "erlaubt"
                } else {
                    "abgelehnt"
                };
                self.line(&format!("  Freigabe {text}"));
            }
            "turn.interrupted" => self.line("(unterbrochen)"),
            "turn.failed" => {
                let problem = &p["problem"];
                let msg = problem["detail"]
                    .as_str()
                    .or_else(|| problem["title"].as_str())
                    .unwrap_or("Turn fehlgeschlagen");
                self.error(&format!("Fehler: {msg}"));
            }
            "error" => {
                let msg = p["problem"]["detail"]
                    .as_str()
                    .or_else(|| p["problem"]["title"].as_str())
                    .unwrap_or("Fehler");
                self.error(&format!("Fehler: {msg}"));
            }
            "harness.auth_required" => self.error(&format!(
                "{} ist nicht angemeldet. Abhilfe: {}",
                p["harness"].as_str().unwrap_or("Harness"),
                p["hint"].as_str().unwrap_or("Login der Vendor-CLI")
            )),
            "harness.incompatible" => self.error(&format!(
                "Harness-Version {} passt nicht (erwartet {})",
                p["detected_version"].as_str().unwrap_or("?"),
                p["expected_range"].as_str().unwrap_or("?")
            )),
            "notice" => {
                if let Some(text) = p["text"].as_str() {
                    self.line(&format!("· {text}"));
                }
            }
            _ => {}
        }
    }

    /// Prompt-Zeile vor einer Freigabe-Frage.
    pub fn ask(&mut self, question: &str) {
        self.end_stream();
        eprint!("{question} ");
        let _ = std::io::stderr().flush();
    }

    pub fn finish(&mut self) {
        self.end_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_text_joins_text_blocks() {
        let p = json!({"content": [{"type": "text", "text": "Hal"}, {"type": "tool_use"}, {"type": "text", "text": "lo"}]});
        assert_eq!(message_text(&p), "Hallo");
        assert_eq!(message_text(&json!({})), "");
    }

    #[test]
    fn tool_and_approval_summaries() {
        assert_eq!(
            tool_summary("Bash", &json!({"command": "git push origin main"})),
            "Bash: git push origin main"
        );
        assert_eq!(tool_summary("Read", &json!({})), "Read");
        assert_eq!(
            approval_summary(
                &json!({"kind": "tool", "subject": {"tool": "Bash", "args": {"command": "ls"}}})
            ),
            "Bash: ls"
        );
    }
}
