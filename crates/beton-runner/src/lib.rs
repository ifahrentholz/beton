//! Runner: führt genau eine Session aus, supervidiert den Harness und streamt Events über
//! den Tunnel an den Home-Knoten (RUN-002, RUN-003, PROTO-015).
//!
//! Der Runner baut nur **ausgehende** Verbindungen auf (Unix-Socket des lokalen Daemons) und
//! öffnet keinen Port. Unbestätigte Events hält er bis 64 MiB vor und sendet sie nach einem
//! Reconnect erneut; darüber pausiert er das Lesen vom Harness.

pub mod state;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beton_core::event::{Actor, Event, EventPayload, SessionStatus, SessionStatusChanged};
use beton_core::id::{RunnerId, SessionId};
use beton_harness::process::RealLauncher;
use beton_harness::registry::Registry;
use beton_harness::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessId, HarnessSession, HostEnv,
    NormalizedEvent, SessionSpec, Shutdown, UserInput,
};
use beton_proto::tunnel::{PROTOCOL, PeerKind, RseqEvent, SUBPROTOCOL, TunnelDown, TunnelUp};
use beton_proto::ws::Backoff;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::state::{Lifecycle, RunnerState};

/// Obergrenze für unbestätigte Events (PROTO-015).
pub const MAX_UNACKED_BYTES: usize = 64 * 1024 * 1024;

/// Umgebungsvariablen, mit denen der Host den Runner startet.
pub mod env {
    pub const SOCKET: &str = "BETON_TUNNEL_SOCKET";
    pub const SESSION: &str = "BETON_SESSION_ID";
    pub const RUNNER: &str = "BETON_RUNNER_ID";
    pub const EPOCH: &str = "BETON_EPOCH";
    pub const HARNESS: &str = "BETON_HARNESS";
    pub const SCENARIO: &str = "BETON_SCENARIO";
    pub const MODEL: &str = "BETON_MODEL";
    pub const PARENT_PID: &str = "BETON_RUNNER_PARENT_PID";
    pub const DEV: &str = "BETON_DEV";
    /// Native Session-Referenz zum Fortsetzen (SES-003).
    pub const RESUME: &str = "BETON_RESUME";
}

/// Startparameter eines Runners. Das Token kommt über stdin, nie über Env oder argv
/// (AUTH-011).
#[derive(Debug, Clone)]
pub struct RunnerBoot {
    pub socket: PathBuf,
    pub token: String,
    pub session_id: SessionId,
    pub runner_id: RunnerId,
    pub epoch: u64,
    pub harness: HarnessId,
    pub workdir: PathBuf,
    pub scenario: Option<PathBuf>,
    pub model: Option<String>,
    pub parent_pid: Option<u32>,
    pub dev: bool,
    pub resume: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("Startparameter fehlen oder sind ungültig: {0}")]
    Boot(String),
    #[error("Harness: {0}")]
    Harness(#[from] beton_harness::HarnessError),
    #[error("Tunnel: {0}")]
    Tunnel(String),
    #[error("stale_epoch: Session gehört inzwischen einem anderen Schreiber")]
    StaleEpoch,
}

impl RunnerBoot {
    /// Liest Env und das Token aus stdin (erste Zeile).
    pub fn from_env_and_stdin() -> Result<Self, RunnerError> {
        let var = |k: &str| std::env::var(k).map_err(|_| RunnerError::Boot(format!("{k} fehlt")));
        let parse = |k: &str| -> Result<String, RunnerError> { var(k) };
        let mut token = String::new();
        std::io::stdin()
            .read_line(&mut token)
            .map_err(|e| RunnerError::Boot(format!("Token von stdin: {e}")))?;
        Ok(Self {
            socket: PathBuf::from(var(env::SOCKET)?),
            token: token.trim().to_owned(),
            session_id: parse(env::SESSION)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            runner_id: parse(env::RUNNER)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            epoch: parse(env::EPOCH)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            harness: parse(env::HARNESS)?
                .parse()
                .map_err(|e| RunnerError::Boot(format!("{e}")))?,
            workdir: std::env::current_dir().map_err(|e| RunnerError::Boot(e.to_string()))?,
            scenario: std::env::var_os(env::SCENARIO).map(PathBuf::from),
            model: std::env::var(env::MODEL).ok(),
            parent_pid: std::env::var(env::PARENT_PID)
                .ok()
                .and_then(|p| p.parse().ok()),
            dev: std::env::var(env::DEV).is_ok_and(|v| v == "1"),
            resume: std::env::var(env::RESUME).ok().filter(|r| !r.is_empty()),
        })
    }
}

