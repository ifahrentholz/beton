//! Abbildung der stream-json-Nachrichten von Claude Code auf das Event-Modell (HAR-004,
//! HAR-021). Reine Funktionen ohne IO, damit sie direkt testbar sind.
//!
//! Gegen `claude` 2.1.285 verifiziert. Nachrichten, die beton bewusst nicht abbildet
//! (Hooks, Status, Befehlslisten …), werden still übergangen; alles Unbekannte wird
//! `harness.unmapped` (AC4).

use beton_core::event::{
    AuthSource, ContextSource, ContextUsage, CostDelta, CostSource, EventPayload, HarnessReady,
    HarnessUnmapped, MessageCompleted, MessageRole, ReasoningCompleted, TextDelta,
    ToolCallCompleted, ToolCallRequested, ToolSource, ToolStatus, TurnCompleted, TurnFailed,
    TurnInterrupted, UsageSubscription,
};
use beton_core::id::{PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};

/// Zustand, den die Abbildung über Zeilen hinweg braucht.
#[derive(Debug, Clone, Default)]
pub struct MapState {
    pub turn: Option<TurnId>,
    pub interrupt_requested: bool,
    pub auth_source: Option<AuthSource>,
    /// ID der Nachricht, deren Deltas gerade gestreamt werden.
    pub message_id: Option<String>,
    pub session_ref: Option<String>,
    /// Tool-Calls, die beton über die Permission-Bridge abgelehnt hat.
    pub denied: std::collections::HashSet<String>,
}

/// `system`-Untertypen, die beton nicht als Event braucht.
const IGNORED_SYSTEM: [&str; 9] = [
    "status",
    "hook_started",
    "hook_response",
    "commands_changed",
    "thinking_tokens",
    "post_turn_summary",
    "compact_boundary",
    "api_retry",
    "task_summary",
];

/// Eine Zeile auf null oder mehr Events abbilden. `control_request` und
/// `control_response` behandelt die Session selbst.
pub fn map_line(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    match v["type"].as_str() {
        Some("system") => map_system(v, st),
        Some("stream_event") => map_stream_event(&v["event"], st).into_iter().collect(),
        Some("assistant") => map_assistant(v),
        Some("user") => map_tool_results(v, st),
        Some("result") => map_result(v, st),
        Some("rate_limit_event") => map_rate_limit(v).into_iter().collect(),
        _ => vec![unmapped(v)],
    }
}

pub fn unmapped(v: &Value) -> EventPayload {
    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() })
}

fn map_system(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    match v["subtype"].as_str() {
        Some("init") => {
            st.session_ref = v["session_id"].as_str().map(str::to_owned);
            let names = |key: &str| -> Vec<String> {
                v[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().or_else(|| t["name"].as_str()))
                    .map(str::to_owned)
                    .collect()
            };
            vec![EventPayload::HarnessReady(HarnessReady {
                harness_session_ref: st.session_ref.clone(),
                tools: names("tools"),
                mcp_servers: names("mcp_servers"),
            })]
        }
        Some(sub) if IGNORED_SYSTEM.contains(&sub) => Vec::new(),
        _ => vec![unmapped(v)],
    }
}

fn map_stream_event(e: &Value, st: &mut MapState) -> Option<EventPayload> {
    match e["type"].as_str()? {
        "message_start" => {
            st.message_id = e["message"]["id"].as_str().map(str::to_owned);
            None
        }
        "content_block_delta" => {
            let id = st.message_id.clone().unwrap_or_default();
            let d = &e["delta"];
            match d["type"].as_str()? {
                "text_delta" => Some(EventPayload::MessageDelta(TextDelta {
                    message_id: id,
                    text: d["text"].as_str()?.to_owned(),
                    snapshot: false,
                })),
                "thinking_delta" => {
                    let text = d["thinking"].as_str()?;
                    (!text.is_empty()).then(|| {
                        EventPayload::ReasoningDelta(TextDelta {
                            message_id: id,
                            text: text.to_owned(),
                            snapshot: false,
                        })
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn map_assistant(v: &Value) -> Vec<EventPayload> {
    let msg = &v["message"];
    let id = msg["id"].as_str().unwrap_or_default().to_owned();
    let mut out = Vec::new();
    let mut texts = Vec::new();
    for block in msg["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => texts.push(json!({"type": "text", "text": block["text"]})),
            Some("thinking") => {
                let text = block["thinking"].as_str().unwrap_or_default();
                out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id.clone(),
                    summary: (!text.is_empty()).then(|| text.to_owned()),
                    redacted: text.is_empty(),
                }));
            }
            Some("redacted_thinking") => {
                out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id.clone(),
                    summary: None,
                    redacted: true,
                }));
            }
            Some("tool_use") => out.push(EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: block["id"].as_str().unwrap_or_default().to_owned(),
                tool: block["name"].as_str().unwrap_or_default().to_owned(),
                mcp_server: None,
                args: block["input"].clone(),
                source: ToolSource::Harness,
            })),
            _ => out.push(unmapped(block)),
        }
    }
    if !texts.is_empty() {
        out.push(EventPayload::MessageCompleted(MessageCompleted {
            message_id: id,
            role: MessageRole::Assistant,
            content: texts,
            author: None,
        }));
    }
    out
}

