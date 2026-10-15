//! Tool-Normalisierung (POL-005): kanonische Klasse `tool.kind`, Shell-Parsing
//! (`tool.command`, `tool.argv`, `tool.parse_uncertain`), Pfade (`tool.paths`, absolut und
//! normalisiert) und URLs (`tool.urls`).
//!
//! Die Abbildung nativer Tools liegt als Daten in `data/tool-map.yaml` (AC3). Die
//! Normalisierung ist rein (ohne IO); Symlinks löst sie nicht auf.

pub mod shell;

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cel::ext::normalize_path;

/// Die eingebettete Abbildungstabelle.
const TOOL_MAP_YAML: &str = include_str!("../../data/tool-map.yaml");

/// Ein Tool-Call, wie ihn ein Adapter meldet.
#[derive(Debug, Clone, Copy)]
pub struct RawToolCall<'a> {
    /// Harness-Familie: `claude`, `codex`, `acp`, `direct`, …
    pub harness: &'a str,
    /// Nativer Tool-Name (bei ACP der Titel).
    pub native_name: &'a str,
    /// ACP `tool_call.kind`.
    pub acp_kind: Option<&'a str>,
    /// MCP-Server, falls der Adapter ihn separat meldet (Codex, Direkt-Harness).
    pub mcp_server: Option<&'a str>,
    /// Argumente des Tool-Calls.
    pub args: &'a Value,
    /// Arbeitsverzeichnis der Session (absolut); Basis für relative Pfade.
    pub cwd: &'a str,
    /// Home-Verzeichnis für `~`, falls bekannt.
    pub home: Option<&'a str>,
    /// Server, die beton selbst eingebunden hat (Relay-Hub, Workspace-Tools des Direkt-
    /// Harness). Nur für sie gelten die Klassen der reservierten Namen (`beton*`) aus der
    /// Abbildungstabelle; ein gleichnamiger fremder Server ist `mcp` (#234). `None`: keine
    /// Herkunftsprüfung (Policy-Tests ohne Session).
    pub trusted_servers: Option<&'a [String]>,
}

/// Reservierter Server-Name von beton (`beton`, `beton-*`, `beton_*`).
pub fn reserved_server(name: &str) -> bool {
    name == "beton" || name.starts_with("beton-") || name.starts_with("beton_")
}

/// Fehler der Normalisierung; der Aufrufer lehnt dann ab (fail closed).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Tool-Normalisierung: {0}")]
pub struct NormalizeError(pub String);

/// Ein normalisierter Tool-Call (Felder von `tool.*` in POL-004).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedTool {
    /// Tool-Name; bei MCP-Tools ohne Server-Präfix.
    pub name: String,
    pub native_name: String,
    /// MCP-Server oder `builtin`.
    pub server: String,
    pub kind: String,
    pub args: Value,
    pub command: Option<String>,
    pub argv: Vec<Vec<String>>,
    pub cwd: String,
    pub paths: Vec<String>,
    pub urls: Vec<String>,
    pub is_subagent_spawn: bool,
    pub parse_uncertain: bool,
}

impl NormalizedTool {
    /// Das `tool`-Objekt für den Auswertungskontext.
    pub fn to_context(&self) -> Value {
        json!({
            "name": self.name,
            "native_name": self.native_name,
            "server": self.server,
            "kind": self.kind,
            "args": self.args,
            "command": self.command,
            "argv": self.argv,
            "cwd": self.cwd,
            "paths": self.paths,
            "urls": self.urls,
            "is_subagent_spawn": self.is_subagent_spawn,
            "parse_uncertain": self.parse_uncertain,
        })
    }
}

/// Eintrag der Abbildungstabelle für ein Tool.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    pub kind: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub changes: Option<String>,
    #[serde(default)]
    pub subagent_spawn: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessMap {
    #[serde(default)]
    mcp_prefix: Option<String>,
    #[serde(default)]
    by_kind: bool,
    tools: BTreeMap<String, ToolSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerMap {
    kind: String,
    #[serde(default)]
    tools: BTreeMap<String, ToolSpec>,
}

/// Die Abbildungstabelle aus `data/tool-map.yaml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolMap {
    spec_version: u32,
    kinds: Vec<String>,
    harnesses: BTreeMap<String, HarnessMap>,
    mcp_servers: BTreeMap<String, ServerMap>,
}