/// Gate, das Freigaben über `approval.resolve`-Kommandos vom Server erhält.
#[derive(Default)]
pub struct RunnerGate {
    pending: Mutex<HashMap<String, oneshot::Sender<GateDecision>>>,
}

#[async_trait]
impl Gate for RunnerGate {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(request.call_id.clone(), tx);
        }
        // Ohne Entscheidung (Runner endet) gilt fail closed.
        rx.await.unwrap_or(GateDecision::Deny {
            reason: Some("Runner beendet".into()),
        })
    }
}

impl RunnerGate {
    pub fn resolve(&self, call_id: &str, decision: GateDecision) -> bool {
        self.pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(call_id))
            .is_some_and(|tx| tx.send(decision).is_ok())
    }
}

/// Unbestätigte Events in Runner-Reihenfolge.
#[derive(Default)]
struct Unacked {
    next_rseq: u64,
    queue: VecDeque<(RseqEvent, usize)>,
    bytes: usize,
}

impl Unacked {
    fn push(&mut self, event: Event) -> RseqEvent {
        self.next_rseq += 1;
        let item = RseqEvent {
            rseq: self.next_rseq,
            event,
        };
        let size = serde_json::to_string(&item).map(|s| s.len()).unwrap_or(256);
        self.bytes += size;
        self.queue.push_back((item.clone(), size));
        item
    }

    fn ack(&mut self, upto: u64) {
        while self.queue.front().is_some_and(|(e, _)| e.rseq <= upto) {
            if let Some((_, size)) = self.queue.pop_front() {
                self.bytes -= size;
            }
        }
    }

    fn acked(&self) -> u64 {
        self.queue
            .front()
            .map(|(e, _)| e.rseq - 1)
            .unwrap_or(self.next_rseq)
    }
}

/// Leitet `session.status` aus dem Event-Strom ab und meldet nur Änderungen.
#[derive(Default)]
struct StatusTracker {
    current: Option<SessionStatus>,
}

impl StatusTracker {
    fn set(&mut self, boot: &RunnerBoot, to: SessionStatus, out: &mut Vec<Event>) {
        if self.current == Some(to) {
            return;
        }
        self.current = Some(to);
        out.push(Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::SessionStatus(SessionStatusChanged {
                status: to,
                reason: None,
            }),
        ));
    }
}

/// Wie der Runner endete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Server hat `runner.stop` geschickt (z. B. Idle-Timeout).
    Stopped,
    /// Harness beendet (Absturz oder Ende).
    HarnessExited { code: Option<i32> },
    /// Elternprozess (Daemon) ist weg.
    ParentGone,
}

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>;

async fn connect(boot: &RunnerBoot) -> Result<Ws, RunnerError> {
    let stream = tokio::net::UnixStream::connect(&boot.socket)
        .await
        .map_err(|e| RunnerError::Tunnel(format!("{}: {e}", boot.socket.display())))?;
    let mut req = "ws://localhost/v1/tunnel"
        .into_client_request()
        .map_err(|e| RunnerError::Tunnel(e.to_string()))?;
    let headers = req.headers_mut();
    headers.insert(
        "authorization",
        format!("Bearer {}", boot.token)
            .parse()
            .map_err(|_| RunnerError::Boot("Token ungültig".into()))?,
    );
    headers.insert(
        "sec-websocket-protocol",
        SUBPROTOCOL
            .parse()
            .map_err(|_| RunnerError::Boot("Subprotokoll".into()))?,
    );
    let (ws, _) = tokio_tungstenite::client_async(req, stream)
        .await
        .map_err(|e| RunnerError::Tunnel(e.to_string()))?;
    Ok(ws)
}

async fn send(ws: &mut Ws, msg: &TunnelUp) -> bool {
    let text = serde_json::to_string(msg).unwrap_or_default();
    ws.send(Message::Text(text.into())).await.is_ok()
}

