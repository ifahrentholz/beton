//! Transcript-Import für Claude Code (HAR-023).
//!
//! Quelle: `$CLAUDE_CONFIG_DIR/projects/<slug>/<session-uuid>.jsonl` (ohne Variable unter
//! `~/.claude`). Gelesen werden nur Dateien mit diesem Namensmuster unterhalb von `projects/`;
//! Konfiguration und Credentials der CLI (Credentials-Datei, `settings.json`) liegen
//! außerhalb und sind nicht erreichbar (ADR-0005).
//!
//! Format (vendor-intern, undokumentiert, *Annahme* nach der Spec, `rebuild.rs` und den
//! Golden-Transcripts): eine JSON-Zeile je Eintrag. `user`, `assistant` und `system` tragen
//! `uuid` und `parentUuid` und bilden einen Baum; der aktive Zweig ist der Pfad zum zuletzt
//! geschriebenen Blatt (HAR-023 AC2). Nach einer Compaction verweist `logicalParentUuid` auf
//! den Vorgänger. `isSidechain: true` sind Einträge eines Vendor-Sub-Agents; sie werden als
//! Tool-Calls unter dem auslösenden `Task`-Aufruf verschachtelt (`parent_call_id`). `summary`,
//! `file-history-snapshot` und `queue-operation` tragen keine Inhalte für den Verlauf.
//! Unbekannte Einträge werden `harness.unmapped`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use beton_core::event::{
    EventPayload, HarnessUnmapped, MessageCompleted, MessageRole, RawJson, ReasoningCompleted,
    ToolCallCompleted, ToolCallRequested, ToolSource, ToolStatus,
};
use beton_core::time::Timestamp;
use beton_harness::HostEnv;
use beton_harness::import::{
    AccessLog, ExternalSessionRef, HEAD_BYTES, ImportError, ImportWarning, ImportedEvent,
    ImportedTranscript, Line, MAX_CANDIDATES, TranscriptImporter, TurnBuilder, VendorFiles,
    WarningKind, bad_line_warning, is_uuid, lines, title_from, ts_of,
};
use serde_json::{Value, json};

use crate::mapping::split_mcp_name;
use crate::rebuild::{config_dir, project_dir};

/// Erlaubte Dateinamen: `<uuid>.jsonl`.
pub fn session_file_name(name: &str) -> bool {
    name.strip_suffix(".jsonl").is_some_and(is_uuid)
}

/// Einträge im Session-Baum mit bekanntem Typ.
const CHAIN_TYPES: [&str; 3] = ["user", "assistant", "system"];
/// Bekannte Einträge ohne Bedeutung für den Verlauf.
const META_TYPES: [&str; 3] = ["summary", "file-history-snapshot", "queue-operation"];
/// Tools, die einen Vendor-Sub-Agent starten.
const SUBAGENT_TOOLS: [&str; 2] = ["Task", "Agent"];
/// Marker der CLI für einen Abbruch durch den Nutzer.
const INTERRUPT_MARKER: &str = "[Request interrupted by user";

/// Importer für Claude-Code-Sessions.
#[derive(Debug, Clone, Default)]
pub struct ClaudeImporter {
    /// Protokoll der geöffneten Dateien (Tests, HAR-023 AC4).
    pub audit: Option<AccessLog>,
}

