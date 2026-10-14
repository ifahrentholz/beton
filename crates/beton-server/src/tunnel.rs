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

impl RunnerConn {
    /// Ohne Turn, ohne offenes Kommando und seit `timeout` ohne Aktivität (RUN-003 AC2). Ein
    /// offenes Kommando zählt als Aktivität: `input.submit` antwortet erst nach der Phase
    /// `model_request` (POL-003), die je nach Last länger dauern kann.
    fn idle(&self, timeout: Duration) -> bool {
        !self.busy && self.pending.is_empty() && self.last_activity.elapsed() >= timeout
    }
}

/// Wartet, bis der Daemon herunterfährt; ohne Signal nie.
async fn shutting_down(rx: &mut Option<tokio::sync::watch::Receiver<bool>>) {
    match rx {
        Some(rx) => {
            while !*rx.borrow() {
                if rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        }
        None => std::future::pending::<()>().await,
    }
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
    /// Offene Freigaben je Session mit Ablauf (Unix-Sekunden) aus dem Event-Strom des Runners.
    approvals: Mutex<HashMap<SessionId, HashMap<beton_core::id::ApprovalId, i64>>>,
    /// Abgekoppelte Runner eines vorigen Daemons, die sich bis zur Frist neu verbinden dürfen
    /// (POL-010 AC4).
    expected: Mutex<HashMap<SessionId, Instant>>,
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

    /// Verfolgt offene Freigaben aus den Events des Runners (POL-010 AC4, Leerlauf).
    pub fn note_events<'a>(
        &self,
        session: SessionId,
        events: impl IntoIterator<Item = &'a beton_core::event::Event>,
    ) {
        let mut all = lock(&self.approvals);
        let open = all.entry(session).or_default();
        for e in events {
            match e.payload() {
                Some(EventPayload::ApprovalRequested(r)) => {
                    open.insert(
                        r.approval_id,
                        r.expires_at.as_offset_date_time().unix_timestamp(),
                    );
                }
                Some(EventPayload::ApprovalResolved(r)) => {
                    open.remove(&r.approval_id);
                }
                _ => {}
            }
        }
        if open.is_empty() {
            all.remove(&session);
        }
    }

    /// Wartet der Runner der Session auf eine Freigabe?
    pub fn has_open_approvals(&self, session: SessionId) -> bool {
        lock(&self.approvals)
            .get(&session)
            .is_some_and(|m| !m.is_empty())
    }

    /// Koppelt die Runner mit offener Freigabe ab (POL-010 AC4): Session, Hash des Tokens und
    /// Ablauf der letzten offenen Freigabe. Runner ohne offene Freigabe fehlen.
    pub fn detach(&self, sessions: &[SessionId]) -> Vec<crate::detached::DetachedRunner> {
        let approvals = lock(&self.approvals);
        let tokens = lock(&self.tokens);
        let mut out = Vec::new();
        for session in sessions {
            let Some(until) = approvals
                .get(session)
                .and_then(|m| m.values().max().copied())
            else {
                continue;
            };
            for (hash, s) in tokens.iter() {
                if s == session {
                    out.push(crate::detached::DetachedRunner {
                        session_id: *session,
                        token_sha256: hex::encode(hash),
                        until,
                    });
                }
            }
        }
        out
    }

    /// Nimmt einen abgekoppelten Runner eines vorigen Daemons an (POL-010 AC4).
    pub fn adopt(&self, d: &crate::detached::DetachedRunner) {
        let Ok(bytes) = hex::decode(&d.token_sha256) else {
            return;
        };
        let Ok(hash) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            return;
        };
        let left = d.until - time::OffsetDateTime::now_utc().unix_timestamp();
        if left <= 0 {
            return;
        }
        lock(&self.tokens).insert(hash, d.session_id);
        lock(&self.expected).insert(
            d.session_id,
            Instant::now() + Duration::from_secs(left.unsigned_abs()),
        );
    }

    /// Wird ein abgekoppelter Runner für die Session noch erwartet?
    pub fn expected(&self, session: SessionId) -> bool {
        let mut expected = lock(&self.expected);
        match expected.get(&session) {
            Some(until) if *until > Instant::now() && !self.connected(session) => true,
            Some(_) => {
                expected.remove(&session);
                false
            }
            None => false,
        }
    }

    /// Ein erwarteter Runner kommt nicht zurück (z. B. Wartezeit abgelaufen).
    pub fn forget_expected(&self, session: SessionId) {
        lock(&self.expected).remove(&session);
    }

    /// Verbundene Sessions.
    pub fn connected_sessions(&self) -> Vec<SessionId> {
        lock(&self.conns).keys().copied().collect()
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
        "sandbox_required" => ProblemCode::SandboxRequired,
        "unexpected_input" | "validation_failed" => ProblemCode::ValidationFailed,
        "not_found" => ProblemCode::NotFound,
        "no_active_turn" => ProblemCode::NoActiveTurn,
        "policy_denied" => ProblemCode::PolicyDenied,
        "approval_required" => ProblemCode::ApprovalRequired,
        "unknown_command" => ProblemCode::UnknownCommand,
        "session_closed" => ProblemCode::Unavailable,
        _ => ProblemCode::Conflict,
    }
}

