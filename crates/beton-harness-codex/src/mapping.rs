//! Abbildung der Notifications von `codex app-server` auf das Event-Modell (HAR-006,
//! HAR-021). Reine Funktionen ohne IO, damit sie direkt testbar sind.
//!
//! Methoden und Felder laut `codex app-server generate-json-schema` (codex-cli 0.153.2).
//! Bewusst übergangene Notifications stehen in [`IGNORED`]; alles Unbekannte wird
//! `harness.unmapped` mit `raw` (nicht verworfen).

use std::collections::HashSet;

use beton_core::event::{
    AuthSource, ContextSource, ContextUsage, CostDelta, CostSource, EventPayload,
    HarnessAuthRequired, HarnessUnmapped, McpServerFailed, MessageCompleted, MessageRole,
    OutputStream, ReasoningCompleted, TextDelta, ToolCallCompleted, ToolCallOutputDelta,
    ToolCallRequested, ToolCallStarted, ToolSource, ToolStatus, TurnCompleted, TurnFailed,
    TurnInterrupted, UsageSubscription,
};
use beton_core::id::{PrincipalId, TurnId, UserId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};

/// Notifications, die beton nicht als Event braucht.
pub const IGNORED: [&str; 31] = [
    "thread/started",
    "thread/status/changed",
    "thread/name/updated",
    "thread/settings/updated",
    "thread/goal/updated",
    "thread/goal/cleared",
    "thread/queue/changed",
    "thread/compacted",
    "turn/started",
    "turn/diff/updated",
    "turn/plan/updated",
    "turn/moderationMetadata",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/fileChange/outputDelta",
    "item/fileChange/patchUpdated",
    "item/commandExecution/terminalInteraction",
    "item/mcpToolCall/progress",
    "item/autoApprovalReview/started",
    "item/autoApprovalReview/completed",
    "serverRequest/resolved",
    "skills/changed",
    "hook/started",
    "hook/completed",
    "remoteControl/status/changed",
    "account/updated",
    "app/list/updated",
    "configWarning",
    "warning",
    "deprecationNotice",
    "model/rerouted",
];

/// Token-Zähler eines Threads (`tokenUsage.total`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl Tokens {
    pub fn from_breakdown(v: &Value) -> Self {
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        Self {
            input: n("inputTokens"),
            cached: n("cachedInputTokens"),
            cache_write: n("cacheWriteInputTokens"),
            output: n("outputTokens"),
        }
    }

    fn minus(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_sub(other.input),
            cached: self.cached.saturating_sub(other.cached),
            cache_write: self.cache_write.saturating_sub(other.cache_write),
            output: self.output.saturating_sub(other.output),
        }
    }
}

/// Zustand, den die Abbildung über Nachrichten hinweg braucht.
#[derive(Debug, Clone, Default)]
pub struct MapState {
    /// Turn aus Sicht von beton.
    pub turn: Option<TurnId>,
    /// Turn-ID von Codex (für `turn/interrupt`).
    pub codex_turn: Option<String>,
    pub thread_id: Option<String>,
    pub model: String,
    pub auth_source: Option<AuthSource>,
    /// Tool-Calls, für die `tool.call.requested` schon kam.
    pub requested: HashSet<String>,
    /// Tool-Calls, für die `tool.call.started` schon kam.
    pub started: HashSet<String>,
    /// Tool-Calls, die beton über das Gate abgelehnt hat.
    pub denied: HashSet<String>,
    /// Thread-Summe zuletzt und zu Beginn des laufenden Turns.
    pub total: Tokens,
    pub turn_start_total: Tokens,
    /// Letzter Fehler des laufenden Turns aus der `error`-Notification.
    pub turn_error: Option<Value>,
}

pub fn unmapped(v: &Value) -> EventPayload {
    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() })
}