/// Lebt der Elternprozess noch? (RUN-002 AC2)
fn parent_alive(pid: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        match pid {
            Some(p) => {
                rustix::process::getppid().is_some_and(|pp| pp.as_raw_nonzero().get() as u32 == p)
            }
            None => true,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Führt die Session aus, bis der Server stoppt, der Harness endet oder der Daemon stirbt.
pub async fn run(boot: RunnerBoot, registry: Registry) -> Result<Exit, RunnerError> {
    let gate = Arc::new(RunnerGate::default());
    let mut env = HostEnv::from_process();
    env.vars.remove("BETON_RUNNER_TOKEN");
    let ctx = AdapterContext {
        gate: gate.clone(),
        launcher: Arc::new(RealLauncher),
        env,
    };
    let actor = Actor::Agent {
        id: None,
        harness: boot.harness.to_string(),
        agent_ref: None,
    };
    let mut lifecycle = Lifecycle::new(boot.runner_id);
    let mut pending_status: Vec<Event> = Vec::new();
    let status =
        |lc: &mut Lifecycle, to: RunnerState, reason: Option<String>, out: &mut Vec<Event>| {
            if let Ok(s) = lc.go(to, reason) {
                out.push(Event::new(
                    boot.session_id,
                    0,
                    Actor::System {
                        component: beton_core::event::SystemComponent::Runner,
                    },
                    EventPayload::RunnerStatus(s),
                ));
            }
        };
    status(
        &mut lifecycle,
        RunnerState::Provisioning,
        None,
        &mut pending_status,
    );
    status(
        &mut lifecycle,
        RunnerState::Starting,
        None,
        &mut pending_status,
    );

    let probe = match registry.get(&boot.harness) {
        Some(adapter) => adapter.probe(&ctx.env).await,
        None => beton_harness::ProbeReport::default(),
    };
    let capabilities = registry
        .get(&boot.harness)
        .map(|a| {
            serde_json::to_value(a.capabilities(beton_harness::Mode::Native, &probe))
                .unwrap_or(Value::Null)
        })
        .unwrap_or(Value::Null);
    let session = registry
        .start(
            &boot.harness,
            SessionSpec {
                workdir: boot.workdir.clone(),
                model: boot.model.clone(),
                scenario: boot.scenario.clone(),
                resume: boot.resume.clone(),
                ..SessionSpec::default()
            },
            ctx,
        )
        .await;
    let mut session = match session {
        Ok(s) => s,
        Err(e) => {
            // Start gescheitert (z. B. inkompatible CLI, HAR-002 AC2): melden, dann enden.
            report_start_failure(&boot, &e).await;
            return Err(e.into());
        }
    };
    // SES-001: nach `session.created` folgt `session.started`.
    pending_status.insert(
        0,
        Event::new(
            boot.session_id,
            0,
            Actor::System {
                component: beton_core::event::SystemComponent::Runner,
            },
            EventPayload::SessionStarted(beton_core::event::SessionStarted {
                runner_id: boot.runner_id,
                host_id: beton_core::id::HostId::LOCAL,
                harness: boot.harness.to_string(),
                harness_version: probe.version.unwrap_or_default(),
                capabilities,
                harness_session_ref: boot.resume.clone(),
            }),
        ),
    );
    if boot.resume.is_some() {
        pending_status.insert(
            1,
            Event::new(
                boot.session_id,
                0,
                Actor::System {
                    component: beton_core::event::SystemComponent::Runner,
                },
                EventPayload::SessionResumed(beton_core::event::SessionResumed {
                    mode: beton_core::event::ResumeMode::Native,
                }),
            ),
        );
    }
    let mut harness_rx = session
        .events()
        .ok_or(RunnerError::Boot("Event-Strom fehlt".into()))?;

    let mut unacked = Unacked::default();
    let mut tracker = StatusTracker::default();
    let mut turn_running = false;
    for e in pending_status.drain(..) {
        unacked.push(e);
    }
    let mut backoff = Backoff::default();
    let mut parent_check = tokio::time::interval(Duration::from_millis(500));

    loop {
        // (Wieder-)Verbinden.
        let mut ws = match connect(&boot).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::warn!("Tunnel nicht erreichbar: {e}");
                if !parent_alive(boot.parent_pid) {
                    return finish(session, Exit::ParentGone).await;
                }
                tokio::time::sleep(backoff.next_delay()).await;
                continue;
            }
        };
        backoff.reset();
        let hello = TunnelUp::Hello {
            kind: PeerKind::Runner,
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL.into(),
            harnesses: vec![boot.harness.to_string()],
        };
        let bind = TunnelUp::SessionBind {
            session_id: boot.session_id,
            epoch: boot.epoch,
            last_acked_rseq: unacked.acked(),
        };
        if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
            continue;
        }
        if lifecycle.state() == RunnerState::Lost {
            let _ = lifecycle.go(RunnerState::Reconnecting, None);
        }
        let mut extra = Vec::new();
        status(&mut lifecycle, RunnerState::Connected, None, &mut extra);
        status(&mut lifecycle, RunnerState::Idle, None, &mut extra);
        if !turn_running {
            tracker.set(&boot, SessionStatus::Idle, &mut extra);
        }
        for e in extra {
            unacked.push(e);
        }
        // Alles Unbestätigte erneut senden (Server dedupliziert über Event-IDs).
        let resend: Vec<RseqEvent> = unacked.queue.iter().map(|(e, _)| e.clone()).collect();
        if !resend.is_empty()
            && !send(
                &mut ws,
                &TunnelUp::EventsPush {
                    session_id: boot.session_id,
                    epoch: boot.epoch,
                    batch: resend,
                },
            )
            .await
        {
            continue;
        }

        let outcome: Option<Exit> = loop {
            let paused = unacked.bytes > MAX_UNACKED_BYTES;
            tokio::select! {
                ev = harness_rx.recv(), if !paused => {
                    let Some(first) = ev else { break Some(Exit::HarnessExited { code: None }) };
                    // Bereits anstehende Events bündeln: ein Push, eine Transaktion beim Server.
                    let mut incoming = vec![first];
                    while incoming.len() < 256 {
                        match harness_rx.try_recv() {
                            Ok(e) => incoming.push(e),
                            Err(_) => break,
                        }
                    }
                    let mut batch = Vec::with_capacity(incoming.len() + 4);
                    let mut exited = None;
                    for ev in incoming {
                        if let EventPayload::HarnessExited(x) = &ev.payload {
                            exited = Some(x.clone());
                        }
                        let busy = matches!(ev.payload, EventPayload::TurnStarted(_));
                        let next_status = match &ev.payload {
                            EventPayload::TurnStarted(_) | EventPayload::ApprovalResolved(_) => Some(SessionStatus::Running),
                            EventPayload::ApprovalRequested(_) => Some(SessionStatus::WaitingApproval),
                            EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_) | EventPayload::TurnInterrupted(_) => Some(SessionStatus::Idle),
                            _ => None,
                        };
                        let done = matches!(ev.payload, EventPayload::TurnCompleted(_) | EventPayload::TurnFailed(_) | EventPayload::TurnInterrupted(_));
                        batch.push(to_event(&boot, &actor, ev));
                        if busy { status(&mut lifecycle, RunnerState::Busy, None, &mut batch); turn_running = true; }
                        if done { status(&mut lifecycle, RunnerState::Idle, None, &mut batch); turn_running = false; }
                        if let Some(st) = next_status {
                            tracker.set(&boot, st, &mut batch);
                        }
                    }
                    if let Some(x) = &exited {
                        // RUN-003 AC3 / HAR-001 AC1: Absturz → Runner failed, Session failed.
                        let tail: Vec<&str> = x.stderr_tail.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev().collect();
                        let reason = format!("Exit-Code {:?}, Signal {:?}\n{}", x.code, x.signal, tail.join("\n"));
                        status(&mut lifecycle, RunnerState::Failed, Some(reason), &mut batch);
                        batch.push(Event::new(boot.session_id, 0, Actor::System { component: beton_core::event::SystemComponent::Runner },
                            EventPayload::SessionStatus(SessionStatusChanged { status: SessionStatus::Failed, reason: Some("Harness beendet".into()) })));
                    }
                    if !push(&mut ws, &boot, &mut unacked, batch).await {
                        break None;
                    }
                    if let Some(x) = exited {
                        // Bestätigung abwarten, damit nichts verloren geht.
                        drain_acks(&mut ws, &mut unacked).await;
                        return Ok(Exit::HarnessExited { code: x.code });
                    }
                }
                msg = ws.next() => {
                    let Some(Ok(msg)) = msg else { break None };
                    let Message::Text(text) = msg else { continue };
                    let Ok(down) = serde_json::from_str::<TunnelDown>(&text) else { continue };
                    match down {
                        TunnelDown::EventsAck { upto_rseq, .. } => unacked.ack(upto_rseq),
                        TunnelDown::Bound { acked_rseq, .. } => unacked.ack(acked_rseq),
                        TunnelDown::CmdDeliver { cmd_id, name, args } => {
                            let reply = deliver(&mut *session, &gate, &name, args, turn_running).await;
                            let msg = match reply {
                                Ok(result) => TunnelUp::CmdResult { cmd_id, result: Some(result), problem: None },
                                Err(problem) => TunnelUp::CmdResult { cmd_id, result: None, problem: Some(problem) },
                            };
                            if !send(&mut ws, &msg).await { break None; }
                        }
                        TunnelDown::RunnerStop { grace_s, .. } => {
                            let mut batch = Vec::new();
                            status(&mut lifecycle, RunnerState::Draining, Some("runner.stop".into()), &mut batch);
                            status(&mut lifecycle, RunnerState::Terminated, None, &mut batch);
                            let _ = push(&mut ws, &boot, &mut unacked, batch).await;
                            drain_acks(&mut ws, &mut unacked).await;
                            let _ = session.shutdown(Shutdown::Graceful { timeout: Duration::from_secs(grace_s) }).await;
                            return Ok(Exit::Stopped);
                        }
                        TunnelDown::Problem { problem } => {
                            if problem["code"] == "stale_epoch" {
                                let _ = session.shutdown(Shutdown::Kill).await;
                                return Err(RunnerError::StaleEpoch);
                            }
                            tracing::warn!(?problem, "Tunnel-Problem");
                        }
                        TunnelDown::Welcome { .. } => {}
                    }
                }
                _ = parent_check.tick() => {
                    if !parent_alive(boot.parent_pid) {
                        return finish(session, Exit::ParentGone).await;
                    }
                }
            }
        };
        if let Some(exit) = outcome {
            return finish(session, exit).await;
        }
        // Verbindung verloren: weiterlaufen, puffern, neu verbinden.
        if lifecycle
            .go(RunnerState::Lost, Some("Tunnel getrennt".into()))
            .is_ok()
        {
            tracing::warn!("Tunnel getrennt, verbinde neu");
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// Meldet einen gescheiterten Harness-Start über den Tunnel und setzt die Session auf `failed`.
async fn report_start_failure(boot: &RunnerBoot, error: &beton_harness::HarnessError) {
    let Ok(mut ws) = connect(boot).await else {
        return;
    };
    let hello = TunnelUp::Hello {
        kind: PeerKind::Runner,
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL.into(),
        harnesses: vec![boot.harness.to_string()],
    };
    let bind = TunnelUp::SessionBind {
        session_id: boot.session_id,
        epoch: boot.epoch,
        last_acked_rseq: 0,
    };
    if !send(&mut ws, &hello).await || !send(&mut ws, &bind).await {
        return;
    }
    let system = Actor::System {
        component: beton_core::event::SystemComponent::Runner,
    };
    let mut events = Vec::new();
    if let beton_harness::HarnessError::Incompatible { detected, expected } = error {
        events.push(Event::new(
            boot.session_id,
            0,
            system.clone(),
            EventPayload::HarnessIncompatible(beton_core::event::HarnessIncompatible {
                detected_version: detected.clone(),
                expected_range: expected.clone(),
            }),
        ));
    } else {
        events.push(Event::new(boot.session_id, 0, system.clone(), EventPayload::Error(beton_core::event::ErrorEvent {
            problem: json!({"type": format!("urn:beton:problem:{}", error.code()), "code": error.code(), "title": error.to_string()}),
        })));
    }
    events.push(Event::new(
        boot.session_id,
        0,
        system,
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Failed,
            reason: Some(error.to_string()),
        }),
    ));
    let mut unacked = Unacked::default();
    if push(&mut ws, boot, &mut unacked, events).await {
        drain_acks(&mut ws, &mut unacked).await;
    }
}

