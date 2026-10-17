//! Abbildung von ACP-`session/update` auf das Event-Modell (HAR-007). Reine Funktionen ohne
//! IO.
//!
//! `agent_message_chunk` → `message.delta` (eine Nachricht bis zum nächsten Tool-Call bzw.
//! Turn-Ende, dann `message.completed`), `agent_thought_chunk` → `reasoning.delta` und
//! `reasoning.completed`, `tool_call`/`tool_call_update` → `tool.call.*`. ACP-Nachrichten
//! tragen keine IDs; beton vergibt sie je Turn deterministisch (`msg_<turn>_<n>`). `plan`
//! wird zu `plan.updated` (PROTO-002).

use std::collections::HashMap;

use beton_core::event::{
    EventPayload, HarnessUnmapped, MessageCompleted, MessageRole, PlanEntry, PlanPriority,
    PlanStatus, PlanUpdated, ReasoningCompleted, TextDelta, ToolCallCompleted, ToolCallRequested,
    ToolCallStarted, ToolSource, ToolStatus,
};
use serde_json::{Value, json};

/// Updates, die beton nicht als Event braucht (u. a. die Wiedergabe bei `session/load`).
pub const IGNORED: [&str; 4] = [
    "user_message_chunk",
    "available_commands_update",
    "current_mode_update",
    "config_option_update",
];

/// Kanonische Tool-Klasse (POL-005) eines ACP-`kind` aus der Abbildungstabelle von
/// `beton-policy` (#182; `delete`/`move` → `file_edit`).
pub fn tool_kind(acp_kind: Option<&str>) -> String {
    beton_harness::tool_kind("acp", "", acp_kind, None)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallState {
    pub started: bool,
    pub denied: bool,
    pub completed: bool,
}

/// Zustand eines Turns.
#[derive(Debug, Clone, Default)]
pub struct MapState {
    /// Laufende Nummer des Turns (für IDs).
    pub turn_no: u32,
    /// Nummer der nächsten Nachricht bzw. Überlegung im Turn.
    pub parts: u32,
    pub message: Option<(String, String)>,
    pub thought: Option<(String, String)>,
    pub calls: HashMap<String, CallState>,
}

pub fn unmapped(v: &Value) -> EventPayload {
    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() })
}

impl MapState {
    /// Beginnt einen neuen Turn.
    pub fn begin_turn(&mut self) {
        self.turn_no += 1;
        self.parts = 0;
        self.message = None;
        self.thought = None;
        self.calls.clear();
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.parts += 1;
        format!("{prefix}_{}_{}", self.turn_no, self.parts)
    }

    /// Schließt offene Nachrichten und Überlegungen ab.
    pub fn flush(&mut self) -> Vec<EventPayload> {
        let mut out = Vec::new();
        if let Some((id, text)) = self.thought.take() {
            out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                message_id: id,
                summary: (!text.is_empty()).then_some(text),
                redacted: false,
            }));
        }
        if let Some((id, text)) = self.message.take() {
            out.push(EventPayload::MessageCompleted(MessageCompleted {
                message_id: id,
                role: MessageRole::Assistant,
                content: vec![json!({"type": "text", "text": text})],
                author: None,
            }));
        }
        out
    }

    /// `tool.call.requested` für einen Tool-Call, höchstens einmal.
    pub fn requested(&mut self, tool: &Value) -> Option<EventPayload> {
        let id = tool["toolCallId"].as_str()?.to_owned();
        if self.calls.contains_key(&id) {
            return None;
        }
        self.calls.insert(id.clone(), CallState::default());
        Some(EventPayload::ToolCallRequested(ToolCallRequested {
            call_id: id,
            tool: tool["title"].as_str().unwrap_or("tool").to_owned(),
            kind: Some(tool_kind(tool["kind"].as_str())),
            mcp_server: None,
            args: match &tool["rawInput"] {
                Value::Null => json!({}),
                v => v.clone(),
            },
            source: ToolSource::Acp,
            parent_call_id: None,
        }))
    }

    /// `tool.call.started`, höchstens einmal und nie für abgelehnte Calls.
    pub fn started(&mut self, id: &str) -> Option<EventPayload> {
        let call = self.calls.entry(id.to_owned()).or_default();
        if call.started || call.denied {
            return None;
        }
        call.started = true;
        Some(EventPayload::ToolCallStarted(ToolCallStarted {
            call_id: id.to_owned(),
            sandbox_stage: None,
            args: None,
        }))
    }

    pub fn deny(&mut self, id: &str) {
        self.calls.entry(id.to_owned()).or_default().denied = true;
    }
}