impl TranscriptImporter for ClaudeImporter {
    fn discover(&self, env: &HostEnv) -> Result<Vec<ExternalSessionRef>, ImportError> {
        let config = config_dir(env).ok_or_else(|| {
            ImportError::Unconfigured("weder CLAUDE_CONFIG_DIR noch HOME ist gesetzt".into())
        })?;
        let Some(files) = VendorFiles::open(
            &config.join("projects"),
            session_file_name,
            self.audit.clone(),
        )?
        else {
            return Ok(Vec::new());
        };
        let mut found = files.walk(2);
        // Nur `projects/<slug>/<uuid>.jsonl`.
        found.retain(|f| f.path.parent().and_then(Path::parent) == Some(files.root()));
        found.sort_by_key(|f| std::cmp::Reverse(f.modified));
        found.truncate(MAX_CANDIDATES);
        let mut out = Vec::new();
        for f in found {
            let Some(id) = f
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let Ok(head) = files.read(&f.path, HEAD_BYTES, false) else {
                continue;
            };
            let info = head_info(&head);
            out.push(ExternalSessionRef {
                harness: "claude".into(),
                id,
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
        let files = VendorFiles::open(&r.root, session_file_name, self.audit.clone())?
            .ok_or_else(|| ImportError::NotAllowed("Wurzel existiert nicht".into()))?;
        let bytes = files.read(&r.path, 0, true)?;
        parse_session(&bytes, &r.id)
    }

    fn native_resumable(&self, env: &HostEnv, cwd: &Path, vendor_session_id: &str) -> bool {
        if !is_uuid(vendor_session_id) {
            return false;
        }
        let Some(config) = config_dir(env) else {
            return false;
        };
        let path = project_dir(&config, cwd).join(format!("{vendor_session_id}.jsonl"));
        // Nur Existenz, kein Symlink; gelesen wird nichts.
        std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
    }
}

/// Was `discover()` aus dem Anfang einer Datei liest.
#[derive(Debug, Default, PartialEq)]
struct HeadInfo {
    cwd: Option<String>,
    title: Option<String>,
    model: Option<String>,
}

fn head_info(bytes: &[u8]) -> HeadInfo {
    let mut info = HeadInfo::default();
    let mut summary = None;
    let mut prompt = None;
    for line in lines(bytes) {
        let Line::Json { value: v, .. } = line else {
            continue;
        };
        if info.cwd.is_none() {
            info.cwd = v["cwd"].as_str().map(str::to_owned);
        }
        match v["type"].as_str() {
            Some("summary") if summary.is_none() => {
                summary = v["summary"].as_str().and_then(title_from);
            }
            Some("user") if prompt.is_none() && v["isMeta"] != true && v["isSidechain"] != true => {
                prompt = prompt_text(&v["message"]["content"]).and_then(|t| title_from(&t));
            }
            Some("assistant") if info.model.is_none() => info.model = model_of(&v),
            _ => {}
        }
    }
    info.title = summary.or(prompt);
    info
}

fn model_of(v: &Value) -> Option<String> {
    v["message"]["model"]
        .as_str()
        .filter(|m| !m.is_empty() && !m.starts_with('<'))
        .map(str::to_owned)
}

/// Text einer Nutzer-Eingabe (ohne Tool-Results); `None` bei reinen Tool-Results.
fn prompt_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            if parts.iter().any(|p| p["type"] == "tool_result") {
                return None;
            }
            let text: Vec<&str> = parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect();
            Some(text.join("\n"))
        }
        _ => None,
    }
}

/// Inhaltsblöcke einer Nutzer-Nachricht; Bilder werden nicht übernommen, nur vermerkt.
fn user_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(s) if !s.trim().is_empty() => vec![json!({"type": "text", "text": s})],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| match p["type"].as_str() {
                Some("text") => p["text"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| json!({"type": "text", "text": t})),
                Some("image") => Some(json!({"type": "text", "text": "[Bild nicht übernommen]"})),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Ein Eintrag mit `uuid`.
struct Rec<'a> {
    no: u64,
    text: &'a str,
    v: Value,
    uuid: String,
    side: bool,
}

impl Rec<'_> {
    /// Vorgänger: `parentUuid`, nach einer Compaction `logicalParentUuid`.
    fn parent(&self) -> Option<&str> {
        self.v["parentUuid"]
            .as_str()
            .or_else(|| self.v["logicalParentUuid"].as_str())
    }

    fn ts(&self) -> Option<Timestamp> {
        ts_of(&self.v["timestamp"])
    }

    fn kind(&self) -> &str {
        self.v["type"].as_str().unwrap_or("")
    }
}