/// Treffer in der Tabelle: Eintrag, Server und Tool-Name (ohne Präfix).
struct Lookup<'a> {
    spec: std::borrow::Cow<'a, ToolSpec>,
    server: String,
    name: String,
}

impl ToolMap {
    fn parse(text: &str) -> Result<Self, String> {
        let doc = crate::yaml::parse(text).map_err(|e| e.message)?;
        let map: ToolMap = serde_json::from_value(doc.value).map_err(|e| e.to_string())?;
        if map.spec_version != 1 {
            return Err(format!(
                "spec_version {} nicht unterstützt",
                map.spec_version
            ));
        }
        let all_specs = map
            .harnesses
            .values()
            .flat_map(|h| h.tools.values())
            .chain(map.mcp_servers.values().flat_map(|s| s.tools.values()));
        for spec in all_specs {
            if !map.kinds.contains(&spec.kind) {
                return Err(format!("unbekannte Klasse `{}`", spec.kind));
            }
        }
        for server in map.mcp_servers.values() {
            if !map.kinds.contains(&server.kind) {
                return Err(format!("unbekannte Klasse `{}`", server.kind));
            }
        }
        Ok(map)
    }

    fn lookup(
        &self,
        harness: &str,
        name: &str,
        acp_kind: Option<&str>,
        server: Option<&str>,
    ) -> Option<Lookup<'_>> {
        let table = self.harnesses.get(harness);
        if let Some(server) = server {
            // Der Claude-Hook meldet den vollen Namen `mcp__<server>__<tool>` und den Server
            // (#232): `tool.name` ist der Name ohne Präfix.
            let prefixed = table
                .and_then(|t| t.mcp_prefix.as_deref())
                .and_then(|p| name.strip_prefix(p))
                .and_then(|rest| rest.strip_prefix(server))
                .and_then(|rest| rest.strip_prefix("__"))
                .filter(|tool| !tool.is_empty());
            return Some(self.server_tool(server, prefixed.unwrap_or(name)));
        }
        let table = table?;
        let key = if table.by_kind { acp_kind? } else { name };
        if let Some(spec) = table.tools.get(key) {
            return Some(Lookup {
                spec: std::borrow::Cow::Borrowed(spec),
                server: "builtin".into(),
                name: name.into(),
            });
        }
        let rest = name.strip_prefix(table.mcp_prefix.as_deref()?)?;
        let (server, tool) = rest.split_once("__")?;
        Some(self.server_tool(server, tool))
    }

    fn server_tool(&self, server: &str, tool: &str) -> Lookup<'_> {
        let spec = match self.mcp_servers.get(server) {
            Some(s) => match s.tools.get(tool) {
                Some(spec) => std::borrow::Cow::Borrowed(spec),
                None => std::borrow::Cow::Owned(ToolSpec {
                    kind: s.kind.clone(),
                    ..ToolSpec::default()
                }),
            },
            None => std::borrow::Cow::Owned(ToolSpec {
                kind: "mcp".into(),
                ..ToolSpec::default()
            }),
        };
        Lookup {
            spec,
            server: server.into(),
            name: tool.into(),
        }
    }

    /// Klassen aller Tools mit diesem Namen (über alle Harnesses und MCP-Server; POL-024,
    /// statische Analyse von `tool.name == …`). Leer, wenn der Name unbekannt ist.
    pub fn kinds_of_name(&self, name: &str) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for table in self.harnesses.values().filter(|t| !t.by_kind) {
            if let Some(spec) = table.tools.get(name) {
                out.insert(spec.kind.clone());
            }
        }
        for server in self.mcp_servers.values() {
            if let Some(spec) = server.tools.get(name) {
                out.insert(spec.kind.clone());
            }
        }
        out
    }

    /// Kanonische Klasse oder `None`, wenn das Tool nicht abgebildet ist.
    pub fn kind_of(
        &self,
        harness: &str,
        name: &str,
        acp_kind: Option<&str>,
        server: Option<&str>,
    ) -> Option<&str> {
        let found = self.lookup(harness, name, acp_kind, server)?;
        // Die Klasse steht immer in `kinds`; so ist der Rückgabewert an `self` gebunden.
        self.kinds
            .iter()
            .find(|k| **k == found.spec.kind)
            .map(String::as_str)
    }
}

