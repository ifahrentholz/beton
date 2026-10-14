//! Der eingebaute MCP-Server `beton` mit den System-Tools (AGT-007).
//!
//! Sichtbar sind nur freigeschaltete Tools: `tools.system` des Agents wählt aus, die
//! Freischaltungsregeln (Tabelle in `docs/spec/02-agents.md`) begrenzen. Ein direkter Aufruf
//! eines nicht freigeschalteten Tools wird mit `tool_not_enabled` abgelehnt (fail closed).
//!
//! Verfügbar: `policy_query`, `session_spawn` (synchron oder mit `async: true` im
//! Hintergrund), `session_send`, `session_wait`, `session_status`, `session_list`,
//! `session_cancel` (Sub-Agents, AGT-009), `skill_load`, `skill_read_file` sowie seit M2
//! `inbox_read` und `ask_user` (Inbox, UX-001; immer sichtbar). `timer_*` und `schedule_*`
//! folgen ab M5 (ASY-003, ASY-011). Die `session_*`-Tools führt der Server aus (`system.call`, PROTO-015); er prüft
//! Grenzen (`max_depth`, `max_concurrent`) und dass nur eigene Childs angesprochen werden.

use std::sync::Arc;

use async_trait::async_trait;
use beton_agents::AgentSpec;
use beton_agents::spec::SystemTool;
use serde_json::{Value, json};

use crate::protocol::{self, Kind, codes};
use crate::skills::{self, SkillSet};

/// System-Tools, die diese Version anbietet.
pub const AVAILABLE: [SystemTool; 11] = [
    SystemTool::PolicyQuery,
    SystemTool::InboxRead,
    SystemTool::AskUser,
    SystemTool::SessionSpawn,
    SystemTool::SessionSend,
    SystemTool::SessionWait,
    SystemTool::SessionStatus,
    SystemTool::SessionList,
    SystemTool::SessionCancel,
    SystemTool::SkillLoad,
    SystemTool::SkillReadFile,
];

/// Die Tools, die nur mit `session_spawn` freigeschaltet sind (AGT-009).
const CHILD_TOOLS: [SystemTool; 5] = [
    SystemTool::SessionSend,
    SystemTool::SessionWait,
    SystemTool::SessionStatus,
    SystemTool::SessionList,
    SystemTool::SessionCancel,
];

/// Name eines System-Tools, z. B. `session_spawn`.
pub fn tool_name(tool: SystemTool) -> String {
    serde_json::to_value(tool)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn tool_from_name(name: &str) -> Option<SystemTool> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

/// Ist die Freischaltungsregel des Tools erfüllt?
fn unlocked(tool: SystemTool, agent: Option<&AgentSpec>, skills_active: bool) -> bool {
    match tool {
        SystemTool::PolicyQuery | SystemTool::InboxRead | SystemTool::AskUser => true,
        SystemTool::SkillLoad | SystemTool::SkillReadFile => skills_active,
        SystemTool::SessionSpawn
        | SystemTool::SessionSend
        | SystemTool::SessionWait
        | SystemTool::SessionStatus
        | SystemTool::SessionList
        | SystemTool::SessionCancel => {
            agent.is_some_and(|a| a.spawn.as_ref().is_some_and(|s| !s.agents.is_empty()))
        }
        _ => false,
    }
}

/// Sichtbare System-Tools einer Session (AGT-007). Ohne Agent: `policy_query`, `inbox_read`,
/// `ask_user` und, wenn Skills aktiv sind, `skill_load`/`skill_read_file`. Mit Agent wählt `tools.system` aus
/// (leer: alle verfügbaren); Skill-Tools kommen mit aktiven Skills immer hinzu. Jede Auswahl
/// gilt nur, wenn die Freischaltungsregel erfüllt ist; Tools späterer Meilensteine bleiben
/// unsichtbar.
pub fn enabled_tools(agent: Option<&AgentSpec>, skills_active: bool) -> Vec<SystemTool> {
    let requested: Vec<SystemTool> = match agent.and_then(|a| a.tools.as_ref()) {
        Some(t) if !t.system.is_empty() => t.system.clone(),
        _ => AVAILABLE.to_vec(),
    };
    // Wer `session_spawn` auswählt, bekommt die Tools für seine Childs dazu (AGT-009).
    let spawn_selected = requested.contains(&SystemTool::SessionSpawn);
    AVAILABLE
        .into_iter()
        .filter(|t| {
            let skill_tool = matches!(t, SystemTool::SkillLoad | SystemTool::SkillReadFile);
            let child_tool = spawn_selected && CHILD_TOOLS.contains(t);
            (requested.contains(t) || skill_tool || child_tool)
                && unlocked(*t, agent, skills_active)
        })
        .collect()
}

/// Ein Sub-Agent, den `session_spawn` starten darf (aus `spawn.agents`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnTarget {
    /// Schlüssel unter `agents`, z. B. `quick-check`.
    pub name: String,
    pub description: Option<String>,
    /// Harness des Childs laut `executor.harness`.
    pub harness: String,
    pub model: Option<String>,
    /// Grund, falls der Harness auf diesem Host nicht eingerichtet ist (nicht installiert,
    /// nicht angemeldet); der Runner prüft das beim Start (AGT-011, AGT-012 Preflight).
    pub unavailable: Option<String>,
}

