//! WebSocket-Endpunkt `/v1/ws` (PROTO-004, PROTO-005, PROTO-006, PROTO-008, PROTO-009).
//!
//! Je Verbindung: ein Leser (Steuernachrichten), ein Schreiber (Ausgangswarteschlange mit
//! Byte-Zähler) und je abonnierter Session ein Weiterleiter. Authentisierung und
//! Origin-Prüfung erledigt die Sicherheitsschicht vor dem Upgrade (AUTH-001, AUTH-003).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use beton_core::event::Event;
use beton_core::id::SessionId;
use beton_proto::ws::{
    ClientMsg, PROTOCOL_MINOR, SUBPROTOCOL, ServerMsg, SessionLimits, close, negotiate,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::app::AppState;
use crate::commands::CommandCtx;
use crate::problem::{Problem, ProblemCode};
use crate::security::Authenticated;

/// Zeit- und Größengrenzen der Verbindung.
#[derive(Debug, Clone, Copy)]
pub struct WsConfig {
    pub hello_timeout: Duration,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    /// Spätestens so lange sammelt der Server Live-Events zu einem Batch.
    pub batch_window: Duration,
    /// Ab hier werden transiente Deltas zurückgehalten und später als Snapshot gesendet.
    pub coalesce_bytes: usize,
    /// Ab hier: `overflow` und Ende der Weiterleitung für die Session.
    pub overflow_bytes: usize,
    /// So lange darf das Kommando-Limit am Stück überschritten werden, dann Close 4429.
    pub rate_violation_close: Duration,
    pub limits: SessionLimits,
    pub server_minor: u32,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(20),
            pong_timeout: Duration::from_secs(60),
            batch_window: Duration::from_millis(16),
            coalesce_bytes: 1024 * 1024,
            overflow_bytes: 4 * 1024 * 1024,
            rate_violation_close: Duration::from_secs(10),
            limits: SessionLimits::default(),
            server_minor: PROTOCOL_MINOR,
        }
    }
}

/// Wer Events einer Session lesen darf. Lokal (M0) gehört alles dem einen Benutzer;
/// Freigaben (COL-001) ersetzen das in M4.
pub trait Authorizer: Send + Sync {
    fn can_read(&self, auth: Authenticated, session: &beton_store::SessionRecord) -> bool;
}

#[derive(Debug, Default)]
pub struct LocalAuthorizer;

impl Authorizer for LocalAuthorizer {
    fn can_read(&self, _auth: Authenticated, _session: &beton_store::SessionRecord) -> bool {
        true
    }
}

/// WebSocket-Verbindung (Upgrade, Subprotokoll `beton.v1`).
#[utoipa::path(get, path = "/v1/ws", tag = "ws",
    responses((status = 101, description = "Upgrade auf WebSocket, Subprotokoll beton.v1")))]
pub async fn ws_upgrade(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Ok(ws) = ws else {
        return Problem::new(ProblemCode::BadRequest)
            .detail("WebSocket-Upgrade erwartet (Subprotokoll beton.v1)")
            .into_response();
    };
    let max = state.runtime.ws.limits.max_text_frame_bytes as usize;
    ws.protocols([SUBPROTOCOL])
        .max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| connection(socket, state, auth))
}

/// Ausgangswarteschlange mit Byte-Zähler (PROTO-008).
#[derive(Clone)]
struct Outbox {
    tx: mpsc::UnboundedSender<Message>,
    bytes: Arc<AtomicUsize>,
}

impl Outbox {
    fn queued(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    fn send(&self, msg: &ServerMsg) -> bool {
        let text = serde_json::to_string(msg).unwrap_or_default();
        self.bytes.fetch_add(text.len(), Ordering::Relaxed);
        self.tx.send(Message::Text(text.into())).is_ok()
    }

    fn raw(&self, msg: Message) -> bool {
        self.tx.send(msg).is_ok()
    }

    fn close(&self, code: u16, reason: &str) {
        let _ = self.tx.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.to_owned().into(),
        })));
    }
}

fn nack(out: &Outbox, id: &str, problem: Problem) {
    out.send(&ServerMsg::Nack {
        id: id.to_owned(),
        problem: serde_json::to_value(problem).unwrap_or(Value::Null),
    });
}

