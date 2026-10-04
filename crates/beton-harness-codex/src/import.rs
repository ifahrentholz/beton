//! Transcript-Import für Codex (HAR-024).
//!
//! Quelle: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` (ohne Variable `~/.codex`,
//! HAR-024 AC3). Gelesen werden nur Dateien mit diesem Namensmuster unterhalb von `sessions/`;
//! `auth.json` und `config.toml` liegen außerhalb und sind nicht erreichbar (ADR-0005).
//!
//! Format (vendor-intern, *Annahme* nach der Spec und den Typen des App-Server-Protokolls):
//! jede Zeile `{timestamp, type, payload}`. Die erste ist `session_meta` (`id`, `cwd`,
//! `cli_version`); `turn_context` nennt Modell und Arbeitsverzeichnis; `response_item` trägt
//! Nachrichten, Reasoning und Function-/Custom-Tool-Calls samt Ausgaben; `event_msg` trägt
//! u. a. Token-Zählungen und Abbrüche. Ein anderes Format (z. B. die frühen Rollouts ohne
//! `session_meta`) wird abgelehnt statt geraten (fail closed).

use std::collections::HashMap;
use std::path::Path;

use beton_core::event::{
    EventPayload, FsChange, FsChangeKind, FsChanged, HarnessUnmapped, MessageCompleted,
    MessageRole, RawJson, ReasoningCompleted, ToolCallCompleted, ToolCallRequested, ToolSource,
    ToolStatus,
};
use beton_harness::HostEnv;
use beton_harness::import::{
    AccessLog, ExternalSessionRef, HEAD_BYTES, ImportError, ImportWarning, ImportedTranscript,
    Line, MAX_CANDIDATES, TranscriptImporter, TurnBuilder, VendorFiles, WarningKind,
    bad_line_warning, lines, title_from, ts_of,
};
use serde_json::{Value, json};

/// `source` der `fs.changed`-Events aus einem Import.
pub const FS_SOURCE_IMPORT: &str = "import";

