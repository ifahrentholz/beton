//! Agent-Starts (AGT-004, AGT-005, AGT-010): Agent über den Suchpfad auflösen, prüfen,
//! Executor bestimmen, Parameter auflösen und den Agent als Snapshot im Blob-Store festhalten
//! (`agent.resolved`). Runner, Resume und Forks lesen den Agent danach nur aus dem Snapshot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beton_agents::params::{ParamError, ParamErrorKind, ParamValues};
use beton_agents::{
    AgentRef, AgentSnapshot, AgentSpec, Builtins, Overrides, SearchPath, Validator,
};
use beton_core::event::{Actor, AgentResolved, EventPayload, Notice, NoticeLevel, SystemComponent};
use beton_core::id::SessionId;
use serde_json::Value;

use crate::problem::{FieldError, Problem, ProblemCode};
use crate::sessions::SessionManager;

/// Ein aufgelöster Agent-Start, noch ohne Session.
#[derive(Debug, Clone)]
pub struct AgentStart {
    pub snapshot: AgentSnapshot,
    /// Harness laut Executor bzw. Override.
    pub harness: beton_harness::HarnessId,
    /// Modell laut Override bzw. Executor.
    pub model: Option<String>,
    /// Warnungen für das Event-Log (Verschattung, Validierung).
    pub notices: Vec<String>,
}

/// Braucht der Agent MCP-Injektion (MCP-Server, System-Tools, Skills, Sub-Agents; HAR-009,
/// AGT-006 bis AGT-008)?
pub(crate) fn needs_injection(s: &AgentSpec) -> bool {
    s.tools
        .as_ref()
        .is_some_and(|t| !t.mcp.is_empty() || !t.system.is_empty())
        || s.skills.is_some()
        || !s.agents.is_empty()
        || s.spawn.is_some()
}

/// Parameterfehler als Problem: fehlende Pflichtwerte als `params_required` (die UI fragt
/// nach), sonst `invalid_param`.
pub(crate) fn params_problem(errors: &[ParamError]) -> Problem {
    let only_missing = errors.iter().all(|e| e.kind == ParamErrorKind::Missing);
    let code = if only_missing {
        ProblemCode::ParamsRequired
    } else {
        ProblemCode::InvalidParam
    };
    let detail = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Problem::new(code).detail(detail).errors(
        errors
            .iter()
            .map(|e| FieldError {
                pointer: format!("/params/{}", e.name),
                detail: e.message.clone(),
            })
            .collect(),
    )
}

/// Werte aus einem JSON-Objekt (`params`); `null` = keine.
pub(crate) fn params_from(value: &Value) -> Result<BTreeMap<String, Value>, Problem> {
    match value {
        Value::Null => Ok(BTreeMap::new()),
        Value::Object(m) => Ok(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => {
            Err(Problem::new(ProblemCode::ValidationFailed).detail("`params` muss ein Objekt sein"))
        }
    }
}

/// Skill-Verzeichnisse außerhalb des Agents in Discovery-Reihenfolge (AGT-008), wie
/// `beton agent validate`.
fn skill_roots(project: &Path, beton_home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        project.join(".beton/skills"),
        project.join(".claude/skills"),
        project.join(".agents/skills"),
        beton_home.join("skills"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".claude/skills"));
        roots.push(home.join(".agents/skills"));
    }
    roots
}

