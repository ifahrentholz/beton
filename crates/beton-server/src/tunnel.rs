//! Tunnel-Endpunkt für Runner (PROTO-015, RUN-003).
//!
//! Lokal nur über den Unix-Socket `~/.beton/run/tunnel.sock` (0600), nicht über TCP. Runner
//! authentisieren sich mit einem sessiongebundenen Token (`bt_run_…`), das der Daemon beim
//! Start mintet und per stdin übergibt (AUTH-011-Prinzip). Der Server vergibt `seq`,
//! dedupliziert erneut gesendete Events über `rseq` und bestätigt per `events.ack`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use beton_core::event::{
    Actor, Event, EventPayload, SessionStatus, SessionStatusChanged, SystemComponent,
};
use beton_core::id::SessionId;
use beton_proto::tunnel::{PROTOCOL, SUBPROTOCOL, TunnelDown, TunnelUp};
use beton_proto::ws::close;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot};

use crate::hub::EventService;
use crate::problem::{Problem, ProblemCode};

/// Pfad des Tunnel-Endpunkts (nur auf dem Tunnel-Socket, nicht in der öffentlichen API).
pub const TUNNEL_PATH: &str = "/v1/tunnel";

#[derive(Debug, Clone, Copy)]
pub struct TunnelConfig {
    /// Runner ohne Turn-Aktivität und ohne Client werden danach beendet (RUN-003 AC2).
    pub idle_timeout: Duration,
    pub idle_check: Duration,
    pub cmd_timeout: Duration,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(60 * 60),
            idle_check: Duration::from_secs(30),
            cmd_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(20),
            pong_timeout: Duration::from_secs(60),
        }
    }
}

type CmdReply = Result<Value, Value>;

struct RunnerConn {
    id: u64,
    tx: mpsc::UnboundedSender<TunnelDown>,
    last_activity: Instant,
    busy: bool,
    pending: HashMap<String, oneshot::Sender<CmdReply>>,
}

/// Verbundene Runner und ihre Tokens.
#[derive(Default)]
pub struct RunnerRegistry {
    tokens: Mutex<HashMap<[u8; 32], SessionId>>,
    conns: Mutex<HashMap<SessionId, RunnerConn>>,
    /// Höchste gespeicherte `rseq` je Session (Dedup nach Reconnect).
    persisted: Mutex<HashMap<SessionId, u64>>,
    /// Sessions, deren Runner geordnet gestoppt wird, mit Grund.
    stopping: Mutex<HashMap<SessionId, String>>,
    /// Test-Haken: Sessions, deren Verbindungen bis zum Zeitpunkt abgelehnt werden.
    refuse_until: Mutex<HashMap<SessionId, Instant>>,
    next_conn: AtomicU64,
    next_cmd: AtomicU64,
}

impl std::fmt::Debug for RunnerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunnerRegistry")
    }
}

fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl RunnerRegistry {
    /// Neues Runner-Token für genau eine Session.
    /// Token für einen neuen Runner-Prozess. Der zählt `rseq` wieder ab 1; der gespeicherte
    /// Stand des Vorgängers darf seine Events nicht als Duplikate verwerfen (PROTO-015).
    pub fn mint_token(&self, session: SessionId) -> Result<String, crate::local_auth::AuthError> {
        let token = format!("bt_run_{}", crate::local_auth::random_hex(32)?);
        lock(&self.tokens).insert(hash(&token), session);
        lock(&self.persisted).remove(&session);
        Ok(token)
    }

    pub fn revoke_tokens(&self, session: SessionId) {
        lock(&self.tokens).retain(|_, s| *s != session);
    }

    fn session_for(&self, token: &str) -> Option<SessionId> {
        let h = hash(token);
        lock(&self.tokens)
            .iter()
            .find(|(k, _)| bool::from(k.ct_eq(&h)))
            .map(|(_, s)| *s)
    }

    pub fn connected(&self, session: SessionId) -> bool {
        lock(&self.conns).contains_key(&session)
    }

    /// Kommando an den Runner zustellen (`cmd.deliver`) und auf `cmd.result` warten.
    pub async fn deliver(
        &self,
        session: SessionId,
        name: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value, Problem> {
        let (tx, rx) = oneshot::channel();
        let cmd_id = format!("cmd_{}", self.next_cmd.fetch_add(1, Ordering::Relaxed) + 1);
        {
            let mut conns = lock(&self.conns);
            let conn = conns.get_mut(&session).ok_or_else(|| {
                Problem::new(ProblemCode::Unavailable).detail("Für diese Session läuft kein Runner")
            })?;
            conn.pending.insert(cmd_id.clone(), tx);
            conn.last_activity = Instant::now();
            let _ = conn.tx.send(TunnelDown::CmdDeliver {
                cmd_id,
                name: name.to_owned(),
                args,
            });
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(problem))) => Err(Problem::new(runner_code(&problem)).detail(
                problem["detail"]
                    .as_str()
                    .unwrap_or("Runner meldet einen Fehler")
                    .to_owned(),
            )),
            _ => Err(Problem::new(ProblemCode::Unavailable).detail("Runner antwortet nicht")),
        }
    }

    /// Runner beenden lassen; nach dem Trennen gilt die Session als `stopped`.
    pub fn stop(&self, session: SessionId, grace_s: u64) -> bool {
        self.stop_with_reason(session, grace_s, "Runner im Leerlauf beendet")
    }

    pub fn stop_with_reason(&self, session: SessionId, grace_s: u64, reason: &str) -> bool {
        let conns = lock(&self.conns);
        let Some(conn) = conns.get(&session) else {
            return false;
        };
        lock(&self.stopping).insert(session, reason.to_owned());
        conn.tx
            .send(TunnelDown::RunnerStop {
                session_id: session,
                grace_s,
            })
            .is_ok()
    }

    /// Trennt die Verbindung und lehnt Neuverbindungen für `duration` ab (Tests, Diagnose).
    pub fn disconnect_for(&self, session: SessionId, duration: Duration) {
        lock(&self.refuse_until).insert(session, Instant::now() + duration);
        lock(&self.conns).remove(&session);
    }
}

