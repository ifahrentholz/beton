//! MCP-Injektion im Runner (HAR-009, AGT-006 bis AGT-008): Plan der Session bestimmen,
//! Relay-Hub starten, System-Tools bereitstellen und Session-Skills auspacken.
//!
//! Fail closed: Ein Agent, der sich nicht laden lässt, verhindert den Start. Ohne Hub (z. B.
//! kein privates Laufzeitverzeichnis) bekommt der Harness keine MCP-Server, und jeder
//! konfigurierte Server meldet `mcp.server_failed`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use beton_agents::spec::SystemTool;
use beton_core::event::{EventPayload, McpServerFailed, Notice, NoticeLevel};
use beton_harness::registry::HarnessLayers;
use beton_harness::{Capabilities, HarnessId, McpInjection};
use beton_mcp::hub::{Hub, HubOptions, RelayCommand};
use beton_mcp::plan::{self, LoadedAgent, PlanInput, SessionPlan};
use beton_mcp::skills::{self, SkillSet, VendorDir};
use beton_mcp::system::{SpawnTarget, SystemBackend, SystemServer, ToolFailure};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// Antwort von `policy_query` ohne Policy-Gate (z. B. Werkzeuge ohne Session): nie `allow`.
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

/// Stellt dem Menschen eine Frage (`ask_user`, AGT-007 AC4); im Runner das Policy-Gate.
#[async_trait]
pub trait Asker: Send + Sync {
    async fn ask(
        &self,
        question: &str,
        choices: &[String],
        timeout: std::time::Duration,
    ) -> Option<String>;
}

/// Beantwortet `policy_query` mit der Entscheidung, die die Engine bei tatsächlicher
/// Ausführung träfe (AGT-007 AC3, POL-027); im Runner das Policy-Gate.
#[async_trait]
pub trait PolicyOracle: Send + Sync {
    async fn query(&self, args: &Value) -> Value;
}

/// System-Tools im Runner.
struct RunnerBackend {
    calls: mpsc::UnboundedSender<SystemRequest>,
    asker: Option<Arc<dyn Asker>>,
    oracle: Option<Arc<dyn PolicyOracle>>,
}

#[async_trait]
impl SystemBackend for RunnerBackend {
    async fn policy_query(&self, args: &Value) -> Result<Value, ToolFailure> {
        Ok(match &self.oracle {
            Some(o) => o.query(args).await,
            None => policy_answer(args),
        })
    }

    async fn session_spawn(
        &self,
        target: &SpawnTarget,
        args: &Value,
    ) -> Result<Value, ToolFailure> {
        // Harness, Modell und Agent stammen aus dem Plan, nicht vom Modell (AGT-007); der
        // Server prüft sie gegen den Snapshot und die Grenzen (AGT-009).
        self.server(
            "session.spawn",
            json!({
                "agent": target.name,
                "harness": target.harness,
                "model": target.model,
                "prompt": args["prompt"],
                "params": args.get("params").cloned().unwrap_or(Value::Null),
                "async": args.get("async").cloned().unwrap_or(Value::Bool(false)),
                "worktree": args.get("worktree").cloned().unwrap_or(Value::Null),
            }),
        )
        .await
    }

    async fn session_tool(&self, tool: SystemTool, args: &Value) -> Result<Value, ToolFailure> {
        // `session_wait` → `session.wait` usw.
        let name = beton_mcp::system::tool_name(tool).replacen('_', ".", 1);
        self.server(&name, args.clone()).await
    }

    async fn ask_user(
        &self,
        question: &str,
        choices: &[String],
        timeout: std::time::Duration,
    ) -> Result<Option<String>, ToolFailure> {
        match &self.asker {
            Some(a) => Ok(a.ask(question, choices, timeout).await),
            None => Err(ToolFailure::new(
                "unavailable",
                "Fragen sind hier nicht möglich",
            )),
        }
    }

    async fn inbox_read(&self, args: &Value) -> Result<Value, ToolFailure> {
        self.server("inbox.read", args.clone()).await
    }
}