/// Ausführung von `system.call` (AGT-007), z. B. `session.spawn`.
#[async_trait::async_trait]
pub trait SystemCalls: Send + Sync {
    /// Gilt für die gebundene Session des Runners. `Err` ist ein Problem `{code, detail}`.
    async fn call(&self, session: SessionId, tool: &str, args: Value) -> Result<Value, Value>;
}

/// Reagiert auf gespeicherte Runner-Events, ohne den Tunnel aufzuhalten (z. B. Titel nach dem
/// ersten Turn, SES-010).
pub trait EventObserver: Send + Sync {
    fn observe(&self, session: SessionId, written: &[Event]);
}

/// Zustand des Tunnel-Endpunkts.
#[derive(Clone)]
pub struct TunnelState {
    pub events: EventService,
    pub local: beton_store::LocalIdentity,
    pub runners: Arc<RunnerRegistry>,
    pub config: TunnelConfig,
    /// Ohne Handler werden System-Tools abgelehnt (fail closed).
    pub system: Option<Arc<dyn SystemCalls>>,
    /// Queues der Sessions: Turn-Ende arbeitet die nächste Eingabe ab (SES-004).
    pub queues: Arc<crate::queue::Queues>,
    /// Weitere Beobachter gespeicherter Events (Session-Titel, SES-010).
    pub observer: Option<Arc<dyn EventObserver>>,
    /// Herunterfahren: Verbindungen enden mit 4503, Runner verbinden sich neu (POL-010 AC4).
    pub shutdown: Option<tokio::sync::watch::Receiver<bool>>,
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
    lock(&state.runners.expected).remove(&session_id);
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
    let mut shutdown = state.shutdown.clone();
    loop {
        tokio::select! {
            () = shutting_down(&mut shutdown) => {
                let _ = sink
                    .send(close_msg(close::SHUTTING_DOWN, "Daemon fährt herunter"))
                    .await;
                break;
            }
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
        TunnelUp::SystemCall {
            call_id,
            tool,
            args,
        } => {
            // Lange Aufrufe (`session.spawn`) blockieren die Verbindung nicht.
            let tx = lock(&runners.conns).get(&session).map(|c| c.tx.clone());
            let handler = state.system.clone();
            tokio::spawn(async move {
                let reply = match handler {
                    Some(h) => h.call(session, &tool, args).await,
                    None => Err(
                        serde_json::json!({"code": "unavailable", "detail": "System-Tools sind auf diesem Server nicht verfügbar"}),
                    ),
                };
                if let Some(tx) = tx {
                    let (result, problem) = match reply {
                        Ok(v) => (Some(v), None),
                        Err(p) => (None, Some(p)),
                    };
                    let _ = tx.send(TunnelDown::SystemResult {
                        call_id,
                        result,
                        problem,
                    });
                }
            });
            None
        }
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
            runners.note_events(session, fresh.iter().map(|e| &e.event));
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
                    if let Some(o) = &state.observer {
                        o.observe(session, &written);
                    }
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
                c.idle(state.config.idle_timeout)
                    && hub.subscribers(**s) == 0
                    && !state.runners.has_open_approvals(**s)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> RunnerConn {
        RunnerConn {
            id: 1,
            tx: mpsc::unbounded_channel().0,
            last_activity: Instant::now() - Duration::from_secs(10),
            busy: false,
            pending: HashMap::new(),
        }
    }

    #[test]
    fn run_003_ac2_runner_with_an_open_command_is_not_idle() {
        let timeout = Duration::from_secs(1);
        let mut c = conn();
        assert!(c.idle(timeout));
        c.pending.insert("cmd_1".into(), oneshot::channel().0);
        assert!(!c.idle(timeout), "wartet noch auf cmd.result");
        c.pending.clear();
        c.busy = true;
        assert!(!c.idle(timeout));
        c.busy = false;
        c.last_activity = Instant::now();
        assert!(!c.idle(timeout));
    }

    fn approval(
        session: SessionId,
        id: beton_core::id::ApprovalId,
        secs: i64,
    ) -> beton_core::event::Event {
        beton_core::event::Event::new(
            session,
            0,
            beton_core::event::Actor::default(),
            EventPayload::ApprovalRequested(beton_core::event::ApprovalRequested {
                approval_id: id,
                expires_at: (time::OffsetDateTime::now_utc() + time::Duration::seconds(secs))
                    .into(),
                ..beton_core::event::ApprovalRequested::default()
            }),
        )
    }

    #[test]
    fn pol_010_ac4_runner_with_open_approval_is_detached_and_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runners-detached.json");
        let (s1, s2) = (SessionId::new(), SessionId::new());
        let old = RunnerRegistry::default();
        let t1 = old.mint_token(s1).unwrap();
        let _t2 = old.mint_token(s2).unwrap();
        let a = beton_core::id::ApprovalId::new();
        old.note_events(s1, [&approval(s1, a, 600)]);
        assert!(old.has_open_approvals(s1));
        assert!(!old.has_open_approvals(s2));
        // Nur Runner mit offener Freigabe überdauern den Daemon.
        let detached = old.detach(&[s1, s2]);
        assert_eq!(detached.len(), 1);
        assert_eq!(detached[0].session_id, s1);
        crate::detached::save(&file, &detached).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Die Datei enthält nur den Hash des Tokens.
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains(&t1));

        let new = RunnerRegistry::default();
        for d in crate::detached::take(&file) {
            new.adopt(&d);
        }
        assert!(!file.exists(), "einmal gelesen, dann gelöscht");
        assert_eq!(new.session_for(&t1), Some(s1));
        assert!(new.expected(s1));
        assert!(!new.expected(s2));
        // Aufgelöst: nicht mehr offen.
        let resolved = beton_core::event::Event::new(
            s1,
            0,
            beton_core::event::Actor::default(),
            EventPayload::ApprovalResolved(beton_core::event::ApprovalResolved {
                approval_id: a,
                ..beton_core::event::ApprovalResolved::default()
            }),
        );
        old.note_events(s1, [&resolved]);
        assert!(!old.has_open_approvals(s1));
    }

    #[test]
    fn pol_010_ac4_expired_detached_runners_are_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runners-detached.json");
        let s = SessionId::new();
        let old = RunnerRegistry::default();
        old.mint_token(s).unwrap();
        old.note_events(s, [&approval(s, beton_core::id::ApprovalId::new(), -5)]);
        let detached = old.detach(&[s]);
        crate::detached::save(&file, &detached).unwrap();
        assert!(crate::detached::take(&file).is_empty());
        // Kaputte Datei: nichts übernehmen, fail closed.
        std::fs::write(&file, "kein json").unwrap();
        assert!(crate::detached::take(&file).is_empty());
    }
}