/// Kanonische Tool-Klasse (POL-005) eines Codex-Items.
pub fn item_kind(item_type: &str) -> &'static str {
    match item_type {
        "commandExecution" => "shell",
        "fileChange" => "file_edit",
        "mcpToolCall" => "mcp",
        "webSearch" => "web_fetch",
        "imageView" => "file_read",
        _ => "other",
    }
}

/// Item-Typen, die Tool-Calls sind.
fn is_tool(item_type: &str) -> bool {
    matches!(
        item_type,
        "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" | "webSearch"
    )
}

/// Tool-Name (`tool.native_name`), Server und Argumente eines Tool-Items.
pub fn tool_of(item: &Value) -> (String, Option<String>, Value) {
    let ty = item["type"].as_str().unwrap_or_default();
    match ty {
        "commandExecution" => (
            ty.into(),
            None,
            json!({"command": item["command"], "cwd": item["cwd"]}),
        ),
        "fileChange" => (ty.into(), None, json!({"changes": item["changes"]})),
        "mcpToolCall" => (
            item["tool"].as_str().unwrap_or_default().into(),
            item["server"].as_str().map(str::to_owned),
            item["arguments"].clone(),
        ),
        "dynamicToolCall" => (
            item["tool"].as_str().unwrap_or_default().into(),
            None,
            item["arguments"].clone(),
        ),
        "webSearch" => (ty.into(), None, json!({"query": item["query"]})),
        _ => (ty.into(), None, Value::Null),
    }
}

/// `tool.call.requested` für ein Item, höchstens einmal je Item.
pub fn requested(item: &Value, st: &mut MapState) -> Option<EventPayload> {
    let id = item["id"].as_str()?.to_owned();
    if !st.requested.insert(id.clone()) {
        return None;
    }
    let (tool, mcp_server, args) = tool_of(item);
    Some(EventPayload::ToolCallRequested(ToolCallRequested {
        call_id: id,
        tool,
        // System-Tools des Servers `beton` (AGT-007) als `source: beton_mcp`.
        source: if mcp_server.as_deref() == Some("beton") {
            ToolSource::BetonMcp
        } else {
            ToolSource::Harness
        },
        mcp_server,
        args,
        parent_call_id: None,
    }))
}

/// `tool.call.started`, höchstens einmal je Item und nie für abgelehnte.
pub fn started(id: &str, st: &mut MapState) -> Option<EventPayload> {
    if st.denied.contains(id) || !st.started.insert(id.to_owned()) {
        return None;
    }
    Some(EventPayload::ToolCallStarted(ToolCallStarted {
        call_id: id.to_owned(),
        sandbox_stage: None,
        args: None,
    }))
}

/// Eine Notification auf null oder mehr Events abbilden.
pub fn map_notification(method: &str, params: &Value, st: &mut MapState) -> Vec<EventPayload> {
    match method {
        "item/started" => item_started(&params["item"], st),
        "item/completed" => item_completed(&params["item"], st),
        "item/agentMessage/delta" => text_delta(params, false).into_iter().collect(),
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            text_delta(params, true).into_iter().collect()
        }
        "item/commandExecution/outputDelta" => {
            let id = params["itemId"].as_str().unwrap_or_default();
            let mut out: Vec<EventPayload> = started(id, st).into_iter().collect();
            out.push(EventPayload::ToolCallOutputDelta(ToolCallOutputDelta {
                call_id: id.to_owned(),
                stream: OutputStream::Stdout,
                text: params["delta"].as_str().unwrap_or_default().to_owned(),
                snapshot: false,
            }));
            out
        }
        "thread/tokenUsage/updated" => token_usage(&params["tokenUsage"], st),
        "turn/started" => {
            st.codex_turn = params["turn"]["id"].as_str().map(str::to_owned);
            Vec::new()
        }
        "turn/completed" => turn_completed(&params["turn"], st),
        "error" => {
            if params["willRetry"] != true {
                st.turn_error = Some(params["error"].clone());
            }
            Vec::new()
        }
        "account/rateLimits/updated" => rate_limits(&params["rateLimits"]),
        "mcpServer/startupStatus/updated" if params["status"] == "failed" => {
            vec![EventPayload::McpServerFailed(McpServerFailed {
                name: params["name"].as_str().unwrap_or_default().to_owned(),
                error: params["error"]
                    .as_str()
                    .unwrap_or("Start fehlgeschlagen")
                    .to_owned(),
            })]
        }
        "mcpServer/startupStatus/updated" => Vec::new(),
        m if IGNORED.contains(&m) => Vec::new(),
        _ => vec![unmapped(&json!({"method": method, "params": params}))],
    }
}

