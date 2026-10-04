//! History-Rebuild für Claude Code (HAR-019): rekonstruiert aus dem Event-Log eine
//! Session-Datei im Projektverzeichnis der CLI, die `claude --resume <id>` fortsetzen kann.
//!
//! Ablage: `$CLAUDE_CONFIG_DIR/projects/<slug>/<id>.jsonl`, ohne Variable unter `~/.claude`;
//! `<slug>` ist das Arbeitsverzeichnis, in dem jedes Zeichen außer ASCII-Buchstaben und
//! -Ziffern zu `-` wird. Das Format ist vendor-intern und undokumentiert *(Annahme, gegen die
//! echte CLI manuell zu prüfen)*: eine JSON-Zeile je Eintrag mit `type` (`user`|`assistant`),
//! `message` (wie in der Messages-API), `uuid`/`parentUuid` als Kette, `sessionId`, `cwd`,
//! `timestamp`. Gelesen wird aus dem Verzeichnis nie (keine Credentials, ADR-0005).
//!
//! Verlustfrei, wo `raw` gespeichert ist: Die stream-json-Zeilen `assistant` und `user`
//! enthalten `message` im selben Format. Sonst Synthese aus den normalisierten Events.

use std::path::{Path, PathBuf};

use beton_core::event::{EventPayload, MessageRole, ToolStatus};
use beton_harness::handover::HistoryEvent;
use beton_harness::{HarnessError, HostEnv, RebuildRequest};
use serde_json::{Value, json};

/// Konfigurationsverzeichnis der CLI: `CLAUDE_CONFIG_DIR`, sonst `$HOME/.claude`.
pub fn config_dir(env: &HostEnv) -> Option<PathBuf> {
    if let Some(dir) = env.vars.get("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    env.vars
        .get("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| Path::new(h).join(".claude"))
}

/// Projektverzeichnis der Session-Dateien für ein Arbeitsverzeichnis.
pub fn project_dir(config: &Path, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    config.join("projects").join(slug)
}

/// Zufällige UUID (Version 4) als Text.
pub fn new_uuid() -> String {
    let n = beton_core::id::EventId::new().ulid().0;
    let n = (n & !(0xF_u128 << 76)) | (0x4_u128 << 76);
    let n = (n & !(0x3_u128 << 62)) | (0x2_u128 << 62);
    let h = format!("{n:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

/// `message`-Objekte der Session-Datei aus dem Verlauf, in Reihenfolge. Fehler, wenn ein
/// gespeichertes `raw` nicht dem erwarteten Schema entspricht.
pub fn messages(history: &[HistoryEvent]) -> Result<Vec<(String, Value)>, HarnessError> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut open_calls: Vec<String> = Vec::new();
    let close_open = |out: &mut Vec<(String, Value)>, open: &mut Vec<String>| {
        for id in open.drain(..) {
            out.push((
                "user".into(),
                json!({"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": id,
                    "content": "Abgebrochen", "is_error": true,
                }]}),
            ));
        }
    };
    for e in history {
        if let Some(raw) = &e.raw {
            let v: Value = serde_json::from_str(raw)
                .map_err(|err| HarnessError::Protocol(format!("raw ist kein JSON: {err}")))?;
            if let Some(kind @ ("user" | "assistant")) = v["type"].as_str() {
                let message = v.get("message").filter(|m| m.is_object()).ok_or_else(|| {
                    HarnessError::Protocol(format!(
                        "Schema unbekannt: `{kind}`-Zeile ohne `message` (seq {})",
                        e.seq
                    ))
                })?;
                for c in message["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("tool_use") => {
                            open_calls.extend(c["id"].as_str().map(str::to_owned));
                        }
                        Some("tool_result") => {
                            let id = c["tool_use_id"].as_str().unwrap_or_default();
                            open_calls.retain(|o| o != id);
                        }
                        _ => {}
                    }
                }
                out.push((kind.to_owned(), message.clone()));
                continue;
            }
        }
        match &e.payload {
            EventPayload::MessageCompleted(m) if m.role == MessageRole::User => {
                close_open(&mut out, &mut open_calls);
                out.push((
                    "user".into(),
                    json!({"role": "user", "content": text_of(&m.content)}),
                ));
            }
            EventPayload::MessageCompleted(m) => {
                let text = text_of(&m.content);
                if text.is_empty() {
                    continue;
                }
                out.push((
                    "assistant".into(),
                    json!({"type": "message", "role": "assistant",
                           "content": [{"type": "text", "text": text}]}),
                ));
            }
            // Tool-Calls eines Vendor-Sub-Agents (Import, HAR-023) gehören nicht in den
            // Hauptverlauf; ihr Ergebnis steckt im Ergebnis des Task-Aufrufs.
            EventPayload::ToolCallRequested(c) if c.parent_call_id.is_some() => {}
            EventPayload::ToolCallRequested(c) => {
                let name = match &c.mcp_server {
                    Some(server) => format!("mcp__{server}__{}", c.tool),
                    None => c.tool.clone(),
                };
                open_calls.push(c.call_id.clone());
                out.push((
                    "assistant".into(),
                    json!({"type": "message", "role": "assistant", "content": [{
                        "type": "tool_use", "id": c.call_id, "name": name, "input": c.args,
                    }]}),
                ));
            }
            EventPayload::ToolCallCompleted(c) => {
                if !open_calls.contains(&c.call_id) {
                    continue;
                }
                open_calls.retain(|o| *o != c.call_id);
                let content = match &c.result {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                out.push((
                    "user".into(),
                    json!({"role": "user", "content": [{
                        "type": "tool_result", "tool_use_id": c.call_id, "content": content,
                        "is_error": c.status != ToolStatus::Ok,
                    }]}),
                ));
            }
            _ => {}
        }
    }
    close_open(&mut out, &mut open_calls);
    Ok(out)
}