/// Fehler eines Tool-Aufrufs, wie ihn das Modell sieht (`isError: true`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    pub code: String,
    pub message: String,
}

impl ToolFailure {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Was der Runner für die System-Tools bereitstellt.
#[async_trait]
pub trait SystemBackend: Send + Sync {
    /// „Wäre Aktion X erlaubt?“ → Entscheidung und Begründung (ohne Seiteneffekte).
    async fn policy_query(&self, args: &Value) -> Result<Value, ToolFailure>;
    /// Child-Session für einen erlaubten Sub-Agent starten; synchron mit ihrem Ergebnis, mit
    /// `async: true` sofort mit `session_id`.
    async fn session_spawn(&self, target: &SpawnTarget, args: &Value)
    -> Result<Value, ToolFailure>;
    /// `session_send`, `session_wait`, `session_status`, `session_list`, `session_cancel`
    /// (AGT-009): führt der Server für die eigenen Childs aus.
    async fn session_tool(&self, tool: SystemTool, args: &Value) -> Result<Value, ToolFailure>;
    /// `ask_user` (AGT-007 AC4): Inbox-Frage an den Menschen; blockiert bis zur Antwort oder
    /// bis `timeout` (`None`: keine Antwort). Ohne Implementierung: abgelehnt.
    async fn ask_user(
        &self,
        _question: &str,
        _choices: &[String],
        _timeout: std::time::Duration,
    ) -> Result<Option<String>, ToolFailure> {
        Err(ToolFailure::new(
            "unavailable",
            "Fragen sind hier nicht möglich",
        ))
    }
    /// `inbox_read`: ungelesene Nachrichten an diese Session (Agent-Inbox), danach gelesen.
    async fn inbox_read(&self, _args: &Value) -> Result<Value, ToolFailure> {
        Err(ToolFailure::new(
            "unavailable",
            "Keine Agent-Inbox verfügbar",
        ))
    }
}

/// Frist von `ask_user` ohne Angabe bzw. höchstens.
pub const ASK_DEFAULT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
pub const ASK_MAX: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// Länge der Frage bzw. Zahl und Länge der Antwortvorschläge.
const QUESTION_MAX: usize = 4096;
const CHOICES_MAX: usize = 10;
const CHOICE_MAX: usize = 200;

/// Der MCP-Server `beton` einer Session.
pub struct SystemServer {
    tools: Vec<SystemTool>,
    skills: SkillSet,
    spawn: Vec<SpawnTarget>,
    backend: Arc<dyn SystemBackend>,
}

impl std::fmt::Debug for SystemServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemServer")
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl SystemServer {
    pub fn new(
        tools: Vec<SystemTool>,
        skills: SkillSet,
        spawn: Vec<SpawnTarget>,
        backend: Arc<dyn SystemBackend>,
    ) -> Self {
        Self {
            tools,
            skills,
            spawn,
            backend,
        }
    }

    /// Namen der sichtbaren Tools.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|t| tool_name(*t)).collect()
    }

