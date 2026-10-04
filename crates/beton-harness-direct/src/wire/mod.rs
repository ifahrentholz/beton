//! Wire-Formate (HAR-011): ein neutrales Gesprächsmodell und seine Abbildung auf die
//! Anthropic-Messages-API und OpenAI Chat Completions, jeweils mit Streaming (SSE) und
//! Tool-Calls.

pub mod anthropic;
pub mod openai;
pub mod sse;

use serde_json::Value;

use crate::config::WireKind;

/// Rolle einer Nachricht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// Ein Inhaltsblock.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    /// Überlegung des Modells; `signature` muss Anthropic unverändert zurückbekommen.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
}

/// Eine Nachricht im Verlauf.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            blocks: vec![Block::Text(text.into())],
        }
    }

    /// Grobe Größe in Zeichen (für die Schätzung der Tokens).
    pub fn chars(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| match b {
                Block::Text(t) => t.len(),
                Block::Thinking { text, .. } => text.len(),
                Block::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
                Block::ToolResult { content, .. } => content.len(),
            })
            .sum()
    }
}

/// Grobe Token-Schätzung (vier Zeichen je Token).
pub fn estimate_tokens(chars: usize) -> u64 {
    u64::try_from(chars.div_ceil(4)).unwrap_or(u64::MAX)
}

/// Ein Tool, wie es das Modell sieht.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Ein Model-Request.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub prompt_caching: bool,
}

/// Verbrauch eines Requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
    }

    /// Belegter Kontext nach dem Request: Eingabe (inkl. Cache) plus Ausgabe.
    pub fn context(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens + self.output_tokens
    }
}

/// Warum das Modell aufgehört hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

impl StopReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::ToolUse => "tool_use",
            Self::MaxTokens => "max_tokens",
            Self::Other(s) => s,
        }
    }
}

/// Ein Ereignis aus dem Antwort-Strom.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// ID der Antwort (z. B. `msg_…`), falls der Anbieter eine nennt.
    MessageId(String),
    TextDelta(String),
    ThinkingDelta(String),
    /// Vollständige Überlegung (mit Signatur), sobald ihr Block endet.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Vollständiger Tool-Call, sobald seine Argumente komplett sind.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    Usage(Usage),
    Stop(StopReason),
}

/// Fehler, den der Anbieter im Strom meldet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    pub kind: String,
    pub message: String,
}

/// Zustandsbehafteter Parser eines Antwort-Stroms.
pub trait StreamParser: Send {
    /// Verarbeitet ein SSE-Ereignis (`event`-Name und `data`).
    fn on_event(
        &mut self,
        event: Option<&str>,
        data: &str,
    ) -> Result<Vec<StreamEvent>, StreamError>;
    /// Der Strom ist zu Ende (letzte offene Blöcke abschließen).
    fn finish(&mut self) -> Vec<StreamEvent>;
}

/// JSON-Body eines Requests für die Wire-Familie.
pub fn request_body(wire: WireKind, req: &ModelRequest) -> Value {
    match wire {
        WireKind::Anthropic => anthropic::request_body(req),
        WireKind::Openai => openai::request_body(req),
    }
}

/// Neuer Parser für die Wire-Familie.
pub fn parser(wire: WireKind) -> Box<dyn StreamParser> {
    match wire {
        WireKind::Anthropic => Box::new(anthropic::Parser::default()),
        WireKind::Openai => Box::new(openai::Parser::default()),
    }
}

/// Fügt aufeinanderfolgende Nachrichten derselben Rolle zusammen (Anthropic verlangt
/// abwechselnde Rollen; nach einem Abbruch können zwei User-Nachrichten folgen).
pub fn merged(messages: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for m in messages {
        match out.last_mut() {
            Some(last) if last.role == m.role => last.blocks.extend(m.blocks.iter().cloned()),
            _ => out.push(m.clone()),
        }
    }
    // Tool-Ergebnisse zuerst, dann Text (Anthropic verlangt `tool_result` am Anfang).
    for m in &mut out {
        if m.role == Role::User {
            m.blocks
                .sort_by_key(|b| !matches!(b, Block::ToolResult { .. }));
        }
    }
    out
}

/// Text aus einem Fehler-Body eines Anbieters (`{"error": {"message": …}}` o. Ä.).
pub fn error_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let msg = parsed.as_ref().and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .or_else(|| v["message"].as_str())
            .map(str::to_owned)
    });
    let text = msg.unwrap_or_else(|| body.trim().to_owned());
    text.chars().take(2000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_user_messages_are_merged_with_tool_results_first() {
        let msgs = vec![
            Message::user_text("a"),
            Message {
                role: Role::User,
                blocks: vec![Block::ToolResult {
                    id: "t".into(),
                    content: "x".into(),
                    is_error: false,
                }],
            },
            Message {
                role: Role::Assistant,
                blocks: vec![Block::Text("b".into())],
            },
        ];
        let m = merged(&msgs);
        assert_eq!(m.len(), 2);
        assert!(matches!(m[0].blocks[0], Block::ToolResult { .. }));
        assert_eq!(m[0].blocks[1], Block::Text("a".into()));
    }

    #[test]
    fn provider_errors_are_extracted() {
        assert_eq!(
            error_message(r#"{"error":{"type":"x","message":"kaputt"}}"#),
            "kaputt"
        );
        assert_eq!(error_message("Bad Gateway"), "Bad Gateway");
    }
}
