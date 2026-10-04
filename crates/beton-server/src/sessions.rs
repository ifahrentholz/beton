//! Session-Lebenszyklus (SES-001, SES-003, SES-005): anlegen, Runner starten, Eingaben
//! zustellen, unterbrechen, archivieren, löschen, fortsetzen.
//!
//! Der Server führt nie Agent-Code aus; er startet Runner über den `RunnerProvider` und
//! spricht mit ihnen über den Tunnel (`RunnerRegistry`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Empty, Event, EventPayload, SessionKind, SessionTitleChanged, SessionTrigger,
    TitleSource,
};
use beton_core::id::{OrgId, PrincipalId, RunnerId, SessionId, UserId};
use beton_host::{RunnerBoot, RunnerHandle, RunnerProvider, RunnerSpec, TerminateMode};
use beton_store::{DeleteAuthority, NewSession, SessionRecord};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::app::AppState;
use crate::problem::{Problem, ProblemCode};

/// Wie lange Eingaben auf einen frisch gestarteten Runner warten.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Höchstdauer eines synchronen `session_spawn` (wie `session_wait`, AGT-007).
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Gestartete Runner dieses Knotens.
#[derive(Default)]
pub struct Launched {
    handles: Mutex<HashMap<SessionId, RunnerHandle>>,
}

impl std::fmt::Debug for Launched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Launched")
    }
}

/// Wie Sessions gestartet werden.
#[derive(Clone)]
pub struct SessionsConfig {
    pub provider: Arc<dyn RunnerProvider>,
    pub tunnel_socket: Option<PathBuf>,
    /// Entwicklermodus: Fake-Harness erlaubt (HAR-026 AC3).
    pub dev: bool,
    pub launched: Arc<Launched>,
    /// `harnesses:` aus der User-Konfiguration; die Projekt-Konfiguration liest der Daemon
    /// beim Start aus dem Arbeitsverzeichnis der Session (HAR-003).
    pub harnesses_user: beton_harness::registry::HarnessesConfig,
}

impl std::fmt::Debug for SessionsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionsConfig")
            .field("dev", &self.dev)
            .finish_non_exhaustive()
    }
}

/// System-Tools, die der Runner über den Tunnel an den Server gibt (`system.call`, AGT-007).
pub struct ServerSystemCalls(pub AppState);

#[async_trait::async_trait]
impl crate::tunnel::SystemCalls for ServerSystemCalls {
    async fn call(&self, session: SessionId, tool: &str, args: Value) -> Result<Value, Value> {
        let problem = |p: Problem| {
            let v = serde_json::to_value(&p).unwrap_or(Value::Null);
            json!({"code": v["code"], "detail": v["detail"]})
        };
        match tool {
            "session.spawn" => self
                .0
                .sessions()
                .spawn_child(session, &args)
                .await
                .map_err(problem),
            other => Err(
                json!({"code": "unknown_command", "detail": format!("System-Tool {other} kennt der Server nicht")}),
            ),
        }
    }
}

/// Parameter für `POST /v1/sessions`.
#[derive(Debug, Clone, Default)]
pub struct CreateSession {
    pub target: String,
    pub cwd: String,
    pub title: Option<String>,
    pub model: Option<String>,
    pub harness_opts: Value,
}

pub struct SessionManager<'a> {
    state: &'a AppState,
}

fn user_actor(by: PrincipalId) -> Actor {
    Actor::User {
        id: by,
        device_id: None,
    }
}

