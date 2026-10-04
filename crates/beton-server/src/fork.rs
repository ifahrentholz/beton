//! Fork ab Event X (SES-006) und Harness-Wechsel per Fork (SES-007).
//!
//! Der Server normalisiert den Fork-Punkt auf ein Turn-Ende, legt die neue Session mit dem
//! gewählten Workspace an, übernimmt die Inhalts-Events bis dahin (neu nummeriert) und
//! vermerkt den Fork in der Quelle (`session.fork_created`). Wie der Verlauf beim Ziel-Harness
//! ankommt (`native`, `rebuild`, `preamble`), entscheidet der Runner anhand des Fork-Plans
//! (HAR-018, HAR-019) und schreibt `session.forked` mit dem tatsächlichen Modus.
//!
//! Ein Harness-Wechsel geschieht ausschließlich per Fork: Die Quelle bleibt auf ihrem Harness.

use std::path::PathBuf;

use beton_core::event::{
    Actor, Event, EventPayload, ForkReason, HistoryMode, SessionForkCreated, SessionKind,
    SessionTitleChanged, SessionTrigger, TitleSource,
};
use beton_core::id::{PrincipalId, SessionId, UserId};
use beton_harness::handover::{self, ForkPlan, HistoryEvent, PlanKind};
use beton_host::ForkBoot;
use beton_store::{NewSession, SessionRecord};
use serde_json::Value;

use crate::problem::{Problem, ProblemCode};
use crate::sessions::{CreateSession, SessionManager, WorktreeSpec};

/// Woher die Dateien des Forks kommen (SES-006).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    utoipa::ToSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ForkWorkspace {
    /// Dieselben Dateien wie die Quelle (Worktree bzw. Arbeitsverzeichnis).
    Shared,
    /// Neuer `git worktree` mit eigenem Branch vom Stand der Quelle inkl. WIP-Snapshot.
    NewWorktree,
    /// Leeres Verzeichnis, nur der Verlauf.
    Fresh,
}

/// Parameter eines Forks.
#[derive(Debug, Clone, Default)]
pub struct ForkSession {
    /// Fork-Punkt; ohne Angabe das Ende der Quelle (`head_seq`).
    pub at_seq: Option<u64>,
    /// Ziel-Harness; ohne Angabe der Harness der Quelle.
    pub harness: Option<String>,
    pub model: Option<String>,
    /// Ohne Angabe `new_worktree`, wenn die Quelle in einem Git-Repository arbeitet, sonst
    /// `shared`.
    pub workspace: Option<ForkWorkspace>,
    pub title: Option<String>,
    /// Startoptionen des Ziel-Harness; ohne Angabe die der Quelle, falls der Harness gleich
    /// bleibt.
    pub harness_opts: Value,
}

/// Ergebnis eines Forks.
#[derive(Debug, Clone)]
pub struct Forked {
    pub session: SessionRecord,
    /// Tatsächlicher Fork-Punkt nach der Normalisierung (SES-006 AC2).
    pub effective_seq: u64,
    pub workspace: ForkWorkspace,
}

fn incompatible(detail: String) -> Problem {
    Problem::new(ProblemCode::HarnessIncompatible).detail(detail)
}

