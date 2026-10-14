//! Abbildung der stream-json-Nachrichten von Claude Code auf das Event-Modell (HAR-004,
//! HAR-021). Reine Funktionen ohne IO, damit sie direkt testbar sind.
//!
//! Gegen `claude` 2.1.285 verifiziert. Nachrichten, die beton bewusst nicht abbildet
//! (Hooks, Status, Befehlslisten …), werden still übergangen; alles Unbekannte wird
//! `harness.unmapped` (AC4).

use beton_core::event::{
    AuthSource, Compaction, ContextSource, ContextUsage, CostDelta, CostSource, EventPayload,
    HarnessReady, HarnessUnmapped, MessageCompleted, MessageRole, ReasoningCompleted, TextDelta,
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
    /// `/compact` läuft (HAR-022): Das nächste `result` beendet die Compaction, keinen Turn.
    pub compacting: bool,
    /// `pre_tokens` bzw. `post_tokens` aus `compact_boundary`.
    pub compact_before: Option<u64>,
    pub compact_after: Option<u64>,
    /// Zuletzt gemeldeter Kontext (`context.usage`).
    pub last_context: Option<u64>,
    pub context_window: Option<u64>,
    /// Modell der Session laut `system/init`; bei mehreren Modellen in `modelUsage` (etwa
    /// Sub-Agents mit eigenem Modell) gehört der Kontext-Füllstand zu diesem.
    pub model: Option<String>,
    /// Welche Permission-Modes die CLI melden darf (HAR-027).
    pub expected_mode: crate::permission::ModeGuard,
}

/// `system`-Untertypen, die beton nicht als Event braucht.
const IGNORED_SYSTEM: [&str; 8] = [
    "status",
    "hook_started",
    "hook_response",
    "commands_changed",
    "thinking_tokens",
    "post_turn_summary",
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
        Some("rate_limit_event") => map_rate_limit(v),
        _ => vec![unmapped(v)],
    }
}

pub fn unmapped(v: &Value) -> EventPayload {
    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() })
}

/// `mcp__<server>__<tool>` → (`tool`, `Some(server)`); andere Namen unverändert (HAR-009).
pub fn split_mcp_name(name: &str) -> (String, Option<String>) {
    name.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .filter(|(server, tool)| !server.is_empty() && !tool.is_empty())
        .map_or_else(
            || (name.to_owned(), None),
            |(server, tool)| (tool.to_owned(), Some(server.to_owned())),
        )
}