impl<'a> SessionManager<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    fn org(&self) -> OrgId {
        self.state.local.org
    }

    fn cfg(&self) -> &SessionsConfig {
        &self.state.runtime.sessions
    }

    /// Legt eine Session an und startet ihren Runner (SES-001 AC1).
    pub async fn create(&self, by: UserId, req: CreateSession) -> Result<SessionRecord, Problem> {
        let harness: beton_harness::HarnessId = req
            .target
            .parse()
            .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(format!("{e}")))?;
        if harness.as_str() == beton_harness::HarnessId::FAKE && !self.cfg().dev {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail("Der Fake-Harness gibt es nur im Entwicklermodus (--dev)."));
        }
        if !std::path::Path::new(&req.cwd).is_dir() {
            return Err(Problem::new(ProblemCode::ValidationFailed)
                .detail(format!("Arbeitsverzeichnis {} existiert nicht", req.cwd)));
        }
        let session = self
            .state
            .store
            .create_session(
                self.org(),
                NewSession {
                    id: SessionId::new(),
                    owner: by,
                    kind: SessionKind::Main,
                    harness: harness.to_string(),
                    cwd: req.cwd.clone(),
                    model: req.model.clone(),
                    agent_ref: None,
                    project_id: None,
                    parent_id: None,
                    trigger: SessionTrigger::Api,
                    home_node: self.state.local.node,
                    harness_opts: req.harness_opts.clone(),
                },
            )
            .await?;
        if let Some(title) = req.title.filter(|t| !t.trim().is_empty()) {
            self.append(
                session.id,
                user_actor(PrincipalId::User(by)),
                EventPayload::SessionTitleChanged(SessionTitleChanged {
                    title,
                    source: TitleSource::User,
                }),
            )
            .await?;
        }
        self.launch(&session, None).await?;
        Ok(self.state.store.session(self.org(), session.id).await?)
    }

    async fn append(
        &self,
        session: SessionId,
        actor: Actor,
        payload: EventPayload,
    ) -> Result<(), Problem> {
        let events = self.state.events();
        for _ in 0..5 {
            let record = self.state.store.session(self.org(), session).await?;
            let e = Event::new(session, 0, actor.clone(), payload.clone());
            match events
                .append(self.org(), session, record.head_seq, record.epoch, vec![e])
                .await
            {
                Err(beton_store::Error::SeqConflict { .. }) => continue,
                other => return other.map(|_| ()).map_err(Problem::from),
            }
        }
        Err(Problem::new(ProblemCode::SeqConflict))
    }

    /// Startoptionen aus `session.created`.
    async fn created(
        &self,
        session: SessionId,
    ) -> Result<beton_core::event::SessionCreated, Problem> {
        let first = self.state.store.events(self.org(), session, 0, 1).await?;
        match first.first().and_then(|e| e.payload().cloned()) {
            Some(EventPayload::SessionCreated(c)) => Ok(c),
            _ => Err(Problem::internal(&"session.created fehlt")),
        }
    }

    /// Letzte native Session-Referenz des Harness (für Resume, SES-003).
    async fn last_native_ref(&self, session: SessionId) -> Result<Option<String>, Problem> {
        let mut after = 0;
        let mut found = None;
        loop {
            let page = self
                .state
                .store
                .events(self.org(), session, after, 500)
                .await?;
            let Some(last) = page.last() else { break };
            after = last.seq;
            for e in &page {
                if let Some(EventPayload::HarnessReady(r)) = e.payload()
                    && r.harness_session_ref.is_some()
                {
                    found.clone_from(&r.harness_session_ref);
                }
            }
        }
        Ok(found)
    }

    /// Startet einen Runner für die Session.
    async fn launch(&self, session: &SessionRecord, resume: Option<String>) -> Result<(), Problem> {
        let cfg = self.cfg();
        let socket = cfg.tunnel_socket.clone().ok_or_else(|| {
            Problem::new(ProblemCode::Unavailable).detail("Kein Tunnel-Socket konfiguriert")
        })?;
        let created = self.created(session.id).await?;
        let project =
            beton_harness::registry::HarnessesConfig::load_project(Path::new(&created.cwd))
                .map_err(|e| Problem::new(ProblemCode::ValidationFailed).detail(e))?;
        let token = self
            .state
            .runtime
            .runners
            .mint_token(session.id)
            .map_err(|e| Problem::internal(&e))?;
        let spec = RunnerSpec {
            runner_id: RunnerId::new(),
            session_id: session.id,
            harness: session.harness.clone(),
            workspace: PathBuf::from(&created.cwd),
            env_allowlist: Vec::new(),
        };
        let provisioned = cfg
            .provider
            .provision(&spec)
            .await
            .map_err(|e| Problem::new(ProblemCode::Unavailable).detail(e.to_string()))?;
        let handle = cfg
            .provider
            .start(
                &provisioned,
                &RunnerBoot {
                    tunnel_socket: socket,
                    token,
                    session_id: session.id,
                    epoch: session.epoch,
                    harness: session.harness.clone(),
                    scenario: created.harness_opts["scenario"].as_str().map(PathBuf::from),
                    model: created.model.clone(),
                    dev: cfg.dev,
                    resume,
                    agent_ref: created.agent_ref.clone(),
                    harnesses: beton_harness::registry::HarnessLayers {
                        user: cfg.harnesses_user.clone(),
                        project,
                        user_file: None,
                        project_file: Some(
                            Path::new(&created.cwd).join(".beton").join("config.yaml"),
                        ),
                    },
                },
            )
            .await
            .map_err(|e| Problem::new(ProblemCode::Unavailable).detail(e.to_string()))?;
        cfg.launched.handles.lock().await.insert(session.id, handle);
        Ok(())
    }

    async fn wait_connected(&self, session: SessionId) -> Result<(), Problem> {
        let start = Instant::now();
        while !self.state.runtime.runners.connected(session) {
            if start.elapsed() > CONNECT_TIMEOUT {
                return Err(
                    Problem::new(ProblemCode::Unavailable).detail("Runner meldet sich nicht")
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// Setzt eine gestoppte Session mit neuem Runner fort (SES-003).
    pub async fn resume(&self, session: SessionId) -> Result<SessionRecord, Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        if !self.state.runtime.runners.connected(session) {
            let reference = self.last_native_ref(session).await?;
            self.launch(&record, reference).await?;
        }
        self.wait_connected(session).await?;
        Ok(self.state.store.session(self.org(), session).await?)
    }

    /// Eingabe zustellen; eine gestoppte Session wird dafür fortgesetzt (SES-003 AC3).
    pub async fn input(
        &self,
        session: SessionId,
        text: String,
        by: PrincipalId,
    ) -> Result<Value, Problem> {
        self.input_as(session, text, by, user_actor(by)).await
    }

    /// Wie [`Self::input`], mit eigenem Akteur der Nachricht (z. B. der Parent-Agent).
    async fn input_as(
        &self,
        session: SessionId,
        text: String,
        by: PrincipalId,
        actor: Actor,
    ) -> Result<Value, Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        if record.archived {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Archivierte Session; erst wiederherstellen"));
        }
        if !self.state.runtime.runners.connected(session) {
            // Ein frisch gestarteter Runner verbindet sich gleich; ein beendeter wird ersetzt.
            let handle = self
                .cfg()
                .launched
                .handles
                .lock()
                .await
                .get(&session)
                .cloned();
            let starting = match &handle {
                Some(h) => matches!(
                    self.cfg().provider.status(h).await,
                    Ok(beton_host::RunnerStatus::Running)
                ),
                None => false,
            };
            if !starting {
                self.cfg().launched.handles.lock().await.remove(&session);
                self.resume(session).await?;
            }
            self.wait_connected(session).await?;
        }
        // Die Eingabe gehört zum Verlauf (Chat, Replay, Export): als Nachricht des Nutzers
        // vor der Zustellung, damit sie vor der Antwort steht.
        self.append(
            session,
            actor,
            EventPayload::MessageCompleted(beton_core::event::MessageCompleted {
                message_id: format!("msg_user_{}", RunnerId::new()),
                role: beton_core::event::MessageRole::User,
                content: vec![json!({"type": "text", "text": text})],
                author: Some(by),
            }),
        )
        .await?;
        let result = self
            .state
            .runtime
            .runners
            .deliver(
                session,
                "input.submit",
                json!({"text": text}),
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await?;
        Ok(json!({"input_id": result["turn_id"], "turn_id": result["turn_id"]}))
    }

    /// Bricht den laufenden Turn ab; ohne Turn bzw. Runner ein No-op (SES-005 AC3).
    pub async fn interrupt(&self, session: SessionId) -> Result<(), Problem> {
        self.state.store.session(self.org(), session).await?;
        if !self.state.runtime.runners.connected(session) {
            return Ok(());
        }
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "turn.interrupt",
                Value::Null,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// Einstellungen ändern, z. B. Modell (HAR-002 AC3: ohne Capability `capability_unsupported`).
    pub async fn set(&self, session: SessionId, args: Value) -> Result<(), Problem> {
        self.state.store.session(self.org(), session).await?;
        if !self.state.runtime.runners.connected(session) {
            return Err(
                Problem::new(ProblemCode::Conflict).detail("Session läuft nicht; erst fortsetzen")
            );
        }
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "session.set",
                args,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// Entscheidung für eine offene Freigabe an den Runner (WEB-018, HAR-005).
    pub async fn resolve_approval(
        &self,
        session: SessionId,
        approval: &str,
        args: Value,
    ) -> Result<(), Problem> {
        let open = self.state.store.open_approvals(self.org()).await?;
        let found = open
            .iter()
            .find(|a| a.session_id == session && a.id.to_string() == approval)
            .ok_or_else(|| {
                Problem::new(ProblemCode::NotFound).detail(format!("Freigabe {approval}"))
            })?;
        let call_id = found.subject["call_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut payload = args;
        payload["call_id"] = Value::String(call_id);
        self.state
            .runtime
            .runners
            .deliver(
                session,
                "approval.resolve",
                payload,
                self.state.runtime.tunnel.cmd_timeout,
            )
            .await
            .map(|_| ())
    }

    /// `session_spawn` (AGT-007): Child-Session für einen erlaubten Sub-Agent starten, den
    /// Auftrag zustellen, auf das Turn-Ende warten und die Abschlussnachricht liefern.
    ///
    /// Arbeitsverzeichnis, Owner und Projekt kommen aus der Parent-Session, nicht aus den
    /// Argumenten; welcher Agent mit welchem Harness laufen darf, hat der Runner anhand von
    /// `spawn.agents` entschieden. Grenzen wie `max_depth`, `max_concurrent` und eigene
    /// Worktrees folgen mit AGT-009.
    pub async fn spawn_child(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let invalid = |d: &str| Problem::new(ProblemCode::ValidationFailed).detail(d.to_owned());
        let record = self.state.store.session(self.org(), parent).await?;
        let created = self.created(parent).await?;
        let harness: beton_harness::HarnessId = args["harness"]
            .as_str()
            .unwrap_or_default()
            .parse()
            .map_err(|e| invalid(&format!("{e}")))?;
        if harness.as_str() == beton_harness::HarnessId::FAKE && !self.cfg().dev {
            return Err(invalid(
                "Der Fake-Harness gibt es nur im Entwicklermodus (--dev).",
            ));
        }
        let prompt = args["prompt"]
            .as_str()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| invalid("`prompt` fehlt"))?
            .to_owned();
        let agent = args["agent"].as_str().unwrap_or("sub-agent").to_owned();
        let child = self
            .state
            .store
            .create_session(
                self.org(),
                NewSession {
                    id: SessionId::new(),
                    owner: record.owner,
                    kind: SessionKind::Subagent,
                    harness: harness.to_string(),
                    cwd: created.cwd.clone(),
                    model: args["model"].as_str().map(str::to_owned),
                    agent_ref: None,
                    project_id: record.project_id,
                    parent_id: Some(parent),
                    trigger: SessionTrigger::Spawn,
                    home_node: self.state.local.node,
                    harness_opts: Value::Null,
                },
            )
            .await?;
        let parent_actor = Actor::Agent {
            id: None,
            harness: record.harness.clone(),
            agent_ref: created.agent_ref.clone(),
        };
        self.append(
            child.id,
            parent_actor.clone(),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: agent,
                source: TitleSource::Generated,
            }),
        )
        .await?;
        self.launch(&child, None).await?;
        self.wait_connected(child.id).await?;
        let from = self
            .state
            .store
            .session(self.org(), child.id)
            .await?
            .head_seq;
        self.input_as(
            child.id,
            prompt,
            PrincipalId::User(record.owner),
            parent_actor,
        )
        .await?;
        let (status, result) = self.wait_turn_end(child.id, from).await?;
        Ok(json!({"session_id": child.id, "status": status, "result": result}))
    }

    /// Wartet auf das Ende des nächsten Turns ab `after`; liefert Status und letzte
    /// Assistant-Nachricht.
    async fn wait_turn_end(
        &self,
        session: SessionId,
        after: u64,
    ) -> Result<(&'static str, String), Problem> {
        let deadline = Instant::now() + SPAWN_TIMEOUT;
        let mut seq = after;
        let mut last = String::new();
        loop {
            let page = self
                .state
                .store
                .events(self.org(), session, seq, 500)
                .await?;
            for e in &page {
                seq = e.seq;
                match e.payload() {
                    Some(EventPayload::MessageCompleted(m))
                        if m.role == beton_core::event::MessageRole::Assistant =>
                    {
                        last = m
                            .content
                            .iter()
                            .filter_map(|c| c["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("");
                    }
                    Some(EventPayload::TurnCompleted(_)) => return Ok(("completed", last)),
                    Some(EventPayload::TurnFailed(_)) => return Ok(("failed", last)),
                    Some(EventPayload::TurnInterrupted(_)) => return Ok(("interrupted", last)),
                    Some(EventPayload::SessionStatus(st))
                        if st.status == beton_core::event::SessionStatus::Failed =>
                    {
                        return Ok(("failed", last));
                    }
                    _ => {}
                }
            }
            if page.is_empty() {
                if Instant::now() > deadline {
                    return Err(Problem::new(ProblemCode::Unavailable)
                        .detail("Child-Session hat nicht rechtzeitig geantwortet"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    /// Archivieren: Runner beenden, Session ausblenden (SES-001 AC2).
    pub async fn archive(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionArchived(Empty {}),
        )
        .await?;
        self.stop(session, "archiviert").await;
        Ok(())
    }

    /// Titel ändern (CLI-003 `session rename`).
    pub async fn rename(
        &self,
        session: SessionId,
        title: &str,
        by: PrincipalId,
    ) -> Result<(), Problem> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail("Titel ist leer"));
        }
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: title.to_owned(),
                source: TitleSource::User,
            }),
        )
        .await
    }

    pub async fn unarchive(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        self.append(
            session,
            user_actor(by),
            EventPayload::SessionUnarchived(Empty {}),
        )
        .await
    }

    /// Runner geordnet stoppen; die Session wird danach `stopped`.
    pub async fn stop(&self, session: SessionId, reason: &str) {
        let runners = &self.state.runtime.runners;
        if runners.stop_with_reason(session, 10, reason) {
            let start = Instant::now();
            while runners.connected(session) && start.elapsed() < Duration::from_secs(15) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        if let Some(h) = self.cfg().launched.handles.lock().await.remove(&session) {
            let _ = self
                .cfg()
                .provider
                .terminate(
                    &h,
                    TerminateMode::Graceful {
                        grace: Duration::from_secs(5),
                    },
                )
                .await;
        }
    }

    /// Löschen nur durch den Owner (SES-001 AC3, DATA-008).
    pub async fn delete(&self, session: SessionId, by: PrincipalId) -> Result<(), Problem> {
        let record = self.state.store.session(self.org(), session).await?;
        if PrincipalId::User(record.owner) != by {
            return Err(Problem::new(ProblemCode::Forbidden).detail("Nur der Owner darf löschen"));
        }
        self.state.runtime.runners.revoke_tokens(session);
        if let Some(h) = self.cfg().launched.handles.lock().await.remove(&session) {
            let _ = self
                .cfg()
                .provider
                .terminate(&h, TerminateMode::Force)
                .await;
        }
        self.state
            .store
            .delete_session(self.org(), session, by, DeleteAuthority::Owner)
            .await?;
        Ok(())
    }
}

/// Beim Herunterfahren alle Runner dieses Knotens beenden.
pub async fn shutdown_all(cfg: &SessionsConfig, runners: &crate::tunnel::RunnerRegistry) {
    let handles: Vec<(SessionId, RunnerHandle)> =
        cfg.launched.handles.lock().await.drain().collect();
    for (session, h) in handles {
        runners.revoke_tokens(session);
        let _ = cfg
            .provider
            .terminate(
                &h,
                TerminateMode::Graceful {
                    grace: Duration::from_secs(5),
                },
            )
            .await;
    }
}