impl SessionManager<'_> {
    /// Alle dauerhaften Events einer Session, Nutzlasten aufgelöst.
    pub(crate) async fn all_events(&self, session: SessionId) -> Result<Vec<Event>, Problem> {
        let store = &self.state().store;
        let mut out = Vec::new();
        let mut after = 0;
        loop {
            let page = store.events(self.org(), session, after, 500).await?;
            let Some(last) = page.last() else { break };
            after = last.seq;
            out.extend(page);
        }
        Ok(out)
    }

    /// Inhalts-Events bis `upto` mit aufgelöster Nutzlast und `raw` (HAR-019).
    async fn history(
        &self,
        events: &[Event],
        upto: u64,
    ) -> Result<Vec<(Event, HistoryEvent)>, Problem> {
        let store = &self.state().store;
        let mut out = Vec::new();
        for e in events.iter().take_while(|e| e.seq <= upto) {
            if !handover::is_content_type(e.type_name()) {
                continue;
            }
            let payload = store.resolve_payload(self.org(), e).await?;
            let raw = store
                .event_raw(self.org(), e.session_id, e.seq)
                .await?
                .map(|v| v.to_string());
            out.push((
                e.clone(),
                HistoryEvent {
                    seq: e.seq,
                    payload,
                    turn_id: e.turn_id,
                    raw,
                },
            ));
        }
        Ok(out)
    }

    /// Fork einer Session ab `at_seq` (SES-006), optional auf einen anderen Harness (SES-007).
    pub async fn fork(
        &self,
        source: SessionId,
        by: UserId,
        req: ForkSession,
    ) -> Result<Forked, Problem> {
        let record = self.state().store.session(self.org(), source).await?;
        let created = self.created(source).await?;
        let events = self.all_events(source).await?;
        let head = record.head_seq;
        let at = req.at_seq.unwrap_or(head);
        if at == 0 || at > head {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail(format!(
                "at_seq {at} liegt außerhalb der Session (1 bis {head})"
            )));
        }
        let effective = handover::normalize_fork_point(
            events
                .iter()
                .filter_map(|e| e.payload().map(|p| (e.seq, p))),
            at,
        );

        // Ziel-Harness prüfen, bevor etwas angelegt wird (SES-007: 422 harness_incompatible).
        let target_text = req
            .harness
            .clone()
            .unwrap_or_else(|| record.harness.clone());
        let target: beton_harness::HarnessId = target_text
            .parse()
            .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(format!("{e}")))?;
        if target.as_str() == beton_harness::HarnessId::FAKE && !self.fake_allowed() {
            return Err(Problem::new(ProblemCode::FeatureDisabled).detail(
                "Der Fake-Harness ist nur mit --dev oder BETON_FEATURES=fake_harness verfügbar.",
            ));
        }
        // Workspace (SES-006, SES-015).
        let source_root = self.workspace_root(&record).await?;
        self.check_target(&target, created.agent_ref.as_deref(), &source_root)
            .await?;
        let in_repo = {
            let root = source_root.clone();
            tokio::task::spawn_blocking(move || beton_git::worktree::main_checkout(&root).is_ok())
                .await
                .map_err(|e| Problem::internal(&e))?
        };
        let workspace = req.workspace.unwrap_or(if in_repo {
            ForkWorkspace::NewWorktree
        } else {
            ForkWorkspace::Shared
        });
        let id = SessionId::new();
        let source_title = Some(record.title.clone()).filter(|t| !t.trim().is_empty());
        let title = req
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                let base = source_title.clone().unwrap_or_else(|| source.to_string());
                if target.as_str() == record.harness {
                    format!("{base} (Fork)")
                } else {
                    format!("{base} ({})", handover::harness_label(target.as_str()))
                }
            });
        let mut cwd = created.cwd.clone();
        let mut worktree = None;
        match workspace {
            ForkWorkspace::Shared => cwd = source_root.display().to_string(),
            ForkWorkspace::Fresh => {
                let dir = self.cfg().fresh_dir(id);
                std::fs::create_dir_all(&dir).map_err(|e| Problem::internal(&e))?;
                cwd = dir.display().to_string();
            }
            ForkWorkspace::NewWorktree => {
                if !in_repo {
                    return Err(Problem::new(ProblemCode::NotAGitRepo).detail(
                        "Die Quelle arbeitet nicht in einem Git-Repository; ein Worktree ist \
                         nicht möglich.",
                    ));
                }
                let root = source_root.clone();
                let base =
                    tokio::task::spawn_blocking(move || beton_git::worktree::wip_snapshot(&root))
                        .await
                        .map_err(|e| Problem::internal(&e))?
                        .map_err(|e| crate::sessions::git_problem(&e))?;
                let spec = WorktreeSpec {
                    branch: None,
                    base: Some(base),
                    fetch: false,
                };
                let request = CreateSession {
                    cwd: source_root.display().to_string(),
                    title: Some(title.clone()),
                    ..CreateSession::default()
                };
                worktree = Some(self.create_worktree(id, &request, &spec).await?);
            }
        }

        let harness_opts = if req.harness_opts.is_null() && target.as_str() == record.harness {
            created.harness_opts.clone()
        } else {
            req.harness_opts.clone()
        };
        let session = self
            .state()
            .store
            .create_session(
                self.org(),
                NewSession {
                    id,
                    owner: by,
                    kind: SessionKind::Main,
                    harness: target.to_string(),
                    cwd,
                    model: req.model.clone(),
                    agent_ref: created.agent_ref.clone(),
                    project_id: record.project_id,
                    parent_id: None,
                    trigger: SessionTrigger::User,
                    home_node: self.state().local.node,
                    harness_opts,
                },
            )
            .await;
        let session = match session {
            Ok(s) => s,
            Err(e) => {
                if let Some(wt) = worktree {
                    crate::sessions::discard_worktree(wt).await;
                }
                if workspace == ForkWorkspace::Fresh {
                    let _ = std::fs::remove_dir_all(self.cfg().fresh_dir(id));
                }
                return Err(e.into());
            }
        };
        let user = Actor::User {
            id: PrincipalId::User(by),
            device_id: None,
        };
        self.append(
            session.id,
            user.clone(),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title,
                source: TitleSource::User,
            }),
        )
        .await?;
        if let Some(wt) = &worktree {
            self.log_worktree(session.id, wt).await?;
        }

        // Verlauf übernehmen: Inhalts-Events bis zum Fork-Punkt, neu nummeriert (SES-006 AC1).
        let history = self.history(&events, effective).await?;
        self.copy_history(session.id, &history).await?;

        // Plan für den Runner: Er wählt native, rebuild oder preamble (HAR-018, HAR-019).
        let tail_empty = !events
            .iter()
            .filter(|e| e.seq > effective)
            .any(|e| handover::is_content_type(e.type_name()));
        let native_ref = if tail_empty && target.as_str() == record.harness {
            self.last_native_ref(source).await?
        } else {
            None
        };
        let plan = ForkPlan {
            kind: PlanKind::Fork,
            from_session: source,
            from_harness: record.harness.clone(),
            source_title,
            at_seq: effective,
            reason: ForkReason::User,
            native_ref,
            worktree: worktree
                .as_ref()
                .map(|w| w.path.display().to_string())
                .or_else(|| record.worktree.as_ref().map(|w| w.path.clone())),
            branch: worktree
                .as_ref()
                .map(|w| w.branch.clone())
                .or_else(|| record.worktree.as_ref().map(|w| w.branch.clone())),
            agent_ref: created.agent_ref.clone(),
            history: history.into_iter().map(|(_, h)| h).collect(),
        };
        self.write_plan(session.id, &plan)?;

        // Die Quelle erfährt vom Fork; sonst bleibt sie unverändert (SES-006 AC1).
        self.append(
            source,
            user,
            EventPayload::SessionForkCreated(SessionForkCreated {
                child: session.id,
                at_seq: effective,
                reason: ForkReason::User,
            }),
        )
        .await?;
        let record = self.state().store.session(self.org(), session.id).await?;
        self.launch(&record, None).await?;
        Ok(Forked {
            session: self.state().store.session(self.org(), session.id).await?,
            effective_seq: effective,
            workspace,
        })
    }

    /// Prüft den Ziel-Harness (SES-007): bekannt, Verlauf übernehmbar und passend zum Agent.
    async fn check_target(
        &self,
        target: &beton_harness::HarnessId,
        agent_ref: Option<&str>,
        workdir: &std::path::Path,
    ) -> Result<(), Problem> {
        let probe = beton_harness::ProbeReport::default();
        let mode = beton_harness::Mode::Native;
        let caps = if target.as_str() == beton_harness::HarnessId::FAKE {
            Some(beton_harness::fake::default_capabilities())
        } else if let Some(a) = self.state().runtime.harnesses.get(target) {
            Some(a.capabilities(mode, &probe))
        } else {
            // ACP-Agents (HAR-008) und Direkt-API-Provider (HAR-011) aus der Konfiguration.
            let layers = beton_harness::registry::HarnessLayers {
                user: self.cfg().harnesses_user.clone(),
                project: beton_harness::registry::HarnessesConfig::load_project(workdir)
                    .unwrap_or_default(),
                user_file: None,
                project_file: None,
                // Direkt-API-Provider nur aus der User-Konfiguration (HAR-011).
                providers: self.cfg().providers.clone(),
            };
            let mut r =
                beton_harness::registry::Registry::new(beton_harness::registry::RegistryOptions {
                    dev: false,
                });
            beton_harness_acp::register(&mut r, &layers);
            beton_harness_direct::register(
                &mut r,
                &layers.providers,
                &beton_harness_direct::DirectOptions::default(),
            );
            r.get(target).map(|a| a.capabilities(mode, &probe))
        };
        let Some(caps) = caps else {
            return Err(incompatible(format!(
                "Harness `{target}` ist auf diesem Host nicht verfügbar."
            )));
        };
        if caps.fork_history == beton_harness::ForkHistory::None {
            return Err(incompatible(format!(
                "{} kann keinen Verlauf übernehmen (fork_history: none).",
                handover::harness_label(target.as_str())
            )));
        }
        if let Some(agent) = agent_ref {
            // Agents mit MCP-Servern, System-Tools, Skills oder Sub-Agents brauchen
            // `mcp_injection` (HAR-009, AGT-006 bis AGT-008).
            let needs_injection = agent_needs_injection(self, agent, workdir).await;
            if needs_injection && !caps.mcp_injection {
                return Err(incompatible(format!(
                    "{} kann den Agent `{agent}` nicht ausführen: Er braucht MCP-Server, \
                     System-Tools oder Skills (mcp_injection).",
                    handover::harness_label(target.as_str())
                )));
            }
        }
        Ok(())
    }

    /// Übernimmt Inhalts-Events in die neue Session (neue IDs und `seq`, sonst unverändert).
    async fn copy_history(
        &self,
        session: SessionId,
        history: &[(Event, HistoryEvent)],
    ) -> Result<(), Problem> {
        let events = self.state().events();
        for chunk in history.chunks(200) {
            let batch: Vec<Event> = chunk
                .iter()
                .map(|(src, h)| {
                    let mut e = Event::new(session, 0, src.actor.clone(), h.payload.clone());
                    e.ts = src.ts;
                    e.turn_id = src.turn_id;
                    e.raw = h
                        .raw
                        .as_ref()
                        .and_then(|r| beton_core::event::RawJson::from_string(r.clone()).ok());
                    e
                })
                .collect();
            let record = self.state().store.session(self.org(), session).await?;
            events
                .append(self.org(), session, record.head_seq, record.epoch, batch)
                .await?;
        }
        Ok(())
    }

    fn write_plan(&self, session: SessionId, plan: &ForkPlan) -> Result<(), Problem> {
        let path = self.cfg().fork_plan(session);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Problem::internal(&e))?;
        }
        let json = serde_json::to_vec(plan).map_err(|e| Problem::internal(&e))?;
        std::fs::write(&path, json).map_err(|e| Problem::internal(&e))
    }

    /// Fork-Plan für den Runner-Start: solange `session.forked` fehlt, und danach bei
    /// `preamble`, bis der erste Turn begonnen hat. Eine importierte Session ohne native
    /// Referenz von beton wird über die Vendor-Datei nativ fortgesetzt, falls sie noch existiert,
    /// sonst per Rebuild (HAR-019 AC2, SES-008 AC2).
    pub(crate) async fn fork_boot(
        &self,
        session: &SessionRecord,
        resume: Option<&str>,
    ) -> Result<Option<ForkBoot>, Problem> {
        let plan = self.cfg().fork_plan(session.id);
        let events = self.all_events(session.id).await?;
        let mut forked: Option<Option<HistoryMode>> = None;
        let mut turn_after = false;
        let mut imported: Option<String> = None;
        for e in &events {
            match e.payload() {
                Some(EventPayload::SessionForked(f)) => {
                    forked = Some(f.history_mode);
                    turn_after = false;
                }
                Some(EventPayload::TurnStarted(_)) => turn_after = true,
                Some(EventPayload::SessionImported(i)) => {
                    imported = Some(i.vendor_session_id.clone());
                }
                _ => {}
            }
        }
        match forked {
            Some(mode) => Ok((mode == Some(HistoryMode::Preamble)
                && !turn_after
                && plan.is_file())
            .then_some(ForkBoot { plan, logged: true })),
            None if plan.is_file() && imported.is_none() => Ok(Some(ForkBoot {
                plan,
                logged: false,
            })),
            None if imported.is_some() && resume.is_none() => {
                let created = self.created(session.id).await?;
                // Solange die Vendor-Datei existiert, setzt die CLI sie nativ fort (als neue
                // Session, die Datei bleibt unverändert); sonst Rebuild bzw. Handover (SES-008
                // AC2).
                let native_ref = match &imported {
                    Some(vendor) => {
                        self.imported_native_ref(&session.harness, &created.cwd, vendor)
                            .await
                    }
                    None => None,
                };
                let head = session.head_seq;
                let history = self.history(&events, head).await?;
                if history.is_empty() {
                    return Ok(None);
                }
                let resume_plan = ForkPlan {
                    kind: PlanKind::Resume,
                    from_session: session.id,
                    from_harness: session.harness.clone(),
                    source_title: Some(session.title.clone()).filter(|t| !t.is_empty()),
                    at_seq: head,
                    reason: ForkReason::User,
                    native_ref,
                    worktree: session.worktree.as_ref().map(|w| w.path.clone()),
                    branch: session.worktree.as_ref().map(|w| w.branch.clone()),
                    agent_ref: created.agent_ref,
                    history: history.into_iter().map(|(_, h)| h).collect(),
                };
                self.write_plan(session.id, &resume_plan)?;
                Ok(Some(ForkBoot {
                    plan,
                    logged: false,
                }))
            }
            None => Ok(None),
        }
    }
}

/// Braucht der Agent MCP-Injektion? Unbekannte Agents gelten als nicht prüfbar (der Runner
/// lehnt sie beim Start ab, fail closed).
async fn agent_needs_injection(
    m: &SessionManager<'_>,
    agent: &str,
    workdir: &std::path::Path,
) -> bool {
    let workdir = workdir.to_path_buf();
    let home = m
        .cfg()
        .worktrees_root
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    let agent = agent.to_owned();
    tokio::task::spawn_blocking(move || {
        beton_mcp::plan::load_agent(&agent, &workdir, &home, beton_agents::Builtins::embedded())
            .map(|a| {
                let s = &a.spec;
                s.tools
                    .as_ref()
                    .is_some_and(|t| !t.mcp.is_empty() || !t.system.is_empty())
                    || s.skills.is_some()
                    || !s.agents.is_empty()
                    || s.spawn.is_some()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}
