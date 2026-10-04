//! MCP-Injektion im Runner (HAR-009, AGT-006 bis AGT-008): Plan der Session bestimmen,
//! Relay-Hub starten, System-Tools bereitstellen und Session-Skills auspacken.
//!
//! Fail closed: Ein Agent, der sich nicht laden lässt, verhindert den Start. Ohne Hub (z. B.
//! kein privates Laufzeitverzeichnis) bekommt der Harness keine MCP-Server, und jeder
//! konfigurierte Server meldet `mcp.server_failed`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use beton_core::event::{EventPayload, McpServerFailed, Notice, NoticeLevel};
use beton_harness::registry::HarnessLayers;
use beton_harness::{Capabilities, HarnessId, McpInjection};
use beton_mcp::hub::{Hub, HubOptions, RelayCommand};
use beton_mcp::plan::{self, LoadedAgent, PlanInput, SessionPlan};
use beton_mcp::skills::{self, SkillSet, VendorDir};
use beton_mcp::system::{SpawnTarget, SystemBackend, SystemServer, ToolFailure};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// Antwort von `policy_query`, solange es keine Policy-Engine gibt (POL-027, ab M2).
pub fn policy_answer(args: &Value) -> Value {
    json!({
        "decision": "ask",
        "engine": "none",
        "action": args.get("action").cloned().unwrap_or(Value::Null),
        "explain": "In dieser Version gibt es noch keine Policy-Engine (POL, ab M2). Genehmigungspflichtige Aktionen legt beton dem Menschen zur Freigabe vor.",
    })
}

/// Ein System-Tool, das der Server ausführen muss (über den Tunnel).
#[derive(Debug)]
pub struct SystemRequest {
    pub tool: String,
    pub args: Value,
    pub reply: oneshot::Sender<Result<Value, Value>>,
}

/// System-Tools im Runner.
struct RunnerBackend {
    calls: mpsc::UnboundedSender<SystemRequest>,
}

#[async_trait]
impl SystemBackend for RunnerBackend {
    async fn policy_query(&self, args: &Value) -> Result<Value, ToolFailure> {
        Ok(policy_answer(args))
    }

    async fn session_spawn(
        &self,
        target: &SpawnTarget,
        args: &Value,
    ) -> Result<Value, ToolFailure> {
        let (reply, rx) = oneshot::channel();
        // Harness, Modell und Agent stammen aus dem Plan, nicht vom Modell (AGT-007).
        let request = SystemRequest {
            tool: "session.spawn".into(),
            args: json!({
                "agent": target.name,
                "harness": target.harness,
                "model": target.model,
                "prompt": args["prompt"],
            }),
            reply,
        };
        self.calls
            .send(request)
            .map_err(|_| ToolFailure::new("unavailable", "Runner beendet"))?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(problem)) => Err(ToolFailure::new(
                problem["code"].as_str().unwrap_or("spawn_failed"),
                problem["detail"]
                    .as_str()
                    .unwrap_or("Child-Session nicht gestartet")
                    .to_owned(),
            )),
            Err(_) => Err(ToolFailure::new("unavailable", "keine Antwort vom Server")),
        }
    }
}

/// Entfernt ein Verzeichnis beim Drop.
#[derive(Debug)]
pub struct DirGuard(pub PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Ergebnis der Vorbereitung.
#[derive(Default)]
pub struct McpSetup {
    pub injection: McpInjection,
    pub hub: Option<Hub>,
    /// `mcp.server_failed` aus dem Hub.
    pub events: Option<mpsc::UnboundedReceiver<EventPayload>>,
    /// System-Tools, die der Server ausführt.
    pub calls: Option<mpsc::UnboundedReceiver<SystemRequest>>,
    /// Events beim Start (Hinweise, nicht injizierbare Server).
    pub initial: Vec<EventPayload>,
    pub dirs: Vec<DirGuard>,
    /// `executor.max_turns` des Agents (HAR-010), für Harnesses mit eigenem Loop.
    pub max_turns: Option<u32>,
}

/// Wo die Dateien des Nutzers liegen.
#[derive(Debug, Clone)]
pub struct Paths {
    pub beton_home: PathBuf,
    pub home: Option<PathBuf>,
    /// Privates Laufzeitverzeichnis (0700) für Socket, Token und Session-Skills.
    pub run_dir: PathBuf,
    /// Programm, das die Relays startet (`beton` bzw. das Test-Binary des Runners).
    pub relay: PathBuf,
}

impl Paths {
    /// Aus der Prozessumgebung: `BETON_HOME` (sonst `~/.beton`), `HOME`, das eigene Binary.
    pub fn from_process(run_dir: PathBuf) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let beton_home = std::env::var_os("BETON_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".beton")))
            .unwrap_or_else(|| PathBuf::from(".beton"));
        Self {
            beton_home,
            home,
            run_dir,
            relay: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("beton")),
        }
    }
}