fn map_tool_results(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    v["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "tool_result")
        .map(|b| {
            let call_id = b["tool_use_id"].as_str().unwrap_or_default().to_owned();
            let status = if st.denied.remove(&call_id) {
                ToolStatus::Denied
            } else if b["is_error"] == true {
                ToolStatus::Error
            } else {
                ToolStatus::Ok
            };
            EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id,
                status,
                result: Some(b["content"].clone()),
                result_ref: None,
                duration_ms: 0,
            })
        })
        .collect()
}

fn map_result(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    let turn_id = st.turn.take().unwrap_or_default();
    let mut out = Vec::new();
    let usage = &v["usage"];
    let tok = |k: &str| usage[k].as_u64().unwrap_or(0);
    let model = v["modelUsage"]
        .as_object()
        .and_then(|m| m.keys().next().cloned())
        .unwrap_or_default();
    let auth = st.auth_source.unwrap_or(AuthSource::VendorCli);
    let subscription = auth == AuthSource::VendorCli;
    // Bei Subscription meldet die CLI nur ein API-Äquivalent; das ist keine Ausgabe (HAR-021 AC2).
    out.push(EventPayload::CostDelta(CostDelta {
        harness: "claude".into(),
        model: model.clone(),
        input_tokens: tok("input_tokens"),
        output_tokens: tok("output_tokens"),
        cache_read_tokens: tok("cache_read_input_tokens"),
        cache_write_tokens: tok("cache_creation_input_tokens"),
        cost_micro: if subscription {
            None
        } else {
            v["total_cost_usd"]
                .as_f64()
                .map(|usd| (usd * 1_000_000.0).round() as i64)
        },
        currency: "USD".into(),
        source: if subscription {
            CostSource::Subscription
        } else {
            CostSource::Reported
        },
        auth_source: auth,
        purpose: None,
    }));
    if let Some(window) = v["modelUsage"][&model]["contextWindow"].as_u64() {
        out.push(EventPayload::ContextUsage(ContextUsage {
            used_tokens: tok("input_tokens")
                + tok("cache_read_input_tokens")
                + tok("cache_creation_input_tokens"),
            window_tokens: window,
            source: ContextSource::Harness,
        }));
    }
    let stop_reason = v["stop_reason"]
        .as_str()
        .or(v["subtype"].as_str())
        .unwrap_or("end_turn")
        .to_owned();
    if std::mem::take(&mut st.interrupt_requested) {
        out.push(EventPayload::TurnInterrupted(TurnInterrupted {
            turn_id,
            by: PrincipalId::User(UserId::LOCAL),
        }));
    } else if v["is_error"] == true
        || v["subtype"]
            .as_str()
            .is_some_and(|s| s.starts_with("error"))
    {
        // Abgelaufener Login: Hinweis auf die Vendor-CLI, kein eigener Login (HAR-015 AC3).
        let text = v["result"].as_str().unwrap_or_default();
        if v["api_error_status"] == 401 || text.contains("authentication_error") {
            out.push(EventPayload::HarnessAuthRequired(
                beton_core::event::HarnessAuthRequired {
                    harness: "claude".into(),
                    hint: "claude auth login".into(),
                },
            ));
        }
        out.push(EventPayload::TurnFailed(TurnFailed {
            turn_id,
            problem: json!({
                "type": format!("urn:beton:problem:harness_{}", v["subtype"].as_str().unwrap_or("error")),
                "title": v["result"].as_str().unwrap_or("Claude Code meldet einen Fehler"),
                "terminal_reason": v["terminal_reason"],
            }),
        }));
    } else {
        out.push(EventPayload::TurnCompleted(TurnCompleted {
            turn_id,
            stop_reason,
            usage_summary: json!({
                "input_tokens": tok("input_tokens"),
                "output_tokens": tok("output_tokens"),
                "num_turns": v["num_turns"],
                "duration_ms": v["duration_ms"],
            }),
        }));
    }
    out
}

