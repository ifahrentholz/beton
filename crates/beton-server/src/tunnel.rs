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
    /// Vorab-Verbindung vor dem Start des Harness (#218): kein verbundener Runner.
    prestart: bool,
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

/// Offene Freigaben einer Session: Ablauf (Unix-Sekunden) und Art.
type OpenApprovals = HashMap<beton_core::id::ApprovalId, (i64, beton_core::event::ApprovalKind)>;

/// Höchstdauer, die ein abgekoppelter Runner auf den nächsten Daemon wartet (POL-010 AC4).
pub const DETACH_MAX: Duration = Duration::from_secs(3600);

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
    approvals: Mutex<HashMap<SessionId, OpenApprovals>>,
    /// Abgekoppelte Runner eines vorigen Daemons, die sich bis zur Frist neu verbinden dürfen
    /// (POL-010 AC4).
    expected: Mutex<HashMap<SessionId, (Instant, [u8; 32])>>,
    /// Beim Herunterfahren abgekoppelte Sessions: Close-Frame mit Grund `detached`.
    detached: Mutex<std::collections::HashSet<SessionId>>,
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
    ///
    /// Je Session gilt nur das jüngste Token (Generation): Ältere, auch die nie verbundener
    /// Starts oder eines abgekoppelten Runners, verfallen hier. Bestehende Verbindungen bleiben
    /// davon unberührt; neu anmelden kann sich nur der neue Runner.
    pub fn mint_token(&self, session: SessionId) -> Result<String, crate::local_auth::AuthError> {
        let token = format!("bt_run_{}", crate::local_auth::random_hex(32)?);
        {
            let mut tokens = lock(&self.tokens);
            tokens.retain(|_, s| *s != session);
            tokens.insert(hash(&token), session);
        }
        lock(&self.expected).remove(&session);
        lock(&self.persisted).remove(&session);
        Ok(token)
    }

    pub fn revoke_tokens(&self, session: SessionId) {
        lock(&self.tokens).retain(|_, s| *s != session);
    }

    /// Gilt das Token noch (Runner darf sich anmelden)?
    pub fn token_valid(&self, token: &str) -> bool {
        self.session_for(token).is_some()
    }

    /// Sperrt genau ein Token, z. B. das eines geordnet gestoppten Runners. Das Token eines
    /// inzwischen gestarteten Nachfolgers derselben Session bleibt gültig (#198).
    pub fn revoke_token(&self, token: &str) {
        lock(&self.tokens).remove(&hash(token));
    }

    fn session_for(&self, token: &str) -> Option<SessionId> {
        let h = hash(token);
        lock(&self.tokens)
            .iter()
            .find(|(k, _)| bool::from(k.ct_eq(&h)))
            .map(|(_, s)| *s)
    }

    pub fn connected(&self, session: SessionId) -> bool {
        lock(&self.conns).get(&session).is_some_and(|c| !c.prestart)
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
                        (r.expires_at.as_offset_date_time().unix_timestamp(), r.kind),
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

    /// Offene Freigaben eines (übernommenen) Runners aus der Projektion: ID, Ablauf, Art.
    pub fn restore_open(
        &self,
        session: SessionId,
        items: impl IntoIterator<
            Item = (
                beton_core::id::ApprovalId,
                i64,
                beton_core::event::ApprovalKind,
            ),
        >,
    ) {
        let mut all = lock(&self.approvals);
        let open = all.entry(session).or_default();
        for (id, until, kind) in items {
            open.insert(id, (until, kind));
        }
        if open.is_empty() {
            all.remove(&session);
        }
    }

    /// Koppelt Runner mit offener **Policy-Freigabe** ab (POL-010 AC4): Session, Hash des
    /// Tokens und Frist = Ablauf der letzten Policy-Freigabe, höchstens [`DETACH_MAX`]. Fragen
    /// (`ask_user`) halten keinen Runner über den Daemon hinaus.
    pub fn detach(&self, sessions: &[SessionId]) -> Vec<crate::detached::DetachedRunner> {
        let approvals = lock(&self.approvals);
        let tokens = lock(&self.tokens);
        let mut detached = lock(&self.detached);
        let cap = time::OffsetDateTime::now_utc().unix_timestamp()
            + i64::try_from(DETACH_MAX.as_secs()).unwrap_or(3600);
        let mut out = Vec::new();
        for session in sessions {
            let Some(until) = approvals.get(session).and_then(|m| {
                m.values()
                    .filter(|(_, k)| *k == beton_core::event::ApprovalKind::Policy)
                    .map(|(u, _)| *u)
                    .max()
            }) else {
                continue;
            };
            for (hash, s) in tokens.iter() {
                if s == session {
                    detached.insert(*session);
                    out.push(crate::detached::DetachedRunner {
                        session_id: *session,
                        token_sha256: hex::encode(hash),
                        until: until.min(cap),
                    });
                }
            }
        }
        out
    }

    /// Wurde die Session beim Herunterfahren abgekoppelt?
    pub fn is_detached(&self, session: SessionId) -> bool {
        lock(&self.detached).contains(&session)
    }

    /// Nimmt einen abgekoppelten Runner eines vorigen Daemons an (POL-010 AC4): sein Token gilt
    /// nur bis zur Frist bzw. bis er sich verbunden hat.
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
        let left = Duration::from_secs(left.unsigned_abs()).min(DETACH_MAX);
        lock(&self.tokens).insert(hash, d.session_id);
        lock(&self.expected).insert(d.session_id, (Instant::now() + left, hash));
    }

    /// Wird ein abgekoppelter Runner für die Session noch erwartet? Nach der Frist verfällt
    /// sein Token.
    pub fn expected(&self, session: SessionId) -> bool {
        if self.connected(session) {
            lock(&self.expected).remove(&session);
            return false;
        }
        let entry = lock(&self.expected).get(&session).copied();
        match entry {
            Some((until, _)) if until > Instant::now() => true,
            Some(_) => {
                self.forget_expected(session);
                false
            }
            None => false,
        }
    }

    /// Ein erwarteter Runner kommt nicht zurück: Erwartung und Token verfallen.
    pub fn forget_expected(&self, session: SessionId) {
        if let Some((_, hash)) = lock(&self.expected).remove(&session)
            && !self.connected(session)
        {
            lock(&self.tokens).remove(&hash);
        }
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
            // Vor dem Start nimmt der Runner nur Freigaben an (`session_start` mit `ask`, #218).
            let conn = conns
                .get_mut(&session)
                .filter(|c| !c.prestart || name == "approval.resolve")
                .ok_or_else(|| {
                    Problem::new(ProblemCode::Unavailable)
                        .detail("Für diese Session läuft kein Runner")
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

/// Mehrere Beobachter nacheinander (Session-Titel, Abo-Fenster).
pub struct Observers(pub Vec<Arc<dyn EventObserver>>);

impl EventObserver for Observers {
    fn observe(&self, session: SessionId, written: &[Event]) {
        for o in &self.0 {
            o.observe(session, written);
        }
    }
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
    /// Automatische Compaction vor dem nächsten Turn aus der Queue (USE-009).
    pub compaction: beton_harness::CompactionSettings,
}

impl TunnelState {
    fn queue(&self) -> crate::queue::QueueCtx {
        crate::queue::QueueCtx {
            events: self.events.clone(),
            org: self.local.org,
            runners: self.runners.clone(),
            cmd_timeout: self.config.cmd_timeout,
            queues: self.queues.clone(),
            compaction: self.compaction,
        }
    }
}

/// Upgrade auf den Tunnel; Token vor dem Upgrade prüfen.
pub async fn tunnel_upgrade(
    State(state): State<TunnelState>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some((token, session)) = crate::security::bearer(&headers)
        .and_then(|t| state.runners.session_for(t).map(|s| (t.to_owned(), s)))
    else {
        return Problem::new(ProblemCode::Unauthorized).into_response();
    };
    let Ok(ws) = ws else {
        return Problem::new(ProblemCode::BadRequest)
            .detail("Tunnel-Upgrade erwartet")
            .into_response();
    };
    ws.protocols([SUBPROTOCOL])
        .on_upgrade(move |socket| connection(socket, state, session, token))
}

fn problem_msg(p: Problem) -> Message {
    let msg = TunnelDown::Problem {
        problem: serde_json::to_value(p).unwrap_or(Value::Null),
    };
    Message::Text(serde_json::to_string(&msg).unwrap_or_default().into())
}

/// Antwort auf einen Push, den der Store vorübergehend nicht speichern konnte (5xx, z. B.
/// `database is locked`). Fachliche Ablehnungen (4xx) wiederholen sich beim erneuten Senden
/// und trennen deshalb nicht.
fn unsaved_push(reply: &TunnelDown) -> bool {
    matches!(reply, TunnelDown::Problem { problem }
        if problem["status"].as_u64().is_some_and(|s| s >= 500))
}

fn close_msg(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_owned().into(),
    }))
}