/// Schreibt die Session-Datei und liefert die neue Session-ID.
pub fn rebuild(request: &RebuildRequest, env: &HostEnv) -> Result<String, HarnessError> {
    let messages = messages(&request.history)?;
    if messages.is_empty() {
        return Err(HarnessError::Protocol("Verlauf ohne Nachrichten".into()));
    }
    let config = config_dir(env).ok_or_else(|| {
        HarnessError::StartRefused("Claude-Konfigurationsverzeichnis unbekannt".into())
    })?;
    let dir = project_dir(&config, &request.workdir);
    std::fs::create_dir_all(&dir)?;
    let session = new_uuid();
    let cwd = request.workdir.display().to_string();
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let mut parent: Option<String> = None;
    let mut text = String::new();
    for (kind, message) in messages {
        let uuid = new_uuid();
        let line = json!({
            "parentUuid": parent,
            "isSidechain": false,
            "userType": "external",
            "cwd": cwd,
            "sessionId": session,
            "version": crate::TESTED_VERSIONS[0],
            "type": kind,
            "message": message,
            "uuid": uuid,
            "timestamp": now,
        });
        text.push_str(&line.to_string());
        text.push('\n');
        parent = Some(uuid);
    }
    let file = dir.join(format!("{session}.jsonl"));
    let tmp = dir.join(format!(".{session}.jsonl.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &file)?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beton_core::event::{MessageCompleted, ToolCallCompleted, ToolCallRequested, ToolSource};

    fn ev(seq: u64, payload: EventPayload, raw: Option<&str>) -> HistoryEvent {
        HistoryEvent {
            seq,
            payload,
            turn_id: None,
            raw: raw.map(str::to_owned),
        }
    }

    fn msg(role: MessageRole, text: &str) -> EventPayload {
        EventPayload::MessageCompleted(MessageCompleted {
            message_id: "m".into(),
            role,
            content: vec![json!({"type": "text", "text": text})],
            author: None,
        })
    }

    #[test]
    fn har_019_raw_is_used_verbatim_and_synthesis_fills_the_rest() {
        let raw = r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"aus raw"}]}}"#;
        let history = vec![
            ev(2, msg(MessageRole::User, "Hallo"), None),
            ev(4, msg(MessageRole::Assistant, "ignoriert"), Some(raw)),
            ev(
                5,
                EventPayload::ToolCallRequested(ToolCallRequested {
                    call_id: "c1".into(),
                    tool: "Bash".into(),
                    mcp_server: None,
                    args: json!({"command": "ls"}),
                    source: ToolSource::Harness,
                    parent_call_id: None,
                }),
                None,
            ),
            ev(
                6,
                EventPayload::ToolCallCompleted(ToolCallCompleted {
                    call_id: "c1".into(),
                    status: ToolStatus::Ok,
                    result: Some(json!("a.txt")),
                    result_ref: None,
                    duration_ms: 1,
                }),
                None,
            ),
        ];
        let m = messages(&history).unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0].1["content"], "Hallo");
        assert_eq!(m[1].1["content"][0]["text"], "aus raw");
        assert_eq!(m[2].1["content"][0]["type"], "tool_use");
        assert_eq!(m[3].1["content"][0]["tool_use_id"], "c1");
    }

    #[test]
    fn har_023_nested_subagent_calls_stay_out_of_the_rebuilt_history() {
        let call = |id: &str, parent: Option<&str>| {
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: id.into(),
                tool: "Grep".into(),
                mcp_server: None,
                args: json!({}),
                source: ToolSource::Harness,
                parent_call_id: parent.map(str::to_owned),
            })
        };
        let done = |id: &str| {
            EventPayload::ToolCallCompleted(ToolCallCompleted {
                call_id: id.into(),
                status: ToolStatus::Ok,
                result: Some(json!("ok")),
                result_ref: None,
                duration_ms: 0,
            })
        };
        let history = vec![
            ev(1, msg(MessageRole::User, "Suche"), None),
            ev(2, call("task", None), None),
            ev(3, call("sub", Some("task")), None),
            ev(4, done("sub"), None),
            ev(5, done("task"), None),
        ];
        let m = messages(&history).unwrap();
        let ids: Vec<&str> = m
            .iter()
            .filter_map(|(_, v)| {
                v["content"][0]["id"]
                    .as_str()
                    .or(v["content"][0]["tool_use_id"].as_str())
            })
            .collect();
        assert_eq!(ids, ["task", "task"]);
    }

    #[test]
    fn har_019_ac3_unknown_raw_schema_fails() {
        let history = vec![ev(
            2,
            msg(MessageRole::Assistant, "x"),
            Some(r#"{"type":"assistant","content":"ohne message"}"#),
        )];
        let err = messages(&history).unwrap_err();
        assert!(err.to_string().contains("Schema unbekannt"), "{err}");
    }

    #[test]
    fn slug_replaces_non_alphanumerics() {
        assert_eq!(
            project_dir(Path::new("/c"), Path::new("/Users/a.b/My Repo")),
            PathBuf::from("/c/projects/-Users-a-b-My-Repo")
        );
        let u = new_uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
    }
}