fn text_delta(params: &Value, reasoning: bool) -> Option<EventPayload> {
    let delta = TextDelta {
        message_id: params["itemId"].as_str()?.to_owned(),
        text: params["delta"].as_str()?.to_owned(),
        snapshot: false,
    };
    if delta.text.is_empty() {
        return None;
    }
    Some(if reasoning {
        EventPayload::ReasoningDelta(delta)
    } else {
        EventPayload::MessageDelta(delta)
    })
}

fn item_started(item: &Value, st: &mut MapState) -> Vec<EventPayload> {
    let ty = item["type"].as_str().unwrap_or_default();
    if is_tool(ty) {
        return requested(item, st).into_iter().collect();
    }
    Vec::new()
}

fn item_completed(item: &Value, st: &mut MapState) -> Vec<EventPayload> {
    let ty = item["type"].as_str().unwrap_or_default();
    let id = item["id"].as_str().unwrap_or_default().to_owned();
    match ty {
        "agentMessage" => {
            let text = item["text"].as_str().unwrap_or_default();
            if text.is_empty() {
                return Vec::new();
            }
            vec![EventPayload::MessageCompleted(MessageCompleted {
                message_id: id,
                role: MessageRole::Assistant,
                content: vec![json!({"type": "text", "text": text})],
                author: None,
            })]
        }
        "reasoning" => {
            let summary: Vec<&str> = item["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let content_empty = item["content"].as_array().is_none_or(Vec::is_empty);
            vec![EventPayload::ReasoningCompleted(ReasoningCompleted {
                message_id: id,
                summary: (!summary.is_empty()).then(|| summary.join("\n")),
                redacted: summary.is_empty() && content_empty,
            })]
        }
        t if is_tool(t) => {
            let mut out: Vec<EventPayload> = requested(item, st).into_iter().collect();
            let status = match (item["status"].as_str(), item["exitCode"].as_i64()) {
                _ if st.denied.contains(&id) => ToolStatus::Denied,
                (Some("declined"), _) => ToolStatus::Denied,
                (Some("failed"), _) => ToolStatus::Error,
                (Some("completed"), Some(code)) if code != 0 => ToolStatus::Error,
                (Some("completed"), _) => ToolStatus::Ok,
                _ => ToolStatus::Error,
            };
            if status != ToolStatus::Denied {
                out.extend(started(&id, st));
            }
            let result = match t {
                "commandExecution" => {
                    json!({"exit_code": item["exitCode"], "output": item["aggregatedOutput"]})
                }
                "fileChange" => json!({"changes": item["changes"]}),
                _ => {
                    if item["error"].is_null() {
                        item["result"].clone()
                    } else {
                        item["error"].clone()
                    }
                }
            };
            st.denied.remove(&id);
            out.push(EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id: id,
                status,
                result: (!result.is_null()).then_some(result),
                result_ref: None,
                duration_ms: item["durationMs"].as_u64().unwrap_or(0),
            }));
            out
        }
        // Eingaben loggt beton selbst; Compaction-Events folgen mit HAR-022.
        "userMessage" | "hookPrompt" | "plan" | "contextCompaction" => Vec::new(),
        _ => vec![unmapped(item)],
    }
}