fn map_rate_limit(v: &Value) -> Option<EventPayload> {
    let info = &v["rate_limit_info"];
    let kind = info["rateLimitType"].as_str()?;
    let window = &info["unifiedWindows"][kind];
    let used = window["utilization"].as_f64()?;
    let resets = window["resetsAt"].as_i64().or(info["resetsAt"].as_i64())?;
    let resets_at = time::OffsetDateTime::from_unix_timestamp(resets).ok()?;
    Some(EventPayload::UsageSubscription(UsageSubscription {
        vendor: "anthropic".into(),
        window: kind.to_owned(),
        used_pct: (used * 100.0 * 10.0).round() / 10.0,
        resets_at: Timestamp::from(resets_at),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(events: &[EventPayload]) -> Vec<&'static str> {
        events.iter().map(EventPayload::type_name).collect()
    }

    #[test]
    fn har_004_ac4_unknown_messages_become_unmapped() {
        let mut st = MapState::default();
        let out = map_line(&json!({"type": "neu_in_2_2", "x": 1}), &mut st);
        assert_eq!(types(&out), ["harness.unmapped"]);
        let out = map_line(
            &json!({"type": "system", "subtype": "hook_started"}),
            &mut st,
        );
        assert!(out.is_empty(), "bekannte, irrelevante Nachrichten still");
        let out = map_line(&json!({"type": "system", "subtype": "unbekannt"}), &mut st);
        assert_eq!(types(&out), ["harness.unmapped"]);
    }

    #[test]
    fn har_004_stream_deltas_carry_the_message_id() {
        let mut st = MapState::default();
        map_line(
            &json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "msg_1"}}}),
            &mut st,
        );
        let out = map_line(
            &json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hal"}}}),
            &mut st,
        );
        assert!(
            matches!(&out[0], EventPayload::MessageDelta(d) if d.message_id == "msg_1" && d.text == "Hal")
        );
    }

    #[test]
    fn har_021_ac2_subscription_cost_has_no_amount() {
        let result = json!({
            "type": "result", "subtype": "success", "is_error": false, "stop_reason": "end_turn",
            "total_cost_usd": 0.0122, "num_turns": 1, "duration_ms": 2533,
            "usage": {"input_tokens": 10, "output_tokens": 82, "cache_read_input_tokens": 23762, "cache_creation_input_tokens": 4719},
            "modelUsage": {"claude-haiku-4-5-20251001": {"contextWindow": 200000}}
        });
        let mut st = MapState {
            turn: Some(TurnId::new()),
            ..MapState::default()
        };
        let out = map_line(&result, &mut st);
        assert_eq!(
            types(&out),
            ["cost.delta", "context.usage", "turn.completed"]
        );
        let EventPayload::CostDelta(c) = &out[0] else {
            panic!()
        };
        assert_eq!(c.cost_micro, None);
        assert_eq!(c.source, CostSource::Subscription);
        assert_eq!(c.cache_read_tokens, 23762);
        assert_eq!(c.model, "claude-haiku-4-5-20251001");
        let EventPayload::ContextUsage(u) = &out[1] else {
            panic!()
        };
        assert_eq!((u.used_tokens, u.window_tokens), (28491, 200000));

        let mut st = MapState {
            auth_source: Some(AuthSource::ApiKey),
            ..MapState::default()
        };
        let out = map_line(&result, &mut st);
        let EventPayload::CostDelta(c) = &out[0] else {
            panic!()
        };
        assert_eq!(
            (c.cost_micro, c.source),
            (Some(12200), CostSource::Reported)
        );
    }

    #[test]
    fn har_021_rate_limit_event_maps_to_usage_subscription() {
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1791051000, "rateLimitType": "five_hour",
            "unifiedWindows": {"five_hour": {"utilization": 0.19, "resetsAt": 1791051000}}}});
        let out = map_line(&v, &mut MapState::default());
        let EventPayload::UsageSubscription(u) = &out[0] else {
            panic!()
        };
        assert_eq!((u.window.as_str(), u.used_pct), ("five_hour", 19.0));
    }

    #[test]
    fn error_result_fails_the_turn() {
        let v =
            json!({"type": "result", "subtype": "error_max_turns", "is_error": true, "usage": {}});
        let out = map_line(
            &v,
            &mut MapState {
                turn: Some(TurnId::new()),
                ..MapState::default()
            },
        );
        assert_eq!(types(&out), ["cost.delta", "turn.failed"]);
    }
}
