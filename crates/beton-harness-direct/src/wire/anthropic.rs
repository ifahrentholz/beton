//! Anthropic Messages-API: `POST {base_url}/v1/messages` mit `stream: true`.
//!
//! Strom: `message_start` (ID, Eingabe-Tokens), `content_block_start|delta|stop`
//! (`text_delta`, `input_json_delta`, `thinking_delta`, `signature_delta`),
//! `message_delta` (`stop_reason`, Ausgabe-Tokens), `message_stop`, `ping`, `error`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    Block, ModelRequest, Role, StopReason, StreamError, StreamEvent, StreamParser, Usage, merged,
};

/// Header `anthropic-version`.
pub const API_VERSION: &str = "2023-06-01";

fn block_json(b: &Block) -> Value {
    match b {
        Block::Text(t) => json!({"type": "text", "text": t}),
        Block::Thinking { text, signature } => {
            json!({"type": "thinking", "thinking": text, "signature": signature.clone().unwrap_or_default()})
        }
        Block::ToolUse { id, name, input } => {
            json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        Block::ToolResult {
            id,
            content,
            is_error,
        } => {
            json!({"type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error})
        }
    }
}

/// Request-Body.
pub fn request_body(req: &ModelRequest) -> Value {
    let messages: Vec<Value> = merged(&req.messages)
        .iter()
        .map(|m| {
            json!({
                "role": match m.role { Role::User => "user", Role::Assistant => "assistant" },
                "content": m.blocks.iter()
                    // Überlegungen ohne Signatur lehnt die API ab; sie bleiben lokal.
                    .filter(|b| !matches!(b, Block::Thinking { signature: None, .. }))
                    .map(block_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut system = json!({"type": "text", "text": req.system});
    let mut tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
        .collect();
    if req.prompt_caching {
        system["cache_control"] = json!({"type": "ephemeral"});
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "system": [system],
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    body
}

#[derive(Debug)]
enum Open {
    Text,
    Tool {
        id: String,
        name: String,
        json: String,
    },
    Thinking {
        text: String,
        signature: Option<String>,
    },
}

/// Parser des Anthropic-Stroms.
#[derive(Debug, Default)]
pub struct Parser {
    open: BTreeMap<u64, Open>,
    usage: Usage,
}

impl Parser {
    fn close(&mut self, index: u64) -> Option<StreamEvent> {
        match self.open.remove(&index)? {
            Open::Text => None,
            Open::Tool { id, name, json } => {
                let input = if json.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&json).unwrap_or_else(|_| json!({"_raw": json}))
                };
                Some(StreamEvent::ToolUse { id, name, input })
            }
            Open::Thinking { text, signature } => Some(StreamEvent::Thinking { text, signature }),
        }
    }
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_owned()),
    }
}

impl StreamParser for Parser {
    fn on_event(
        &mut self,
        event: Option<&str>,
        data: &str,
    ) -> Result<Vec<StreamEvent>, StreamError> {
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Ok(Vec::new());
        };
        let kind = v["type"].as_str().or(event).unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                let m = &v["message"];
                if let Some(id) = m["id"].as_str() {
                    out.push(StreamEvent::MessageId(id.to_owned()));
                }
                let u = &m["usage"];
                self.usage.input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
                self.usage.cache_read_tokens = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.usage.cache_write_tokens =
                    u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                self.usage.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                let index = v["index"].as_u64().unwrap_or(0);
                let b = &v["content_block"];
                let open = match b["type"].as_str() {
                    Some("tool_use") => Open::Tool {
                        id: b["id"].as_str().unwrap_or_default().to_owned(),
                        name: b["name"].as_str().unwrap_or_default().to_owned(),
                        json: String::new(),
                    },
                    Some("thinking") => Open::Thinking {
                        text: b["thinking"].as_str().unwrap_or_default().to_owned(),
                        signature: None,
                    },
                    _ => {
                        if let Some(t) = b["text"].as_str().filter(|t| !t.is_empty()) {
                            out.push(StreamEvent::TextDelta(t.to_owned()));
                        }
                        Open::Text
                    }
                };
                self.open.insert(index, open);
            }
            "content_block_delta" => {
                let index = v["index"].as_u64().unwrap_or(0);
                let d = &v["delta"];
                match (d["type"].as_str(), self.open.get_mut(&index)) {
                    (Some("text_delta"), _) => {
                        if let Some(t) = d["text"].as_str().filter(|t| !t.is_empty()) {
                            out.push(StreamEvent::TextDelta(t.to_owned()));
                        }
                    }
                    (Some("input_json_delta"), Some(Open::Tool { json, .. })) => {
                        json.push_str(d["partial_json"].as_str().unwrap_or_default());
                    }
                    (Some("thinking_delta"), Some(Open::Thinking { text, .. })) => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        text.push_str(t);
                        if !t.is_empty() {
                            out.push(StreamEvent::ThinkingDelta(t.to_owned()));
                        }
                    }
                    (Some("signature_delta"), Some(Open::Thinking { signature, .. })) => {
                        signature
                            .get_or_insert_with(String::new)
                            .push_str(d["signature"].as_str().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                out.extend(self.close(v["index"].as_u64().unwrap_or(0)));
            }
            "message_delta" => {
                if let Some(n) = v["usage"]["output_tokens"].as_u64() {
                    self.usage.output_tokens = n;
                }
                if let Some(n) = v["usage"]["input_tokens"].as_u64() {
                    self.usage.input_tokens = n;
                }
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    out.push(StreamEvent::Usage(self.usage));
                    out.push(StreamEvent::Stop(stop_reason(r)));
                }
            }
            "error" => {
                return Err(StreamError {
                    kind: v["error"]["type"].as_str().unwrap_or("error").to_owned(),
                    message: v["error"]["message"]
                        .as_str()
                        .unwrap_or("Fehler im Strom")
                        .to_owned(),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        let open: Vec<u64> = self.open.keys().copied().collect();
        open.into_iter().filter_map(|i| self.close(i)).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::wire::{Message, ToolDef};

    #[test]
    fn har_011_anthropic_request_has_tools_results_and_caching() {
        let req = ModelRequest {
            model: "claude-x".into(),
            system: "sys".into(),
            messages: vec![
                Message::user_text("hallo"),
                Message {
                    role: Role::Assistant,
                    blocks: vec![Block::ToolUse {
                        id: "toolu_1".into(),
                        name: "fs_read".into(),
                        input: json!({"path": "a"}),
                    }],
                },
                Message {
                    role: Role::User,
                    blocks: vec![Block::ToolResult {
                        id: "toolu_1".into(),
                        content: "inhalt".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![ToolDef {
                name: "fs_read".into(),
                description: "lesen".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 1000,
            prompt_caching: true,
        };
        let body = request_body(&req);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn har_011_anthropic_stream_yields_text_tool_use_and_usage() {
        let mut p = Parser::default();
        let mut all = Vec::new();
        for (e, d) in [
            (
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":10,"output_tokens":1}}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hal"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"fs_read","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"a\"}"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ] {
            all.extend(p.on_event(Some(e), d).unwrap());
        }
        assert_eq!(
            all,
            vec![
                StreamEvent::MessageId("msg_1".into()),
                StreamEvent::TextDelta("Hal".into()),
                StreamEvent::TextDelta("lo".into()),
                StreamEvent::ToolUse {
                    id: "toolu_1".into(),
                    name: "fs_read".into(),
                    input: json!({"path": "a"})
                },
                StreamEvent::Usage(Usage {
                    input_tokens: 10,
                    output_tokens: 7,
                    ..Usage::default()
                }),
                StreamEvent::Stop(StopReason::ToolUse),
            ]
        );
        let err = p
            .on_event(
                Some("error"),
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            )
            .unwrap_err();
        assert_eq!(err.kind, "overloaded_error");
    }
}