async fn finish(session: Box<dyn HarnessSession>, exit: Exit) -> Result<Exit, RunnerError> {
    let _ = session.shutdown(Shutdown::Kill).await;
    Ok(exit)
}

fn to_event(boot: &RunnerBoot, actor: &Actor, ev: NormalizedEvent) -> Event {
    let mut e = Event::new(boot.session_id, 0, actor.clone(), ev.payload);
    e.raw = ev.raw;
    e.turn_id = ev.turn_id;
    e
}

/// Sendet dauerhafte Events (mit `rseq`) bzw. transiente direkt.
async fn push(ws: &mut Ws, boot: &RunnerBoot, unacked: &mut Unacked, events: Vec<Event>) -> bool {
    let (transient, durable): (Vec<Event>, Vec<Event>) = events
        .into_iter()
        .partition(|e| e.payload().is_some_and(EventPayload::is_transient));
    if !transient.is_empty()
        && !send(
            ws,
            &TunnelUp::TransientPush {
                session_id: boot.session_id,
                events: transient,
            },
        )
        .await
    {
        return false;
    }
    if durable.is_empty() {
        return true;
    }
    let batch: Vec<RseqEvent> = durable.into_iter().map(|e| unacked.push(e)).collect();
    send(
        ws,
        &TunnelUp::EventsPush {
            session_id: boot.session_id,
            epoch: boot.epoch,
            batch,
        },
    )
    .await
}