async fn connection(socket: WebSocket, state: AppState, auth: Authenticated) {
    let cfg = state.runtime.ws;
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let out = Outbox {
        tx,
        bytes: Arc::new(AtomicUsize::new(0)),
    };
    let counter = out.bytes.clone();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let len = match &msg {
                Message::Text(t) => t.len(),
                _ => 0,
            };
            let closing = matches!(msg, Message::Close(_));
            let sent = sink.send(msg).await;
            counter.fetch_sub(len, Ordering::Relaxed);
            if sent.is_err() || closing {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Handshake (PROTO-004): `hello` innerhalb der Frist.
    let hello = tokio::time::timeout(cfg.hello_timeout, stream.next()).await;
    let client = match hello {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<ClientMsg>(&text).ok(),
        _ => None,
    };
    let Some(ClientMsg::Hello { protocol, .. }) = client else {
        out.close(close::PROTOCOL, "hello erwartet");
        let _ = writer.await;
        return;
    };
    match negotiate(&protocol, cfg.server_minor) {
        Ok(version) => {
            out.send(&ServerMsg::Welcome {
                protocol: version,
                server_version: env!("CARGO_PKG_VERSION").into(),
                session_limits: cfg.limits,
            });
        }
        Err(supported) => {
            let reason = json!({"supported": {"min": supported.min, "max": supported.max}});
            out.close(close::PROTOCOL, &reason.to_string());
            let _ = writer.await;
            return;
        }
    }

    let mut conn = Conn {
        state: state.clone(),
        auth,
        out: out.clone(),
        forwarders: HashMap::new(),
        rate: RateLimit::new(cfg.limits.max_commands_per_s, cfg.rate_violation_close),
    };
    let mut ping = tokio::time::interval(cfg.ping_interval);
    ping.reset();
    let mut last_pong = Instant::now();
    let mut shutdown = state.runtime.shutdown.clone();
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Some(code) = conn.handle(&text).await {
                        out.close(code, "Kommando-Limit dauerhaft überschritten");
                        break;
                    }
                }
                Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                Some(Ok(Message::Ping(p))) => {
                    out.raw(Message::Pong(p));
                }
                Some(Ok(Message::Binary(_))) => {
                    // Binärkanäle folgen mit PROTO-007 (M3).
                    out.close(close::PROTOCOL, "Binärkanäle werden noch nicht unterstützt");
                    break;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            _ = ping.tick() => {
                if last_pong.elapsed() > cfg.pong_timeout {
                    out.close(close::HEARTBEAT_TIMEOUT, "kein Pong");
                    break;
                }
                out.raw(Message::Ping(Vec::new().into()));
            }
            () = wait_shutdown(&mut shutdown) => {
                out.close(close::SHUTTING_DOWN, "Server fährt herunter");
                break;
            }
        }
    }
    for (_, f) in conn.forwarders.drain() {
        f.abort();
    }
    drop(conn);
    drop(out);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

async fn wait_shutdown(rx: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = rx.wait_for(|s| *s).await;
}

/// Kommando-Limit je Verbindung (PROTO-008 AC3).
struct RateLimit {
    max: u32,
    window_start: Instant,
    count: u32,
    violating_since: Option<Instant>,
    close_after: Duration,
}

enum RateVerdict {
    Ok,
    Reject,
    Close,
}

impl RateLimit {
    fn new(max: u32, close_after: Duration) -> Self {
        Self {
            max,
            window_start: Instant::now(),
            count: 0,
            violating_since: None,
            close_after,
        }
    }

    fn check(&mut self) -> RateVerdict {
        let now = Instant::now();
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            if self.count <= self.max {
                self.violating_since = None;
            }
            self.window_start = now;
            self.count = 0;
        }
        self.count += 1;
        if self.count <= self.max {
            return RateVerdict::Ok;
        }
        let since = *self.violating_since.get_or_insert(now);
        if now.duration_since(since) >= self.close_after {
            RateVerdict::Close
        } else {
            RateVerdict::Reject
        }
    }
}

struct Conn {
    state: AppState,
    auth: Authenticated,
    out: Outbox,
    forwarders: HashMap<SessionId, JoinHandle<()>>,
    rate: RateLimit,
}

impl Conn {
    /// Verarbeitet eine Steuernachricht; `Some(code)` beendet die Verbindung.
    async fn handle(&mut self, text: &str) -> Option<u16> {
        let msg = match serde_json::from_str::<ClientMsg>(text) {
            Ok(m) => m,
            Err(e) => {
                nack(
                    &self.out,
                    "",
                    Problem::new(ProblemCode::BadRequest)
                        .detail(format!("Nachricht ungültig: {e}")),
                );
                return None;
            }
        };
        match msg {
            ClientMsg::Hello { .. } => {}
            ClientMsg::Attach {
                id,
                session_id,
                from_seq,
                transient,
                tail,
            } => self.attach(id, session_id, from_seq, transient, tail).await,
            ClientMsg::Detach { id, session_id } => {
                if let Some(f) = self.forwarders.remove(&session_id) {
                    f.abort();
                }
                self.out.send(&ServerMsg::Ack {
                    id,
                    result: Value::Null,
                });
            }
            ClientMsg::Cmd {
                id,
                session_id,
                name,
                args,
                idempotency_key,
            } => {
                match self.rate.check() {
                    RateVerdict::Ok => {}
                    RateVerdict::Reject => {
                        nack(&self.out, &id, Problem::new(ProblemCode::RateLimited));
                        return None;
                    }
                    RateVerdict::Close => return Some(close::RATE_LIMIT),
                }
                self.command(id, session_id, name, args, idempotency_key)
                    .await;
            }
        }
        None
    }