static TOOL_MAP: LazyLock<Result<ToolMap, String>> =
    LazyLock::new(|| ToolMap::parse(TOOL_MAP_YAML));

/// Die eingebettete Tabelle; ein Fehler darin lässt jede Normalisierung scheitern.
pub fn tool_map() -> Result<&'static ToolMap, NormalizeError> {
    TOOL_MAP
        .as_ref()
        .map_err(|e| NormalizeError(format!("data/tool-map.yaml: {e}")))
}

/// Herkunft eines Tool-Calls ohne seine Argumente: damit lässt sich ein Call nach einer
/// Änderung der Argumente (`modify` mit `args`, Pass 1) neu normalisieren (#182).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolSource {
    pub harness: String,
    pub native_name: String,
    pub acp_kind: Option<String>,
    pub mcp_server: Option<String>,
    pub cwd: String,
    pub home: Option<String>,
    /// Siehe [`RawToolCall::trusted_servers`].
    pub trusted_servers: Option<Vec<String>>,
}

impl ToolSource {
    pub fn raw<'a>(&'a self, args: &'a Value) -> RawToolCall<'a> {
        RawToolCall {
            harness: &self.harness,
            native_name: &self.native_name,
            acp_kind: self.acp_kind.as_deref(),
            mcp_server: self.mcp_server.as_deref(),
            args,
            cwd: &self.cwd,
            home: self.home.as_deref(),
            trusted_servers: self.trusted_servers.as_deref(),
        }
    }
}

/// Normalisiert einen Tool-Call aus seiner Herkunft und den Argumenten.
pub fn normalize_source(
    source: &ToolSource,
    args: &Value,
) -> Result<NormalizedTool, NormalizeError> {
    normalize(&source.raw(args))
}

/// Harness-Familie für die Abbildungstabelle: `acp:gemini` → `acp`, `direct:x` → `direct`.
pub fn harness_family(harness: &str) -> &str {
    harness.split(':').next().unwrap_or(harness)
}