/// Wartet kurz auf ausstehende Bestätigungen.
async fn drain_acks(ws: &mut Ws, unacked: &mut Unacked) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !unacked.queue.is_empty() {
        let Ok(Some(Ok(Message::Text(text)))) = tokio::time::timeout_at(deadline, ws.next()).await
        else {
            return;
        };
        if let Ok(TunnelDown::EventsAck { upto_rseq, .. }) = serde_json::from_str(&text) {
            unacked.ack(upto_rseq);
        }
    }
}

/// Kommandos vom Server an den Harness (`cmd.deliver`).
async fn deliver(
    session: &mut dyn HarnessSession,
    gate: &RunnerGate,
    name: &str,
    args: Value,
    turn_running: bool,
) -> Result<Value, Value> {
    let problem = |code: &str, detail: String| json!({"code": code, "detail": detail});
    match name {
        "input.submit" => {
            let text = args["text"].as_str().unwrap_or_default().to_owned();
            session
                .send(UserInput { text })
                .await
                .map(|turn| json!({"turn_id": turn}))
                .map_err(|e| problem(e.code(), e.to_string()))
        }
        // SES-005 AC3: ohne laufenden Turn ist Interrupt ein No-op.
        "turn.interrupt" if !turn_running => Ok(Value::Null),
        "turn.interrupt" => session
            .interrupt()
            .await
            .map(|()| Value::Null)
            .map_err(|e| problem(e.code(), e.to_string())),
        "session.set" => {
            let model = args["model"].as_str().map(str::to_owned);
            match model {
                Some(model) => session
                    .set_model(model, args["effort"].as_str().map(str::to_owned))
                    .await
                    .map(|_| Value::Null)
                    .map_err(|e| problem(e.code(), e.to_string())),
                None => Ok(Value::Null),
            }
        }
        "approval.resolve" => {
            let call_id = args["call_id"].as_str().unwrap_or_default();
            let decision = if args["decision"] == "allow" {
                GateDecision::Allow {
                    updated_args: args.get("updated_args").cloned(),
                }
            } else {
                GateDecision::Deny {
                    reason: args["reason"].as_str().map(str::to_owned),
                }
            };
            if gate.resolve(call_id, decision) {
                Ok(Value::Null)
            } else {
                Err(problem(
                    "not_found",
                    format!("keine offene Freigabe für {call_id}"),
                ))
            }
        }
        _ => Err(problem(
            "unknown_command",
            format!("`{name}` kennt der Runner nicht"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice() -> Event {
        Event::new(
            SessionId::new(),
            0,
            Actor::default(),
            EventPayload::default(),
        )
    }

    #[test]
    fn unacked_buffer_tracks_rseq_and_bytes() {
        let mut u = Unacked::default();
        assert_eq!(u.acked(), 0);
        for _ in 0..5 {
            u.push(notice());
        }
        assert_eq!(u.acked(), 0);
        u.ack(3);
        assert_eq!(u.acked(), 3);
        assert_eq!(u.queue.len(), 2);
        u.ack(5);
        assert_eq!(u.bytes, 0);
        assert_eq!(u.acked(), 5);
    }

    #[tokio::test]
    async fn gate_waits_for_resolution_and_fails_closed() {
        let gate = Arc::new(RunnerGate::default());
        let g = gate.clone();
        let waiting = tokio::spawn(async move {
            g.decide(GateRequest {
                turn_id: None,
                call_id: "c1".into(),
                tool: "Bash".into(),
                kind: "shell".into(),
                args: Value::Null,
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(gate.resolve("c1", GateDecision::Allow { updated_args: None }));
        assert_eq!(
            waiting.await.unwrap(),
            GateDecision::Allow { updated_args: None }
        );
        assert!(!gate.resolve("c1", GateDecision::Deny { reason: None }));

        let g = gate.clone();
        let waiting = tokio::spawn(async move {
            g.decide(GateRequest {
                turn_id: None,
                call_id: "c2".into(),
                tool: "Bash".into(),
                kind: "shell".into(),
                args: Value::Null,
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        gate.pending.lock().unwrap().clear();
        assert!(matches!(waiting.await.unwrap(), GateDecision::Deny { .. }));
    }
}