    async fn attach(
        &mut self,
        id: String,
        session: SessionId,
        from_seq: u64,
        transient: bool,
        tail: Option<u32>,
    ) {
        let org = self.state.local.org;
        let record = match self.state.store.session(org, session).await {
            Ok(r) => r,
            Err(e) => return nack(&self.out, &id, Problem::from(e)),
        };
        if !self.state.runtime.authorizer.can_read(self.auth, &record) {
            return nack(&self.out, &id, Problem::new(ProblemCode::Forbidden));
        }
        if from_seq > record.head_seq {
            return nack(
                &self.out,
                &id,
                Problem::new(ProblemCode::SeqAhead).detail(format!(
                    "from_seq {from_seq} > head_seq {}; neu ab 0 attachen",
                    record.head_seq
                )),
            );
        }
        if let Some(old) = self.forwarders.remove(&session) {
            old.abort();
        }
        // Erst abonnieren, dann nachliefern: so geht zwischen Replay und Live nichts verloren.
        let (rx, snapshot) = self.state.runtime.hub.subscribe(session);
        let fwd = Forwarder {
            state: self.state.clone(),
            out: self.out.clone(),
            session,
            transient,
            last: from_seq,
        };
        self.forwarders.insert(
            session,
            tokio::spawn(fwd.run(rx, snapshot, tail, record.head_seq)),
        );
    }

    async fn command(
        &mut self,
        id: String,
        session: Option<SessionId>,
        name: String,
        args: Value,
        key: Option<String>,
    ) {
        let Some(command) = self.state.runtime.commands.get(&name).cloned() else {
            return nack(
                &self.out,
                &id,
                Problem::new(ProblemCode::UnknownCommand).detail(format!("`{name}` gibt es nicht")),
            );
        };
        let org = self.state.local.org;
        let store = &self.state.store;
        let hash = hex::encode(Sha256::digest(
            json!({"name": name, "session": session, "args": args}).to_string(),
        ));
        let key = key.map(|k| format!("ws:{k}"));
        if let Some(k) = &key {
            match store.idempotency_lookup(org, k).await {
                Ok(Some(stored)) if stored.request_hash != hash => {
                    return nack(
                        &self.out,
                        &id,
                        Problem::new(ProblemCode::IdempotencyKeyReused),
                    );
                }
                Ok(Some(stored)) => {
                    let result = serde_json::from_slice(&stored.body).unwrap_or(Value::Null);
                    self.out.send(&ServerMsg::Ack { id, result });
                    return;
                }
                Ok(None) => {}
                Err(e) => return nack(&self.out, &id, Problem::from(e)),
            }
        }
        let ctx = CommandCtx {
            events: self.state.events(),
            local: self.state.local,
            auth: self.auth,
        };
        match command.run(&ctx, session, args).await {
            Ok(result) => {
                if let Some(k) = &key {
                    let stored = beton_store::StoredResponse {
                        request_hash: hash,
                        status: 200,
                        content_type: "application/json".into(),
                        body: serde_json::to_vec(&result).unwrap_or_default(),
                    };
                    let _ = store.idempotency_save(org, k, &stored).await;
                }
                self.out.send(&ServerMsg::Ack { id, result });
            }
            Err(problem) => nack(&self.out, &id, problem),
        }
    }
}

/// Liefert eine Session an eine Verbindung: Replay, `live`, Snapshot, dann Live-Events.
struct Forwarder {
    state: AppState,
    out: Outbox,
    session: SessionId,
    transient: bool,
    /// Höchste bereits gesendete dauerhafte `seq`.
    last: u64,
}

enum Flow {
    Continue,
    Stop,
}

impl Forwarder {
    fn cfg(&self) -> WsConfig {
        self.state.runtime.ws
    }

