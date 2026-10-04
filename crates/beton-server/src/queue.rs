//! Serverseitige Queue je Session (SES-004, SES-005 AC2).
//!
//! Inputs während eines laufenden Turns landen in der Queue und werden nach dem Turn-Ende
//! nacheinander als eigene Turns ausgeführt. Jede Änderung erzeugt `queue.updated` mit dem
//! vollständigen Stand; nach einem Daemon-Neustart stellt das letzte `queue.updated` die Queue
//! wieder her. Nach `turn.interrupted` bleibt eine nicht leere Queue pausiert, bis ein Client
//! fortsetzt oder neuen Input sendet.
//!
//! Zwei Sperren je Session: ein kurzer, synchroner Zustand (`busy`, Items), den auch der
//! Tunnel aktualisiert, ohne zu warten, und eine asynchrone Operationssperre, unter der
//! Änderungen und Zustellungen nacheinander laufen. Der Tunnel nimmt nie die
//! Operationssperre, sonst käme ein `cmd.result` nicht mehr an, auf das eine Zustellung wartet.
//!
//! Den Policy-Hook „vor Model-Request“ beim Dequeue bringt die Policy-Engine (POL, M2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{
    Actor, ErrorEvent, Event, EventPayload, MessageCompleted, MessageRole, QueueItem, QueueUpdated,
    SessionStatus, SystemComponent,
};
use beton_core::id::{InputId, OrgId, PrincipalId, SessionId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};

use crate::hub::EventService;
use crate::problem::{Problem, ProblemCode};
use crate::tunnel::RunnerRegistry;

/// Zustand der Queue einer Session.
#[derive(Debug, Clone, Default)]
struct QueueState {
    items: Vec<QueueItem>,
    paused: bool,
    /// Ein Turn läuft oder ist zugestellt und noch nicht gestartet.
    busy: bool,
    loaded: bool,
    /// `busy` stammt aus beobachteten Events (statt aus der Projektion beim Laden).
    busy_known: bool,
}

#[derive(Default)]
struct Slot {
    state: Mutex<QueueState>,
    op: Arc<tokio::sync::Mutex<()>>,
}

/// Queues aller Sessions dieses Knotens.
#[derive(Default)]
pub struct Queues {
    slots: Mutex<HashMap<SessionId, Arc<Slot>>>,
}

impl std::fmt::Debug for Queues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Queues")
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Queues {
    fn slot(&self, session: SessionId) -> Arc<Slot> {
        lock(&self.slots).entry(session).or_default().clone()
    }

    /// Vergisst die Queue einer gelöschten Session.
    pub fn forget(&self, session: SessionId) {
        lock(&self.slots).remove(&session);
    }
}

/// Was die Queue zum Arbeiten braucht; aus Handlern und aus dem Tunnel erzeugbar.
#[derive(Clone)]
pub struct QueueCtx {
    pub events: EventService,
    pub org: OrgId,
    pub runners: Arc<RunnerRegistry>,
    pub cmd_timeout: Duration,
    pub queues: Arc<Queues>,
}

impl std::fmt::Debug for QueueCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueueCtx")
    }
}

/// Ergebnis einer Eingabe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    /// Neuer Turn gestartet.
    Started { input_id: InputId, turn_id: String },
    /// In die Queue gestellt.
    Queued { input_id: InputId },
    /// In den laufenden Turn eingespeist.
    Steered { input_id: InputId },
}

impl Accepted {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Started { input_id, turn_id } => {
                json!({"input_id": input_id.to_string(), "turn_id": turn_id, "status": "started"})
            }
            Self::Queued { input_id } => {
                json!({"input_id": input_id.to_string(), "status": "queued"})
            }
            Self::Steered { input_id } => {
                json!({"input_id": input_id.to_string(), "status": "steered"})
            }
        }
    }
}

pub fn user_actor(by: PrincipalId) -> Actor {
    Actor::User {
        id: by,
        device_id: None,
    }
}