fn token_usage(usage: &Value, st: &mut MapState) -> Vec<EventPayload> {
    st.total = Tokens::from_breakdown(&usage["total"]);
    let Some(window) = usage["modelContextWindow"].as_u64() else {
        return Vec::new();
    };
    vec![EventPayload::ContextUsage(ContextUsage {
        used_tokens: usage["last"]["totalTokens"].as_u64().unwrap_or(0),
        window_tokens: window,
        source: ContextSource::Harness,
    })]
}

/// `cost.delta` für den Turn: Differenz der Thread-Summe seit Turn-Beginn (HAR-006 AC3).
fn cost(st: &MapState) -> EventPayload {
    let t = st.total.minus(st.turn_start_total);
    let auth = st.auth_source.unwrap_or(AuthSource::VendorCli);
    EventPayload::CostDelta(CostDelta {
        harness: "codex".into(),
        model: st.model.clone(),
        input_tokens: t.input.saturating_sub(t.cached),
        output_tokens: t.output,
        cache_read_tokens: t.cached,
        cache_write_tokens: t.cache_write,
        // Kosten kommen aus dem Preis-Katalog (USE-002, ab M2); Codex meldet nur Tokens.
        cost_micro: None,
        currency: "USD".into(),
        source: if auth == AuthSource::VendorCli {
            CostSource::Subscription
        } else {
            CostSource::Estimated
        },
        auth_source: auth,
        purpose: None,
    })
}

fn turn_completed(turn: &Value, st: &mut MapState) -> Vec<EventPayload> {
    let turn_id = st.turn.take().unwrap_or_default();
    let mut out = vec![cost(st)];
    let usage = st.total.minus(st.turn_start_total);
    st.turn_start_total = st.total;
    st.codex_turn = None;
    let error = std::mem::take(&mut st.turn_error);
    match turn["status"].as_str() {
        Some("completed") => out.push(EventPayload::TurnCompleted(TurnCompleted {
            turn_id,
            stop_reason: "end_turn".into(),
            usage_summary: json!({
                "input_tokens": usage.input,
                "output_tokens": usage.output,
                "duration_ms": turn["durationMs"],
            }),
        })),
        Some("interrupted") => out.push(EventPayload::TurnInterrupted(TurnInterrupted {
            turn_id,
            by: PrincipalId::User(UserId::LOCAL),
            reason: None,
        })),
        _ => {
            let error = match &turn["error"] {
                Value::Null => error.unwrap_or(Value::Null),
                e => e.clone(),
            };
            // Abgelaufener Login: Hinweis auf die Vendor-CLI, kein eigener Login (HAR-015).
            if error["codexErrorInfo"] == "unauthorized" {
                out.push(EventPayload::HarnessAuthRequired(HarnessAuthRequired {
                    harness: "codex".into(),
                    hint: "codex login".into(),
                }));
            }
            out.push(EventPayload::TurnFailed(TurnFailed {
                turn_id,
                problem: json!({
                    "type": "urn:beton:problem:harness_error",
                    "title": error["message"].as_str().unwrap_or("Codex meldet einen Fehler"),
                    "codex_error_info": error["codexErrorInfo"],
                }),
            }));
        }
    }
    out
}