/// Fehlercode des Runners auf den passenden `ProblemCode` abbilden.
fn runner_code(problem: &Value) -> ProblemCode {
    match problem["code"].as_str().unwrap_or_default() {
        "capability_unsupported" => ProblemCode::CapabilityUnsupported,
        "unexpected_input" | "validation_failed" => ProblemCode::ValidationFailed,
        "not_found" => ProblemCode::NotFound,
        "no_active_turn" => ProblemCode::NoActiveTurn,
        "unknown_command" => ProblemCode::UnknownCommand,
        "session_closed" => ProblemCode::Unavailable,
        _ => ProblemCode::Conflict,
    }
}

/// Zustand des Tunnel-Endpunkts.
#[derive(Clone)]
pub struct TunnelState {
    pub events: EventService,
    pub local: beton_store::LocalIdentity,
    pub runners: Arc<RunnerRegistry>,
    pub config: TunnelConfig,
    /// Queues der Sessions: Turn-Ende arbeitet die nächste Eingabe ab (SES-004).
    pub queues: Arc<crate::queue::Queues>,
}

impl TunnelState {
    fn queue(&self) -> crate::queue::QueueCtx {
        crate::queue::QueueCtx {
            events: self.events.clone(),
            org: self.local.org,
            runners: self.runners.clone(),
            cmd_timeout: self.config.cmd_timeout,
            queues: self.queues.clone(),
        }
    }
}

/// Upgrade auf den Tunnel; Token vor dem Upgrade prüfen.
pub async fn tunnel_upgrade(
    State(state): State<TunnelState>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(session) =
        crate::security::bearer(&headers).and_then(|t| state.runners.session_for(t))
    else {
        return Problem::new(ProblemCode::Unauthorized).into_response();
    };
    let Ok(ws) = ws else {
        return Problem::new(ProblemCode::BadRequest)
            .detail("Tunnel-Upgrade erwartet")
            .into_response();
    };
    ws.protocols([SUBPROTOCOL])
        .on_upgrade(move |socket| connection(socket, state, session))
}

fn problem_msg(p: Problem) -> Message {
    let msg = TunnelDown::Problem {
        problem: serde_json::to_value(p).unwrap_or(Value::Null),
    };
    Message::Text(serde_json::to_string(&msg).unwrap_or_default().into())
}

fn close_msg(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_owned().into(),
    }))
}

