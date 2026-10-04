//! Sub-Agents über Harness-Grenzen (AGT-009): Child-Sessions starten (`session_spawn`, auch im
//! Hintergrund), weitere Aufträge (`session_send`), Warten (`session_wait`), Status, Liste und
//! Abbruch, dazu die Grenzen `max_depth` und `max_concurrent`, eigene Worktrees (SES-015) und
//! die Events `agent.spawned`/`agent.completed` im Parent.
//!
//! Maßgeblich ist der Agent-Snapshot des Parents (AGT-004): Er bestimmt, welche Sub-Agents
//! erlaubt sind, auf welchem Harness sie laufen und welche Grenzen gelten. Childs sind
//! eigenständige Sessions (`kind: subagent`, `trigger: spawn`, `parent_session_id`). Wird der
//! Parent abgebrochen (Unterbrechen, Stoppen, Archivieren, Löschen), enden laufende Childs mit
//! `cancel_reason: parent_cancelled` (AC5).
//!
//! Der Zustand laufender Aufträge liegt im Speicher des Servers; nach einem Neustart sind die
//! Runner der Childs ohnehin beendet.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use beton_agents::spec::{DEFAULT_MAX_CONCURRENT, DEFAULT_MAX_DEPTH, DurationText, WorktreeMode};
use beton_core::event::{
    Actor, AgentCompleted, AgentMessage, AgentSpawned, EventPayload, MessageRole, SessionKind,
    SessionTitleChanged, SessionTrigger, SystemComponent, TitleSource,
};
use beton_core::id::{PrincipalId, SessionId};
use beton_store::{NewSession, SessionRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use ts_rs::TS;
use utoipa::ToSchema;

use crate::api::SessionWorktree;
use crate::app::AppState;
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::sessions::{CreateSession, InputMode, SessionManager, WorktreeSpec};

/// Höchstdauer eines synchronen `session_spawn` (AGT-007); danach kommt der Zwischenstand.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Default und Höchstwert für `session_wait(timeout)` (AGT-007).
const WAIT_DEFAULT: Duration = Duration::from_secs(30 * 60);
const WAIT_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// Länge der Zusammenfassung in `agent.completed` (wie ASY-001).
const SUMMARY_MAX: usize = 4096;
/// `cancel_reason`, wenn der Parent abgebrochen wurde (AGT-009 AC5).
pub const PARENT_CANCELLED: &str = "parent_cancelled";
/// `cancel_reason`, wenn der Parent-Agent das Child mit `session_cancel` beendet.
pub const CANCELLED_BY_PARENT: &str = "cancelled_by_parent";

/// Zustand des aktuellen Auftrags an ein Child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
struct Task {
    parent: SessionId,
    agent: String,
    status: TaskStatus,
    result: String,
    cancel_reason: Option<String>,
    /// Zählt Aufträge (Spawn, dann je `session_send`); ein Beobachter beendet nur seinen.
    generation: u64,
}

/// Laufende und beendete Aufträge an Childs dieses Knotens.
#[derive(Debug)]
pub struct Registry {
    tasks: Mutex<HashMap<SessionId, Task>>,
    changed: watch::Sender<u64>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            tasks: Mutex::default(),
            changed: watch::channel(0).0,
        }
    }
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Task>> {
        self.tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn bump(&self) {
        self.changed.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn running_count(tasks: &HashMap<SessionId, Task>, parent: SessionId) -> usize {
        tasks
            .values()
            .filter(|t| t.parent == parent && t.status == TaskStatus::Running)
            .count()
    }

    /// Startet einen Auftrag, wenn weniger als `limit` Childs des Parents laufen (AC3); sonst
    /// die Zahl der laufenden. Prüfen und Eintragen geschehen unter einer Sperre.
    fn begin(
        &self,
        parent: SessionId,
        child: SessionId,
        agent: &str,
        limit: u32,
    ) -> Result<u64, usize> {
        let mut tasks = self.lock();
        let running = Self::running_count(&tasks, parent);
        if tasks
            .get(&child)
            .is_some_and(|t| t.status == TaskStatus::Running)
        {
            return Err(running);
        }
        if running >= limit as usize {
            return Err(running);
        }
        let generation = tasks.get(&child).map_or(1, |t| t.generation + 1);
        tasks.insert(
            child,
            Task {
                parent,
                agent: agent.to_owned(),
                status: TaskStatus::Running,
                result: String::new(),
                cancel_reason: None,
                generation,
            },
        );
        drop(tasks);
        self.bump();
        Ok(generation)
    }

    /// Beendet den Auftrag (nur, wenn er läuft und, falls angegeben, `generation` passt).
    fn finish(
        &self,
        child: SessionId,
        generation: Option<u64>,
        status: TaskStatus,
        result: Option<String>,
        cancel_reason: Option<&str>,
    ) -> Option<Task> {
        let mut tasks = self.lock();
        let task = tasks.get_mut(&child)?;
        if task.status != TaskStatus::Running || generation.is_some_and(|g| g != task.generation) {
            return None;
        }
        task.status = status;
        if let Some(r) = result {
            task.result = r;
        }
        task.cancel_reason = cancel_reason.map(str::to_owned);
        let done = task.clone();
        drop(tasks);
        self.bump();
        Some(done)
    }

    fn forget(&self, child: SessionId) {
        self.lock().remove(&child);
        self.bump();
    }

    fn get(&self, child: SessionId) -> Option<Task> {
        self.lock().get(&child).cloned()
    }

    fn is_running(&self, child: SessionId) -> bool {
        self.lock()
            .get(&child)
            .is_some_and(|t| t.status == TaskStatus::Running)
    }

    /// Laufende Childs eines Parents.
    pub fn running_of(&self, parent: SessionId) -> Vec<SessionId> {
        let mut out: Vec<SessionId> = self
            .lock()
            .iter()
            .filter(|(_, t)| t.parent == parent && t.status == TaskStatus::Running)
            .map(|(id, _)| *id)
            .collect();
        out.sort();
        out
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
}

/// Grenzen aus `spawn` des Parents.
#[derive(Debug, Clone, Copy)]
struct Limits {
    max_concurrent: u32,
    worktree: WorktreeMode,
}

fn denied(detail: String) -> Problem {
    Problem::new(ProblemCode::SpawnDenied).detail(detail)
}

fn invalid(detail: &str) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail.to_owned())
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn worktree_mode(raw: &Value) -> Result<Option<WorktreeMode>, Problem> {
    match raw {
        Value::Null => Ok(None),
        Value::String(s) => serde_json::from_value(Value::String(s.clone()))
            .map(Some)
            .map_err(|_| invalid("`worktree` ist `new`, `inherit` oder `none`")),
        _ => Err(invalid("`worktree` ist `new`, `inherit` oder `none`")),
    }
}

/// Ein Child in Antworten der System-Tools.
fn child_view(record: &SessionRecord, task: Option<&Task>) -> Value {
    let mut v = json!({
        "session_id": record.id,
        "agent": task.map_or_else(|| record.title.clone(), |t| t.agent.clone()),
        "harness": record.harness,
        "session_status": record.status,
        "status": task.map_or("idle", |t| t.status.as_str()),
    });
    if let Some(t) = task {
        if t.status != TaskStatus::Running {
            v["result"] = Value::String(t.result.clone());
        }
        if let Some(r) = &t.cancel_reason {
            v["cancel_reason"] = Value::String(r.clone());
        }
    }
    if let Some(wt) = &record.worktree {
        v["worktree"] = json!({"path": wt.path, "branch": wt.branch, "base_sha": wt.base_sha});
    }
    v
}

impl SessionManager<'_> {
    fn subagents(&self) -> &Registry {
        &self.state().runtime.subagents
    }

    fn server_actor() -> Actor {
        Actor::System {
            component: SystemComponent::Server,
        }
    }

    /// Eine eigene Child-Session des Parents; fremde Sessions gelten als nicht gefunden.
    async fn own_child(&self, parent: SessionId, raw: &Value) -> Result<SessionRecord, Problem> {
        let not_found = || {
            Problem::new(ProblemCode::NotFound).detail(format!(
                "{} ist keine Child-Session dieser Session",
                raw.as_str().unwrap_or("?")
            ))
        };
        let id: SessionId = raw
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or_else(not_found)?;
        match self.state().store.session(self.org(), id).await {
            Ok(r) if r.parent_id == Some(parent) => Ok(r),
            _ => Err(not_found()),
        }
    }

    /// Grenzen und Tiefe laut Snapshot des Parents; ohne Snapshot die Defaults.
    async fn limits(&self, parent: &SessionRecord) -> Result<(Limits, Option<u32>), Problem> {
        let defaults = Limits {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            worktree: WorktreeMode::Inherit,
        };
        let Some(snapshot) = self.load_snapshot(parent.id).await? else {
            return Ok((defaults, None));
        };
        let (spec, _) = snapshot
            .agent()
            .map_err(|e| Problem::new(ProblemCode::AgentInvalid).detail(e.to_string()))?;
        let spawn = spec.spawn.as_ref();
        Ok((
            Limits {
                max_concurrent: spawn
                    .and_then(|s| s.max_concurrent)
                    .unwrap_or(DEFAULT_MAX_CONCURRENT),
                worktree: spawn
                    .and_then(|s| s.worktree)
                    .unwrap_or(WorktreeMode::Inherit),
            },
            Some(snapshot.spawn_depth_limit(&spec)),
        ))
    }

    /// `session_spawn` (AGT-007, AGT-009): Child-Session für einen erlaubten Sub-Agent starten,
    /// den Auftrag zustellen und – ohne `async` – auf sein Ende warten.
    ///
    /// Owner, Projekt und Workspace kommen aus der Parent-Session; Harness, Modell,
    /// Instructions und Parameter des Childs aus dem Snapshot des Parents.
    pub async fn spawn_child(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let record = self.state().store.session(self.org(), parent).await?;
        let created = self.created(parent).await?;
        let agent = args["agent"]
            .as_str()
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| invalid("`agent` fehlt"))?
            .to_owned();
        let prompt = args["prompt"]
            .as_str()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| invalid("`prompt` fehlt"))?
            .to_owned();
        let in_background = args["async"].as_bool().unwrap_or(false);
        let requested_worktree = worktree_mode(&args["worktree"])?;

        // Erlaubt, Harness und Tiefe laut Snapshot des Parents (AC2).
        let parent_snapshot = self.load_snapshot(parent).await?;
        let (limits, depth_limit) = self.limits(&record).await?;
        let (harness, model, snapshot) = match &parent_snapshot {
            Some(ps) => {
                let (spec, _) = ps
                    .agent()
                    .map_err(|e| Problem::new(ProblemCode::AgentInvalid).detail(e.to_string()))?;
                if !spec
                    .spawn
                    .as_ref()
                    .is_some_and(|s| s.agents.contains(&agent))
                {
                    return Err(denied(format!(
                        "not_allowed: „{agent}“ steht nicht in spawn.agents"
                    )));
                }
                let depth = ps.depth + 1;
                let limit = depth_limit.unwrap_or(DEFAULT_MAX_DEPTH);
                if depth > limit {
                    return Err(denied(format!(
                        "max_depth: Ein weiteres Child hätte Tiefe {depth}, erlaubt sind \
                         höchstens {limit} (spawn.max_depth)"
                    )));
                }
                let child = self
                    .child_snapshot(parent, &agent, &args["params"])
                    .await?
                    .ok_or_else(|| Problem::internal(&"Snapshot des Parents fehlt"))?;
                let (cspec, _) = child
                    .agent()
                    .map_err(|e| Problem::new(ProblemCode::AgentInvalid).detail(e.to_string()))?;
                (
                    cspec.executor.harness.clone(),
                    cspec.executor.model.clone(),
                    Some(child),
                )
            }
            None => {
                // Ältere Sessions ohne Snapshot: Harness aus dem Plan des Runners.
                if record.parent_id.is_some() {
                    return Err(denied(format!(
                        "max_depth: Childs ohne Agent-Snapshot starten keine Sub-Agents \
                         (Default max_depth: {DEFAULT_MAX_DEPTH})"
                    )));
                }
                let harness: beton_harness::HarnessId = args["harness"]
                    .as_str()
                    .unwrap_or_default()
                    .parse()
                    .map_err(|e| invalid(&format!("{e}")))?;
                (harness, args["model"].as_str().map(str::to_owned), None)
            }
        };
        if harness.as_str() == beton_harness::HarnessId::FAKE && !self.fake_allowed() {
            return Err(invalid(
                "Der Fake-Harness gibt es nur im Entwicklermodus (--dev).",
            ));
        }

        // Gleichzeitige Childs (AC3): Platz reservieren, bevor etwas angelegt wird.
        let id = SessionId::new();
        let generation = self
            .subagents()
            .begin(parent, id, &agent, limits.max_concurrent)
            .map_err(|running| {
                denied(format!(
                    "max_concurrent: Es laufen bereits {running} Sub-Agents gleichzeitig \
                     (spawn.max_concurrent: {}); warte mit session_wait",
                    limits.max_concurrent
                ))
            })?;
        let started = self
            .start_child(StartChild {
                id,
                parent: &record,
                parent_cwd: &created,
                agent: &agent,
                harness: &harness,
                model,
                snapshot,
                mode: requested_worktree.unwrap_or(limits.worktree),
                in_background,
            })
            .await;
        let child = match started {
            Ok(c) => c,
            Err(e) => {
                self.subagents().forget(id);
                return Err(e);
            }
        };
        self.give_task(parent, &child, &agent, generation, prompt)
            .await?;
        if in_background {
            let task = self.subagents().get(id);
            return Ok(child_view(&child, task.as_ref()));
        }
        self.await_tasks(&[id], false, SPAWN_TIMEOUT).await;
        let record = self.state().store.session(self.org(), id).await?;
        Ok(child_view(&record, self.subagents().get(id).as_ref()))
    }

    /// Legt die Child-Session samt Worktree und Snapshot an und startet ihren Runner.
    async fn start_child(&self, c: StartChild<'_>) -> Result<SessionRecord, Problem> {
        let parent_root = self.workspace_root(c.parent).await?;
        let worktree = match c.mode {
            WorktreeMode::New => {
                // Eigener Worktree vom aktuellen HEAD des Parents (AC1, SES-015).
                let root = parent_root.clone();
                let head = tokio::task::spawn_blocking(move || {
                    beton_git::Git::new(&root).text(&[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        "HEAD^{commit}",
                    ])
                })
                .await
                .map_err(|e| Problem::internal(&e))?
                .ok()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Problem::new(ProblemCode::NotAGitRepo).detail(
                        "worktree: new braucht ein Git-Repository mit mindestens einem Commit",
                    )
                })?;
                let request = CreateSession {
                    cwd: parent_root.display().to_string(),
                    title: Some(c.agent.to_owned()),
                    ..CreateSession::default()
                };
                let spec = WorktreeSpec {
                    branch: None,
                    base: Some(head),
                    fetch: false,
                };
                Some(self.create_worktree(c.id, &request, &spec).await?)
            }
            // `none`: nur lesende Sicht auf den Workspace des Parents; durchgesetzt von der
            // Sandbox ab M2 (SBX-003).
            WorktreeMode::Inherit | WorktreeMode::None => None,
        };
        let created = self
            .state()
            .store
            .create_session(
                self.org(),
                NewSession {
                    id: c.id,
                    owner: c.parent.owner,
                    kind: SessionKind::Subagent,
                    harness: c.harness.to_string(),
                    cwd: parent_root.display().to_string(),
                    model: c.model,
                    agent_ref: c.snapshot.as_ref().map(|s| s.reference.clone()),
                    project_id: c.parent.project_id,
                    parent_id: Some(c.parent.id),
                    trigger: SessionTrigger::Spawn,
                    home_node: self.state().local.node,
                    harness_opts: Value::Null,
                },
            )
            .await;
        let child = match created {
            Ok(s) => s,
            Err(e) => {
                if let Some(wt) = worktree {
                    crate::sessions::discard_worktree(wt).await;
                }
                return Err(e.into());
            }
        };
        if let Some(snapshot) = &c.snapshot {
            self.record_agent(child.id, snapshot, &[]).await?;
        }
        self.append(
            child.id,
            self.parent_actor(c.parent, c.parent_cwd),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: c.agent.to_owned(),
                source: TitleSource::Generated,
            }),
        )
        .await?;
        if let Some(wt) = &worktree {
            self.log_worktree(child.id, wt).await?;
        }
        let record = self.state().store.session(self.org(), child.id).await?;
        self.launch(&record, None).await?;
        self.wait_connected(child.id).await?;
        self.append(
            c.parent.id,
            Self::server_actor(),
            EventPayload::AgentSpawned(AgentSpawned {
                child_session_id: child.id,
                agent_ref: c
                    .snapshot
                    .as_ref()
                    .map_or_else(|| c.agent.to_owned(), |s| s.reference.clone()),
                harness: c.harness.to_string(),
                r#async: c.in_background,
            }),
        )
        .await?;
        Ok(self.state().store.session(self.org(), child.id).await?)
    }

    /// Akteur der Nachrichten, die der Parent-Agent an ein Child schickt.
    fn parent_actor(
        &self,
        parent: &SessionRecord,
        created: &beton_core::event::SessionCreated,
    ) -> Actor {
        Actor::Agent {
            id: None,
            harness: parent.harness.clone(),
            agent_ref: created.agent_ref.clone(),
        }
    }

    /// Stellt einem Child einen Auftrag zu und beobachtet ihn bis zum Turn-Ende.
    async fn give_task(
        &self,
        parent: SessionId,
        child: &SessionRecord,
        agent: &str,
        generation: u64,
        text: String,
    ) -> Result<(), Problem> {
        let parent_record = self.state().store.session(self.org(), parent).await?;
        let created = self.created(parent).await?;
        let from = self
            .state()
            .store
            .session(self.org(), child.id)
            .await?
            .head_seq;
        let delivered = self
            .input_as(
                child.id,
                text,
                PrincipalId::User(parent_record.owner),
                InputMode::Queue,
                self.parent_actor(&parent_record, &created),
            )
            .await;
        if let Err(e) = delivered {
            self.subagents().finish(
                child.id,
                Some(generation),
                TaskStatus::Failed,
                Some(format!(
                    "Auftrag nicht zugestellt: {}",
                    e.detail.unwrap_or_default()
                )),
                None,
            );
            return Err(Problem::new(ProblemCode::Unavailable)
                .detail(format!("Auftrag an {agent} nicht zugestellt")));
        }
        let state = self.state().clone();
        let child = child.id;
        tokio::spawn(async move {
            let m = state.sessions();
            if let Some((status, result)) = m.task_end(child, from, generation).await {
                m.complete(parent, child, generation, status, result).await;
            }
        });
        Ok(())
    }

    /// Wartet auf das Ende des Turns nach `from`; `None`, wenn der Auftrag vorher anders
    /// endete (z. B. abgebrochen).
    async fn task_end(
        &self,
        child: SessionId,
        from: u64,
        generation: u64,
    ) -> Option<(TaskStatus, String)> {
        let mut seq = from;
        let mut last = String::new();
        loop {
            let current = self.subagents().get(child);
            if current.is_none_or(|t| t.generation != generation || t.status != TaskStatus::Running)
            {
                return None;
            }
            let page = self
                .state()
                .store
                .events(self.org(), child, seq, 500)
                .await
                .ok()?;
            for e in &page {
                seq = e.seq;
                match e.payload() {
                    Some(EventPayload::MessageCompleted(m)) if m.role == MessageRole::Assistant => {
                        last = m
                            .content
                            .iter()
                            .filter_map(|c| c["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("");
                    }
                    Some(EventPayload::TurnCompleted(_)) => {
                        return Some((TaskStatus::Completed, last));
                    }
                    Some(EventPayload::TurnFailed(f)) => {
                        let why = if last.is_empty() {
                            f.problem["detail"]
                                .as_str()
                                .or_else(|| f.problem["message"].as_str())
                                .unwrap_or("Turn fehlgeschlagen")
                                .to_owned()
                        } else {
                            last
                        };
                        return Some((TaskStatus::Failed, why));
                    }
                    Some(EventPayload::TurnInterrupted(_)) => {
                        return Some((TaskStatus::Interrupted, last));
                    }
                    Some(EventPayload::SessionStatus(st))
                        if matches!(
                            st.status,
                            beton_core::event::SessionStatus::Failed
                                | beton_core::event::SessionStatus::Stopped
                        ) =>
                    {
                        let why = st.reason.clone().unwrap_or_default();
                        return Some((
                            TaskStatus::Failed,
                            if last.is_empty() { why } else { last },
                        ));
                    }
                    _ => {}
                }
            }
            if page.is_empty() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    /// Kosten des Teilbaums einer Session (eigene und die aller Nachfahren).
    pub async fn subtree_cost(&self, session: SessionId) -> Result<i64, Problem> {
        Ok(self
            .state()
            .store
            .session_tree(self.org(), session)
            .await?
            .iter()
            .map(|e| e.session.cost_micro)
            .sum())
    }

    /// Beendet einen Auftrag und meldet `agent.completed` im Parent (AC4).
    async fn complete(
        &self,
        parent: SessionId,
        child: SessionId,
        generation: u64,
        status: TaskStatus,
        result: String,
    ) {
        if self
            .subagents()
            .finish(child, Some(generation), status, Some(result.clone()), None)
            .is_none()
        {
            return;
        }
        self.report(parent, child, status, &result, None).await;
    }

    async fn report(
        &self,
        parent: SessionId,
        child: SessionId,
        status: TaskStatus,
        result: &str,
        cancel_reason: Option<&str>,
    ) {
        let cost = self.subtree_cost(child).await.ok();
        let event = EventPayload::AgentCompleted(AgentCompleted {
            child_session_id: child,
            status: status.as_str().into(),
            summary: Some(truncate(result, SUMMARY_MAX)).filter(|s| !s.is_empty()),
            cost_micro: cost,
            cancel_reason: cancel_reason.map(str::to_owned),
        });
        if let Err(e) = self.append(parent, Self::server_actor(), event).await {
            tracing::warn!(session_id = %parent, "agent.completed nicht geschrieben: {}", e.code.as_str());
        }
    }

    /// Wartet, bis die Aufträge fertig sind (`any`: einer), höchstens `timeout`.
    async fn await_tasks(&self, ids: &[SessionId], any: bool, timeout: Duration) -> bool {
        let mut rx = self.subagents().subscribe();
        let deadline = Instant::now() + timeout;
        loop {
            let finished = ids.iter().filter(|id| !self.subagents().is_running(**id));
            let done = if any {
                ids.is_empty() || finished.count() > 0
            } else {
                finished.count() == ids.len()
            };
            let left = deadline.saturating_duration_since(Instant::now());
            if done || left.is_zero() {
                return done;
            }
            if tokio::time::timeout(left, rx.changed())
                .await
                .is_ok_and(|r| r.is_err())
            {
                return done;
            }
        }
    }

    /// `session_send`: weiterer Auftrag an ein eigenes, nicht laufendes Child.
    pub async fn send_child(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let child = self.own_child(parent, &args["session_id"]).await?;
        let text = args["text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| invalid("`text` fehlt"))?
            .to_owned();
        let record = self.state().store.session(self.org(), parent).await?;
        let (limits, _) = self.limits(&record).await?;
        let agent = self
            .subagents()
            .get(child.id)
            .map_or_else(|| child.title.clone(), |t| t.agent);
        if self.subagents().is_running(child.id) {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Das Child arbeitet noch; warte mit session_wait"));
        }
        let generation = self
            .subagents()
            .begin(parent, child.id, &agent, limits.max_concurrent)
            .map_err(|running| {
                denied(format!(
                    "max_concurrent: Es laufen bereits {running} Sub-Agents gleichzeitig \
                     (spawn.max_concurrent: {})",
                    limits.max_concurrent
                ))
            })?;
        self.append(
            parent,
            Self::server_actor(),
            EventPayload::AgentMessage(AgentMessage {
                from_session: parent,
                to_session: child.id,
                text: text.clone(),
            }),
        )
        .await?;
        self.give_task(parent, &child, &agent, generation, text)
            .await?;
        Ok(child_view(&child, self.subagents().get(child.id).as_ref()))
    }

    /// `session_wait`: auf eigene Childs warten (`mode: all|any`, `timeout`).
    pub async fn wait_children(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let any = match args["mode"].as_str() {
            None | Some("all") => false,
            Some("any") => true,
            Some(_) => return Err(invalid("`mode` ist `all` oder `any`")),
        };
        let timeout = match &args["timeout"] {
            Value::Null => WAIT_DEFAULT,
            Value::String(t) if DurationText::is_valid(t) => {
                serde_json::from_value::<DurationText>(Value::String(t.clone()))
                    .ok()
                    .and_then(|d| d.to_duration())
                    .ok_or_else(|| invalid("`timeout` ist zu groß"))?
                    .min(WAIT_MAX)
            }
            _ => return Err(invalid("`timeout` ist eine Dauer wie `90s`, `30m`, `2h`")),
        };
        let ids: Vec<SessionId> = match args["ids"].as_array() {
            Some(list) if !list.is_empty() => {
                let mut out = Vec::new();
                for raw in list {
                    out.push(self.own_child(parent, raw).await?.id);
                }
                out
            }
            _ => self.subagents().running_of(parent),
        };
        let done = self.await_tasks(&ids, any, timeout).await;
        let mut results = Vec::new();
        for id in &ids {
            let record = self.state().store.session(self.org(), *id).await?;
            results.push(child_view(&record, self.subagents().get(*id).as_ref()));
        }
        Ok(json!({"done": done, "mode": if any { "any" } else { "all" }, "results": results}))
    }

    /// `session_status`: Stand eines eigenen Childs.
    pub async fn child_status(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let child = self.own_child(parent, &args["session_id"]).await?;
        Ok(child_view(&child, self.subagents().get(child.id).as_ref()))
    }

    /// `session_list`: die direkten Childs (die startbaren Agents ergänzt der Runner).
    pub async fn list_children(&self, parent: SessionId) -> Result<Value, Problem> {
        let tree = self.state().store.session_tree(self.org(), parent).await?;
        let children: Vec<Value> = tree
            .iter()
            .filter(|e| e.depth == 1)
            .map(|e| child_view(&e.session, self.subagents().get(e.session.id).as_ref()))
            .collect();
        Ok(json!({"children": children}))
    }

    /// `session_cancel`: ein eigenes Child samt seinen Childs abbrechen.
    pub async fn cancel_child(&self, parent: SessionId, args: &Value) -> Result<Value, Problem> {
        let child = self.own_child(parent, &args["session_id"]).await?;
        self.halt(parent, child.id, CANCELLED_BY_PARENT).await;
        let record = self.state().store.session(self.org(), child.id).await?;
        Ok(child_view(&record, self.subagents().get(child.id).as_ref()))
    }

    /// Beendet ein Child: Auftrag als abgebrochen melden, Turn unterbrechen, Runner stoppen
    /// (das stoppt rekursiv auch dessen Childs).
    async fn halt(&self, parent: SessionId, child: SessionId, reason: &str) {
        let task = self.subagents().get(child);
        if let Some(t) = self.subagents().finish(
            child,
            None,
            TaskStatus::Cancelled,
            task.map(|t| t.result),
            Some(reason),
        ) {
            self.report(
                parent,
                child,
                TaskStatus::Cancelled,
                &t.result,
                Some(reason),
            )
            .await;
        }
        let runners = &self.state().runtime.runners;
        if runners.connected(child) {
            let _ = runners
                .deliver(
                    child,
                    "turn.interrupt",
                    Value::Null,
                    self.state().runtime.tunnel.cmd_timeout,
                )
                .await;
        }
        self.stop(child, reason).await;
    }

    /// Bricht alle laufenden Childs ab, weil der Parent abgebrochen wurde (AC5).
    pub(crate) fn cancel_children<'b>(
        &'b self,
        parent: SessionId,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'b>> {
        Box::pin(async move {
            for child in self.subagents().running_of(parent) {
                self.halt(parent, child, PARENT_CANCELLED).await;
            }
        })
    }

    /// Sub-Agent-Baum einer Session für die UI (WEB-012).
    pub async fn subagent_tree(&self, root: SessionId) -> Result<SubagentTree, Problem> {
        let store = &self.state().store;
        let entries = store.session_tree(self.org(), root).await?;
        let mut nodes = Vec::with_capacity(entries.len());
        for e in &entries {
            let s = &e.session;
            let created = self.created(s.id).await.ok();
            let agent = match store
                .last_event_of_type(self.org(), s.id, "agent.resolved")
                .await?
                .as_ref()
                .and_then(|ev| ev.payload())
            {
                Some(EventPayload::AgentResolved(r)) => Some(r.name.clone()),
                _ => None,
            };
            let auth_source = match store
                .last_event_of_type(self.org(), s.id, "cost.delta")
                .await?
                .as_ref()
                .and_then(|ev| ev.payload())
            {
                Some(EventPayload::CostDelta(c)) => serde_json::to_value(c.auth_source)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned)),
                _ => None,
            };
            let task = self.subagents().get(s.id);
            nodes.push(SubagentNode {
                id: s.id.to_string(),
                parent_id: s.parent_id.map(|p| p.to_string()),
                depth: e.depth,
                title: s.title.clone(),
                agent,
                harness: s.harness.clone(),
                model: created.and_then(|c| c.model),
                status: serde_json::to_value(s.status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                task: task.map(|t| t.status.as_str().to_owned()),
                cost_micro: s.cost_micro,
                subtree_cost_micro: 0,
                tokens: e.input_tokens + e.output_tokens,
                subtree_tokens: 0,
                auth_source,
                worktree: s.worktree.as_ref().map(|w| SessionWorktree {
                    path: w.path.clone(),
                    branch: w.branch.clone(),
                    base: w.base.clone(),
                    base_sha: w.base_sha.clone(),
                }),
                created_at: s.created_at.to_string(),
            });
        }
        // Kumulierte Werte von unten nach oben (die Liste ist nach Tiefe sortiert).
        let index: BTreeMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        for n in &mut nodes {
            n.subtree_cost_micro = n.cost_micro;
            n.subtree_tokens = n.tokens;
        }
        for i in (0..nodes.len()).rev() {
            let (cost, tokens) = (nodes[i].subtree_cost_micro, nodes[i].subtree_tokens);
            if let Some(p) = nodes[i]
                .parent_id
                .as_ref()
                .and_then(|p| index.get(p))
                .copied()
                && p != i
                && nodes[i].depth > 0
            {
                nodes[p].subtree_cost_micro += cost;
                nodes[p].subtree_tokens += tokens;
            }
        }
        Ok(SubagentTree {
            root: root.to_string(),
            nodes,
        })
    }
}

/// Eingaben für [`SessionManager::start_child`].
struct StartChild<'a> {
    id: SessionId,
    parent: &'a SessionRecord,
    parent_cwd: &'a beton_core::event::SessionCreated,
    agent: &'a str,
    harness: &'a beton_harness::HarnessId,
    model: Option<String>,
    snapshot: Option<beton_agents::AgentSnapshot>,
    mode: WorktreeMode,
    in_background: bool,
}

/// Session-Baum unter einer Session (WEB-012).
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct SubagentTree {
    /// Die angefragte Session.
    pub root: String,
    /// Wurzel zuerst, dann nach Tiefe und Anlage.
    pub nodes: Vec<SubagentNode>,
}

/// Eine Session im Sub-Agent-Baum.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct SubagentNode {
    pub id: String,
    /// Parent im Baum; fehlt an der Wurzel ohne Parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent_id: Option<String>,
    /// Abstand zur angefragten Session (0 = sie selbst).
    pub depth: u32,
    pub title: String,
    /// Name des Agents laut `agent.resolved`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<String>,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    /// Status der Session (`SessionStatus`).
    pub status: String,
    /// Stand des aktuellen Auftrags vom Parent: `running`, `completed`, `failed`,
    /// `interrupted`, `cancelled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub task: Option<String>,
    /// Eigene Kosten in Mikro-Einheiten (bei Subscription 0).
    pub cost_micro: i64,
    /// Kosten samt aller Nachfahren.
    pub subtree_cost_micro: i64,
    /// Eigene Tokens (Eingabe inkl. Cache plus Ausgabe).
    pub tokens: u64,
    /// Tokens samt aller Nachfahren.
    pub subtree_tokens: u64,
    /// `vendor_cli` (Subscription), `api_key` … laut letztem `cost.delta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auth_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub worktree: Option<SessionWorktree>,
    pub created_at: String,
}