/// Normalisiert einen Tool-Call.
pub fn normalize(raw: &RawToolCall) -> Result<NormalizedTool, NormalizeError> {
    let map = tool_map()?;
    if normalize_path(raw.cwd).is_none() {
        return Err(NormalizeError(format!(
            "Arbeitsverzeichnis `{}` ist nicht absolut",
            raw.cwd
        )));
    }
    let found = map.lookup(raw.harness, raw.native_name, raw.acp_kind, raw.mcp_server);
    let (spec, server, name) = match found {
        // Reservierter Name, aber nicht von beton eingebunden: gewöhnliches MCP-Tool (#234).
        Some(l)
            if l.server != "builtin"
                && reserved_server(&l.server)
                && raw.trusted_servers.is_some_and(|t| !t.contains(&l.server)) =>
        {
            (
                ToolSpec {
                    kind: "mcp".into(),
                    ..ToolSpec::default()
                },
                l.server,
                l.name,
            )
        }
        Some(l) => (l.spec.into_owned(), l.server, l.name),
        None => (
            ToolSpec {
                kind: "other".into(),
                ..ToolSpec::default()
            },
            raw.mcp_server.unwrap_or("builtin").to_owned(),
            raw.native_name.to_owned(),
        ),
    };
    let cwd = match raw.args.get("cwd").and_then(Value::as_str) {
        Some(c) if spec.command.is_some() && c.starts_with('/') => {
            normalize_path(c).unwrap_or_else(|| raw.cwd.to_owned())
        }
        _ => normalize_path(raw.cwd).unwrap_or_default(),
    };
    let abs = |p: &str| absolute(p, &cwd, raw.home);

    let mut paths: Vec<String> = Vec::new();
    let push = |p: Option<String>, paths: &mut Vec<String>| {
        if let Some(p) = p
            && !paths.contains(&p)
        {
            paths.push(p);
        }
    };
    for key in &spec.paths {
        match raw.args.get(key) {
            Some(Value::String(p)) => push(abs(p), &mut paths),
            Some(Value::Array(items)) => {
                for p in items.iter().filter_map(Value::as_str) {
                    push(abs(p), &mut paths);
                }
            }
            _ => {}
        }
    }
    if let Some(key) = &spec.patch
        && let Some(text) = raw.args.get(key).and_then(Value::as_str)
    {
        for p in patch_paths(text) {
            push(abs(&p), &mut paths);
        }
    }
    if let Some(key) = &spec.changes
        && let Some(items) = raw.args.get(key).and_then(Value::as_array)
    {
        for item in items {
            if let Some(p) = item.get("path").and_then(Value::as_str) {
                push(abs(p), &mut paths);
            }
            if let Some(p) = item.pointer("/kind/move_path").and_then(Value::as_str) {
                push(abs(p), &mut paths);
            }
        }
    }

    let command = spec
        .command
        .as_ref()
        .and_then(|key| raw.args.get(key))
        .and_then(command_string);
    let mut argv = Vec::new();
    let mut parse_uncertain = false;
    if let Some(cmd) = &command {
        let parsed = shell::parse(cmd);
        for target in &parsed.redirects {
            if target != "/dev/null" {
                push(abs(target), &mut paths);
            }
        }
        for sub in &parsed.argv {
            for word in sub.iter().skip(1) {
                if looks_like_path(word) {
                    push(abs(word), &mut paths);
                }
            }
        }
        argv = parsed.argv;
        parse_uncertain = parsed.uncertain;
    }

    Ok(NormalizedTool {
        name,
        native_name: raw.native_name.to_owned(),
        server,
        kind: spec.kind,
        args: raw.args.clone(),
        command,
        argv,
        cwd,
        paths,
        urls: urls(raw.args),
        is_subagent_spawn: spec.subagent_spawn,
        parse_uncertain,
    })
}

/// Das Kommando als String; Listen (`["bash", "-lc", "…"]`) werden shell-gequotet verbunden.
fn command_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let words: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
            Some(
                words?
                    .iter()
                    .map(|w| quote(w))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
        _ => None,
    }
}

fn quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));
    if safe {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn looks_like_path(word: &str) -> bool {
    (word.starts_with('/')
        || word.starts_with("./")
        || word.starts_with("../")
        || word.starts_with("~/")
        || word == "~")
        && !word.contains("://")
}

/// Absoluter, lexikalisch normalisierter Pfad.
fn absolute(path: &str, cwd: &str, home: Option<&str>) -> Option<String> {
    let expanded = if path == "~" {
        home?.to_owned()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{rest}", home?)
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{cwd}/{path}")
    };
    normalize_path(&expanded)
}

/// Pfade aus einem `apply_patch`-Text (`*** Add/Update/Delete File:`, `*** Move to:`).
fn patch_paths(text: &str) -> Vec<String> {
    const HEADERS: [&str; 4] = [
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    text.lines()
        .filter_map(|line| {
            HEADERS
                .iter()
                .find_map(|h| line.strip_prefix(h))
                .map(|p| p.trim().to_owned())
        })
        .filter(|p| !p.is_empty())
        .collect()
}

static URL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?i)\bhttps?://[^\s"'<>()\[\]{}`]+"#).ok());

/// URLs aus allen String-Werten der Argumente (begrenzt auf 1 000 Strings), sortiert.
fn urls(args: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![args];
    let mut seen = 0;
    let Some(re) = URL_RE.as_ref() else {
        return out;
    };
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) => {
                seen += 1;
                for m in re.find_iter(s) {
                    let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
                    if !out.iter().any(|u| u == url) {
                        out.push(url.to_owned());
                    }
                }
            }
            Value::Array(items) => stack.extend(items.iter().rev()),
            Value::Object(map) => stack.extend(map.values().rev()),
            _ => {}
        }
        if seen >= 1000 {
            break;
        }
    }
    out.sort();
    out
}