async fn connection(socket: WebSocket, state: TunnelState, token_session: SessionId) {
    let (mut sink, mut stream) = socket.split();
    if lock(&state.runners.refuse_until)
        .get(&token_session)
        .is_some_and(|t| Instant::now() < *t)
    {
        let _ = sink
            .send(close_msg(
                close::SHUTTING_DOWN,
                "vorübergehend nicht verfügbar",
            ))
            .await;
        return;
    }
    // hello
    let hello = tokio::time::timeout(Duration::from_secs(10), stream.next()).await;
    let ok = matches!(&hello, Ok(Some(Ok(Message::Text(t)))) if matches!(TunnelUp::from_json(t), Ok(TunnelUp::Hello { .. })));
    if !ok {
        let _ = sink
            .send(close_msg(close::PROTOCOL, "hello erwartet"))
            .await;
        return;
    }
    let welcome = TunnelDown::Welcome {
        server_version: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL.into(),
    };
    if sink
        .send(Message::Text(
            serde_json::to_string(&welcome).unwrap_or_default().into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    // session.bind
    let bind = tokio::time::timeout(Duration::from_secs(10), stream.next()).await;
    let Ok(Some(Ok(Message::Text(text)))) = bind else {
        return;
    };
    let Ok(TunnelUp::SessionBind {
        session_id, epoch, ..
    }) = TunnelUp::from_json(&text)
    else {
        let _ = sink
            .send(close_msg(close::PROTOCOL, "session.bind erwartet"))
            .await;
        return;
    };
    if session_id != token_session {
        let _ = sink
            .send(close_msg(
                close::FORBIDDEN,
                "Token gilt für eine andere Session",
            ))
            .await;
        return;
    }
    let org = state.local.org;
    let record = match state.events.store.session(org, session_id).await {
        Ok(r) => r,
        Err(_) => {
            let _ = sink
                .send(close_msg(close::UNKNOWN_SESSION, "Session unbekannt"))
                .await;
            return;
        }
    };
    if record.epoch != epoch {
        let _ = sink
            .send(problem_msg(Problem::new(ProblemCode::StaleEpoch)))
            .await;
        let _ = sink.send(close_msg(close::FORBIDDEN, "stale_epoch")).await;
        return;
    }
    let acked = lock(&state.runners.persisted)
        .get(&session_id)
        .copied()
        .unwrap_or(0);
    let bound = TunnelDown::Bound {
        session_id,
        epoch,
        head_seq: record.head_seq,
        acked_rseq: acked,
    };
    if sink
        .send(Message::Text(
            serde_json::to_string(&bound).unwrap_or_default().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<TunnelDown>();
    let conn_id = state.runners.next_conn.fetch_add(1, Ordering::Relaxed);
    lock(&state.runners.conns).insert(
        session_id,
        RunnerConn {
            id: conn_id,
            tx,
            last_activity: Instant::now(),
            busy: false,
            pending: HashMap::new(),
        },
    );

    let mut ping = tokio::time::interval(state.config.ping_interval);
    ping.reset();
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            down = rx.recv() => {
                let Some(down) = down else { break };
                if sink.send(Message::Text(serde_json::to_string(&down).unwrap_or_default().into())).await.is_err() {
                    break;
                }
            }
            msg = stream.next() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    Message::Text(text) => {
                        let Ok(up) = TunnelUp::from_json(&text) else { continue };
                        if let Some(reply) = handle(&state, session_id, up).await {
                            let stale = matches!(&reply, TunnelDown::Problem { problem } if problem["code"] == "stale_epoch");
                            if sink.send(Message::Text(serde_json::to_string(&reply).unwrap_or_default().into())).await.is_err() || stale {
                                break;
                            }
                        }
                    }
                    Message::Pong(_) => last_pong = Instant::now(),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if last_pong.elapsed() > state.config.pong_timeout {
                    let _ = sink.send(close_msg(close::HEARTBEAT_TIMEOUT, "kein Pong")).await;
                    break;
                }
                let _ = sink.send(Message::Ping(Vec::new().into())).await;
            }
        }
    }
    // Abmelden (nur wenn die Verbindung noch die aktuelle ist).
    {
        let mut conns = lock(&state.runners.conns);
        if conns.get(&session_id).is_some_and(|c| c.id == conn_id) {
            conns.remove(&session_id);
        }
    }
    let stopped = lock(&state.runners.stopping).remove(&session_id);
    if let Some(reason) = stopped {
        // RUN-003 AC2, SES-001 AC2: nach geordnetem Stopp ist die Session `stopped`.
        append_status(&state, session_id, SessionStatus::Stopped, &reason).await;
        state.runners.revoke_tokens(session_id);
    }
}

async fn append_status(
    state: &TunnelState,
    session: SessionId,
    status: SessionStatus,
    reason: &str,
) {
    let event = Event::new(
        session,
        0,
        Actor::System {
            component: SystemComponent::Runner,
        },
        EventPayload::SessionStatus(SessionStatusChanged {
            status,
            reason: Some(reason.into()),
        }),
    );
    if let Ok(written) = append(state, session, vec![event]).await {
        state.queue().observe(session, &written);
    }
}

/// Anhängen mit Wiederholung, falls der Kopf sich parallel bewegt hat.
async fn append(
    state: &TunnelState,
    session: SessionId,
    events: Vec<Event>,
) -> Result<Vec<Event>, Problem> {
    let org = state.local.org;
    for _ in 0..5 {
        let record = state.events.store.session(org, session).await?;
        match state
            .events
            .append(org, session, record.head_seq, record.epoch, events.clone())
            .await
        {
            Err(beton_store::Error::SeqConflict { .. }) => continue,
            other => return other.map_err(Problem::from),
        }
    }
    Err(Problem::new(ProblemCode::SeqConflict))
}

async fn handle(state: &TunnelState, session: SessionId, up: TunnelUp) -> Option<TunnelDown> {
    let runners = &state.runners;
    match up {
        TunnelUp::EventsPush {
            session_id,
            epoch,
            batch,
        } => {
            if session_id != session {
                return Some(TunnelDown::Problem {
                    problem: serde_json::to_value(Problem::new(ProblemCode::Forbidden)).ok()?,
                });
            }
            let current = state
                .events
                .store
                .session(state.local.org, session)
                .await
                .ok()?
                .epoch;
            if epoch != current {
                // PROTO-015 AC2: veraltete Epoch → Runner hört auf zu schreiben.
                return Some(TunnelDown::Problem {
                    problem: serde_json::to_value(Problem::new(ProblemCode::StaleEpoch)).ok()?,
                });
            }
            let persisted = lock(&runners.persisted).get(&session).copied().unwrap_or(0);
            let upto = batch
                .iter()
                .map(|e| e.rseq)
                .max()
                .unwrap_or(persisted)
                .max(persisted);
            let fresh: Vec<_> = batch.into_iter().filter(|e| e.rseq > persisted).collect();
            let mut busy = None;
            for e in &fresh {
                match e.event.payload() {
                    Some(EventPayload::TurnStarted(_)) => busy = Some(true),
                    Some(
                        EventPayload::TurnCompleted(_)
                        | EventPayload::TurnFailed(_)
                        | EventPayload::TurnInterrupted(_),
                    ) => busy = Some(false),
                    _ => {}
                }
            }
            let seq_range =
                if fresh.is_empty() {
                    None
                } else {
                    let written =
                        match append(state, session, fresh.into_iter().map(|e| e.event).collect())
                            .await
                        {
                            Ok(w) => w,
                            Err(problem) => {
                                // Nicht still verwerfen: ohne Ack behält der Runner die Events,
                                // und das Problem wird sichtbar.
                                tracing::warn!(%session, "Events nicht gespeichert");
                                return Some(TunnelDown::Problem {
                                    problem: serde_json::to_value(problem).ok()?,
                                });
                            }
                        };
                    state.queue().observe(session, &written);
                    Some((written.first()?.seq, written.last()?.seq))
                };
            lock(&runners.persisted).insert(session, upto);
            if let Some(conn) = lock(&runners.conns).get_mut(&session) {
                conn.last_activity = Instant::now();
                if let Some(b) = busy {
                    conn.busy = b;
                }
            }
            Some(TunnelDown::EventsAck {
                session_id: session,
                upto_rseq: upto,
                seq_range,
            })
        }
        TunnelUp::TransientPush { events, .. } => {
            for mut e in events {
                e.transient = true;
                let _ = state.events.hub.publish_transient(e);
            }
            None
        }
        TunnelUp::CmdResult {
            cmd_id,
            result,
            problem,
        } => {
            let tx = lock(&runners.conns)
                .get_mut(&session)
                .and_then(|c| c.pending.remove(&cmd_id));
            if let Some(tx) = tx {
                let _ = tx.send(match problem {
                    Some(p) => Err(p),
                    None => Ok(result.unwrap_or(Value::Null)),
                });
            }
            None
        }
        TunnelUp::Hello { .. } | TunnelUp::SessionBind { .. } => None,
    }
}

/// Beendet Runner ohne Turn-Aktivität und ohne angeschlossenen Client (RUN-003 AC2).
pub async fn idle_reaper(
    state: TunnelState,
    hub: Arc<crate::hub::Hub>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(state.config.idle_check);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = shutdown.changed() => return,
        }
        let idle: Vec<SessionId> = lock(&state.runners.conns)
            .iter()
            .filter(|(s, c)| {
                !c.busy
                    && c.last_activity.elapsed() >= state.config.idle_timeout
                    && hub.subscribers(**s) == 0
            })
            .map(|(s, _)| *s)
            .collect();
        for session in idle {
            if !lock(&state.runners.stopping).contains_key(&session) {
                tracing::info!(%session, "Runner im Leerlauf, wird beendet");
                state.runners.stop(session, 10);
            }
        }
    }
}