/// Codex-Verzeichnis: `CODEX_HOME`, sonst `$HOME/.codex` (HAR-024 AC3).
pub fn codex_home(env: &HostEnv) -> Option<std::path::PathBuf> {
    if let Some(dir) = env.vars.get("CODEX_HOME").filter(|d| !d.is_empty()) {
        return Some(dir.into());
    }
    env.vars
        .get("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| Path::new(h).join(".codex"))
}

/// Erlaubte Dateinamen: `rollout-<zeit>-<id>.jsonl`.
pub fn rollout_file_name(name: &str) -> bool {
    name.len() <= 200
        && name.starts_with("rollout-")
        && name.ends_with(".jsonl")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// `event_msg`-Typen, die keine Events brauchen (Nachrichten kommen aus `response_item`).
const IGNORED_EVENTS: [&str; 12] = [
    "user_message",
    "agent_message",
    "agent_reasoning",
    "agent_reasoning_raw_content",
    "agent_reasoning_section_break",
    "token_count",
    "task_started",
    "task_complete",
    "entered_review_mode",
    "exited_review_mode",
    "context_compacted",
    "turn_context",
];

/// Eingeschobener Kontext der CLI, kein Nutzer-Prompt.
fn is_context_injection(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("# AGENTS.md instructions")
        || t.starts_with("<turn_aborted>")
}

/// Importer für Codex-Sessions.
#[derive(Debug, Clone, Default)]
pub struct CodexImporter {
    /// Protokoll der geöffneten Dateien (Tests).
    pub audit: Option<AccessLog>,
}

impl TranscriptImporter for CodexImporter {
    fn discover(&self, env: &HostEnv) -> Result<Vec<ExternalSessionRef>, ImportError> {
        let home = codex_home(env).ok_or_else(|| {
            ImportError::Unconfigured("weder CODEX_HOME noch HOME ist gesetzt".into())
        })?;
        let Some(files) = VendorFiles::open(
            &home.join("sessions"),
            rollout_file_name,
            self.audit.clone(),
        )?
        else {
            return Ok(Vec::new());
        };
        let mut found = files.walk(4);
        found.sort_by_key(|f| std::cmp::Reverse(f.modified));
        found.truncate(MAX_CANDIDATES);
        let mut out = Vec::new();
        for f in found {
            let Ok(head) = files.read(&f.path, HEAD_BYTES, false) else {
                continue;
            };
            let Some(info) = head_info(&head) else {
                continue;
            };
            out.push(ExternalSessionRef {
                harness: "codex".into(),
                id: info.id,
                path: f.path,
                root: files.root().to_path_buf(),
                cwd: info.cwd,
                title: info.title,
                model: info.model,
                updated_at: f.modified,
                size_bytes: f.size,
            });
        }
        Ok(out)
    }

    fn parse(&self, r: &ExternalSessionRef) -> Result<ImportedTranscript, ImportError> {
        let files = VendorFiles::open(&r.root, rollout_file_name, self.audit.clone())?
            .ok_or_else(|| ImportError::NotAllowed("Wurzel existiert nicht".into()))?;
        let bytes = files.read(&r.path, 0, true)?;
        parse_rollout(&bytes)
    }
}

struct HeadInfo {
    id: String,
    cwd: Option<String>,
    title: Option<String>,
    model: Option<String>,
}

fn head_info(bytes: &[u8]) -> Option<HeadInfo> {
    let mut it = lines(bytes).into_iter();
    let meta = loop {
        match it.next()? {
            Line::Json { value, .. } => break value,
            Line::Bad { .. } => continue,
        }
    };
    if meta["type"] != "session_meta" {
        return None;
    }
    let p = &meta["payload"];
    let mut info = HeadInfo {
        id: p["id"].as_str()?.to_owned(),
        cwd: p["cwd"].as_str().map(str::to_owned),
        title: None,
        model: p["model"].as_str().map(str::to_owned),
    };
    for line in it {
        let Line::Json { value: v, .. } = line else {
            continue;
        };
        match v["type"].as_str() {
            Some("turn_context") if info.model.is_none() => {
                info.model = v["payload"]["model"].as_str().map(str::to_owned);
            }
            Some("response_item")
                if info.title.is_none()
                    && v["payload"]["type"] == "message"
                    && v["payload"]["role"] == "user" =>
            {
                let text = message_text(&v["payload"]);
                if !is_context_injection(&text) {
                    info.title = title_from(&text);
                }
            }
            _ => {}
        }
        if info.title.is_some() && info.model.is_some() {
            break;
        }
    }
    Some(info)
}

fn message_text(item: &Value) -> String {
    item["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            matches!(
                c["type"].as_str(),
                Some("input_text" | "output_text" | "text")
            )
        })
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// MCP-Tools heißen `<server>__<tool>` (bzw. `mcp__<server>__<tool>`) → (`tool`,
/// `Some(server)`); andere Namen unverändert.
fn split_mcp_name(name: String) -> (String, Option<String>) {
    let rest = name.strip_prefix("mcp__").unwrap_or(&name);
    match rest.split_once("__") {
        Some((server, tool)) if !server.is_empty() && !tool.is_empty() => {
            (tool.to_owned(), Some(server.to_owned()))
        }
        _ => (name, None),
    }
}

/// Eine Datei von `apply_patch` (Patch-Format von Codex).
#[derive(Debug, Clone, PartialEq)]
struct PatchFile {
    path: String,
    kind: &'static str,
    move_path: Option<String>,
    diff: String,
}

/// Zerlegt einen `apply_patch`-Patch in Dateien (`*** Add|Update|Delete File: <pfad>`,
/// `*** Move to: <pfad>`).
fn parse_patch(patch: &str) -> Vec<PatchFile> {
    let mut out: Vec<PatchFile> = Vec::new();
    for line in patch.lines() {
        let header = [
            ("*** Add File: ", "add"),
            ("*** Update File: ", "update"),
            ("*** Delete File: ", "delete"),
        ]
        .iter()
        .find_map(|(prefix, kind)| line.strip_prefix(prefix).map(|p| (p.trim(), *kind)));
        if let Some((path, kind)) = header {
            out.push(PatchFile {
                path: path.to_owned(),
                kind,
                move_path: None,
                diff: String::new(),
            });
            continue;
        }
        if line.starts_with("*** Begin Patch") || line.starts_with("*** End Patch") {
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        if let Some(to) = line.strip_prefix("*** Move to: ") {
            cur.move_path = Some(to.trim().to_owned());
            continue;
        }
        if line.starts_with("*** End of File") {
            continue;
        }
        cur.diff.push_str(line);
        cur.diff.push('\n');
    }
    out
}

/// Pfad relativ zum Arbeitsverzeichnis, falls er darin liegt.
fn relative(path: &str, cwd: Option<&str>) -> String {
    match cwd {
        Some(cwd) => Path::new(path)
            .strip_prefix(cwd)
            .map_or_else(|_| path.to_owned(), |p| p.display().to_string()),
        None => path.to_owned(),
    }
}

/// Patch-Text eines Tool-Calls, falls es `apply_patch` ist (Custom-Tool, Function mit
/// `input` oder Shell-Aufruf `["apply_patch", "<patch>"]`).
fn patch_of(name: &str, args: &Value) -> Option<String> {
    if name == "apply_patch" {
        return args
            .as_str()
            .or_else(|| args["input"].as_str())
            .or_else(|| args["patch"].as_str())
            .map(str::to_owned);
    }
    let cmd = args["command"].as_array()?;
    (cmd.first()?.as_str()? == "apply_patch")
        .then(|| cmd.get(1)?.as_str().map(str::to_owned))
        .flatten()
}

/// Ergebnis einer Tool-Ausgabe: Status aus dem Exit-Code (JSON-`metadata` oder Textzeile).
fn output_result(output: &Value) -> (ToolStatus, Value) {
    let text = output
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| output.to_string());
    if let Ok(v) = serde_json::from_str::<Value>(&text)
        && v.is_object()
        && v.get("output").is_some()
    {
        let code = v["metadata"]["exit_code"].as_i64();
        let status = if code.unwrap_or(0) == 0 {
            ToolStatus::Ok
        } else {
            ToolStatus::Error
        };
        return (status, json!({"exit_code": code, "output": v["output"]}));
    }
    let code = text.lines().find_map(|l| {
        l.strip_prefix("Process exited with code ")
            .or_else(|| l.strip_prefix("Exit code: "))
            .and_then(|c| c.trim().parse::<i64>().ok())
    });
    let status = if code.unwrap_or(0) == 0 {
        ToolStatus::Ok
    } else {
        ToolStatus::Error
    };
    (status, Value::String(text))
}

/// Übersetzt eine Codex-Rollout-Datei (HAR-024).
pub fn parse_rollout(bytes: &[u8]) -> Result<ImportedTranscript, ImportError> {
    let mut warnings = Vec::new();
    let all = lines(bytes);
    let first = all.iter().find_map(|l| match l {
        Line::Json { value, .. } => Some(value),
        Line::Bad { .. } => None,
    });
    let Some(meta) = first.filter(|v| v["type"] == "session_meta") else {
        // Fail closed: kein bekanntes Rollout-Format.
        return Err(ImportError::UnknownFormat(
            "erste Zeile ist kein `session_meta`".into(),
        ));
    };
    let mp = &meta["payload"];
    let id = mp["id"]
        .as_str()
        .ok_or_else(|| ImportError::UnknownFormat("`session_meta` ohne `id`".into()))?
        .to_owned();
    let mut t = ImportedTranscript {
        native_session_ref: id,
        cwd: mp["cwd"].as_str().map(str::to_owned),
        model: mp["model"].as_str().map(str::to_owned),
        cli_version: mp["cli_version"].as_str().map(str::to_owned),
        started_at: ts_of(&mp["timestamp"]).or_else(|| ts_of(&meta["timestamp"])),
        ..ImportedTranscript::default()
    };
    let mut b = TurnBuilder::default();
    let mut unmapped = 0u64;
    let mut patches: HashMap<String, Vec<PatchFile>> = HashMap::new();
    let mut msg_no = 0u64;
    let mut seen_meta = false;
    for line in &all {
        let (text, v) = match line {
            Line::Bad { no, kind } => {
                warnings.push(bad_line_warning(*no, *kind));
                continue;
            }
            Line::Json { text, value, .. } => (*text, value),
        };
        let ts = ts_of(&v["timestamp"]);
        if ts.is_some() {
            t.updated_at = ts;
        }
        let p = &v["payload"];
        let raw = RawJson::from_string((*text).to_owned()).ok();
        match v["type"].as_str() {
            Some("session_meta") if !seen_meta => seen_meta = true,
            // Wiederholt nach `codex resume`; der Thread bleibt derselbe.
            Some("session_meta") => {}
            Some("turn_context") => {
                if let Some(m) = p["model"].as_str() {
                    t.model = Some(m.to_owned());
                }
                if t.cwd.is_none() {
                    t.cwd = p["cwd"].as_str().map(str::to_owned);
                }
            }
            Some("compacted") => {}
            Some("event_msg") => match p["type"].as_str() {
                Some("turn_aborted") => b.interrupt(ts),
                Some(k) if IGNORED_EVENTS.contains(&k) => {}
                _ => {
                    unmapped += 1;
                    b.push(
                        EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() }),
                        ts,
                        None,
                    );
                }
            },
            Some("response_item") => {
                let ty = p["type"].as_str().unwrap_or_default();
                match ty {
                    "message" => {
                        let text = message_text(p);
                        match p["role"].as_str() {
                            Some("user") => {
                                if text.trim().is_empty() || is_context_injection(&text) {
                                    continue;
                                }
                                if t.title.is_none() {
                                    t.title = title_from(&text);
                                }
                                msg_no += 1;
                                b.user_message(
                                    p["id"].as_str().map_or_else(
                                        || format!("codex-user-{msg_no}"),
                                        str::to_owned,
                                    ),
                                    vec![json!({"type": "text", "text": text})],
                                    ts,
                                    raw,
                                );
                            }
                            Some("assistant") if !text.is_empty() => {
                                msg_no += 1;
                                b.push(
                                    EventPayload::MessageCompleted(MessageCompleted {
                                        message_id: p["id"].as_str().map_or_else(
                                            || format!("codex-msg-{msg_no}"),
                                            str::to_owned,
                                        ),
                                        role: MessageRole::Assistant,
                                        content: vec![json!({"type": "text", "text": text})],
                                        author: None,
                                    }),
                                    ts,
                                    raw,
                                );
                            }
                            // System-/Developer-Anweisungen der CLI.
                            _ => {}
                        }
                    }
                    "reasoning" => {
                        let summary: Vec<&str> = p["summary"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|s| s["text"].as_str())
                            .collect();
                        msg_no += 1;
                        b.push(
                            EventPayload::ReasoningCompleted(ReasoningCompleted {
                                message_id: p["id"]
                                    .as_str()
                                    .map_or_else(|| format!("codex-rs-{msg_no}"), str::to_owned),
                                summary: (!summary.is_empty()).then(|| summary.join("\n")),
                                redacted: summary.is_empty(),
                            }),
                            ts,
                            raw,
                        );
                    }
                    "function_call" | "custom_tool_call" | "local_shell_call" => {
                        let call_id = p["call_id"]
                            .as_str()
                            .or_else(|| p["id"].as_str())
                            .unwrap_or_default()
                            .to_owned();
                        let (name, mut args) = match ty {
                            "function_call" => {
                                let a = p["arguments"].as_str().map_or_else(
                                    || p["arguments"].clone(),
                                    |s| {
                                        serde_json::from_str(s)
                                            .unwrap_or_else(|_| Value::String(s.to_owned()))
                                    },
                                );
                                (p["name"].as_str().unwrap_or_default().to_owned(), a)
                            }
                            "custom_tool_call" => (
                                p["name"].as_str().unwrap_or_default().to_owned(),
                                p["input"].clone(),
                            ),
                            _ => (
                                "local_shell".to_owned(),
                                json!({"command": p["action"]["command"]}),
                            ),
                        };
                        let (name, mcp_server) = split_mcp_name(name);
                        if let Some(patch) = patch_of(&name, &args) {
                            let files = parse_patch(&patch);
                            let changes: Vec<Value> = files
                                .iter()
                                .map(|f| {
                                    let mut c =
                                        json!({"path": f.path, "kind": f.kind, "diff": f.diff});
                                    if let Some(to) = &f.move_path {
                                        c["move_path"] = json!(to);
                                    }
                                    c
                                })
                                .collect();
                            args = json!({"patch": patch, "changes": changes});
                            patches.insert(call_id.clone(), files);
                        }
                        b.push(
                            EventPayload::ToolCallRequested(ToolCallRequested {
                                call_id,
                                tool: name,
                                source: if mcp_server.as_deref() == Some("beton") {
                                    ToolSource::BetonMcp
                                } else {
                                    ToolSource::Harness
                                },
                                mcp_server,
                                args,
                                parent_call_id: None,
                            }),
                            ts,
                            raw,
                        );
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let call_id = p["call_id"].as_str().unwrap_or_default().to_owned();
                        let (status, result) = output_result(&p["output"]);
                        let patch = patches.remove(&call_id);
                        b.push(
                            EventPayload::ToolCallCompleted(ToolCallCompleted {
                                call_id,
                                status,
                                result: Some(result),
                                result_ref: None,
                                duration_ms: 0,
                            }),
                            ts,
                            raw,
                        );
                        if let (Some(files), ToolStatus::Ok) = (patch, status) {
                            let changes = files
                                .iter()
                                .map(|f| {
                                    let path = relative(
                                        f.move_path.as_deref().unwrap_or(&f.path),
                                        t.cwd.as_deref(),
                                    );
                                    FsChange {
                                        path,
                                        change: match (f.kind, &f.move_path) {
                                            ("add", _) => FsChangeKind::Added,
                                            ("delete", _) => FsChangeKind::Deleted,
                                            (_, Some(_)) => FsChangeKind::Renamed,
                                            _ => FsChangeKind::Modified,
                                        },
                                        from: f
                                            .move_path
                                            .as_ref()
                                            .map(|_| relative(&f.path, t.cwd.as_deref())),
                                    }
                                })
                                .collect();
                            b.push(
                                EventPayload::FsChanged(FsChanged {
                                    changes,
                                    source: FS_SOURCE_IMPORT.into(),
                                }),
                                ts,
                                None,
                            );
                        }
                    }
                    // Interne Schnappschüsse der CLI.
                    "ghost_snapshot" => {}
                    _ => {
                        unmapped += 1;
                        b.push(
                            EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() }),
                            ts,
                            None,
                        );
                    }
                }
            }
            _ => {
                unmapped += 1;
                b.push(
                    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v.clone() }),
                    ts,
                    None,
                );
            }
        }
    }
    b.finish();
    if unmapped > 0 {
        warnings.push(ImportWarning::counted(
            WarningKind::Unmapped,
            unmapped,
            format!("{unmapped} unbekannte(r) Eintrag/Einträge als Rohdaten übernommen"),
        ));
    }
    t.events = b.events;
    // Events ohne eigenen Zeitstempel (frühe Zeilen) bekommen den Beginn der Session.
    for e in &mut t.events {
        if e.ts.is_none() {
            e.ts = t.started_at;
        }
    }
    t.warnings = warnings;
    Ok(t)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn har_024_patch_is_split_into_files() {
        let patch = "*** Begin Patch\n*** Add File: neu.txt\n+hallo\n*** Update File: src/a.rs\n*** Move to: src/b.rs\n@@\n-alt\n+neu\n*** Delete File: weg.txt\n*** End Patch\n";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 3);
        assert_eq!((files[0].path.as_str(), files[0].kind), ("neu.txt", "add"));
        assert_eq!(files[0].diff, "+hallo\n");
        assert_eq!(files[1].move_path.as_deref(), Some("src/b.rs"));
        assert_eq!(files[1].diff, "@@\n-alt\n+neu\n");
        assert_eq!(files[2].kind, "delete");
        assert_eq!(
            patch_of("shell", &json!({"command": ["apply_patch", patch]})).as_deref(),
            Some(patch)
        );
        assert_eq!(patch_of("shell", &json!({"command": ["ls"]})), None);
    }

    #[test]
    fn har_024_output_status_comes_from_the_exit_code() {
        let (s, r) = output_result(&json!(
            "{\"output\":\"x\",\"metadata\":{\"exit_code\":1,\"duration_seconds\":0.1}}"
        ));
        assert_eq!(s, ToolStatus::Error);
        assert_eq!(r["exit_code"], 1);
        let (s, _) = output_result(&json!(
            "Chunk ID: 1\nProcess exited with code 0\nOutput:\nok"
        ));
        assert_eq!(s, ToolStatus::Ok);
        let (s, _) = output_result(&json!("Process exited with code 2\n"));
        assert_eq!(s, ToolStatus::Error);
    }

    #[test]
    fn har_024_unknown_format_fails_closed() {
        let legacy = b"{\"id\":\"x\",\"timestamp\":\"2025-05-01T00:00:00Z\",\"instructions\":null}\n{\"type\":\"message\",\"role\":\"user\",\"content\":[]}\n";
        let err = parse_rollout(legacy).unwrap_err();
        assert!(matches!(err, ImportError::UnknownFormat(_)), "{err}");
        assert!(matches!(
            parse_rollout(b"").unwrap_err(),
            ImportError::UnknownFormat(_)
        ));
    }

    #[test]
    fn rollout_names_are_allowlisted() {
        assert!(rollout_file_name(
            "rollout-2026-10-03T08-15-22-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl"
        ));
        assert!(!rollout_file_name("auth.json"));
        assert!(!rollout_file_name("rollout-x.json"));
        assert!(!rollout_file_name("rollout-../x.jsonl"));
    }
}