fn map_system(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    match v["subtype"].as_str() {
        Some("init") => {
            st.session_ref = v["session_id"].as_str().map(str::to_owned);
            if let Some(m) = v["model"].as_str() {
                st.model = Some(m.to_owned());
            }
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
        // Compaction (HAR-022): `/compact` von beton oder automatisch durch die CLI.
        Some("compact_boundary") => {
            let meta = &v["compact_metadata"];
            let pre = meta["pre_tokens"].as_u64();
            let post = meta["post_tokens"].as_u64();
            if st.compacting {
                st.compact_before = pre.or(st.compact_before);
                st.compact_after = post;
                return Vec::new();
            }
            let before = pre.or(st.last_context).unwrap_or(0);
            vec![
                EventPayload::CompactionStarted(Compaction {
                    before_tokens: before,
                    after_tokens: None,
                }),
                EventPayload::CompactionCompleted(Compaction {
                    before_tokens: before,
                    after_tokens: post,
                }),
            ]
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
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or_default();
                let (tool, mcp_server) = split_mcp_name(name);
                out.push(EventPayload::ToolCallRequested(ToolCallRequested {
                    call_id: block["id"].as_str().unwrap_or_default().to_owned(),
                    kind: Some(crate::kind_of(name)),
                    // System-Tools des Servers `beton` (AGT-007) als `source: beton_mcp`.
                    source: if mcp_server.as_deref() == Some("beton") {
                        ToolSource::BetonMcp
                    } else {
                        ToolSource::Harness
                    },
                    tool,
                    mcp_server,
                    args: block["input"].clone(),
                    parent_call_id: None,
                }));
            }
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

/// `cost.delta` aus dem `result` eines Einmal-Aufrufs (SES-010); `purpose` setzt der Aufrufer.
/// Bei mehreren Modellen zählt die Summe (ein Einmal-Aufruf hat ein Modell).
pub fn result_cost(v: &Value, auth: AuthSource) -> CostDelta {
    let st = MapState {
        auth_source: Some(auth),
        ..MapState::default()
    };
    let mut deltas = cost_deltas(v, &st, None)
        .into_iter()
        .filter_map(|e| match e {
            EventPayload::CostDelta(c) => Some(c),
            _ => None,
        });
    let mut first = deltas.next().unwrap_or_default();
    for c in deltas {
        first.input_tokens += c.input_tokens;
        first.output_tokens += c.output_tokens;
        first.cache_read_tokens += c.cache_read_tokens;
        first.cache_write_tokens += c.cache_write_tokens;
        first.cost_micro = match (first.cost_micro, c.cost_micro) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
    }
    first
}

/// Modell laut `modelUsage` eines `result`.
pub fn result_model(v: &Value) -> Option<String> {
    v["modelUsage"]
        .as_object()
        .and_then(|m| m.keys().next().cloned())
}

/// Modell, zu dem der Kontext-Füllstand gehört: das der Session (`system/init`), sonst das
/// erste in `modelUsage`.
fn main_model(v: &Value, st: &MapState) -> String {
    let usage = v["modelUsage"].as_object();
    match (&st.model, usage) {
        (Some(m), Some(map)) if map.contains_key(m) => m.clone(),
        _ => usage
            .and_then(|m| m.keys().next().cloned())
            .unwrap_or_default(),
    }
}

/// `cost.delta` je Modell (USE-004 AC1). Mit einem Modell (Regelfall) stammen die Tokens aus
/// `usage` und die Kosten aus `total_cost_usd` – genau ein Event je Turn (HAR-021 AC1). Nennt
/// `modelUsage` mehrere Modelle (z. B. Sub-Agents mit eigenem Modell), entsteht je Modell ein
/// Event mit dessen Tokens und `costUSD`, damit Summen je Modell stimmen (gegen 2.1.296
/// verifiziert: bei einem Modell stimmen `usage` und `modelUsage` überein).
fn cost_deltas(v: &Value, st: &MapState, purpose: Option<&str>) -> Vec<EventPayload> {
    let auth = st.auth_source.unwrap_or(AuthSource::VendorCli);
    let subscription = auth == AuthSource::VendorCli;
    // Bei Subscription meldet die CLI nur ein API-Äquivalent; das ist keine Ausgabe (HAR-021 AC2).
    let delta = |model: String, tokens: [u64; 4], usd: Option<f64>| {
        EventPayload::CostDelta(CostDelta {
            harness: "claude".into(),
            model,
            input_tokens: tokens[0],
            output_tokens: tokens[1],
            cache_read_tokens: tokens[2],
            cache_write_tokens: tokens[3],
            cost_micro: if subscription {
                None
            } else {
                usd.map(|usd| (usd * 1_000_000.0).round() as i64)
            },
            currency: "USD".into(),
            source: if subscription {
                CostSource::Subscription
            } else {
                CostSource::Reported
            },
            auth_source: auth,
            purpose: purpose.map(str::to_owned),
            ..CostDelta::default()
        })
    };
    if let Some(models) = v["modelUsage"].as_object()
        && models.len() > 1
    {
        return models
            .iter()
            .map(|(model, u)| {
                let tok = |k: &str| u[k].as_u64().unwrap_or(0);
                delta(
                    model.clone(),
                    [
                        tok("inputTokens"),
                        tok("outputTokens"),
                        tok("cacheReadInputTokens"),
                        tok("cacheCreationInputTokens"),
                    ],
                    u["costUSD"].as_f64(),
                )
            })
            .collect();
    }
    let usage = &v["usage"];
    let tok = |k: &str| usage[k].as_u64().unwrap_or(0);
    vec![delta(
        main_model(v, st),
        [
            tok("input_tokens"),
            tok("output_tokens"),
            tok("cache_read_input_tokens"),
            tok("cache_creation_input_tokens"),
        ],
        v["total_cost_usd"].as_f64(),
    )]
}

/// `result` nach `/compact` (HAR-022): `compaction.completed` und der neue Kontext; kein
/// Turn-Ende. Ohne `post_tokens` gilt die Länge der Zusammenfassung als geschätzter Kontext.
fn map_compaction_result(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    st.compacting = false;
    let model = main_model(v, st);
    if let Some(window) = v["modelUsage"][&model]["contextWindow"].as_u64() {
        st.context_window = Some(window);
    }
    let before = st.compact_before.take().or(st.last_context).unwrap_or(0);
    let reported = st.compact_after.take();
    let mut out = cost_deltas(v, st, Some("compaction"));
    if v["is_error"] == true
        || v["subtype"]
            .as_str()
            .is_some_and(|s| s.starts_with("error"))
    {
        out.push(EventPayload::Notice(beton_core::event::Notice {
            level: beton_core::event::NoticeLevel::Warn,
            text: format!(
                "Compaction fehlgeschlagen: {}",
                v["result"]
                    .as_str()
                    .unwrap_or("Claude Code meldet einen Fehler")
            ),
        }));
        out.push(EventPayload::CompactionCompleted(Compaction {
            before_tokens: before,
            after_tokens: None,
        }));
        return out;
    }
    let estimated = v["usage"]["output_tokens"].as_u64().filter(|n| *n > 0);
    let after = reported.or(estimated);
    out.push(EventPayload::CompactionCompleted(Compaction {
        before_tokens: before,
        after_tokens: after,
    }));
    if let (Some(used), Some(window)) = (after, st.context_window) {
        st.last_context = Some(used);
        out.push(EventPayload::ContextUsage(ContextUsage {
            used_tokens: used,
            window_tokens: window,
            source: if reported.is_some() {
                ContextSource::Harness
            } else {
                ContextSource::Estimated
            },
        }));
    }
    out
}

fn map_result(v: &Value, st: &mut MapState) -> Vec<EventPayload> {
    if st.compacting {
        return map_compaction_result(v, st);
    }
    let turn_id = st.turn.take().unwrap_or_default();
    let usage = &v["usage"];
    let tok = |k: &str| usage[k].as_u64().unwrap_or(0);
    let model = main_model(v, st);
    let mut out = cost_deltas(v, st, None);
    if let Some(window) = v["modelUsage"][&model]["contextWindow"].as_u64() {
        let used = tok("input_tokens")
            + tok("cache_read_input_tokens")
            + tok("cache_creation_input_tokens");
        st.context_window = Some(window);
        st.last_context = Some(used);
        out.push(EventPayload::ContextUsage(ContextUsage {
            used_tokens: used,
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
            reason: None,
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

/// `rate_limit_event` → je Fenster ein `usage.subscription` (USE-004, HAR-021). Gegen 2.1.296
/// verifiziert: `unifiedWindows` nennt alle Fenster (`five_hour`, `seven_day`) mit
/// `utilization` (0–1) und `resetsAt` (Unix-Sekunden), `rateLimitType` nur das gerade
/// maßgebliche. Ohne `unifiedWindows` gilt das Fenster aus `rateLimitType`.
fn map_rate_limit(v: &Value) -> Vec<EventPayload> {
    let info = &v["rate_limit_info"];
    let event = |kind: &str, used: Option<f64>, resets: Option<i64>| {
        let resets_at = time::OffsetDateTime::from_unix_timestamp(resets?).ok()?;
        Some(EventPayload::UsageSubscription(UsageSubscription {
            vendor: "anthropic".into(),
            window: beton_core::usage::window_name(kind),
            used_pct: (used? * 100.0 * 10.0).round() / 10.0,
            resets_at: Timestamp::from(resets_at),
        }))
    };
    match info["unifiedWindows"].as_object() {
        Some(windows) if !windows.is_empty() => windows
            .iter()
            .filter_map(|(kind, w)| {
                event(
                    kind,
                    w["utilization"].as_f64(),
                    w["resetsAt"].as_i64().or(info["resetsAt"].as_i64()),
                )
            })
            .collect(),
        _ => info["rateLimitType"]
            .as_str()
            .and_then(|kind| {
                event(
                    kind,
                    info["utilization"].as_f64(),
                    info["resetsAt"].as_i64(),
                )
            })
            .into_iter()
            .collect(),
    }
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
    fn use_004_ac2_rate_limit_event_gives_every_window() {
        // Form wie von 2.1.296 gemeldet: `rateLimitType` nennt nur das maßgebliche Fenster,
        // `unifiedWindows` alle.
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1791051000, "rateLimitType": "five_hour",
            "overageStatus": "allowed", "isUsingOverage": false,
            "unifiedWindows": {"five_hour": {"utilization": 0.19, "resetsAt": 1791051000},
                               "seven_day": {"utilization": 0.374, "resetsAt": 1791586800}}}});
        let out = map_line(&v, &mut MapState::default());
        let windows: Vec<_> = out
            .iter()
            .map(|e| match e {
                EventPayload::UsageSubscription(u) => (
                    u.vendor.as_str(),
                    u.window.as_str(),
                    u.used_pct,
                    u.resets_at.to_string(),
                ),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            windows,
            [
                (
                    "anthropic",
                    "5h",
                    19.0,
                    "2026-10-03T18:10:00.000Z".to_owned()
                ),
                (
                    "anthropic",
                    "7d",
                    37.4,
                    "2026-10-09T23:00:00.000Z".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn use_004_rate_limit_event_without_unified_windows_uses_the_type() {
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning",
            "resetsAt": 1791051000, "rateLimitType": "seven_day_opus", "utilization": 0.91}});
        let out = map_line(&v, &mut MapState::default());
        let [EventPayload::UsageSubscription(u)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!((u.window.as_str(), u.used_pct), ("7d_opus", 91.0));
        // Ohne Auslastung kein erfundener Wert.
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "rateLimitType": "five_hour", "resetsAt": 1791051000}});
        assert!(map_line(&v, &mut MapState::default()).is_empty());
    }

    #[test]
    fn use_004_ac1_result_with_several_models_gives_tokens_per_model() {
        let result = json!({
            "type": "result", "subtype": "success", "is_error": false, "stop_reason": "end_turn",
            "total_cost_usd": 0.05,
            "usage": {"input_tokens": 12, "output_tokens": 300, "cache_read_input_tokens": 9000, "cache_creation_input_tokens": 500},
            "modelUsage": {
                "claude-haiku-5-5": {"inputTokens": 2000, "outputTokens": 40, "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0, "costUSD": 0.01, "contextWindow": 200000},
                "claude-opus-5-5": {"inputTokens": 12, "outputTokens": 300, "cacheReadInputTokens": 9000, "cacheCreationInputTokens": 500, "costUSD": 0.04, "contextWindow": 1000000}
            }
        });
        let mut st = MapState {
            turn: Some(TurnId::new()),
            model: Some("claude-opus-5-5".into()),
            ..MapState::default()
        };
        let out = map_line(&result, &mut st);
        assert_eq!(
            types(&out),
            [
                "cost.delta",
                "cost.delta",
                "context.usage",
                "turn.completed"
            ]
        );
        let per_model: Vec<_> = out
            .iter()
            .filter_map(|e| match e {
                EventPayload::CostDelta(c) => Some((
                    c.model.as_str(),
                    c.input_tokens,
                    c.output_tokens,
                    c.cache_read_tokens,
                    c.cost_micro,
                    c.source,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            per_model,
            [
                (
                    "claude-haiku-5-5",
                    2000,
                    40,
                    0,
                    None,
                    CostSource::Subscription
                ),
                (
                    "claude-opus-5-5",
                    12,
                    300,
                    9000,
                    None,
                    CostSource::Subscription
                ),
            ]
        );
        // Kontext gehört zum Modell der Session, nicht zum alphabetisch ersten.
        let EventPayload::ContextUsage(u) = &out[2] else {
            panic!()
        };
        assert_eq!(u.window_tokens, 1_000_000);

        // Mit API-Key trägt jedes Modell seine gemeldeten Kosten.
        let mut st = MapState {
            auth_source: Some(AuthSource::ApiKey),
            ..MapState::default()
        };
        let costs: Vec<_> = map_line(&result, &mut st)
            .into_iter()
            .filter_map(|e| match e {
                EventPayload::CostDelta(c) => c.cost_micro,
                _ => None,
            })
            .collect();
        assert_eq!(costs, [10_000, 40_000]);
        assert_eq!(
            result_cost(&result, AuthSource::ApiKey).cost_micro,
            Some(50_000)
        );
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