/// Aktiver Pfad einer Gruppe (Hauptverlauf oder ein Sub-Agent): vom zuletzt geschriebenen
/// Blatt zur Wurzel. Liefert den Pfad in Vorwärtsrichtung und die Zahl verworfener Blätter.
fn active_path(
    recs: &[Rec<'_>],
    by_uuid: &HashMap<&str, usize>,
    members: &[usize],
    warnings: &mut Vec<ImportWarning>,
) -> (Vec<usize>, usize) {
    let member_set: HashSet<usize> = members.iter().copied().collect();
    let referenced: HashSet<&str> = members.iter().filter_map(|i| recs[*i].parent()).collect();
    let leaves: Vec<usize> = members
        .iter()
        .copied()
        .filter(|i| !referenced.contains(recs[*i].uuid.as_str()))
        .collect();
    let Some(&leaf) = leaves.last() else {
        return (Vec::new(), 0);
    };
    let mut path = Vec::new();
    let mut visited = HashSet::new();
    let mut cur = leaf;
    loop {
        path.push(cur);
        visited.insert(cur);
        let Some(parent) = recs[cur].parent() else {
            break;
        };
        match by_uuid.get(parent) {
            Some(&idx) if member_set.contains(&idx) && !visited.contains(&idx) => cur = idx,
            Some(_) => break,
            None => {
                // Vorgänger fehlt (z. B. beschädigte Zeile): an den vorigen Eintrag anschließen,
                // damit der Rest erhalten bleibt (HAR-023 AC3).
                let prev = members
                    .iter()
                    .rev()
                    .copied()
                    .find(|i| *i < cur && !visited.contains(i));
                warnings.push(ImportWarning::at(
                    WarningKind::DanglingParent,
                    recs[cur].no,
                    format!(
                        "Zeile {} verweist auf einen fehlenden Eintrag; an den vorigen Eintrag \
                         angeschlossen",
                        recs[cur].no
                    ),
                ));
                match prev {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
    }
    path.reverse();
    let discarded = leaves.iter().filter(|l| !visited.contains(*l)).count();
    (path, discarded)
}

/// Übersetzt eine Claude-Code-Session-Datei (HAR-023).
pub fn parse_session(bytes: &[u8], session_id: &str) -> Result<ImportedTranscript, ImportError> {
    let mut warnings: Vec<ImportWarning> = Vec::new();
    let mut recs: Vec<Rec<'_>> = Vec::new();
    let mut extra: Vec<(u64, Value)> = Vec::new();
    let mut summaries: Vec<(String, String)> = Vec::new();
    let mut recognized = 0usize;
    let mut version = None;
    for line in lines(bytes) {
        let (no, text, v) = match line {
            Line::Bad { no, kind } => {
                warnings.push(bad_line_warning(no, kind));
                continue;
            }
            Line::Json { no, text, value } => (no, text, value),
        };
        let kind = v["type"].as_str().unwrap_or("");
        if let Some(ver) = v["version"].as_str() {
            version = Some(ver.to_owned());
        }
        if let Some(uuid) = v["uuid"].as_str() {
            if CHAIN_TYPES.contains(&kind) {
                recognized += 1;
            }
            recs.push(Rec {
                no,
                text,
                uuid: uuid.to_owned(),
                side: v["isSidechain"] == true,
                v,
            });
        } else if META_TYPES.contains(&kind) {
            recognized += 1;
            if kind == "summary"
                && let (Some(leaf), Some(s)) = (v["leafUuid"].as_str(), v["summary"].as_str())
            {
                summaries.push((leaf.to_owned(), s.to_owned()));
            }
        } else {
            extra.push((no, v));
        }
    }
    if recognized == 0 {
        // Fail closed: nichts erkannt, nichts geraten.
        return Err(ImportError::UnknownFormat(
            "keine bekannten Claude-Code-Einträge gefunden".into(),
        ));
    }
    let mut by_uuid: HashMap<&str, usize> = HashMap::new();
    for (i, r) in recs.iter().enumerate() {
        by_uuid.entry(r.uuid.as_str()).or_insert(i);
    }
    let main: Vec<usize> = (0..recs.len())
        .filter(|i| !recs[*i].side && by_uuid.get(recs[*i].uuid.as_str()) == Some(i))
        .collect();
    let (path, discarded) = active_path(&recs, &by_uuid, &main, &mut warnings);
    if discarded > 0 {
        warnings.push(ImportWarning::counted(
            WarningKind::DiscardedBranches,
            discarded as u64,
            format!("{discarded} verworfene(r) Zweig(e) nicht übernommen, nur der aktive Verlauf"),
        ));
    }

    let mut nested = sidechains(&recs, &by_uuid, &path, &mut warnings);
    let mut b = TurnBuilder::default();
    let mut unmapped = 0u64;
    let mut model = None;
    let mut cwd = None;
    let mut first_prompt = None;
    for &i in &path {
        let r = &recs[i];
        let ts = r.ts();
        if cwd.is_none() {
            cwd = r.v["cwd"].as_str().map(str::to_owned);
        }
        let mut raw = RawJson::from_string(r.text.to_owned()).ok();
        match r.kind() {
            "user" => {
                if r.v["isMeta"] == true {
                    continue;
                }
                let content = &r.v["message"]["content"];
                let results: Vec<&Value> = content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p["type"] == "tool_result")
                    .collect();
                if !results.is_empty() {
                    for res in results {
                        let call_id = res["tool_use_id"].as_str().unwrap_or_default().to_owned();
                        for (payload, nts) in nested.remove(&call_id).unwrap_or_default() {
                            b.push(payload, nts.or(ts), None);
                        }
                        let status = if res["is_error"] == true {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Ok
                        };
                        b.push(
                            EventPayload::ToolCallCompleted(ToolCallCompleted {
                                call_id,
                                status,
                                result: Some(res["content"].clone()),
                                result_ref: None,
                                duration_ms: 0,
                            }),
                            ts,
                            raw.take(),
                        );
                    }
                    continue;
                }
                let parts = user_parts(content);
                if parts.is_empty() {
                    continue;
                }
                let text = prompt_text(content).unwrap_or_default();
                if text.trim_start().starts_with(INTERRUPT_MARKER) {
                    b.interrupt(ts);
                    continue;
                }
                if first_prompt.is_none() {
                    first_prompt = title_from(&text);
                }
                b.user_message(r.uuid.clone(), parts, ts, raw);
            }
            "assistant" => {
                if let Some(m) = model_of(&r.v) {
                    model = Some(m);
                }
                let msg = &r.v["message"];
                let id = msg["id"]
                    .as_str()
                    .map_or_else(|| r.uuid.clone(), str::to_owned);
                let events = assistant_events(msg, &id, None, &mut unmapped);
                let single = events.len() == 1;
                for e in events {
                    b.push(e, ts, if single { raw.take() } else { None });
                }
            }
            // Hinweise der CLI selbst (Compaction-Grenze, Fehlermeldungen …).
            "system" => {}
            _ => {
                unmapped += 1;
                b.push(
                    EventPayload::HarnessUnmapped(HarnessUnmapped { raw: r.v.clone() }),
                    ts,
                    None,
                );
            }
        }
    }
    b.finish();
    let orphaned: usize = nested.values().filter(|v| !v.is_empty()).count();
    if orphaned > 0 {
        warnings.push(ImportWarning::counted(
            WarningKind::OrphanSidechain,
            orphaned as u64,
            format!("{orphaned} Sub-Agent-Verlauf/-Verläufe ohne Ergebnis nicht übernommen"),
        ));
    }
    let mut events = b.events;
    for (_, v) in extra {
        unmapped += 1;
        events.push(ImportedEvent {
            ts: ts_of(&v["timestamp"]),
            payload: EventPayload::HarnessUnmapped(HarnessUnmapped { raw: v }),
            turn_id: None,
            by_user: false,
            raw: None,
        });
    }
    if unmapped > 0 {
        warnings.push(ImportWarning::counted(
            WarningKind::Unmapped,
            unmapped,
            format!("{unmapped} unbekannte(r) Eintrag/Einträge als Rohdaten übernommen"),
        ));
    }
    let on_path: HashSet<&str> = path.iter().map(|i| recs[*i].uuid.as_str()).collect();
    let title = summaries
        .iter()
        .rev()
        .find(|(leaf, _)| on_path.contains(leaf.as_str()))
        .and_then(|(_, s)| title_from(s))
        .or(first_prompt);
    Ok(ImportedTranscript {
        native_session_ref: session_id.to_owned(),
        cwd: cwd.or_else(|| {
            recs.iter()
                .find_map(|r| r.v["cwd"].as_str().map(str::to_owned))
        }),
        model,
        title,
        started_at: path.iter().find_map(|i| recs[*i].ts()),
        updated_at: path.iter().rev().find_map(|i| recs[*i].ts()),
        cli_version: version,
        events,
        warnings,
    })
}

/// Events einer Assistant-Nachricht in Block-Reihenfolge.
fn assistant_events(
    msg: &Value,
    id: &str,
    parent: Option<&str>,
    unmapped: &mut u64,
) -> Vec<EventPayload> {
    let blocks: Vec<Value> = match &msg["content"] {
        Value::String(s) => vec![json!({"type": "text", "text": s})],
        Value::Array(a) => a.clone(),
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for block in &blocks {
        match block["type"].as_str() {
            Some("text") if parent.is_none() => {
                let text = block["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    out.push(EventPayload::MessageCompleted(MessageCompleted {
                        message_id: id.to_owned(),
                        role: MessageRole::Assistant,
                        content: vec![json!({"type": "text", "text": text})],
                        author: None,
                    }));
                }
            }
            Some("thinking") if parent.is_none() => {
                let text = block["thinking"].as_str().unwrap_or_default();
                out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id.to_owned(),
                    summary: (!text.is_empty()).then(|| text.to_owned()),
                    redacted: text.is_empty(),
                }));
            }
            Some("redacted_thinking") if parent.is_none() => {
                out.push(EventPayload::ReasoningCompleted(ReasoningCompleted {
                    message_id: id.to_owned(),
                    summary: None,
                    redacted: true,
                }));
            }
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or_default();
                let (tool, mcp_server) = split_mcp_name(name);
                out.push(EventPayload::ToolCallRequested(ToolCallRequested {
                    call_id: block["id"].as_str().unwrap_or_default().to_owned(),
                    source: if mcp_server.as_deref() == Some("beton") {
                        ToolSource::BetonMcp
                    } else {
                        ToolSource::Harness
                    },
                    tool,
                    mcp_server,
                    args: block["input"].clone(),
                    parent_call_id: parent.map(str::to_owned),
                }));
            }
            // Text und Reasoning eines Sub-Agents stehen im Ergebnis des Task-Aufrufs.
            Some("text" | "thinking" | "redacted_thinking") => {}
            _ => {
                if parent.is_none() {
                    *unmapped += 1;
                    out.push(EventPayload::HarnessUnmapped(HarnessUnmapped {
                        raw: block.clone(),
                    }));
                }
            }
        }
    }
    out
}