/// Sub-Agent-Baum einer Session (AGT-009, WEB-012): die Session und alle Nachfahren mit
/// Harness, Status und kumulierten Kosten.
#[utoipa::path(get, path = "/v1/sessions/{id}/subagents", tag = "sessions",
    params(("id" = String, Path)),
    responses((status = 200, description = "Session-Baum", body = SubagentTree)))]
pub async fn get_subagents(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<SubagentTree>> {
    let id: SessionId = id
        .parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {id}")))?;
    Ok(axum::Json(state.sessions().subagent_tree(id).await?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn agt_009_ac3_registry_reserves_concurrent_slots_atomically() {
        let r = Registry::default();
        let parent = SessionId::new();
        let kids: Vec<SessionId> = (0..4).map(|_| SessionId::new()).collect();
        for k in &kids[..3] {
            assert_eq!(r.begin(parent, *k, "a", 3), Ok(1));
        }
        assert_eq!(r.begin(parent, kids[3], "a", 3), Err(3));
        // Ein anderer Parent hat eigene Plätze.
        assert!(r.begin(SessionId::new(), kids[3], "a", 3).is_ok());
        r.forget(kids[3]);
        // Fertig = Platz frei; ein neuer Auftrag an dasselbe Child zählt hoch.
        assert!(
            r.finish(kids[0], Some(1), TaskStatus::Completed, None, None)
                .is_some()
        );
        assert_eq!(r.running_of(parent).len(), 2);
        assert_eq!(r.begin(parent, kids[0], "a", 3), Ok(2));
        // Ein alter Beobachter beendet den neuen Auftrag nicht.
        assert!(
            r.finish(kids[0], Some(1), TaskStatus::Completed, None, None)
                .is_none()
        );
        assert!(r.is_running(kids[0]));
    }

    #[test]
    fn summaries_are_cut_on_char_boundaries() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("äöü", 3), "ä…");
    }
}
