//! Binärkanäle (PROTO-007) für Rust-Clients, z. B. die Terminal-Panes der TUI (TUI-002).
//!
//! Wie `Channel` im TypeScript-SDK: eine eigene WebSocket-Verbindung je Kanal, Kredit für die
//! Daten des Servers, Eingaben als Frames mit `INPUT` innerhalb des Fensters, Reconnect mit
//! Backoff. Nach einem Reconnect beginnt der Kanal mit einem neuen Keyframe, das den Zustand
//! ersetzt.

use std::collections::VecDeque;
use std::time::Duration;

use beton_core::id::SessionId;
use beton_proto::channel::{ChannelAccess, ChannelKind, Frame, MAX_PAYLOAD, flags};
use beton_proto::ws::{Backoff, ClientMsg, ServerMsg, close};
use tokio::sync::mpsc;

use crate::ws::{Connection, Incoming, SHUTDOWN_JITTER};
use crate::{Client, Error};

/// Fenster für Daten des Servers (wie im TS-SDK).
pub const RECEIVE_WINDOW: u64 = 1024 * 1024;
/// Ab so vielen verarbeiteten Bytes gibt der Kanal Kredit zurück.
const CREDIT_STEP: u64 = 64 * 1024;
/// Close-Codes, nach denen ein Reconnect nichts bringt.
const FATAL_CLOSE: [u16; 3] = [
    close::UNAUTHENTICATED,
    close::FORBIDDEN,
    close::UNKNOWN_SESSION,
];

/// Was ein Kanal meldet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEvent {
    /// Kanal offen (auch nach einem Reconnect) mit dem Recht des Clients.
    Opened { access: ChannelAccess },
    /// Daten; `keyframe`: erstes Fragment eines vollständigen Zustands (Terminal: beginnt mit
    /// einem Reset, danach Scrollback und Bildschirm), der den bisherigen ersetzt.
    Data { bytes: Vec<u8>, keyframe: bool },
    /// Verbindung verloren; nächster Versuch nach `delay`.
    Reconnecting { attempt: u32, delay: Duration },
    /// Kanal endgültig zu (`closed`, `not_found`, `forbidden`, …).
    Closed { reason: String },
}

/// Ein offener Binärkanal. Fallen alle Handles weg, schließt er sich (`close_channel`).
#[derive(Debug)]
pub struct Channel {
    input: mpsc::UnboundedSender<Vec<u8>>,
    events: mpsc::Receiver<ChannelEvent>,
}

impl Channel {
    /// Öffnet `kind`/`target` einer Session, z. B. ein Terminal (`target`: `terminal_id`).
    /// Kehrt sofort zurück; Erfolg meldet [`ChannelEvent::Opened`].
    pub fn open(client: &Client, session: SessionId, kind: ChannelKind, target: &str) -> Self {
        let (input, input_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel(256);
        let worker = Worker {
            client: client.clone(),
            session,
            kind,
            target: target.to_owned(),
            events: events_tx,
            input: input_rx,
            pending: VecDeque::new(),
            backoff: Backoff::default(),
            attempt: 0,
        };
        tokio::spawn(worker.run());
        Self { input, events }
    }

    /// Eingabe senden (nur mit Recht `drive`); wartet im Kanal, wenn das Fenster erschöpft ist.
    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.input.send(bytes.into());
    }

    /// Nächstes Ereignis; `None`, wenn der Kanal zu ist und alles gemeldet wurde.
    pub async fn next(&mut self) -> Option<ChannelEvent> {
        self.events.recv().await
    }

    /// Teilt den Kanal in Sender und Empfänger, z. B. für eine eigene Lese-Task.
    pub fn split(self) -> (ChannelInput, mpsc::Receiver<ChannelEvent>) {
        (ChannelInput(self.input), self.events)
    }
}

/// Sendeseite eines [`Channel`].
#[derive(Debug, Clone)]
pub struct ChannelInput(mpsc::UnboundedSender<Vec<u8>>);

impl ChannelInput {
    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.0.send(bytes.into());
    }
}

struct Worker {
    client: Client,
    session: SessionId,
    kind: ChannelKind,
    target: String,
    events: mpsc::Sender<ChannelEvent>,
    input: mpsc::UnboundedReceiver<Vec<u8>>,
    pending: VecDeque<Vec<u8>>,
    backoff: Backoff,
    attempt: u32,
}

/// Wie eine Verbindung endete.
enum End {
    /// Neu verbinden (mit dem Close-Code, falls bekannt).
    Retry(Option<u16>),
    /// Kanal endgültig zu.
    Closed(String),
    /// Der Client hat alle Handles fallen lassen.
    Dropped,
}

impl Worker {
    async fn run(mut self) {
        loop {
            let end = match self.client.connect_ws().await {
                Ok(conn) => self.serve(conn).await,
                Err(Error::Problem { status, code, .. }) if status == 401 || status == 403 => {
                    End::Closed(code)
                }
                Err(_) => End::Retry(None),
            };
            match end {
                End::Retry(code) => {
                    if self.events.is_closed() {
                        return;
                    }
                    self.attempt += 1;
                    let delay = if code == Some(close::SHUTTING_DOWN) {
                        SHUTDOWN_JITTER.mul_f64(fastrand::f64())
                    } else {
                        self.backoff.next_delay()
                    };
                    let event = ChannelEvent::Reconnecting {
                        attempt: self.attempt,
                        delay,
                    };
                    if self.events.send(event).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(delay).await;
                }
                End::Closed(reason) => {
                    let _ = self.events.send(ChannelEvent::Closed { reason }).await;
                    return;
                }
                End::Dropped => return,
            }
        }
    }