impl RunnerBackend {
    /// Führt ein System-Tool über den Tunnel auf dem Server aus.
    async fn server(&self, tool: &str, args: Value) -> Result<Value, ToolFailure> {
        let (reply, rx) = oneshot::channel();
        let request = SystemRequest {
            tool: tool.into(),
            args,
            reply,
        };
        self.calls
            .send(request)
            .map_err(|_| ToolFailure::new("unavailable", "Runner beendet"))?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(problem)) => Err(ToolFailure::new(
                problem["code"].as_str().unwrap_or("failed"),
                problem["detail"]
                    .as_str()
                    .unwrap_or("vom Server abgelehnt")
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
    /// Instructions des Agents samt Projektdateien (AGT-005).
    pub instructions: Option<String>,
    /// `executor.timeout`: Wanduhr pro Run (AGT-004 AC3).
    pub timeout: Option<std::time::Duration>,
    /// `executor.permission_mode` des Agents (HAR-027); `plan` bindet die Session (AGT-011).
    pub agent_permission_mode: Option<beton_harness::PermissionMode>,
}

/// Woher der Agent einer Session kommt.
#[derive(Debug, Clone, Copy)]
pub enum AgentSource<'a> {
    /// Session ohne Agent.
    None,
    /// Snapshot aus `agent.resolved` (AGT-004); maßgeblich, sobald vorhanden.
    Snapshot(&'a beton_agents::AgentSnapshot),
    /// Nur der Agent-Ref (Sessions ohne Snapshot): über den Suchpfad von der Platte.
    Ref(&'a str),
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

/// Liest der Harness diesen Skill selbst? Claude Code lädt mit `--setting-sources user` nur
/// noch `~/.claude/skills`, nicht `.claude/skills` des Projekts (gegen 2.1.296 geprüft, #233);
/// Codex liest `.agents/skills` in Projekt und Home.
pub fn reads_skill_itself(harness: &HarnessId, skill: &skills::Skill) -> bool {
    match native_skills(harness) {
        Some(Some(VendorDir::Claude)) => {
            skill.vendor == Some(VendorDir::Claude) && skill.origin == skills::Origin::User
        }
        Some(Some(dir)) => skill.vendor == Some(dir),
        _ => false,
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

/// Lädt den Agent einer Session. Childs dürfen selbst Sub-Agents starten; Grenzen wie
/// `max_depth` prüft der Server (AGT-009).
fn load(
    source: AgentSource<'_>,
    workdir: &Path,
    paths: &Paths,
) -> Result<Option<(LoadedAgent, beton_agents::params::ParamValues)>, String> {
    match source {
        AgentSource::None => Ok(None),
        AgentSource::Ref(r) => Ok(Some((
            plan::load_agent(
                r,
                workdir,
                &paths.beton_home,
                beton_agents::Builtins::embedded(),
            )?,
            beton_agents::params::ParamValues::new(),
        ))),
        AgentSource::Snapshot(snapshot) => {
            let (spec, dir) = snapshot.agent().map_err(|e| e.to_string())?;
            Ok(Some((LoadedAgent { spec, dir }, snapshot.params.clone())))
        }
    }
}

/// Instructions des Agents für diesen Harness (AGT-005); Fehler verhindern den Start.
pub fn compose_instructions(
    agent: &LoadedAgent,
    params: &beton_agents::params::ParamValues,
    caps: &Capabilities,
    workdir: &Path,
) -> Result<beton_agents::instructions::Composed, String> {
    beton_agents::instructions::compose(beton_agents::instructions::ComposeInput {
        spec: &agent.spec,
        dir: &agent.dir,
        params,
        now: &beton_core::time::Timestamp::now().to_string(),
        workdir,
        harness_reads: &caps.native_project_files,
    })
}

/// Warum ein Harness auf diesem Host nicht startbar ist (nicht registriert, nicht installiert,
/// nicht angemeldet); `None`, wenn er bereit ist. Grundlage der Vorab-Prüfung von
/// `session_spawn`/`session_list` (AGT-009, AGT-011 AC4, AGT-012 AC3).
pub async fn harness_unavailable(
    registry: &beton_harness::registry::Registry,
    env: &beton_harness::HostEnv,
    harness: &str,
) -> Option<String> {
    let id: HarnessId = match harness.parse() {
        Ok(id) => id,
        Err(e) => return Some(e.to_string()),
    };
    let Some(adapter) = registry.get(&id) else {
        return Some("auf diesem Host nicht registriert".into());
    };
    let probe = adapter.probe(env).await;
    if !probe.installed {
        return Some("nicht installiert".into());
    }
    if probe.auth_status == beton_harness::AuthStatus::LoggedOut {
        return Some("nicht angemeldet (Login der CLI fehlt)".into());
    }
    None
}

/// Löst `${secret:<name>}` in `env` und `headers` zum Platzhalter des Credential-Bindings
/// `<name>` auf (AGT-001 AC3, AGT-006 AC3), nie zum Wert. HTTP-Server mit Credential in
/// `headers` verbinden sich über eine eigene Proxy-Instanz, die den Platzhalter für die
/// gebundenen Hosts ersetzt; stdio-Server laufen in Stufe 2 und erreichen das Netz ohnehin nur
/// über deren Proxy. Ohne Binding bzw. Stufenmodell startet der Server nicht
/// (`mcp.server_failed`).
pub async fn resolve_secrets(
    servers: Vec<beton_mcp::config::ConfiguredServer>,
    session_id: beton_core::id::SessionId,
    stages: Option<&mut crate::stages::SessionStages>,
    _agent_trusted: bool,
) -> (Vec<beton_mcp::config::ConfiguredServer>, Vec<EventPayload>) {
    let mut stages = stages;
    let mut kept = Vec::with_capacity(servers.len());
    let mut failures = Vec::new();
    for mut server in servers {
        let refs = server.secret_refs();
        if refs.is_empty() {
            kept.push(server);
            continue;
        }
        let resolved = match stages.as_deref() {
            Some(st) => server.resolve_secrets(|n| st.placeholder(n).map(ToString::to_string)),
            None => Err(
                "${secret:…} braucht das Stufenmodell (sandbox.backend) mit \
                         Credential-Bindings (sandbox.credentials, PRX-006)"
                    .to_owned(),
            ),
        };
        if let Err(e) = resolved {
            failures.push(failed(&server.name, e));
            continue;
        }
        if let (Some(url), Some(st)) = (server.server.url.clone(), stages.as_deref_mut()) {
            let Some(target) = https_target(&url) else {
                failures.push(failed(
                    &server.name,
                    "Credentials in `headers` nur für https://-Server",
                ));
                continue;
            };
            let label = format!("{session_id}/mcp-{}", server.name);
            match st.runner_egress(&label, &[target], &refs).await {
                Ok((proxy_url, ca_pem)) => {
                    server.egress = Some(beton_mcp::config::HttpEgress {
                        proxy_url: proxy_url.to_string(),
                        ca_pem,
                    });
                }
                Err(e) => {
                    failures.push(failed(&server.name, format!("Egress-Proxy: {e}")));
                    continue;
                }
            }
        }
        kept.push(server);
    }
    (kept, failures)
}

/// `host` bzw. `host:port` einer https-URL (ohne Zugangsdaten), sonst `None`.
fn https_target(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let target = hostport.strip_suffix(":443").unwrap_or(hostport);
    (!target.is_empty()).then(|| target.to_ascii_lowercase())
}

/// Bereitet die MCP-Injektion einer Session vor, dazu Instructions und Grenzen des Agents.
/// Mit `harnesses` prüft der Runner, welche Sub-Agents hier startbar sind.
#[allow(clippy::too_many_arguments)]
pub async fn prepare(
    session_id: beton_core::id::SessionId,
    harness: &HarnessId,
    caps: &Capabilities,
    workdir: &Path,
    source: AgentSource<'_>,
    layers: &HarnessLayers,
    paths: &Paths,
    harnesses: Option<(&beton_harness::registry::Registry, &beton_harness::HostEnv)>,
    gate: Option<Arc<dyn beton_mcp::hub::McpGate>>,
    asker: Option<Arc<dyn Asker>>,
    tools: Option<beton_sandbox::Sandbox>,
    oracle: Option<Arc<dyn PolicyOracle>>,
    stages: Option<&mut crate::stages::SessionStages>,
) -> Result<McpSetup, String> {
    let mut setup = McpSetup::default();
    let loaded = load(source, workdir, paths)?;
    // AGT-014 (#234): `policies` des Agents wertet beton erst mit WP-49 aus. Bis dahin startet
    // ein solcher Agent nicht, statt seine Regeln still zu ignorieren (fail closed).
    if let Some((a, _)) = &loaded
        && !a.spec.policies.is_empty()
    {
        return Err(format!(
            "Agent `{}` bringt `policies` mit, die beton noch nicht durchsetzt (AGT-014, kommt mit WP-49); Start verweigert",
            a.spec.name
        ));
    }
    if let Some((a, params)) = &loaded {
        let composed = compose_instructions(a, params, caps, workdir)?;
        setup.instructions = composed.text;
        setup
            .initial
            .extend(composed.notices.into_iter().map(|text| {
                EventPayload::Notice(Notice {
                    level: NoticeLevel::Warn,
                    text,
                })
            }));
        setup.timeout = a
            .spec
            .executor
            .timeout
            .as_ref()
            .and_then(beton_agents::spec::DurationText::to_duration);
    }
    if loaded.is_none() {
        // Ohne Agent wie `project_files: auto` (AGT-005): Was der Harness nicht selbst liest,
        // kommt über die Instructions (Claude seit #233 auch `CLAUDE.md`).
        let composed =
            beton_agents::instructions::compose_project_files(workdir, &caps.native_project_files);
        setup.instructions = composed.text;
        setup
            .initial
            .extend(composed.notices.into_iter().map(|text| {
                EventPayload::Notice(Notice {
                    level: NoticeLevel::Warn,
                    text,
                })
            }));
    }
    let agent: Option<LoadedAgent> = loaded.map(|(a, _)| a);
    setup.max_turns = agent.as_ref().and_then(|a| a.spec.executor.max_turns);
    setup.agent_permission_mode = agent.as_ref().and_then(|a| a.spec.executor.permission_mode);
    let suffix = random_suffix();
    let agent_skills = match &agent {
        Some(a) => {
            let scratch = paths.run_dir.join(format!("mcp-{suffix}.agent-skills"));
            let dir = plan::agent_skills_dir(a, &scratch).map_err(|e| e.to_string())?;
            if a.dir.in_memory() && dir.is_some() {
                setup.dirs.push(DirGuard(scratch));
            }
            dir
        }
        None => None,
    };
    let mut plan: SessionPlan = plan::build(&PlanInput {
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

    // AGT-001 AC3, AGT-006 AC3: `${secret:<name>}` → Platzhalter, nie der Wert.
    let (servers, failures) =
        resolve_secrets(std::mem::take(&mut plan.servers), session_id, stages, false).await;
    plan.servers = servers;
    setup.initial.extend(failures);

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
    let mut spawn = plan.spawn.clone();
    if let Some((registry, env)) = harnesses {
        let mut seen: std::collections::BTreeMap<String, Option<String>> = Default::default();
        for t in &mut spawn {
            if !seen.contains_key(&t.harness) {
                let why = harness_unavailable(registry, env, &t.harness).await;
                seen.insert(t.harness.clone(), why);
            }
            t.unavailable = seen.get(&t.harness).cloned().flatten();
        }
    }
    let system = plan.bridge.then(|| {
        Arc::new(SystemServer::new(
            plan.system_tools.clone(),
            SkillSet::new(plan.skills.clone()),
            spawn,
            Arc::new(RunnerBackend {
                calls: calls_tx,
                asker,
                oracle,
            }),
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
        gate,
        tools,
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
    if native_skills(harness).is_some() {
        let native: Vec<skills::Skill> = plan
            .skills
            .iter()
            .filter(|s| !reads_skill_itself(harness, s))
            .cloned()
            .collect();
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
        // ACP-Agents kommen nur aus der User-Konfiguration (SBX-002).
        let layers = HarnessLayers {
            user: acp,
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