type Nested = HashMap<String, Vec<(EventPayload, Option<Timestamp>)>>;

/// Verläufe der Vendor-Sub-Agents (`isSidechain`), zugeordnet zum auslösenden Task-Aufruf im
/// aktiven Zweig über den Auftragstext. Ergebnis: Tool-Calls je Task-Call-ID.
fn sidechains(
    recs: &[Rec<'_>],
    by_uuid: &HashMap<&str, usize>,
    path: &[usize],
    warnings: &mut Vec<ImportWarning>,
) -> Nested {
    // Gruppen nach Wurzel.
    let mut root_of: HashMap<usize, usize> = HashMap::new();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in (0..recs.len()).filter(|i| recs[*i].side) {
        let mut cur = i;
        let mut seen = HashSet::new();
        let root = loop {
            if let Some(r) = root_of.get(&cur) {
                break *r;
            }
            seen.insert(cur);
            match recs[cur].parent().and_then(|p| by_uuid.get(p)) {
                Some(&p) if recs[p].side && !seen.contains(&p) => cur = p,
                _ => break cur,
            }
        };
        root_of.insert(i, root);
        groups.entry(root).or_default().push(i);
    }
    // Task-Aufrufe im aktiven Zweig mit ihrem Auftrag.
    let mut tasks: Vec<(String, String)> = Vec::new();
    for &i in path {
        if recs[i].kind() != "assistant" {
            continue;
        }
        for block in recs[i].v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if block["type"] == "tool_use"
                && SUBAGENT_TOOLS.contains(&block["name"].as_str().unwrap_or(""))
                && let (Some(id), Some(prompt)) =
                    (block["id"].as_str(), block["input"]["prompt"].as_str())
            {
                tasks.push((id.to_owned(), prompt.trim().to_owned()));
            }
        }
    }
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort_unstable();
    let mut out: Nested = HashMap::new();
    for root in roots {
        let members = groups.get(&root).cloned().unwrap_or_default();
        let prompt = prompt_text(&recs[root].v["message"]["content"]).unwrap_or_default();
        let Some(pos) = tasks.iter().position(|(_, p)| *p == prompt.trim()) else {
            warnings.push(ImportWarning::at(
                WarningKind::OrphanSidechain,
                recs[root].no,
                format!(
                    "Sub-Agent-Verlauf ab Zeile {} ohne auslösenden Tool-Call; nicht übernommen",
                    recs[root].no
                ),
            ));
            continue;
        };
        let (task_id, _) = tasks.remove(pos);
        let mut quiet = Vec::new();
        let (sub_path, _) = active_path(recs, by_uuid, &members, &mut quiet);
        let mut requested: HashSet<String> = HashSet::new();
        let mut events = Vec::new();
        let mut ignored = 0;
        for i in sub_path {
            let r = &recs[i];
            match r.kind() {
                "assistant" => {
                    let msg = &r.v["message"];
                    let id = msg["id"].as_str().unwrap_or(&r.uuid).to_owned();
                    for e in assistant_events(msg, &id, Some(&task_id), &mut ignored) {
                        if let EventPayload::ToolCallRequested(c) = &e {
                            requested.insert(c.call_id.clone());
                        }
                        events.push((e, r.ts()));
                    }
                }
                "user" => {
                    for res in r.v["message"]["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|p| p["type"] == "tool_result")
                    {
                        let call_id = res["tool_use_id"].as_str().unwrap_or_default();
                        if !requested.contains(call_id) {
                            continue;
                        }
                        events.push((
                            EventPayload::ToolCallCompleted(ToolCallCompleted {
                                call_id: call_id.to_owned(),
                                status: if res["is_error"] == true {
                                    ToolStatus::Error
                                } else {
                                    ToolStatus::Ok
                                },
                                result: Some(res["content"].clone()),
                                result_ref: None,
                                duration_ms: 0,
                            }),
                            r.ts(),
                        ));
                    }
                }
                _ => {}
            }
        }
        out.insert(task_id, events);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const S: &str = "11111111-2222-4333-8444-555555555555";

    fn rec(uuid: &str, parent: Option<&str>, kind: &str, content: Value) -> String {
        let message = if kind == "assistant" {
            json!({"id": format!("msg_{uuid}"), "role": "assistant", "model": "claude-test", "content": content})
        } else {
            json!({"role": "user", "content": content})
        };
        json!({
            "parentUuid": parent, "isSidechain": false, "userType": "external", "cwd": "/w",
            "sessionId": S, "version": "2.1.285", "type": kind, "message": message,
            "uuid": uuid, "timestamp": "2026-09-01T10:00:00.000Z",
        })
        .to_string()
    }

    fn texts(t: &ImportedTranscript) -> Vec<String> {
        t.events
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::MessageCompleted(m) => {
                    m.content[0]["text"].as_str().map(str::to_owned)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn har_023_ac2_only_the_active_branch_is_imported_and_discarded_branches_are_counted() {
        // a → b → c (verworfen) ; b → d → e (aktiv, zuletzt geschrieben); a → f (verworfen)
        let file = [
            rec("a", None, "user", json!("Frage 1")),
            rec(
                "b",
                Some("a"),
                "assistant",
                json!([{"type": "text", "text": "Antwort 1"}]),
            ),
            rec("c", Some("b"), "user", json!("Verworfen 1")),
            rec(
                "f",
                Some("a"),
                "assistant",
                json!([{"type": "text", "text": "Verworfen 2"}]),
            ),
            rec("d", Some("b"), "user", json!("Frage 2")),
            rec(
                "e",
                Some("d"),
                "assistant",
                json!([{"type": "text", "text": "Antwort 2"}]),
            ),
        ]
        .join("\n");
        let t = parse_session(file.as_bytes(), S).unwrap();
        assert_eq!(texts(&t), ["Frage 1", "Antwort 1", "Frage 2", "Antwort 2"]);
        let w: Vec<_> = t
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::DiscardedBranches)
            .collect();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].count, Some(2));
        // Zwei Turns, beide abgeschlossen.
        let ends = t
            .events
            .iter()
            .filter(|e| e.payload.type_name() == "turn.completed")
            .count();
        assert_eq!(ends, 2);
    }

    #[test]
    fn har_023_ac3_a_corrupt_line_yields_a_warning_with_line_number_and_the_rest_is_imported() {
        let file = [
            rec("a", None, "user", json!("Frage 1")),
            rec(
                "b",
                Some("a"),
                "assistant",
                json!([{"type": "text", "text": "Antwort 1"}]),
            ),
            // Zeile 3: abgeschnitten; sie war der Eintrag `c`.
            r#"{"parentUuid":"b","type":"user","message":{"role":"user","content":"Fra"#.into(),
            rec(
                "d",
                Some("c"),
                "assistant",
                json!([{"type": "text", "text": "Antwort 2"}]),
            ),
            rec("e", Some("d"), "user", json!("Frage 3")),
        ]
        .join("\n");
        let t = parse_session(file.as_bytes(), S).unwrap();
        assert_eq!(texts(&t), ["Frage 1", "Antwort 1", "Antwort 2", "Frage 3"]);
        let corrupt: Vec<_> = t
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::CorruptLine)
            .collect();
        assert_eq!(corrupt.len(), 1);
        assert_eq!(corrupt[0].line, Some(3));
        assert!(corrupt[0].detail.contains("Zeile 3"));
        // Keine Inhalte in Warnungen.
        assert!(t.warnings.iter().all(|w| !w.detail.contains("Fra")));
    }

    #[test]
    fn har_023_unknown_format_fails_closed() {
        let err = parse_session(b"{\"foo\":1}\n{\"bar\":2}\n", S).unwrap_err();
        assert!(matches!(err, ImportError::UnknownFormat(_)), "{err}");
        let err = parse_session(b"", S).unwrap_err();
        assert!(matches!(err, ImportError::UnknownFormat(_)), "{err}");
    }

    #[test]
    fn har_023_meta_messages_are_skipped_and_raw_kept_for_single_event_lines() {
        let mut meta: Value =
            serde_json::from_str(&rec("m", None, "user", json!("Caveat"))).unwrap();
        meta["isMeta"] = json!(true);
        let file = [
            meta.to_string(),
            rec("a", Some("m"), "user", json!("Frage")),
            rec(
                "b",
                Some("a"),
                "assistant",
                json!([{"type": "text", "text": "x"}, {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}]),
            ),
        ]
        .join("\n");
        let t = parse_session(file.as_bytes(), S).unwrap();
        assert_eq!(texts(&t), ["Frage", "x"]);
        let user = t
            .events
            .iter()
            .find(|e| matches!(&e.payload, EventPayload::MessageCompleted(m) if m.role == MessageRole::User))
            .unwrap();
        assert!(user.raw.is_some() && user.by_user);
        // Zwei Events aus einer Zeile: kein `raw` (sonst doppelt beim Rebuild).
        assert!(
            t.events
                .iter()
                .filter(|e| e.payload.type_name() == "tool.call.requested")
                .all(|e| e.raw.is_none())
        );
    }
}