impl SessionManager<'_> {
    /// Harnesses dieses Hosts für die Validierung (Effort-Stufen, HAR-017).
    fn harness_catalog(&self) -> beton_agents::HarnessCatalog {
        let mut catalog = beton_agents::HarnessCatalog::new();
        let probe = beton_harness::ProbeReport::default();
        for id in self.state().runtime.harnesses.ids() {
            if let Some(a) = self.state().runtime.harnesses.get(id) {
                catalog.insert(
                    id,
                    a.capabilities(beton_harness::Mode::Native, &probe).efforts,
                );
            }
        }
        if self.fake_allowed()
            && let Ok(fake) = beton_harness::HarnessId::FAKE.parse::<beton_harness::HarnessId>()
        {
            catalog.insert(&fake, beton_harness::fake::default_capabilities().efforts);
        }
        catalog
    }

    /// Löst einen Agent-Start auf (AGT-003, AGT-004, AGT-010); noch ohne Session.
    pub fn resolve_agent(
        &self,
        reference: &str,
        cwd: &Path,
        harness_override: Option<&str>,
        model_override: Option<&str>,
        given: &BTreeMap<String, Value>,
    ) -> Result<AgentStart, Problem> {
        let invalid = |d: String| Problem::new(ProblemCode::ValidationFailed).detail(d);
        let r = AgentRef::parse(reference).map_err(invalid)?;
        let beton_home = &self.cfg().beton_home;
        let builtins = Builtins::embedded();
        let search = SearchPath::for_cwd(cwd, beton_home, builtins.clone());
        let located = search
            .resolve(&r, cwd)
            .map_err(|e| Problem::new(ProblemCode::AgentNotFound).detail(e.to_string()))?;
        let mut notices: Vec<String> = located.shadow_warning().into_iter().collect();

        // Validierung wie `beton agent validate` (AGT-002, AGT-005 AC3): Fehler verhindern
        // den Start, Warnungen kommen ins Log.
        let catalog = self.harness_catalog();
        let project = beton_agents::resolve::project_root(cwd, beton_home);
        let report = Validator {
            search: &search,
            harnesses: &catalog,
            skill_roots: skill_roots(&project, beton_home),
        }
        .validate(&located);
        let (errors, warnings): (Vec<_>, Vec<_>) =
            report.diagnostics.iter().partition(|d| d.is_error());
        if !errors.is_empty() || report.spec.is_none() {
            return Err(Problem::new(ProblemCode::AgentInvalid)
                .detail(format!(
                    "Agent „{}“ ist ungültig ({} Fehler); Details mit `beton agent validate`",
                    located.name,
                    errors.len()
                ))
                .errors(
                    errors
                        .iter()
                        .map(|d| FieldError {
                            pointer: "/agent".into(),
                            detail: d.to_string(),
                        })
                        .collect(),
                ));
        }
        notices.extend(
            warnings
                .iter()
                .map(|d| format!("Agent {}: {d}", located.name)),
        );
        let spec = report.spec.unwrap_or_else(|| unreachable!());

        // Executor (AGT-004): Overrides gewinnen und stehen im Snapshot.
        let harness_text = harness_override
            .map(str::to_owned)
            .unwrap_or_else(|| spec.executor.harness.to_string());
        let harness: beton_harness::HarnessId =
            harness_text.parse().map_err(|e| invalid(format!("{e}")))?;
        let model = model_override
            .map(str::to_owned)
            .or_else(|| spec.executor.model.clone());

        let params =
            beton_agents::params::resolve(&spec.params, given).map_err(|e| params_problem(&e))?;
        let mut snapshot = AgentSnapshot::capture(&located, reference, &builtins)
            .map_err(|e| Problem::new(ProblemCode::AgentInvalid).detail(e.to_string()))?;
        snapshot.params = params;
        snapshot.overrides = Overrides {
            harness: harness_override.map(str::to_owned),
            model: model_override.map(str::to_owned),
        };
        Ok(AgentStart {
            snapshot,
            harness,
            model,
            notices,
        })
    }

    /// Prüft, ob der Harness den Agent ausführen kann (AGT-004 Details): Ein Harness ohne
    /// MCP-Injektion lehnt Agents mit MCP-Servern, System-Tools, Skills oder Sub-Agents ab.
    pub(crate) fn check_agent_harness(
        &self,
        spec: &AgentSpec,
        harness: &beton_harness::HarnessId,
        workdir: &Path,
    ) -> Result<(), Problem> {
        if !needs_injection(spec) {
            return Ok(());
        }
        let Some(caps) = self.harness_capabilities(harness, workdir) else {
            // Unbekannt: Der Runner lehnt den Start ab (fail closed).
            return Ok(());
        };
        if !caps.mcp_injection {
            return Err(
                Problem::new(ProblemCode::HarnessIncompatible).detail(format!(
                    "{} kann den Agent „{}“ nicht ausführen: Er braucht MCP-Server, System-Tools, \
                 Skills oder Sub-Agents (mcp_injection).",
                    beton_harness::handover::harness_label(harness.as_str()),
                    spec.name
                )),
            );
        }
        Ok(())
    }

    /// Hält den Snapshot einer neuen Session fest: Blob plus `agent.resolved` (AGT-004 AC1,
    /// AGT-010 AC3).
    pub(crate) async fn record_agent(
        &self,
        session: SessionId,
        snapshot: &AgentSnapshot,
        notices: &[String],
    ) -> Result<(), Problem> {
        let blob = self
            .state()
            .store
            .put_session_blob(self.org(), session, &snapshot.to_bytes())
            .await?;
        let (overrides, params) = snapshot.resolved_fields();
        let server = Actor::System {
            component: SystemComponent::Server,
        };
        self.append(
            session,
            server.clone(),
            EventPayload::AgentResolved(AgentResolved {
                name: snapshot.name.clone(),
                version: snapshot.version.clone(),
                source: snapshot.source.clone(),
                hash: snapshot.hash.clone(),
                blob_ref: blob.to_string(),
                overrides,
                params,
            }),
        )
        .await?;
        for text in notices {
            self.append(
                session,
                server.clone(),
                EventPayload::Notice(Notice {
                    level: NoticeLevel::Warn,
                    text: text.clone(),
                }),
            )
            .await?;
        }
        Ok(())
    }

    /// Das letzte `agent.resolved` einer Session samt Snapshot-Inhalt.
    pub(crate) async fn agent_snapshot(
        &self,
        session: SessionId,
    ) -> Result<Option<(AgentResolved, Vec<u8>)>, Problem> {
        let store = &self.state().store;
        let Some(event) = store
            .last_event_of_type(self.org(), session, "agent.resolved")
            .await?
        else {
            return Ok(None);
        };
        let Some(EventPayload::AgentResolved(resolved)) =
            store.resolve_payload(self.org(), &event).await.ok()
        else {
            return Err(Problem::internal(&"agent.resolved unlesbar"));
        };
        let blob: beton_core::event::BlobRef = resolved
            .blob_ref
            .parse()
            .map_err(|e| Problem::internal(&e))?;
        let bytes = store.session_blob(self.org(), session, &blob).await?;
        Ok(Some((resolved, bytes)))
    }

    /// Der Snapshot einer Session als Agent.
    pub(crate) async fn load_snapshot(
        &self,
        session: SessionId,
    ) -> Result<Option<AgentSnapshot>, Problem> {
        match self.agent_snapshot(session).await? {
            Some((_, bytes)) => AgentSnapshot::from_bytes(&bytes)
                .map(Some)
                .map_err(|e| Problem::internal(&e)),
            None => Ok(None),
        }
    }

    /// Übernimmt den Snapshot einer Session in eine andere (Fork, AGT-004 AC2): derselbe
    /// Blob, dasselbe `agent.resolved`, unabhängig vom aktuellen Stand der Agent-Dateien.
    pub(crate) async fn copy_agent(&self, from: SessionId, to: SessionId) -> Result<(), Problem> {
        let Some((resolved, bytes)) = self.agent_snapshot(from).await? else {
            return Ok(());
        };
        let blob = self
            .state()
            .store
            .put_session_blob(self.org(), to, &bytes)
            .await?;
        self.append(
            to,
            Actor::System {
                component: SystemComponent::Server,
            },
            EventPayload::AgentResolved(AgentResolved {
                blob_ref: blob.to_string(),
                ..resolved
            }),
        )
        .await
    }

    /// Schreibt den Snapshot für den Runner-Start (wie den Fork-Plan) und liefert den Pfad.
    pub(crate) async fn snapshot_file(
        &self,
        session: SessionId,
    ) -> Result<Option<PathBuf>, Problem> {
        let Some((_, bytes)) = self.agent_snapshot(session).await? else {
            return Ok(None);
        };
        let path = self.cfg().agent_snapshot(session);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Problem::internal(&e))?;
        }
        write_private(&path, &bytes).map_err(|e| Problem::internal(&e))?;
        Ok(Some(path))
    }

    /// Child-Snapshot für `session_spawn` (AGT-007, AGT-010): Sub-Agent aus dem Snapshot des
    /// Parents, Parameter aus dem Aufruf.
    pub(crate) async fn child_snapshot(
        &self,
        parent: SessionId,
        name: &str,
        params: &Value,
    ) -> Result<Option<AgentSnapshot>, Problem> {
        let Some(snapshot) = self.load_snapshot(parent).await? else {
            return Ok(None);
        };
        let invalid = |e: String| Problem::new(ProblemCode::AgentInvalid).detail(e);
        let mut child = snapshot
            .subagent(name)
            .map_err(|e| invalid(e.to_string()))?;
        let (spec, _) = child.agent().map_err(|e| invalid(e.to_string()))?;
        let values: ParamValues =
            beton_agents::params::resolve(&spec.params, &params_from(params)?)
                .map_err(|e| params_problem(&e))?;
        child.params = values;
        Ok(Some(child))
    }
}

/// Schreibt eine Datei mit Rechten `0600`.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    std::io::Write::write_all(&mut f, bytes)
}
