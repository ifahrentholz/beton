//! OpenAI Chat Completions: `POST {base_url}/chat/completions` mit `stream: true` und
//! `stream_options.include_usage` (OpenAI, OpenRouter, LiteLLM, vLLM, Ollama, LM Studio).
//!
//! Strom: `data: {"id", "choices": [{"delta": {"content", "reasoning_content"|"reasoning",
//! "tool_calls": [{"index", "id", "function": {"name", "arguments"}}]}, "finish_reason"}],
//! "usage"}` und zum Schluss `data: [DONE]`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Block, ModelRequest, Role, StopReason, StreamError, StreamEvent, StreamParser, Usage};

/// Request-Body.
pub fn request_body(req: &ModelRequest) -> Value {
    let mut messages = vec![json!({"role": "system", "content": req.system})];
    for m in &req.messages {
        match m.role {
            Role::User => {
                let mut texts = Vec::new();
                for b in &m.blocks {
                    match b {
                        Block::ToolResult { id, content, .. } => messages
                            .push(json!({"role": "tool", "tool_call_id": id, "content": content})),
                        Block::Text(t) => texts.push(t.clone()),
                        _ => {}
                    }
                }
                if !texts.is_empty() {
                    messages.push(json!({"role": "user", "content": texts.join("\n\n")}));
                }
            }
            Role::Assistant => {
                let text: String = m
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect();
                let calls: Vec<Value> = m
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::ToolUse { id, name, input } => Some(json!({
                            "id": id, "type": "function",
                            "function": {"name": name, "arguments": input.to_string()},
                        })),
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { Value::String(text) },
                });
                if !calls.is_empty() {
                    msg["tool_calls"] = Value::Array(calls);
                }
                messages.push(msg);
            }
        }
    }
    let mut body = json!({
        "model": req.model,
        "stream": true,
        "stream_options": {"include_usage": true},
        "messages": messages,
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}}))
            .collect();
    }
    body
}

#[derive(Debug, Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

/// Parser des OpenAI-Stroms.
#[derive(Debug, Default)]
pub struct Parser {
    id_sent: bool,
    calls: BTreeMap<u64, PendingCall>,
    stop: Option<StopReason>,
    done: bool,
}

impl Parser {
    fn flush_calls(&mut self) -> Vec<StreamEvent> {
        std::mem::take(&mut self.calls)
            .into_values()
            .map(|c| {
                let input = if c.arguments.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&c.arguments)
                        .unwrap_or_else(|_| json!({"_raw": c.arguments}))
                };
                let id = if c.id.is_empty() {
                    format!("call_{}", ulid::Ulid::generate())
                } else {
                    c.id
                };
                StreamEvent::ToolUse {
                    id,
                    name: c.name,
                    input,
                }
            })
            .collect()
    }
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_owned()),
    }
}

impl StreamParser for Parser {
    fn on_event(
        &mut self,
        _event: Option<&str>,
        data: &str,
    ) -> Result<Vec<StreamEvent>, StreamError> {
        if data.trim() == "[DONE]" {
            return Ok(self.finish());
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Ok(Vec::new());
        };
        if !v["error"].is_null() {
            return Err(StreamError {
                kind: v["error"]["type"]
                    .as_str()
                    .or_else(|| v["error"]["code"].as_str())
                    .unwrap_or("error")
                    .to_owned(),
                message: v["error"]["message"]
                    .as_str()
                    .unwrap_or("Fehler im Strom")
                    .to_owned(),
            });
        }
        let mut out = Vec::new();
        if !self.id_sent
            && let Some(id) = v["id"].as_str().filter(|s| !s.is_empty())
        {
            self.id_sent = true;
            out.push(StreamEvent::MessageId(id.to_owned()));
        }
        if let Some(choice) = v["choices"].get(0) {
            let d = &choice["delta"];
            for key in ["reasoning_content", "reasoning"] {
                if let Some(t) = d[key].as_str().filter(|t| !t.is_empty()) {
                    out.push(StreamEvent::ThinkingDelta(t.to_owned()));
                }
            }
            if let Some(t) = d["content"].as_str().filter(|t| !t.is_empty()) {
                out.push(StreamEvent::TextDelta(t.to_owned()));
            }
            for call in d["tool_calls"].as_array().into_iter().flatten() {
                let index = call["index"].as_u64().unwrap_or(0);
                let entry = self.calls.entry(index).or_default();
                if let Some(id) = call["id"].as_str() {
                    entry.id = id.to_owned();
                }
                if let Some(n) = call["function"]["name"].as_str() {
                    entry.name.push_str(n);
                }
                if let Some(a) = call["function"]["arguments"].as_str() {
                    entry.arguments.push_str(a);
                }
            }
            if let Some(r) = choice["finish_reason"].as_str() {
                out.extend(self.flush_calls());
                self.stop = Some(stop_reason(r));
            }
        }
        if v["usage"].is_object() {
            let u = &v["usage"];
            let prompt = u["prompt_tokens"].as_u64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0)
                .min(prompt);
            out.push(StreamEvent::Usage(Usage {
                input_tokens: prompt - cached,
                output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: cached,
                cache_write_tokens: 0,
            }));
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = self.flush_calls();
        if let Some(stop) = self.stop.take() {
            out.push(StreamEvent::Stop(stop));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::wire::{Message, ToolDef};

    #[test]
    fn har_011_openai_request_maps_tool_calls_and_results() {
        let req = ModelRequest {
            model: "qwen/qwen3-coder".into(),
            system: "sys".into(),
            messages: vec![
                Message::user_text("hallo"),
                Message {
                    role: Role::Assistant,
                    blocks: vec![
                        Block::Text("Ich lese.".into()),
                        Block::ToolUse {
                            id: "call_1".into(),
                            name: "fs_read".into(),
                            input: json!({"path": "a"}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    blocks: vec![Block::ToolResult {
                        id: "call_1".into(),
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
            prompt_caching: false,
        };
        let body = request_body(&req);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a\"}"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn har_011_openai_stream_yields_text_tool_calls_and_usage() {
        let mut p = Parser::default();
        let mut all = Vec::new();
        for d in [
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"role":"assistant","content":"Ich "}}]}"#,
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"lese."}}]}"#,
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"fs_read","arguments":"{\"pa"}}]}}]}"#,
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a\"}"}}]}}]}"#,
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":2}}}"#,
            "[DONE]",
        ] {
            all.extend(p.on_event(None, d).unwrap());
        }
        assert_eq!(
            all,
            vec![
                StreamEvent::MessageId("chatcmpl-1".into()),
                StreamEvent::TextDelta("Ich ".into()),
                StreamEvent::TextDelta("lese.".into()),
                StreamEvent::ToolUse {
                    id: "call_1".into(),
                    name: "fs_read".into(),
                    input: json!({"path": "a"})
                },
                StreamEvent::Usage(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: 2,
                    cache_write_tokens: 0
                }),
                StreamEvent::Stop(StopReason::ToolUse),
            ]
        );
    }
}
