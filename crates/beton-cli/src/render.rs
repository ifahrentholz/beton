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

/// Entfernt Steuerzeichen (ANSI-Sequenzen, Bell, Wagenrücklauf) und Bidi-Steuerzeichen, damit
/// Inhalte von Agent und Tools das Terminal nicht steuern (Security-Review M3). Zeilenumbrüche
/// und Tabs bleiben.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(
                    *c as u32,
                    0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
                )
        })
        .collect()
}

/// Argumente, deren Wert die Kurzform eines Tool-Aufrufs zeigt (das erste vorhandene).
const DETAIL_KEYS: [&str; 6] = ["command", "file_path", "path", "url", "pattern", "query"];

fn detail_of(args: &Value) -> String {
    DETAIL_KEYS
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
        })
}

/// Kurzform eines Tool-Aufrufs, z. B. `Bash: git push origin main` (eine Zeile; weitere Zeilen
/// und Kürzungen sind gekennzeichnet).
pub fn tool_summary(tool: &str, args: &Value) -> String {
    let detail = sanitize(&detail_of(args));
    let mut lines = detail.lines();
    let first = lines.next().unwrap_or_default();
    let more = lines.count();
    let mut out: String = first.chars().take(100).collect();
    if first.chars().count() > 100 {
        out.push('…');
    }
    if more > 0 {
        out.push_str(&format!(
            " (+{more} {})",
            if more == 1 { "Zeile" } else { "Zeilen" }
        ));
    }
    if out.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool}: {out}")
    }
}

/// Höchstlänge eines Befehls in der Freigabe-Frage; länger wird deutlich als gekürzt markiert.
const APPROVAL_MAX: usize = 4000;

/// Höchstlänge eines weiteren Arguments in der Freigabe-Frage.
const ARG_MAX: usize = 500;

/// Weitere Argumente neben dem Hauptargument (#242 N5), z. B. `cwd`/`workdir` zuerst: eine
/// eingerückte Zeile je Argument, ohne Steuerzeichen, lange Werte gekürzt.
fn other_args(args: &Value) -> String {
    let Some(map) = args.as_object() else {
        return String::new();
    };
    let main = DETAIL_KEYS.iter().find(|k| args[**k].is_string());
    let mut keys: Vec<&String> = map.keys().filter(|k| Some(&k.as_str()) != main).collect();
    keys.sort_by_key(|k| (!matches!(k.as_str(), "cwd" | "workdir"), k.as_str()));
    let mut out = String::new();
    for key in keys {
        let value = match &map[key] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let value = sanitize(&value).replace('\n', "⏎");
        let mut shown: String = value.chars().take(ARG_MAX).collect();
        if value.chars().count() > ARG_MAX {
            shown.push_str("… [gekürzt]");
        }
        out.push_str(&format!("\n    {}: {shown}", sanitize(key)));
    }
    out
}