fn system() -> Actor {
    Actor::System {
        component: SystemComponent::Server,
    }
}

/// Hängt ein Event an; wiederholt bei Sequenzkonflikten (der Runner schreibt parallel).
pub async fn append(
    events: &EventService,
    org: OrgId,
    session: SessionId,
    actor: Actor,
    payload: EventPayload,
) -> Result<(), Problem> {
    for _ in 0..10 {
        let record = events.store.session(org, session).await?;
        let e = Event::new(session, 0, actor.clone(), payload.clone());
        match events
            .append(org, session, record.head_seq, record.epoch, vec![e])
            .await
        {
            Err(beton_store::Error::SeqConflict { .. }) => continue,
            other => return other.map(|_| ()).map_err(Problem::from),
        }
    }
    Err(Problem::new(ProblemCode::SeqConflict))
}

/// Exklusiver Zugriff auf die Queue einer Session (Operationssperre gehalten).
pub struct Guard {
    ctx: QueueCtx,
    session: SessionId,
    slot: Arc<Slot>,
    _op: tokio::sync::OwnedMutexGuard<()>,
}

impl QueueCtx {
    /// Nimmt die Operationssperre und lädt den Zustand beim ersten Zugriff.
    pub async fn lock(&self, session: SessionId) -> Result<Guard, Problem> {
        let slot = self.queues.slot(session);
        let op = slot.op.clone().lock_owned().await;
        let guard = Guard {
            ctx: self.clone(),
            session,
            slot,
            _op: op,
        };
        guard.load().await?;
        Ok(guard)
    }

    /// Nach jedem gespeicherten Runner-Batch (Tunnel, ohne zu warten): verfolgt `busy`,
    /// pausiert nach `turn.interrupted` (SES-005 AC2) und arbeitet nach dem Turn-Ende die
    /// Queue ab (SES-004 AC1).
    pub fn observe(&self, session: SessionId, written: &[Event]) {
        let slot = self.queues.slot(session);
        let mut follow_up = None;
        {
            let mut st = lock(&slot.state);
            for e in written {
                match e.payload() {
                    Some(EventPayload::TurnStarted(_)) => {
                        st.busy = true;
                        st.busy_known = true;
                    }
                    Some(EventPayload::TurnInterrupted(_)) => {
                        st.busy = false;
                        st.busy_known = true;
                        if !st.items.is_empty() && !st.paused {
                            st.paused = true;
                            follow_up = Some(FollowUp::Pause);
                        }
                    }
                    Some(EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_)) => {
                        st.busy = false;
                        st.busy_known = true;
                        if !st.items.is_empty() && !st.paused {
                            follow_up = Some(FollowUp::Drain);
                        }
                    }
                    Some(EventPayload::SessionStatus(s))
                        if matches!(s.status, SessionStatus::Stopped | SessionStatus::Failed) =>
                    {
                        st.busy = false;
                        st.busy_known = true;
                    }
                    _ => {}
                }
            }
        }
        if let Some(f) = follow_up {
            let ctx = self.clone();
            tokio::spawn(async move {
                let result = match ctx.lock(session).await {
                    Ok(mut g) => match f {
                        FollowUp::Pause => g.publish(system()).await,
                        FollowUp::Drain => g.drain().await,
                    },
                    Err(e) => Err(e),
                };
                if let Err(p) = result {
                    tracing::warn!(%session, code = p.code.as_str(), "Queue: Folgeschritt gescheitert");
                }
            });
        }
    }
}

enum FollowUp {
    Pause,
    Drain,
}