    async fn run(
        mut self,
        mut rx: broadcast::Receiver<Arc<Event>>,
        snapshot: Vec<Event>,
        tail: Option<u32>,
        head: u64,
    ) {
        if let Some(n) = tail {
            let start = head.saturating_sub(u64::from(n)).max(self.last);
            let has_more = start > self.last;
            self.last = start;
            if let Flow::Stop = self.catch_up(head, Some(has_more)).await {
                return;
            }
        } else if let Flow::Stop = self.catch_up(head, None).await {
            return;
        }
        self.out.send(&ServerMsg::Live {
            session_id: self.session,
            head_seq: self.last,
        });
        if self.transient && !snapshot.is_empty() {
            self.send_batch(snapshot);
        }
        let mut pending_snapshot = false;
        loop {
            let first = match rx.recv().await {
                Ok(e) => e,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Broadcast verpasst: dauerhafte Events aus dem Store nachholen.
                    let head = self
                        .state
                        .store
                        .session(self.state.local.org, self.session)
                        .await
                        .map(|s| s.head_seq)
                        .unwrap_or(self.last);
                    if let Flow::Stop = self.catch_up(head, None).await {
                        return;
                    }
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let before = self.last;
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            let deadline = tokio::time::Instant::now() + self.cfg().batch_window;
            let mut next = Some(first);
            loop {
                if let Some(e) = next.take() {
                    match self
                        .accept(&e, &mut batch, &mut bytes, &mut pending_snapshot)
                        .await
                    {
                        Flow::Continue => {}
                        Flow::Stop => return,
                    }
                }
                if batch.len() >= self.cfg().limits.max_batch_events as usize
                    || bytes >= self.cfg().limits.max_batch_bytes as usize
                {
                    break;
                }
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Ok(e)) => next = Some(e),
                    _ => break,
                }
            }
            if let Flow::Stop = self.check_overflow(before) {
                return;
            }
            self.send_batch(batch);
            if pending_snapshot && self.out.queued() < self.cfg().coalesce_bytes {
                pending_snapshot = false;
                let (_, snap) = self.state.runtime.hub.subscribe(self.session);
                self.send_batch(snap);
            }
        }
    }

    /// Nimmt ein Live-Event in den Batch auf: Duplikate weg, Lücken aus dem Store.
    async fn accept(
        &mut self,
        e: &Arc<Event>,
        batch: &mut Vec<Event>,
        bytes: &mut usize,
        pending_snapshot: &mut bool,
    ) -> Flow {
        if e.transient {
            if !self.transient {
                return Flow::Continue;
            }
            if self.out.queued() >= self.cfg().coalesce_bytes {
                *pending_snapshot = true;
                return Flow::Continue;
            }
            *bytes += approx_len(e);
            batch.push((**e).clone());
            return Flow::Continue;
        }
        if e.seq <= self.last {
            return Flow::Continue;
        }
        if e.seq > self.last + 1 {
            // Lücke (paralleles Anhängen, verpasster Broadcast): aus dem Store schließen.
            match self
                .state
                .store
                .events(
                    self.state.local.org,
                    self.session,
                    self.last,
                    (e.seq - self.last - 1) as u32,
                )
                .await
            {
                Ok(missing) => {
                    for m in missing {
                        *bytes += approx_len(&m);
                        self.last = m.seq;
                        batch.push(m);
                    }
                }
                Err(_) => return Flow::Stop,
            }
        }
        *bytes += approx_len(e);
        self.last = e.seq;
        batch.push((**e).clone());
        Flow::Continue
    }

    /// Replay aus dem Store bis `head`.
    async fn catch_up(&mut self, head: u64, has_more: Option<bool>) -> Flow {
        let mut first = true;
        while self.last < head {
            let max = self.cfg().limits.max_batch_events;
            let events = match self
                .state
                .store
                .events(self.state.local.org, self.session, self.last, max)
                .await
            {
                Ok(e) if !e.is_empty() => e,
                _ => break,
            };
            if let Flow::Stop = self.check_overflow(self.last) {
                return Flow::Stop;
            }
            let mut chunk = Vec::new();
            let mut bytes = 0;
            for e in events {
                bytes += approx_len(&e);
                self.last = e.seq;
                chunk.push(e);
                if bytes >= self.cfg().limits.max_batch_bytes as usize {
                    self.send_events(
                        std::mem::take(&mut chunk),
                        first.then_some(has_more).flatten(),
                    );
                    first = false;
                    bytes = 0;
                }
            }
            if !chunk.is_empty() {
                self.send_events(chunk, first.then_some(has_more).flatten());
                first = false;
            }
        }
        if first && has_more == Some(true) {
            self.send_events(Vec::new(), has_more);
        }
        Flow::Continue
    }

    /// PROTO-008: Zu volle Warteschlange → `overflow` und Ende für diese Session.
    fn check_overflow(&self, resume_from: u64) -> Flow {
        if self.out.queued() > self.cfg().overflow_bytes {
            self.out.send(&ServerMsg::Overflow {
                session_id: self.session,
                resume_from,
            });
            return Flow::Stop;
        }
        Flow::Continue
    }

    fn send_batch(&self, events: Vec<Event>) {
        if !events.is_empty() {
            self.send_events(events, None);
        }
    }

    fn send_events(&self, events: Vec<Event>, has_more: Option<bool>) {
        self.out.send(&ServerMsg::Events {
            session_id: self.session,
            events,
            has_more,
        });
    }
}

fn approx_len(e: &Event) -> usize {
    serde_json::to_string(e).map(|s| s.len()).unwrap_or(256)
}