    /// Eine Verbindung: Kanal öffnen, Daten weiterreichen, Eingaben senden.
    async fn serve(&mut self, mut conn: Connection) -> End {
        let open = ClientMsg::OpenChannel {
            id: "c1".into(),
            session_id: self.session,
            kind: self.kind,
            target: self.target.clone(),
        };
        if conn.send(&open).await.is_err() {
            return End::Retry(conn.close_code());
        }
        let mut channel_id: Option<u32> = None;
        let mut access = ChannelAccess::Read;
        let mut window: u64 = 0;
        let mut owed: u64 = 0;
        let mut in_keyframe = false;
        loop {
            tokio::select! {
                incoming = conn.next_incoming() => {
                    let Some(incoming) = incoming else {
                        let code = conn.close_code();
                        if code.is_some_and(|c| FATAL_CLOSE.contains(&c)) {
                            return End::Closed(format!("close_{}", code.unwrap_or_default()));
                        }
                        return End::Retry(code);
                    };
                    let Ok(incoming) = incoming else {
                        return End::Retry(conn.close_code());
                    };
                    match incoming {
                        Incoming::Frame(f) => {
                            if Some(f.channel_id) != channel_id {
                                continue;
                            }
                            let is_key = f.has(flags::KEYFRAME);
                            let fin = f.has(flags::FIN);
                            let len = f.payload.len() as u64;
                            let event = ChannelEvent::Data {
                                bytes: f.payload,
                                keyframe: is_key && !in_keyframe,
                            };
                            in_keyframe = is_key && !fin;
                            if self.events.send(event).await.is_err() {
                                let _ = conn.send(&ClientMsg::CloseChannel { channel_id: f.channel_id }).await;
                                conn.close().await;
                                return End::Dropped;
                            }
                            owed += len;
                            if owed >= CREDIT_STEP {
                                let credit = ClientMsg::Credit { channel_id: f.channel_id, bytes: Some(owed), frames: None };
                                owed = 0;
                                if conn.send(&credit).await.is_err() {
                                    return End::Retry(conn.close_code());
                                }
                            }
                        }
                        Incoming::Msg(ServerMsg::ChannelOpened { channel_id: id, window: w, access: a, .. }) => {
                            channel_id = Some(id);
                            access = a;
                            window = w;
                            in_keyframe = false;
                            self.backoff.reset();
                            self.attempt = 0;
                            let credit = ClientMsg::Credit { channel_id: id, bytes: Some(RECEIVE_WINDOW), frames: None };
                            if conn.send(&credit).await.is_err() {
                                return End::Retry(conn.close_code());
                            }
                            if self.events.send(ChannelEvent::Opened { access }).await.is_err() {
                                conn.close().await;
                                return End::Dropped;
                            }
                        }
                        Incoming::Msg(ServerMsg::Credit { channel_id: id, bytes }) if Some(id) == channel_id => {
                            window += bytes;
                        }
                        Incoming::Msg(ServerMsg::ChannelClosed { channel_id: id, reason }) if Some(id) == channel_id => {
                            let reason = reason.unwrap_or_else(|| "closed".into());
                            // Runner kurz getrennt: neu öffnen, sobald er den Kanal wieder meldet.
                            if reason == "detached" || reason == "stalled" {
                                conn.close().await;
                                return End::Retry(None);
                            }
                            conn.close().await;
                            return End::Closed(reason);
                        }
                        Incoming::Msg(ServerMsg::Nack { problem, .. }) if channel_id.is_none() => {
                            conn.close().await;
                            let code = problem["code"].as_str().unwrap_or("rejected").to_owned();
                            return End::Closed(code);
                        }
                        Incoming::Msg(_) => {}
                    }
                }
                bytes = self.input.recv() => {
                    let Some(bytes) = bytes else {
                        if let Some(id) = channel_id {
                            let _ = conn.send(&ClientMsg::CloseChannel { channel_id: id }).await;
                        }
                        conn.close().await;
                        return End::Dropped;
                    };
                    for chunk in bytes.chunks(MAX_PAYLOAD) {
                        self.pending.push_back(chunk.to_vec());
                    }
                }
            }
            // Eingaben innerhalb des Fensters senden (nur mit `drive`).
            if let Some(id) = channel_id
                && access == ChannelAccess::Drive
            {
                while self
                    .pending
                    .front()
                    .is_some_and(|c| c.len() as u64 <= window)
                {
                    let Some(chunk) = self.pending.pop_front() else {
                        break;
                    };
                    window -= chunk.len() as u64;
                    let frame = Frame::new(id, flags::INPUT | flags::FIN, chunk);
                    if conn.send_frame(&frame).await.is_err() {
                        return End::Retry(conn.close_code());
                    }
                }
            }
        }
    }
}