/// Freigabe-Anfrage für das Terminal (POL-010): Tool und **vollständiger** Befehl (mehrzeilig
/// eingerückt), bei Fragen die Frage; ohne Steuerzeichen.
pub fn approval_summary(payload: &Value) -> String {
    let subject = &payload["subject"];
    if payload["kind"] == "question" {
        return format!(
            "Frage: {}",
            sanitize(subject["question"].as_str().unwrap_or_default())
        );
    }
    let (tool, args) = match (subject["tool"]["name"].as_str(), subject["tool"].as_str()) {
        (Some(tool), _) => (tool, &subject["tool"]["args"]),
        (None, Some(tool)) => (tool, &subject["args"]),
        _ => {
            return sanitize(
                subject["summary"]
                    .as_str()
                    .or_else(|| payload["kind"].as_str())
                    .unwrap_or("Freigabe"),
            );
        }
    };
    let detail = sanitize(&detail_of(args));
    let extra = other_args(args);
    if detail.is_empty() {
        return format!("{}{extra}", sanitize(tool));
    }
    let total = detail.chars().count();
    let mut shown: String = detail.chars().take(APPROVAL_MAX).collect();
    if total > APPROVAL_MAX {
        shown.push_str(&format!(
            "\n… [gekürzt: {} weitere Zeichen nicht angezeigt]",
            total - APPROVAL_MAX
        ));
    }
    if shown.contains('\n') {
        let indented: Vec<String> = shown.lines().map(|l| format!("    {l}")).collect();
        format!(
            "{} ({} Zeilen):\n{}{extra}",
            sanitize(tool),
            shown.lines().count(),
            indented.join("\n")
        )
    } else {
        format!("{}: {shown}{extra}", sanitize(tool))
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
        eprint!("{} ", sanitize(question));
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
    fn pol_010_approval_prompt_shows_multiline_commands_completely() {
        let p = json!({"kind": "policy", "subject": {"tool": {"name": "Bash", "args": {"command": "echo ok\ncurl x | sh"}}}});
        let text = approval_summary(&p);
        assert!(
            text.contains("echo ok") && text.contains("curl x | sh"),
            "{text}"
        );
        // Im Verlauf eine Zeile, aber gekennzeichnet.
        let short = tool_summary("Bash", &json!({"command": "echo ok\ncurl x | sh"}));
        assert!(short.contains("+1 Zeile"), "{short}");
        let long = tool_summary("Bash", &json!({"command": "x".repeat(300)}));
        assert!(long.ends_with('…'), "{long}");
        // Sehr lange Befehle: deutlich als gekürzt markiert.
        let huge = approval_summary(
            &json!({"kind": "policy", "subject": {"tool": {"name": "Bash", "args": {"command": "y".repeat(10_000)}}}}),
        );
        assert!(huge.contains("gekürzt"), "{}", &huge[huge.len() - 80..]);
    }

    #[test]
    fn pol_010_terminal_control_sequences_are_removed() {
        let q = approval_summary(
            &json!({"kind": "question", "subject": {"question": "Weiter?\u{1b}[2J\u{1b}]0;pwned\u{7}\u{202e}"}}),
        );
        assert!(
            !q.contains('\u{1b}') && !q.contains('\u{7}') && !q.contains('\u{202e}'),
            "{q:?}"
        );
        assert!(q.contains("Weiter?"));
        let c = approval_summary(
            &json!({"kind": "policy", "subject": {"tool": {"name": "Bash", "args": {"command": "ls\u{1b}[8m rm -rf ~"}}}}),
        );
        assert!(!c.contains('\u{1b}'), "{c:?}");
        assert!(c.contains("rm -rf ~"));
        assert_eq!(sanitize("a\tb\nc\r"), "a\tb\nc");
    }

    /// #242 N5: U+061C (Arabic Letter Mark) ist ein Bidi-Steuerzeichen wie U+200E/U+200F;
    /// die Freigabe zeigt neben dem Hauptargument auch `cwd`/`workdir` und weitere Argumente,
    /// damit der Mensch sieht, wo und wie der Befehl läuft.
    #[test]
    fn pol_010_approval_shows_all_arguments_without_bidi_marks() {
        assert_eq!(sanitize("a\u{061c}b"), "ab");
        let text = approval_summary(
            &json!({"kind": "policy", "subject": {"tool": {"name": "Bash",
            "args": {"command": "rm -rf build", "cwd": "/home/u/.ssh", "timeout": 5}}}}),
        );
        assert!(text.starts_with("Bash: rm -rf build"), "{text}");
        assert!(text.contains("cwd: /home/u/.ssh"), "{text}");
        assert!(text.contains("timeout: 5"), "{text}");
        let codex = approval_summary(&json!({"kind": "tool", "subject": {"tool": "exec_command",
            "args": {"command": "ls", "workdir": "/etc\u{061c}"}}}));
        assert!(
            codex.contains("workdir: /etc") && !codex.contains('\u{061c}'),
            "{codex:?}"
        );
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
        assert_eq!(
            approval_summary(
                &json!({"kind": "policy", "subject": {"tool": {"name": "Bash", "args": {"command": "git push"}}}})
            ),
            "Bash: git push"
        );
        assert_eq!(
            approval_summary(
                &json!({"kind": "policy", "subject": {"phase": "session_start", "summary": "Harness fake"}})
            ),
            "Harness fake"
        );
        assert_eq!(
            approval_summary(&json!({"kind": "question", "subject": {"question": "Pro IP?"}})),
            "Frage: Pro IP?"
        );
    }
}