fn rate_limits(limits: &Value) -> Vec<EventPayload> {
    ["primary", "secondary"]
        .iter()
        .filter_map(|window| {
            let w = &limits[*window];
            let used = w["usedPercent"].as_f64()?;
            let resets = time::OffsetDateTime::from_unix_timestamp(w["resetsAt"].as_i64()?).ok()?;
            let name = w["windowDurationMins"]
                .as_u64()
                .map_or_else(|| (*window).to_owned(), |m| format!("{m}m"));
            Some(EventPayload::UsageSubscription(UsageSubscription {
                vendor: "openai".into(),
                window: name,
                used_pct: used,
                resets_at: Timestamp::from(resets),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(events: &[EventPayload]) -> Vec<&'static str> {
        events.iter().map(EventPayload::type_name).collect()
    }

    #[test]
    fn har_006_unknown_notifications_become_unmapped() {
        let mut st = MapState::default();
        assert_eq!(
            types(&map_notification("neu/in/0.200", &json!({}), &mut st)),
            ["harness.unmapped"]
        );
        assert!(map_notification("remoteControl/status/changed", &json!({}), &mut st).is_empty());
    }

    #[test]
    fn har_006_ac3_token_usage_and_turn_cost() {
        let mut st = MapState {
            turn: Some(TurnId::new()),
            model: "gpt-5-codex".into(),
            ..MapState::default()
        };
        let usage = json!({
            "total": {"inputTokens": 1300, "cachedInputTokens": 300, "outputTokens": 80, "reasoningOutputTokens": 0, "totalTokens": 1380},
            "last": {"inputTokens": 1300, "cachedInputTokens": 300, "outputTokens": 80, "reasoningOutputTokens": 0, "totalTokens": 1380},
            "modelContextWindow": 272000
        });
        let out = map_notification(
            "thread/tokenUsage/updated",
            &json!({"tokenUsage": usage}),
            &mut st,
        );
        let [EventPayload::ContextUsage(c)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!((c.used_tokens, c.window_tokens), (1380, 272000));
        let out = map_notification(
            "turn/completed",
            &json!({"turn": {"id": "t1", "status": "completed", "items": []}}),
            &mut st,
        );
        assert_eq!(types(&out), ["cost.delta", "turn.completed"]);
        let EventPayload::CostDelta(cost) = &out[0] else {
            panic!()
        };
        assert_eq!(
            (
                cost.input_tokens,
                cost.cache_read_tokens,
                cost.output_tokens
            ),
            (1000, 300, 80)
        );
        assert_eq!(cost.source, CostSource::Subscription);
        assert_eq!(cost.cost_micro, None);
        // Der nächste Turn zählt nur seinen eigenen Verbrauch.
        st.turn = Some(TurnId::new());
        let out = map_notification(
            "turn/completed",
            &json!({"turn": {"id": "t2", "status": "completed", "items": []}}),
            &mut st,
        );
        let EventPayload::CostDelta(cost) = &out[0] else {
            panic!()
        };
        assert_eq!((cost.input_tokens, cost.output_tokens), (0, 0));
    }

    #[test]
    fn har_015_unauthorized_turn_asks_for_codex_login() {
        let mut st = MapState {
            turn: Some(TurnId::new()),
            ..MapState::default()
        };
        map_notification(
            "error",
            &json!({"error": {"message": "401", "codexErrorInfo": "unauthorized"}, "willRetry": false}),
            &mut st,
        );
        let out = map_notification(
            "turn/completed",
            &json!({"turn": {"id": "t", "status": "failed", "error": null, "items": []}}),
            &mut st,
        );
        assert_eq!(
            types(&out),
            ["cost.delta", "harness.auth_required", "turn.failed"]
        );
    }

    #[test]
    fn declined_command_is_denied_and_never_started() {
        let mut st = MapState::default();
        let item = json!({"type": "commandExecution", "id": "c1", "command": "git push", "cwd": "/w", "status": "inProgress", "commandActions": []});
        assert_eq!(
            types(&map_notification(
                "item/started",
                &json!({"item": item}),
                &mut st
            )),
            ["tool.call.requested"]
        );
        let mut done = item.clone();
        done["status"] = json!("declined");
        let out = map_notification("item/completed", &json!({"item": done}), &mut st);
        assert_eq!(types(&out), ["tool.call.completed"]);
        let EventPayload::ToolCallCompleted(c) = &out[0] else {
            panic!()
        };
        assert_eq!(c.status, ToolStatus::Denied);
    }

    #[test]
    fn rate_limits_map_to_usage_subscription() {
        let out = map_notification(
            "account/rateLimits/updated",
            &json!({"rateLimits": {"primary": {"usedPercent": 42, "windowDurationMins": 300, "resetsAt": 1791051000}, "secondary": null}}),
            &mut MapState::default(),
        );
        let [EventPayload::UsageSubscription(u)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!((u.window.as_str(), u.used_pct), ("300m", 42.0));
    }
}