async fn connection(
    socket: WebSocket,
    state: TunnelState,
    token_session: SessionId,
    token: String,
) {
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
        session_id,
        epoch,
        prestart,
        ..
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
    // Offene Freigaben aus der Projektion: ein übernommener Runner meldet sie nicht erneut
    // (POL-010 AC4, Leerlauf).
    if let Ok(open) = state.events.store.open_approvals(org).await {
        state.runners.restore_open(
            session_id,
            open.into_iter()
                .filter(|a| a.session_id == session_id)
                .map(|a| {
                    (
                        a.id,
                        a.expires_at.as_offset_date_time().unix_timestamp(),
                        a.kind,
                    )
                }),
        );
    }
    lock(&state.runners.conns).insert(
        session_id,
        RunnerConn {
            id: conn_id,
            tx,
            last_activity: Instant::now(),
            busy: false,
            pending: HashMap::new(),
            prestart,
        },
    );

    let mut ping = tokio::time::interval(state.config.ping_interval);
    ping.reset();
    let mut last_pong = Instant::now();
    let mut shutdown = state.shutdown.clone();
    loop {
        tokio::select! {
            () = shutting_down(&mut shutdown) => {
                // Abgekoppelte Runner warten auf den nächsten Daemon (POL-010 AC4), alle anderen
                // enden mit dem Daemon.
                let reason = if state.runners.is_detached(session_id) {
                    "detached"
                } else {
                    "Daemon fährt herunter"
                };
                let _ = sink.send(close_msg(close::SHUTTING_DOWN, reason)).await;
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
                            let unsaved = unsaved_push(&reply);
                            if sink.send(Message::Text(serde_json::to_string(&reply).unwrap_or_default().into())).await.is_err() || stale {
                                break;
                            }
                            if unsaved {
                                // Spätere Pushes dieser Verbindung dürfen die Lücke nicht
                                // überspringen: trennen, der Runner sendet nach dem Reconnect
                                // alles Unbestätigte erneut (PROTO-015, #278).
                                let _ = sink.send(close_msg(close::INTERNAL, "Events nicht gespeichert")).await;
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
    // Ein geordnet gestoppter Runner meldet sich nicht wieder an: sein Token sperren, bevor
    // die Verbindung als getrennt gilt. Nur dieses Token, und vor `stopped`: Wer daraufhin eine
    // Eingabe schickt, startet einen Nachfolger mit neuem Token (SES-003 AC3, #198).
    if lock(&state.runners.stopping).contains_key(&session_id) {
        state.runners.revoke_token(&token);
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
            let current = match state.events.store.session(state.local.org, session).await {
                Ok(record) => record.epoch,
                // Ohne Antwort bliebe der Push unbestätigt, und ein späterer überspränge ihn.
                Err(e) => {
                    return Some(TunnelDown::Problem {
                        problem: serde_json::to_value(Problem::from(e)).ok()?,
                    });
                }
            };
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
                                // und das Problem wird sichtbar (bei 5xx trennt die Verbindung
                                // danach, siehe `unsaved_push`).
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
                if let Err(err) = state.events.publish_transient(e) {
                    tracing::warn!(%session, code = err.code(), "Transientes Event verworfen");
                }
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
            prestart: false,
        }
    }

    /// #218 (Security-Review #284, M2): Die Vorab-Verbindung des Runners vor dem Start des
    /// Harness (Policy `session_start` mit Verbrauchswerten bzw. Zustand) zählt nicht als
    /// verbundener Runner; Kommandos warten auf die Verbindung der Hauptschleife.
    #[tokio::test]
    async fn ses_003_a_prestart_connection_is_not_a_connected_runner() {
        let runners = RunnerRegistry::default();
        let session = SessionId::new();
        let bind: TunnelUp = serde_json::from_value(serde_json::json!({
            "t": "session.bind", "session_id": session, "epoch": 0,
            "last_acked_rseq": 0, "prestart": true
        }))
        .unwrap();
        assert!(matches!(bind, TunnelUp::SessionBind { prestart: true, .. }));
        let mut pre = conn();
        pre.prestart = true;
        lock(&runners.conns).insert(session, pre);
        assert!(!runners.connected(session), "Vorab-Verbindung");
        let err = runners
            .deliver(
                session,
                "input.submit",
                Value::Null,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ProblemCode::Unavailable);
        assert!(
            err.detail
                .as_deref()
                .unwrap_or_default()
                .contains("kein Runner")
        );
        // Freigaben (`session_start` mit `ask`) erreichen den Runner schon vorab.
        let err = runners
            .deliver(
                session,
                "approval.resolve",
                Value::Null,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert!(
            err.detail
                .as_deref()
                .unwrap_or_default()
                .contains("antwortet nicht")
        );
        lock(&runners.conns).insert(session, conn());
        assert!(runners.connected(session), "Hauptschleife");
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

    /// #198: Nach dem geordneten Stopp verliert nur das Token des beendeten Runners seine
    /// Gültigkeit. Ein Nachfolger, der schon gestartet wurde (Eingabe an die gestoppte Session),
    /// meldet sich mit seinem eigenen Token weiter an.
    /// Security-Review zu #198: Tokens gelten je Session nur für den zuletzt gestarteten Runner
    /// (Generation). Ein früherer, nie verbundener Start kommt danach nicht mehr in den Tunnel.
    #[test]
    fn ses_003_ac3_minting_revokes_older_tokens_of_the_session() {
        let runners = RunnerRegistry::default();
        let (session, other) = (SessionId::new(), SessionId::new());
        let first = runners.mint_token(session).unwrap();
        let foreign = runners.mint_token(other).unwrap();
        let second = runners.mint_token(session).unwrap();
        assert_eq!(runners.session_for(&first), None, "ältere Generation");
        assert_eq!(runners.session_for(&second), Some(session));
        assert_eq!(runners.session_for(&foreign), Some(other), "andere Session");
    }

    #[test]
    fn ses_003_ac3_stopping_a_runner_keeps_the_token_of_its_successor() {
        let runners = RunnerRegistry::default();
        let session = SessionId::new();
        let old = runners.mint_token(session).unwrap();
        let successor = runners.mint_token(session).unwrap();
        runners.revoke_token(&old);
        assert_eq!(
            runners.session_for(&old),
            None,
            "altes Token bleibt gesperrt"
        );
        assert_eq!(runners.session_for(&successor), Some(session));
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
                kind: beton_core::event::ApprovalKind::Policy,
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

    fn approval_of(
        session: SessionId,
        id: beton_core::id::ApprovalId,
        secs: i64,
        kind: beton_core::event::ApprovalKind,
    ) -> beton_core::event::Event {
        let mut e = approval(session, id, secs);
        if let beton_core::event::EventBody::Inline(EventPayload::ApprovalRequested(r)) =
            &mut e.body
        {
            r.kind = kind;
        }
        e
    }

    #[test]
    fn pol_010_ac4_only_policy_approvals_detach_and_at_most_one_hour() {
        use beton_core::event::ApprovalKind;
        let (policy, question) = (SessionId::new(), SessionId::new());
        let r = RunnerRegistry::default();
        r.mint_token(policy).unwrap();
        r.mint_token(question).unwrap();
        r.note_events(
            policy,
            [&approval_of(
                policy,
                beton_core::id::ApprovalId::new(),
                24 * 3600,
                ApprovalKind::Policy,
            )],
        );
        r.note_events(
            question,
            [&approval_of(
                question,
                beton_core::id::ApprovalId::new(),
                3600,
                ApprovalKind::Question,
            )],
        );
        // Eine Frage (`ask_user`) hält keinen Runner über den Daemon hinaus.
        let d = r.detach(&[policy, question]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].session_id, policy);
        assert!(r.is_detached(policy));
        assert!(!r.is_detached(question));
        // Frist höchstens eine Stunde, auch wenn die Freigabe länger läuft.
        let max = time::OffsetDateTime::now_utc().unix_timestamp() + DETACH_MAX.as_secs() as i64;
        assert!(d[0].until <= max, "{} > {max}", d[0].until);
        assert_eq!(DETACH_MAX, Duration::from_secs(3600));
    }

    #[test]
    fn pol_010_ac4_unclaimed_adoption_expires_with_its_token() {
        let s = SessionId::new();
        let old = RunnerRegistry::default();
        let token = old.mint_token(s).unwrap();
        old.note_events(s, [&approval(s, beton_core::id::ApprovalId::new(), 600)]);
        let mut d = old.detach(&[s]).remove(0);
        let new = RunnerRegistry::default();
        // Abgelaufen: weder erwartet noch Token gültig.
        d.until = time::OffsetDateTime::now_utc().unix_timestamp() - 1;
        new.adopt(&d);
        assert_eq!(new.session_for(&token), None);
        assert!(!new.expected(s));
        // Frist läuft ab, ohne dass der Runner kommt: Token wird entfernt.
        d.until = time::OffsetDateTime::now_utc().unix_timestamp() + 600;
        new.adopt(&d);
        assert_eq!(new.session_for(&token), Some(s));
        new.forget_expected(s);
        assert_eq!(
            new.session_for(&token),
            None,
            "Token gilt nur bis zur Übernahme"
        );
        // Falsches Token: nie gültig.
        new.adopt(&d);
        assert_eq!(new.session_for("bt_run_falsch"), None);
    }

    #[test]
    fn pol_010_ac4_open_approvals_are_restored_from_the_projection() {
        use beton_core::event::ApprovalKind;
        let s = SessionId::new();
        let r = RunnerRegistry::default();
        assert!(!r.has_open_approvals(s));
        let until = time::OffsetDateTime::now_utc().unix_timestamp() + 600;
        r.restore_open(
            s,
            [(
                beton_core::id::ApprovalId::new(),
                until,
                ApprovalKind::Policy,
            )],
        );
        assert!(r.has_open_approvals(s));
        r.mint_token(s).unwrap();
        assert_eq!(r.detach(&[s]).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn pol_010_ac4_detached_file_must_be_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runners-detached.json");
        let s = SessionId::new();
        let r = RunnerRegistry::default();
        r.mint_token(s).unwrap();
        r.note_events(s, [&approval(s, beton_core::id::ApprovalId::new(), 600)]);
        crate::detached::save(&file, &r.detach(&[s])).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            crate::detached::take(&file).is_empty(),
            "lesbar für andere: nicht übernehmen"
        );
        // Ein Symlink statt der Datei wird nicht gelesen.
        let target = dir.path().join("ziel.json");
        crate::detached::save(&target, &r.detach(&[s])).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(crate::detached::take(&file).is_empty());
        // Eine liegengebliebene Temp-Datei verhindert das Speichern nicht.
        let _ = std::fs::remove_file(&file);
        std::fs::write(file.with_extension("json.tmp"), "alt").unwrap();
        crate::detached::save(&file, &r.detach(&[s])).unwrap();
        assert_eq!(crate::detached::take(&file).len(), 1);
    }
}