fn text_of(update: &Value) -> Option<&str> {
    (update["content"]["type"] == "text")
        .then(|| update["content"]["text"].as_str())
        .flatten()
}

/// ACP-`plan` (immer der vollständige Plan) als `plan.updated`; `None` bei unlesbaren Einträgen.
fn plan_of(update: &Value) -> Option<PlanUpdated> {
    let entries = update["entries"]
        .as_array()?
        .iter()
        .map(|e| {
            Some(PlanEntry {
                content: e["content"].as_str()?.to_owned(),
                priority: e["priority"].as_str().and_then(PlanPriority::from_vendor),
                status: PlanStatus::from_vendor(e["status"].as_str()?)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(PlanUpdated {
        entries,
        explanation: None,
    })
}

/// Ein `session/update` auf null oder mehr Events abbilden.
pub fn map_update(update: &Value, st: &mut MapState) -> Vec<EventPayload> {
    let kind = update["sessionUpdate"].as_str().unwrap_or_default();
    match kind {
        "agent_message_chunk" => {
            let Some(text) = text_of(update) else {
                return vec![unmapped(update)];
            };
            let mut out = Vec::new();
            if let Some((id, t)) = st.thought.take() {
                out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id,
                    summary: (!t.is_empty()).then_some(t),
                    redacted: false,
                }));
            }
            if st.message.is_none() {
                let id = st.next_id("msg");
                st.message = Some((id, String::new()));
            }
            if let Some((id, buf)) = st.message.as_mut() {
                buf.push_str(text);
                out.push(EventPayload::MessageDelta(TextDelta {
                    message_id: id.clone(),
                    text: text.to_owned(),
                    snapshot: false,
                }));
            }
            out
        }
        "agent_thought_chunk" => {
            let Some(text) = text_of(update) else {
                return vec![unmapped(update)];
            };
            let mut out = Vec::new();
            if let Some((id, t)) = st.message.take() {
                out.push(EventPayload::MessageCompleted(MessageCompleted {
                    message_id: id,
                    role: MessageRole::Assistant,
                    content: vec![json!({"type": "text", "text": t})],
                    author: None,
                }));
            }
            if st.thought.is_none() {
                let id = st.next_id("rsn");
                st.thought = Some((id, String::new()));
            }
            if let Some((id, buf)) = st.thought.as_mut() {
                buf.push_str(text);
                out.push(EventPayload::ReasoningDelta(TextDelta {
                    message_id: id.clone(),
                    text: text.to_owned(),
                    snapshot: false,
                }));
            }
            out
        }
        "tool_call" | "tool_call_update" => {
            let mut out = st.flush();
            let Some(id) = update["toolCallId"].as_str().map(str::to_owned) else {
                return vec![unmapped(update)];
            };
            out.extend(st.requested(update));
            match update["status"].as_str() {
                Some("in_progress") => out.extend(st.started(&id)),
                Some(status @ ("completed" | "failed")) => {
                    let call = st.calls.entry(id.clone()).or_default().clone();
                    if call.completed {
                        return out;
                    }
                    let status = if call.denied {
                        ToolStatus::Denied
                    } else if status == "completed" {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Error
                    };
                    if status != ToolStatus::Denied {
                        out.extend(st.started(&id));
                    }
                    if let Some(c) = st.calls.get_mut(&id) {
                        c.completed = true;
                    }
                    let result = if update["rawOutput"].is_null() {
                        update["content"].clone()
                    } else {
                        update["rawOutput"].clone()
                    };
                    out.push(EventPayload::ToolCallCompleted(ToolCallCompleted {
                        call_id: id,
                        status,
                        result: (!result.is_null()).then_some(result),
                        result_ref: None,
                        duration_ms: 0,
                    }));
                }
                _ => {}
            }
            out
        }
        "plan" => match plan_of(update) {
            Some(plan) => vec![EventPayload::PlanUpdated(plan)],
            None => vec![unmapped(update)],
        },
        k if IGNORED.contains(&k) => Vec::new(),
        _ => vec![unmapped(update)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(events: &[EventPayload]) -> Vec<&'static str> {
        events.iter().map(EventPayload::type_name).collect()
    }

    #[test]
    fn har_007_chunks_form_one_message_until_a_tool_call() {
        let mut st = MapState::default();
        st.begin_turn();
        let chunk = |t: &str| json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": t}});
        assert_eq!(
            types(&map_update(&chunk("Hal"), &mut st)),
            ["message.delta"]
        );
        assert_eq!(types(&map_update(&chunk("lo"), &mut st)), ["message.delta"]);
        let out = map_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Bash", "kind": "execute", "status": "pending", "rawInput": {"command": "ls"}}),
            &mut st,
        );
        assert_eq!(types(&out), ["message.completed", "tool.call.requested"]);
        let EventPayload::MessageCompleted(m) = &out[0] else {
            panic!()
        };
        assert_eq!(m.message_id, "msg_1_1");
        assert_eq!(m.content[0]["text"], "Hallo");
        let out = map_update(
            &json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed", "rawOutput": "ok"}),
            &mut st,
        );
        assert_eq!(types(&out), ["tool.call.started", "tool.call.completed"]);
    }

    #[test]
    fn har_007_denied_call_is_never_started() {
        let mut st = MapState::default();
        st.begin_turn();
        map_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Bash", "status": "pending"}),
            &mut st,
        );
        st.deny("c1");
        let out = map_update(
            &json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed"}),
            &mut st,
        );
        assert_eq!(types(&out), ["tool.call.completed"]);
        let EventPayload::ToolCallCompleted(c) = &out[0] else {
            panic!()
        };
        assert_eq!(c.status, ToolStatus::Denied);
    }

    #[test]
    fn har_007_acp_kinds_map_to_tool_kinds() {
        assert_eq!(tool_kind(Some("execute")), "shell");
        assert_eq!(tool_kind(Some("edit")), "file_edit");
        assert_eq!(tool_kind(Some("read")), "file_read");
        assert_eq!(tool_kind(Some("fetch")), "web_fetch");
        assert_eq!(tool_kind(Some("think")), "other");
        assert_eq!(tool_kind(None), "other");
    }

    #[test]
    fn proto_002_plan_becomes_plan_updated() {
        let mut st = MapState::default();
        let out = map_update(
            &json!({"sessionUpdate": "plan", "entries": [
                {"content": "Tests schreiben", "priority": "high", "status": "in_progress"},
                {"content": "Pushen", "priority": "low", "status": "pending"}
            ]}),
            &mut st,
        );
        let [EventPayload::PlanUpdated(plan)] = out.as_slice() else {
            panic!("plan.updated erwartet: {out:?}");
        };
        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.entries[0].content, "Tests schreiben");
        assert_eq!(plan.entries[0].priority, Some(PlanPriority::High));
        assert_eq!(plan.entries[0].status, PlanStatus::InProgress);
        assert_eq!(plan.entries[1].status, PlanStatus::Pending);
        // Unlesbar bleibt es `harness.unmapped` statt still zu verschwinden.
        assert_eq!(
            types(&map_update(
                &json!({"sessionUpdate": "plan", "entries": [{"status": "weird"}]}),
                &mut st
            )),
            ["harness.unmapped"]
        );
    }

    #[test]
    fn unknown_updates_become_unmapped() {
        let mut st = MapState::default();
        assert_eq!(
            types(&map_update(
                &json!({"sessionUpdate": "made_up_update"}),
                &mut st
            )),
            ["harness.unmapped"]
        );
        assert!(
            map_update(
                &json!({"sessionUpdate": "available_commands_update"}),
                &mut st
            )
            .is_empty()
        );
    }
}
