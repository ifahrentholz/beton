//! Der eingebaute MCP-Server `beton` mit den System-Tools (AGT-007).
//!
//! Sichtbar sind nur freigeschaltete Tools: `tools.system` des Agents wählt aus, die
//! Freischaltungsregeln (Tabelle in `docs/spec/02-agents.md`) begrenzen. Ein direkter Aufruf
//! eines nicht freigeschalteten Tools wird mit `tool_not_enabled` abgelehnt (fail closed).
//!
//! Verfügbar in M1: `policy_query`, `session_spawn` (synchron), `skill_load`,
//! `skill_read_file`. `inbox_read` und `ask_user` folgen mit der Inbox (UX-001, ab M2),
//! `session_send`/`session_wait`/`session_status`/`session_list`/`session_cancel` mit den
//! Sub-Agents (AGT-009), `timer_*` und `schedule_*` ab M5 (ASY-003, ASY-011).

use std::sync::Arc;

use async_trait::async_trait;
use beton_agents::AgentSpec;
use beton_agents::spec::SystemTool;
use serde_json::{Value, json};

use crate::protocol::{self, Kind, codes};
use crate::skills::{self, SkillSet};

/// System-Tools, die diese Version anbietet.
pub const AVAILABLE: [SystemTool; 4] = [
    SystemTool::PolicyQuery,
    SystemTool::SessionSpawn,
    SystemTool::SkillLoad,
    SystemTool::SkillReadFile,
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
        SystemTool::PolicyQuery => true,
        SystemTool::SkillLoad | SystemTool::SkillReadFile => skills_active,
        SystemTool::SessionSpawn => {
            agent.is_some_and(|a| a.spawn.as_ref().is_some_and(|s| !s.agents.is_empty()))
        }
        _ => false,
    }
}

/// Sichtbare System-Tools einer Session (AGT-007). Ohne Agent: `policy_query` und, wenn
/// Skills aktiv sind, `skill_load`/`skill_read_file`. Mit Agent wählt `tools.system` aus
/// (leer: alle verfügbaren); Skill-Tools kommen mit aktiven Skills immer hinzu. Jede Auswahl
/// gilt nur, wenn die Freischaltungsregel erfüllt ist; Tools späterer Meilensteine bleiben
/// unsichtbar.
pub fn enabled_tools(agent: Option<&AgentSpec>, skills_active: bool) -> Vec<SystemTool> {
    let requested: Vec<SystemTool> = match agent.and_then(|a| a.tools.as_ref()) {
        Some(t) if !t.system.is_empty() => t.system.clone(),
        _ => AVAILABLE.to_vec(),
    };
    AVAILABLE
        .into_iter()
        .filter(|t| {
            let skill_tool = matches!(t, SystemTool::SkillLoad | SystemTool::SkillReadFile);
            (requested.contains(t) || skill_tool) && unlocked(*t, agent, skills_active)
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
    /// Child-Session für einen erlaubten Sub-Agent starten und (synchron) ihr Ergebnis liefern.
    async fn session_spawn(&self, target: &SpawnTarget, args: &Value)
    -> Result<Value, ToolFailure>;
}

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
                    .map(|s| match &s.description {
                        Some(d) => format!("- {} ({}): {d}", s.name, s.harness),
                        None => format!("- {} ({})", s.name, s.harness),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({
                    "name": name,
                    "description": format!("Startet eine Child-Session für einen erlaubten Sub-Agent, wartet auf ihr Ende und liefert ihre Abschlussnachricht.\nErlaubte Agents:\n{list}"),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "agent": {"type": "string", "enum": names},
                            "prompt": {"type": "string", "description": "Auftrag an den Sub-Agent."},
                            "async": {"type": "boolean", "description": "Nur `false` (Default); asynchrone Childs folgen mit ASY-002."}
                        },
                        "required": ["agent", "prompt"]
                    }
                })
            }
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

    async fn spawn(&self, args: &Value) -> Result<Value, ToolFailure> {
        let agent = args["agent"].as_str().unwrap_or_default();
        let target = self.spawn.iter().find(|t| t.name == agent).ok_or_else(|| {
            ToolFailure::new(
                "spawn_denied",
                format!("„{agent}“ steht nicht in spawn.agents"),
            )
        })?;
        if args["async"] == true {
            return Err(ToolFailure::new(
                "not_supported",
                "async: true folgt mit ASY-002 (M5); bitte async: false",
            ));
        }
        if args["prompt"].as_str().is_none_or(|p| p.trim().is_empty()) {
            return Err(ToolFailure::new("invalid_args", "`prompt` fehlt"));
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
        assert_eq!(list(&s).await, ["policy_query"]);
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
        assert_eq!(list(&s).await, ["policy_query", "session_spawn"]);
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
        let not_async = call(
            &s,
            "session_spawn",
            json!({"agent": "quick-check", "prompt": "p", "async": true}),
        )
        .await;
        assert_eq!(not_async["result"]["isError"], true);
        assert_eq!(backend.spawned.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn agt_007_tools_of_later_milestones_stay_invisible() {
        let spec = agent(&format!(
            "{BASE}tools: {{ system: [ask_user, inbox_read, timer_set, policy_query] }}\n"
        ));
        let s = server(Some(&spec), Arc::new(Recorder::default()));
        assert_eq!(list(&s).await, ["policy_query"]);
        let reply = call(&s, "ask_user", json!({})).await;
        assert_eq!(reply["error"]["data"]["code"], "tool_not_enabled");
        // Ein Agent, der policy_query nicht auswählt, bekommt es nicht.
        let spec = agent(&format!("{BASE}tools: {{ system: [ask_user] }}\n"));
        let s = server(Some(&spec), Arc::new(Recorder::default()));
        assert!(list(&s).await.is_empty());
    }

    #[test]
    fn agt_007_skill_tools_follow_active_skills() {
        let names = |v: Vec<SystemTool>| v.into_iter().map(tool_name).collect::<Vec<_>>();
        assert_eq!(names(enabled_tools(None, false)), ["policy_query"]);
        assert_eq!(
            names(enabled_tools(None, true)),
            ["policy_query", "skill_load", "skill_read_file"]
        );
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