impl Guard {
    fn state(&self) -> std::sync::MutexGuard<'_, QueueState> {
        lock(&self.slot.state)
    }

    async fn load(&self) -> Result<(), Problem> {
        if self.state().loaded {
            return Ok(());
        }
        let store = &self.ctx.events.store;
        let last = store
            .last_event_of_type(self.ctx.org, self.session, "queue.updated")
            .await?;
        let record = store.session(self.ctx.org, self.session).await?;
        let running = matches!(
            record.status,
            SessionStatus::Running | SessionStatus::WaitingApproval
        ) && self.ctx.runners.connected(self.session);
        let saved = match last.as_ref().and_then(Event::payload) {
            Some(EventPayload::QueueUpdated(q)) => Some(q.clone()),
            _ => None,
        };
        let mut st = self.state();
        if !st.loaded {
            if let Some(q) = saved {
                st.items = q.items;
                st.paused = q.paused;
            }
            if !st.busy_known {
                st.busy = running;
            }
            st.loaded = true;
        }
        Ok(())
    }

    /// Aktueller Stand für die API.
    pub fn snapshot(&self) -> QueueUpdated {
        let st = self.state();
        QueueUpdated {
            items: st.items.clone(),
            paused: st.paused,
        }
    }

    pub fn busy(&self) -> bool {
        self.state().busy
    }

    pub fn is_empty(&self) -> bool {
        self.state().items.is_empty()
    }

    /// Schreibt `queue.updated` mit dem vollständigen Stand.
    pub async fn publish(&mut self, actor: Actor) -> Result<(), Problem> {
        let snapshot = self.snapshot();
        append(
            &self.ctx.events,
            self.ctx.org,
            self.session,
            actor,
            EventPayload::QueueUpdated(snapshot),
        )
        .await
    }

    /// Reiht einen Input ein; neuer Input hebt eine Pause auf (SES-005).
    pub async fn push(&mut self, text: String, by: PrincipalId) -> Result<InputId, Problem> {
        let id = InputId::new();
        {
            let mut st = self.state();
            st.items.push(QueueItem {
                id: id.to_string(),
                author: by,
                text,
                attachments: Vec::new(),
                created_at: Timestamp::now(),
            });
            st.paused = false;
        }
        self.publish(user_actor(by)).await?;
        Ok(id)
    }

    fn position(&self, item: &str) -> Result<usize, Problem> {
        self.state()
            .items
            .iter()
            .position(|i| i.id == item)
            .ok_or_else(|| {
                Problem::new(ProblemCode::NotFound).detail(format!("Queue-Eintrag {item}"))
            })
    }

    pub async fn edit(&mut self, item: &str, text: String, by: PrincipalId) -> Result<(), Problem> {
        if text.trim().is_empty() {
            return Err(Problem::new(ProblemCode::ValidationFailed).detail("Text ist leer"));
        }
        let at = self.position(item)?;
        self.state().items[at].text = text;
        self.publish(user_actor(by)).await
    }

    pub async fn delete(&mut self, item: &str, by: PrincipalId) -> Result<(), Problem> {
        let at = self.position(item)?;
        self.state().items.remove(at);
        self.publish(user_actor(by)).await
    }

    /// Verschiebt einen Eintrag an `position` (0 = als Nächstes; größer = ans Ende).
    pub async fn reorder(
        &mut self,
        item: &str,
        position: usize,
        by: PrincipalId,
    ) -> Result<(), Problem> {
        let at = self.position(item)?;
        {
            let mut st = self.state();
            let moved = st.items.remove(at);
            let to = position.min(st.items.len());
            st.items.insert(to, moved);
        }
        self.publish(user_actor(by)).await
    }

    /// Nimmt einen Eintrag heraus (z. B. „Als Steer senden“).
    pub async fn take(&mut self, item: &str, by: PrincipalId) -> Result<QueueItem, Problem> {
        let at = self.position(item)?;
        let taken = self.state().items.remove(at);
        self.publish(user_actor(by)).await?;
        Ok(taken)
    }

    /// Stellt einen herausgenommenen Eintrag wieder an den Anfang.
    pub async fn put_front(&mut self, item: QueueItem, by: PrincipalId) -> Result<(), Problem> {
        self.state().items.insert(0, item);
        self.publish(user_actor(by)).await
    }

    /// Hebt die Pause auf und arbeitet weiter, falls kein Turn läuft.
    pub async fn resume(&mut self, by: PrincipalId) -> Result<(), Problem> {
        let changed = {
            let mut st = self.state();
            std::mem::replace(&mut st.paused, false)
        };
        if changed {
            self.publish(user_actor(by)).await?;
        }
        self.drain().await
    }

    /// Startet einen Turn: Nachricht ins Log, dann `input.submit` an den Runner.
    pub async fn start_turn(
        &mut self,
        input_id: InputId,
        text: String,
        by: PrincipalId,
    ) -> Result<Accepted, Problem> {
        self.state().busy = true;
        let result = self.deliver_turn(&text, by).await;
        match result {
            Ok(turn_id) => Ok(Accepted::Started { input_id, turn_id }),
            Err(e) => {
                self.state().busy = false;
                Err(e)
            }
        }
    }

    async fn deliver_turn(&self, text: &str, by: PrincipalId) -> Result<String, Problem> {
        // Die Eingabe gehört zum Verlauf (Chat, Replay, Export): als Nachricht des Nutzers
        // vor der Zustellung, damit sie vor der Antwort steht.
        self.user_message(text, by).await?;
        let result = self
            .ctx
            .runners
            .deliver(
                self.session,
                "input.submit",
                json!({"text": text}),
                self.ctx.cmd_timeout,
            )
            .await?;
        Ok(result["turn_id"].as_str().unwrap_or_default().to_owned())
    }

    async fn user_message(&self, text: &str, by: PrincipalId) -> Result<(), Problem> {
        append(
            &self.ctx.events,
            self.ctx.org,
            self.session,
            user_actor(by),
            EventPayload::MessageCompleted(MessageCompleted {
                message_id: format!("msg_user_{}", InputId::new()),
                role: MessageRole::User,
                content: vec![json!({"type": "text", "text": text})],
                author: Some(by),
            }),
        )
        .await
    }

    /// Speist Text in den laufenden Turn ein (`input.steer`). `Ok(false)`, wenn der Runner
    /// keinen laufenden Turn mehr hat; dann entscheidet der Aufrufer.
    pub async fn steer(&mut self, text: &str, by: PrincipalId) -> Result<bool, Problem> {
        match self
            .ctx
            .runners
            .deliver(
                self.session,
                "input.steer",
                json!({"text": text}),
                self.ctx.cmd_timeout,
            )
            .await
        {
            Ok(_) => {
                self.user_message(text, by).await?;
                Ok(true)
            }
            // Der Turn startet gerade erst oder ist eben zu Ende: Das Turn-Ende in den
            // Events entscheidet, nicht diese Antwort.
            Err(p) if p.code == ProblemCode::NoActiveTurn => Ok(false),
            Err(p) => Err(p),
        }
    }

    /// Führt den nächsten Eintrag aus, wenn kein Turn läuft und die Queue nicht pausiert.
    pub async fn drain(&mut self) -> Result<(), Problem> {
        let next = {
            let st = self.state();
            if st.busy || st.paused || st.items.is_empty() {
                return Ok(());
            }
            st.items[0].clone()
        };
        self.state().items.remove(0);
        self.publish(system()).await?;
        let input_id: InputId = next.id.parse().unwrap_or_else(|_| InputId::new());
        if let Err(p) = self.start_turn(input_id, next.text, next.author).await {
            // Niemand wartet auf diese Antwort: als Fehler im Verlauf sichtbar machen.
            let problem = serde_json::to_value(&p).unwrap_or(Value::Null);
            append(
                &self.ctx.events,
                self.ctx.org,
                self.session,
                system(),
                EventPayload::Error(ErrorEvent { problem }),
            )
            .await?;
            return Err(p);
        }
        Ok(())
    }
}