    fn definition(&self, tool: SystemTool) -> Value {
        let name = tool_name(tool);
        match tool {
            SystemTool::PolicyQuery => json!({
                "name": name,
                "description": "Fragt beton, ob eine geplante Aktion erlaubt wäre, und liefert Entscheidung und Begründung. Führt nichts aus.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "description": "Die geplante Aktion, z. B. `git push origin main`."},
                        "tool": {"type": "string", "description": "Tool, mit dem sie ausgeführt würde, z. B. `Bash`."},
                        "args": {"type": "object", "description": "Argumente des Tool-Calls."}
                    },
                    "required": ["action"]
                }
            }),
            SystemTool::SessionSpawn => {
                let names: Vec<&str> = self.spawn.iter().map(|s| s.name.as_str()).collect();
                let list = self
                    .spawn
                    .iter()
                    .map(|s| {
                        let harness = match &s.unavailable {
                            Some(_) => format!("{}, hier nicht eingerichtet", s.harness),
                            None => s.harness.clone(),
                        };
                        match &s.description {
                            Some(d) => format!("- {} ({harness}): {d}", s.name),
                            None => format!("- {} ({harness})", s.name),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({
                    "name": name,
                    "description": format!("Startet eine Child-Session (eigene Session, ggf. auf einem anderen Harness) für einen erlaubten Sub-Agent. Ohne `async` wartet der Aufruf auf das Ende ihres Auftrags und liefert die Abschlussnachricht; mit `async: true` kehrt er sofort mit `session_id` zurück, das Ergebnis holst du mit session_wait.\nErlaubte Agents:\n{list}"),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "agent": {"type": "string", "enum": names},
                            "prompt": {"type": "string", "description": "Auftrag an den Sub-Agent."},
                            "params": {"type": "object", "description": "Parameter des Sub-Agents (AGT-010), z. B. {\"branch\": \"main\"}."},
                            "async": {"type": "boolean", "description": "`true`: im Hintergrund starten und sofort zurückkehren (Ergebnis über session_wait). Default `false`."},
                            "worktree": {"type": "string", "enum": ["new", "inherit", "none"], "description": "Workspace des Childs: `new` = eigener Git-Worktree mit eigenem Branch vom aktuellen HEAD, `inherit` = dein Workspace, `none` = dein Workspace nur zum Lesen. Default: `spawn.worktree` des Agents."}
                        },
                        "required": ["agent", "prompt"]
                    }
                })
            }
            SystemTool::SessionSend => json!({
                "name": name,
                "description": "Gibt einer eigenen Child-Session einen weiteren Auftrag (z. B. Review-Befunde beheben). Kehrt sofort zurück; das Ergebnis holst du mit session_wait. Läuft das Child noch, wird abgelehnt.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string", "description": "ID des Childs aus session_spawn."},
                        "text": {"type": "string", "description": "Der Auftrag."}
                    },
                    "required": ["session_id", "text"]
                }
            }),
            SystemTool::SessionWait => json!({
                "name": name,
                "description": "Wartet auf die Ergebnisse eigener Child-Sessions: `mode: all` (Default) bis alle fertig sind, `any` bis eine fertig ist. Ohne `ids` alle laufenden Childs. Nach `timeout` (Default 30m, höchstens 24h) kommt der Zwischenstand (`done: false`).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ids": {"type": "array", "items": {"type": "string"}, "description": "IDs der Childs; leer = alle laufenden."},
                        "mode": {"type": "string", "enum": ["all", "any"]},
                        "timeout": {"type": "string", "description": "Dauer wie `90s`, `30m`, `2h`."}
                    }
                }
            }),
            SystemTool::SessionStatus => json!({
                "name": name,
                "description": "Status einer eigenen Child-Session: Agent, Harness, Status des Auftrags, Ergebnis und Worktree.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"session_id": {"type": "string"}},
                    "required": ["session_id"]
                }
            }),
            SystemTool::SessionList => json!({
                "name": name,
                "description": "Listet deine Child-Sessions mit Status sowie die startbaren Sub-Agents mit Harness und ob dieser Harness hier eingerichtet ist (`available`). Nutze es als Vorab-Prüfung, bevor du Arbeit verteilst.",
                "inputSchema": {"type": "object", "properties": {}}
            }),
            SystemTool::SessionCancel => json!({
                "name": name,
                "description": "Bricht eine laufende eigene Child-Session ab (samt ihrer eigenen Childs).",
                "inputSchema": {
                    "type": "object",
                    "properties": {"session_id": {"type": "string"}},
                    "required": ["session_id"]
                }
            }),
            SystemTool::SkillLoad => {
                let names: Vec<&str> = self
                    .skills
                    .model_invocable()
                    .map(|s| s.name.as_str())
                    .collect();
                json!({
                    "name": name,
                    "description": format!("Lädt die Anleitung eines Skills (SKILL.md ohne Frontmatter). Verfügbare Skills:\n{}", skills::index(&self.skills)),
                    "inputSchema": {
                        "type": "object",
                        "properties": {"name": {"type": "string", "enum": names}},
                        "required": ["name"]
                    }
                })
            }
            SystemTool::AskUser => json!({
                "name": name,
                "description": "Stellt dem Menschen eine Frage (erscheint in seiner Inbox und in der Session) und wartet auf die Antwort. Nutze es nur, wenn du ohne Entscheidung des Menschen nicht sinnvoll weiterarbeiten kannst. Nach `timeout` (Default 30m, höchstens 24h) kommt `answered: false`.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "question": {"type": "string", "description": "Die Frage, mit dem nötigen Kontext."},
                        "options": {"type": "array", "items": {"type": "string"}, "description": "Antwortvorschläge (höchstens 10); der Mensch kann auch frei antworten."},
                        "timeout": {"type": "string", "description": "Dauer wie `90s`, `30m`, `2h`."}
                    },
                    "required": ["question"]
                }
            }),
            SystemTool::InboxRead => json!({
                "name": name,
                "description": "Liest ungelesene Nachrichten an diese Session (Agent-Inbox, z. B. Ergebnisse von Hintergrund-Agents). Jede Nachricht kommt genau einmal.",
                "inputSchema": {"type": "object", "properties": {}}
            }),
            SystemTool::SkillReadFile => json!({
                "name": name,
                "description": "Liest eine Zusatzdatei eines Skills, z. B. `scripts/run.sh` (nur innerhalb des Skill-Ordners).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Name des Skills."},
                        "path": {"type": "string", "description": "Pfad relativ zum Skill-Ordner."}
                    },
                    "required": ["name", "path"]
                }
            }),
            _ => Value::Null,
        }
    }

    /// Verarbeitet eine JSON-RPC-Nachricht des Harness; `None` für Notifications.
    pub async fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        match protocol::kind(msg) {
            Kind::Notification | Kind::Response => return None,
            Kind::Invalid => {
                return Some(protocol::error(
                    &id,
                    codes::INVALID_REQUEST,
                    "ungültige JSON-RPC-Nachricht",
                    None,
                ));
            }
            Kind::Request => {}
        }
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let reply = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => protocol::result(
                &id,
                json!({
                    "protocolVersion": protocol::negotiate(params["protocolVersion"].as_str()),
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": crate::config::SYSTEM_SERVER, "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "System-Tools von beton für diese Session.",
                }),
            ),
            "ping" => protocol::result(&id, json!({})),
            "tools/list" => {
                let tools: Vec<Value> = self.tools.iter().map(|t| self.definition(*t)).collect();
                protocol::result(&id, json!({"tools": tools}))
            }
            "tools/call" => self.call(&id, &params).await,
            other => protocol::error(
                &id,
                codes::METHOD_NOT_FOUND,
                format!("Methode {other} gibt es nicht"),
                None,
            ),
        };
        Some(reply)
    }

    async fn call(&self, id: &Value, params: &Value) -> Value {
        let name = params["name"].as_str().unwrap_or_default();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let Some(tool) = tool_from_name(name).filter(|t| self.tools.contains(t)) else {
            return protocol::error(
                id,
                codes::INVALID_PARAMS,
                format!("tool_not_enabled: {name} ist für diese Session nicht freigeschaltet"),
                Some(json!({"code": "tool_not_enabled", "tool": name})),
            );
        };
        let outcome = match tool {
            SystemTool::PolicyQuery => self.backend.policy_query(&args).await,
            SystemTool::SessionSpawn => self.spawn(&args).await,
            SystemTool::SessionSend
            | SystemTool::SessionWait
            | SystemTool::SessionStatus
            | SystemTool::SessionCancel => self.backend.session_tool(tool, &args).await,
            SystemTool::SessionList => self.list(&args).await,
            SystemTool::AskUser => self.ask(&args).await,
            SystemTool::InboxRead => self.backend.inbox_read(&args).await,
            SystemTool::SkillLoad => {
                let name = args["name"].as_str().unwrap_or_default();
                return protocol::result(
                    id,
                    match self.skills.load(name) {
                        Ok(text) => protocol::text_result(text, false),
                        Err(e) => protocol::text_result(e.to_string(), true),
                    },
                );
            }
            SystemTool::SkillReadFile => {
                let name = args["name"].as_str().unwrap_or_default();
                let path = args["path"].as_str().unwrap_or_default();
                let reply = match self.skills.read_file(name, path) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => protocol::text_result(text, false),
                        Err(_) => protocol::text_result(
                            "binary_file: skill_read_file liefert nur Textdateien",
                            true,
                        ),
                    },
                    Err(e) => protocol::text_result(e.to_string(), true),
                };
                return protocol::result(id, reply);
            }
            _ => Err(ToolFailure::new("tool_not_enabled", name)),
        };
        protocol::result(
            id,
            match outcome {
                Ok(v) => protocol::structured_result(&v),
                Err(f) => protocol::text_result(format!("{}: {}", f.code, f.message), true),
            },
        )
    }

    /// `ask_user`: prüft Frage, Vorschläge und Frist, dann fragt der Runner den Menschen.
    async fn ask(&self, args: &Value) -> Result<Value, ToolFailure> {
        let invalid = |m: &str| ToolFailure::new("invalid_args", m);
        let question = args["question"]
            .as_str()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| invalid("`question` fehlt"))?;
        if question.chars().count() > QUESTION_MAX {
            return Err(invalid("`question` ist länger als 4096 Zeichen"));
        }
        let choices: Vec<String> = match args.get("options") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|c| !c.is_empty() && c.chars().count() <= CHOICE_MAX)
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("`options` enthält nur Texte bis 200 Zeichen"))
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(invalid("`options` muss eine Liste sein")),
        };
        if choices.len() > CHOICES_MAX {
            return Err(invalid("höchstens 10 `options`"));
        }
        let timeout = match args.get("timeout").and_then(Value::as_str) {
            None => ASK_DEFAULT,
            Some(t) => serde_json::from_value::<beton_agents::spec::DurationText>(Value::String(
                t.to_owned(),
            ))
            .ok()
            .and_then(|d| d.to_duration())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| invalid("`timeout` ist keine Dauer wie `30m`"))?
            .min(ASK_MAX),
        };
        let answer = self.backend.ask_user(question, &choices, timeout).await?;
        Ok(json!({
            "answered": answer.is_some(),
            "answer": answer,
        }))
    }

    /// `session_list`: Childs vom Server, startbare Sub-Agents samt Verfügbarkeit von hier.
    async fn list(&self, args: &Value) -> Result<Value, ToolFailure> {
        let mut out = self
            .backend
            .session_tool(SystemTool::SessionList, args)
            .await?;
        let agents: Vec<Value> = self
            .spawn
            .iter()
            .map(|t| {
                let mut a = json!({
                    "name": t.name,
                    "harness": t.harness,
                    "available": t.unavailable.is_none(),
                });
                if let Some(why) = &t.unavailable {
                    a["reason"] = Value::String(why.clone());
                }
                a
            })
            .collect();
        if let Value::Object(m) = &mut out {
            m.insert("agents".into(), Value::Array(agents));
        }
        Ok(out)
    }

    async fn spawn(&self, args: &Value) -> Result<Value, ToolFailure> {
        let agent = args["agent"].as_str().unwrap_or_default();
        let target = self.spawn.iter().find(|t| t.name == agent).ok_or_else(|| {
            ToolFailure::new(
                "spawn_denied",
                format!("„{agent}“ steht nicht in spawn.agents"),
            )
        })?;
        if !matches!(args["async"], Value::Null | Value::Bool(_)) {
            return Err(ToolFailure::new(
                "invalid_args",
                "`async` muss true oder false sein",
            ));
        }
        match &args["worktree"] {
            Value::Null => {}
            Value::String(w) if matches!(w.as_str(), "new" | "inherit" | "none") => {}
            _ => {
                return Err(ToolFailure::new(
                    "invalid_args",
                    "`worktree` ist `new`, `inherit` oder `none`",
                ));
            }
        }
        if args["prompt"].as_str().is_none_or(|p| p.trim().is_empty()) {
            return Err(ToolFailure::new("invalid_args", "`prompt` fehlt"));
        }
        if let Some(why) = &target.unavailable {
            return Err(ToolFailure::new(
                "harness_unavailable",
                format!("„{agent}“ läuft auf {}: {why}", target.harness),
            ));
        }
        self.backend.session_spawn(target, args).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        spawned: Mutex<Vec<(String, Value)>>,
        asked: Mutex<Vec<(String, Vec<String>, std::time::Duration)>>,
    }

    #[async_trait]
    impl SystemBackend for Recorder {
        async fn policy_query(&self, args: &Value) -> Result<Value, ToolFailure> {
            Ok(json!({"decision": "ask", "echo": args}))
        }
        async fn session_spawn(
            &self,
            target: &SpawnTarget,
            args: &Value,
        ) -> Result<Value, ToolFailure> {
            self.spawned
                .lock()
                .unwrap()
                .push((target.name.clone(), args.clone()));
            Ok(json!({"session_id": "ses_x", "status": "completed", "result": "fertig"}))
        }
        async fn session_tool(&self, tool: SystemTool, args: &Value) -> Result<Value, ToolFailure> {
            self.spawned
                .lock()
                .unwrap()
                .push((tool_name(tool), args.clone()));
            Ok(json!({"tool": tool_name(tool), "children": []}))
        }
        async fn ask_user(
            &self,
            question: &str,
            choices: &[String],
            timeout: std::time::Duration,
        ) -> Result<Option<String>, ToolFailure> {
            self.asked
                .lock()
                .unwrap()
                .push((question.to_owned(), choices.to_vec(), timeout));
            Ok(choices.first().cloned())
        }
        async fn inbox_read(&self, _args: &Value) -> Result<Value, ToolFailure> {
            Ok(json!({"messages": []}))
        }
    }

    fn agent(yaml: &str) -> AgentSpec {
        let parsed = beton_agents::load::parse_agent_yaml(yaml, "agent.yaml");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        parsed.spec.unwrap()
    }

    const BASE: &str = "spec_version: 1\nname: a\nexecutor: { harness: claude }\n";

    fn server(spec: Option<&AgentSpec>, backend: Arc<Recorder>) -> SystemServer {
        let spawn = spec
            .and_then(|s| s.spawn.as_ref())
            .map(|s| {
                s.agents
                    .iter()
                    .map(|n| SpawnTarget {
                        name: n.clone(),
                        description: None,
                        harness: "codex".into(),
                        model: None,
                        unavailable: (n == "offline").then(|| "nicht installiert".to_owned()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        SystemServer::new(
            enabled_tools(spec, false),
            SkillSet::default(),
            spawn,
            backend,
        )
    }

    async fn list(s: &SystemServer) -> Vec<String> {
        let r = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .await
            .unwrap();
        r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn call(s: &SystemServer, name: &str, args: Value) -> Value {
        s.handle(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn agt_007_ac1_session_spawn_is_hidden_and_rejected_without_spawn_agents() {
        let backend = Arc::new(Recorder::default());
        // Explizit angefordert, aber ohne spawn.agents: nicht freigeschaltet.
        let spec = agent(&format!(
            "{BASE}tools: {{ system: [session_spawn, policy_query] }}\n"
        ));
        let s = server(Some(&spec), backend.clone());
        assert_eq!(list(&s).await, ["policy_query"]);
        let reply = call(&s, "session_spawn", json!({"agent": "x", "prompt": "p"})).await;
        assert_eq!(
            reply["error"]["data"]["code"], "tool_not_enabled",
            "{reply}"
        );
        assert!(backend.spawned.lock().unwrap().is_empty());

        // Ohne Agent ebenso.
        let s = server(None, backend.clone());
        assert_eq!(list(&s).await, ["policy_query", "inbox_read", "ask_user"]);
        let reply = call(&s, "session_spawn", json!({"agent": "x", "prompt": "p"})).await;
        assert_eq!(reply["error"]["data"]["code"], "tool_not_enabled");
    }

    #[tokio::test]
    async fn agt_007_spawn_agents_unlock_session_spawn_for_listed_agents_only() {
        let backend = Arc::new(Recorder::default());
        let spec = agent(&format!(
            "{BASE}agents:\n  quick-check:\n    executor: {{ harness: codex }}\n  other:\n    executor: {{ harness: codex }}\nspawn: {{ agents: [quick-check] }}\n"
        ));
        let s = server(Some(&spec), backend.clone());
        assert_eq!(
            list(&s).await,
            [
                "policy_query",
                "inbox_read",
                "ask_user",
                "session_spawn",
                "session_send",
                "session_wait",
                "session_status",
                "session_list",
                "session_cancel"
            ]
        );
        let ok = call(
            &s,
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "prüfe", "async": false}),
        )
        .await;
        assert_eq!(
            ok["result"]["structuredContent"]["result"], "fertig",
            "{ok}"
        );
        let denied = call(
            &s,
            "session_spawn",
            json!({"agent": "other", "prompt": "p"}),
        )
        .await;
        assert_eq!(denied["result"]["isError"], true);
        assert!(
            denied["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("spawn_denied")
        );
        let bad = call(
            &s,
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "p", "worktree": "irgendwo"}),
        )
        .await;
        assert_eq!(bad["result"]["isError"], true);
        let in_background = call(
            &s,
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "p", "async": true, "worktree": "new"}),
        )
        .await;
        assert_eq!(in_background["result"]["isError"], false, "{in_background}");
        assert_eq!(backend.spawned.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn agt_009_child_tools_come_with_session_spawn_and_go_to_the_server() {
        let backend = Arc::new(Recorder::default());
        let spec = agent(&format!(
            "{BASE}agents:\n  quick-check:\n    executor: {{ harness: codex }}\nspawn: {{ agents: [quick-check] }}\ntools: {{ system: [session_spawn] }}\n"
        ));
        let s = server(Some(&spec), backend.clone());
        assert_eq!(
            list(&s).await,
            [
                "session_spawn",
                "session_send",
                "session_wait",
                "session_status",
                "session_list",
                "session_cancel"
            ]
        );
        for tool in [
            "session_send",
            "session_wait",
            "session_status",
            "session_list",
            "session_cancel",
        ] {
            let r = call(&s, tool, json!({"session_id": "ses_x"})).await;
            assert_eq!(r["result"]["structuredContent"]["tool"], tool, "{r}");
        }
        // session_list ergänzt die startbaren Agents samt Verfügbarkeit.
        let r = call(&s, "session_list", json!({})).await;
        assert_eq!(
            r["result"]["structuredContent"]["agents"],
            json!([{"name": "quick-check", "harness": "codex", "available": true}]),
            "{r}"
        );
        // Ein nicht eingerichteter Harness wird nicht gestartet (Preflight, AGT-011 AC4).
        let spec = agent(&format!(
            "{BASE}agents:\n  offline:\n    executor: {{ harness: codex }}\nspawn: {{ agents: [offline] }}\n"
        ));
        let s = server(Some(&spec), backend.clone());
        let before = backend.spawned.lock().unwrap().len();
        let r = call(
            &s,
            "session_spawn",
            json!({"agent": "offline", "prompt": "p"}),
        )
        .await;
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("harness_unavailable"),
            "{r}"
        );
        assert_eq!(backend.spawned.lock().unwrap().len(), before);
        let r = call(&s, "session_list", json!({})).await;
        assert_eq!(
            r["result"]["structuredContent"]["agents"][0]["available"],
            false
        );
        // Ohne spawn.agents gibt es sie nicht (fail closed).
        let plain = agent(&format!("{BASE}tools: {{ system: [session_wait] }}\n"));
        let s = server(Some(&plain), backend.clone());
        assert!(list(&s).await.is_empty());
        let r = call(&s, "session_wait", json!({})).await;
        assert_eq!(r["error"]["data"]["code"], "tool_not_enabled");
    }

    #[tokio::test]
    async fn agt_007_tools_of_later_milestones_stay_invisible() {
        let spec = agent(&format!(
            "{BASE}tools: {{ system: [ask_user, inbox_read, timer_set, policy_query] }}\n"
        ));
        let s = server(Some(&spec), Arc::new(Recorder::default()));
        assert_eq!(list(&s).await, ["policy_query", "inbox_read", "ask_user"]);
        let reply = call(&s, "timer_set", json!({})).await;
        assert_eq!(reply["error"]["data"]["code"], "tool_not_enabled");
        // Ein Agent, der policy_query nicht auswählt, bekommt es nicht.
        let spec = agent(&format!("{BASE}tools: {{ system: [ask_user] }}\n"));
        let s = server(Some(&spec), Arc::new(Recorder::default()));
        assert_eq!(list(&s).await, ["ask_user"]);
    }

    #[test]
    fn agt_007_skill_tools_follow_active_skills() {
        let names = |v: Vec<SystemTool>| v.into_iter().map(tool_name).collect::<Vec<_>>();
        // Ohne Agent: policy_query, inbox_read, ask_user (ab M2, UX-001).
        assert_eq!(
            names(enabled_tools(None, false)),
            ["policy_query", "inbox_read", "ask_user"]
        );
        assert_eq!(
            names(enabled_tools(None, true)),
            [
                "policy_query",
                "inbox_read",
                "ask_user",
                "skill_load",
                "skill_read_file"
            ]
        );
    }

    #[tokio::test]
    async fn agt_007_ac4_ask_user_passes_question_choices_and_timeout() {
        let backend = Arc::new(Recorder::default());
        let s = server(None, backend.clone());
        let r = call(
            &s,
            "ask_user",
            json!({"question": "Pro IP oder pro Account?", "options": ["Pro IP", "Pro Account"], "timeout": "5m"}),
        )
        .await;
        let content = &r["result"]["structuredContent"];
        assert_eq!(content["answered"], true, "{r}");
        assert_eq!(content["answer"], "Pro IP");
        let asked = backend.asked.lock().unwrap().clone();
        assert_eq!(asked[0].0, "Pro IP oder pro Account?");
        assert_eq!(asked[0].1, ["Pro IP", "Pro Account"]);
        assert_eq!(asked[0].2, std::time::Duration::from_secs(300));
        // Default 30 Minuten, höchstens 24 Stunden.
        call(&s, "ask_user", json!({"question": "a"})).await;
        call(&s, "ask_user", json!({"question": "b", "timeout": "3d"})).await;
        let asked = backend.asked.lock().unwrap().clone();
        assert_eq!(asked[1].2, std::time::Duration::from_secs(30 * 60));
        assert_eq!(asked[2].2, std::time::Duration::from_secs(24 * 3600));
        // Ohne Frage: Fehler für das Modell, keine Inbox-Frage.
        let r = call(&s, "ask_user", json!({"question": " "})).await;
        assert_eq!(r["result"]["isError"], true, "{r}");
        let r = call(&s, "ask_user", json!({"question": "x", "options": [1]})).await;
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert_eq!(backend.asked.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn agt_007_ask_user_without_answer_says_so() {
        #[derive(Default)]
        struct Silent;
        #[async_trait]
        impl SystemBackend for Silent {
            async fn policy_query(&self, _: &Value) -> Result<Value, ToolFailure> {
                Ok(json!({}))
            }
            async fn session_spawn(
                &self,
                _: &SpawnTarget,
                _: &Value,
            ) -> Result<Value, ToolFailure> {
                Ok(json!({}))
            }
            async fn session_tool(&self, _: SystemTool, _: &Value) -> Result<Value, ToolFailure> {
                Ok(json!({}))
            }
            async fn ask_user(
                &self,
                _: &str,
                _: &[String],
                _: std::time::Duration,
            ) -> Result<Option<String>, ToolFailure> {
                Ok(None)
            }
        }
        let s = SystemServer::new(
            enabled_tools(None, false),
            SkillSet::default(),
            Vec::new(),
            Arc::new(Silent),
        );
        let r = call(&s, "ask_user", json!({"question": "Noch da?"})).await;
        let content = &r["result"]["structuredContent"];
        assert_eq!(content["answered"], false, "{r}");
        assert_eq!(content["answer"], Value::Null);
        // inbox_read ohne eigene Implementierung: leer statt Fehler? Nein – fail closed.
        let r = call(&s, "inbox_read", json!({})).await;
        assert_eq!(r["result"]["isError"], true, "{r}");
    }

    #[tokio::test]
    async fn agt_007_inbox_read_comes_from_the_backend() {
        let s = server(None, Arc::new(Recorder::default()));
        let r = call(&s, "inbox_read", json!({})).await;
        assert_eq!(r["result"]["structuredContent"]["messages"], json!([]));
    }

    #[tokio::test]
    async fn mcp_protocol_basics() {
        let s = server(None, Arc::new(Recorder::default()));
        let init = s
            .handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t"}}}))
            .await
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "beton");
        assert!(
            s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await
                .is_none()
        );
        let r = s
            .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}))
            .await
            .unwrap();
        assert_eq!(r["error"]["code"], codes::METHOD_NOT_FOUND);
        let q = call(&s, "policy_query", json!({"action": "git push"})).await;
        assert_eq!(q["result"]["structuredContent"]["decision"], "ask");
    }
}