/// `mcp_bridge` laut Harness-Konfiguration; die Projektebene überschreibt die User-Ebene.
pub fn harness_bridge(harness: &HarnessId, layers: &HarnessLayers) -> bool {
    if let Some(slug) = harness.as_str().strip_prefix("acp:") {
        let (agents, _) = beton_harness_acp::agents(layers);
        return agents
            .iter()
            .find(|a| a.slug == slug)
            .is_none_or(|a| a.config.mcp_bridge != Some(false));
    }
    let id = harness.as_str();
    layers
        .project
        .entries
        .get(id)
        .and_then(|e| e.mcp_bridge)
        .or_else(|| layers.user.entries.get(id).and_then(|e| e.mcp_bridge))
        != Some(false)
}

/// Welches Vendor-Skill-Verzeichnis der Harness selbst liest, falls er native Skills hat.
pub fn native_skills(harness: &HarnessId) -> Option<Option<VendorDir>> {
    match harness.as_str() {
        HarnessId::CLAUDE => Some(Some(VendorDir::Claude)),
        HarnessId::CODEX => Some(Some(VendorDir::Agents)),
        _ => None,
    }
}

fn random_suffix() -> String {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn failed(name: &str, error: impl Into<String>) -> EventPayload {
    EventPayload::McpServerFailed(McpServerFailed {
        name: name.to_owned(),
        error: error.into(),
    })
}

/// Bereitet die MCP-Injektion einer Session vor.
pub async fn prepare(
    session_id: beton_core::id::SessionId,
    harness: &HarnessId,
    caps: &Capabilities,
    workdir: &Path,
    agent_ref: Option<&str>,
    layers: &HarnessLayers,
    paths: &Paths,
) -> Result<McpSetup, String> {
    let mut setup = McpSetup::default();
    let agent: Option<LoadedAgent> = match agent_ref {
        Some(r) => Some(plan::load_agent(
            r,
            workdir,
            &paths.beton_home,
            beton_agents::Builtins::embedded(),
        )?),
        None => None,
    };
    setup.max_turns = agent.as_ref().and_then(|a| a.spec.executor.max_turns);
    let suffix = random_suffix();
    let agent_skills = match &agent {
        Some(a) => {
            let scratch = paths.run_dir.join(format!("mcp-{suffix}.agent-skills"));
            let dir = plan::agent_skills_dir(a, &scratch).map_err(|e| e.to_string())?;
            if matches!(a.dir, beton_agents::AgentDir::Builtin { .. }) && dir.is_some() {
                setup.dirs.push(DirGuard(scratch));
            }
            dir
        }
        None => None,
    };
    let plan: SessionPlan = plan::build(&PlanInput {
        workdir,
        beton_home: &paths.beton_home,
        home: paths.home.as_deref(),
        agent: agent.as_ref(),
        agent_skills: agent_skills.as_deref(),
        harness_bridge: harness_bridge(harness, layers),
    });
    setup.initial.extend(plan.notices.iter().map(|text| {
        EventPayload::Notice(Notice {
            level: NoticeLevel::Warn,
            text: text.clone(),
        })
    }));

    let unavailable = |setup: &mut McpSetup, why: &str| {
        for s in &plan.servers {
            setup.initial.push(failed(&s.name, why));
        }
    };
    if !caps.mcp_injection {
        unavailable(
            &mut setup,
            &format!("Harness {harness} nimmt keine MCP-Server an (mcp_injection)"),
        );
        return Ok(setup);
    }
    if !plan.bridge && plan.servers.is_empty() && plan.skills.is_empty() {
        return Ok(setup);
    }

    let (calls_tx, calls_rx) = mpsc::unbounded_channel();
    let system = plan.bridge.then(|| {
        Arc::new(SystemServer::new(
            plan.system_tools.clone(),
            SkillSet::new(plan.skills.clone()),
            plan.spawn.clone(),
            Arc::new(RunnerBackend { calls: calls_tx }),
        ))
    });
    #[cfg(unix)]
    let started = Hub::start(HubOptions {
        session_id,
        run_dir: paths.run_dir.clone(),
        workdir: workdir.to_owned(),
        servers: plan.servers.clone(),
        system,
        relay: RelayCommand {
            program: paths.relay.clone(),
            prefix: Vec::new(),
        },
    })
    .await;
    #[cfg(not(unix))]
    let started: std::io::Result<(Hub, mpsc::UnboundedReceiver<EventPayload>)> = {
        let _ = (session_id, system);
        Err(std::io::Error::other(
            "MCP-Relay braucht Unix-Sockets (Windows folgt)",
        ))
    };
    match started {
        Ok((hub, events)) => {
            setup.injection.servers = hub.launches();
            setup.hub = Some(hub);
            setup.events = Some(events);
            setup.calls = Some(calls_rx);
        }
        Err(e) => {
            tracing::warn!("MCP-Relay-Hub nicht gestartet: {e}");
            unavailable(&mut setup, &format!("MCP-Relay-Hub nicht verfügbar: {e}"));
        }
    }

    // Native Skills (AGT-008): Session-Skill-Verzeichnis für Claude bzw. Codex.
    if let Some(reads_itself) = native_skills(harness) {
        let native = plan.native_skills(reads_itself);
        if !native.is_empty() {
            let dir = paths.run_dir.join(format!("mcp-{suffix}.skills"));
            match skills::materialize(&native, &dir) {
                Ok(()) => {
                    setup.injection.skills_dir = Some(dir.clone());
                    setup.dirs.push(DirGuard(dir));
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    setup.initial.push(EventPayload::Notice(Notice {
                        level: NoticeLevel::Warn,
                        text: format!("Session-Skills nicht bereitgestellt: {e}"),
                    }));
                }
            }
        }
    }
    Ok(setup)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use beton_harness::registry::{AcpAgentConfig, HarnessCommandConfig, HarnessesConfig};

    #[test]
    fn har_009_ac2_bridge_can_be_disabled_per_harness_and_acp_agent() {
        let claude: HarnessId = "claude".parse().unwrap();
        assert!(harness_bridge(&claude, &HarnessLayers::default()));
        let off = HarnessesConfig {
            entries: [(
                "claude".to_owned(),
                HarnessCommandConfig {
                    mcp_bridge: Some(false),
                    ..HarnessCommandConfig::default()
                },
            )]
            .into(),
            ..HarnessesConfig::default()
        };
        let layers = HarnessLayers {
            user: off.clone(),
            ..HarnessLayers::default()
        };
        assert!(!harness_bridge(&claude, &layers));

        let mut acp = HarnessesConfig::default();
        acp.acp.agents.insert(
            "mein-agent".into(),
            serde_json::to_value(AcpAgentConfig {
                command: "agent".into(),
                mcp_bridge: Some(false),
                ..AcpAgentConfig::default()
            })
            .unwrap(),
        );
        let layers = HarnessLayers {
            project: acp,
            ..HarnessLayers::default()
        };
        let mine: HarnessId = "acp:mein-agent".parse().unwrap();
        assert!(!harness_bridge(&mine, &layers));
        let other: HarnessId = "acp:gemini".parse().unwrap();
        assert!(harness_bridge(&other, &layers));
    }

    #[test]
    fn policy_query_never_allows_without_engine() {
        let a = policy_answer(&json!({"action": "git push"}));
        assert_eq!(a["decision"], "ask");
        assert_eq!(a["action"], "git push");
    }
}
