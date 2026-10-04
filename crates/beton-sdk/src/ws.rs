//! WebSocket-Stream `/v1/ws` (PROTO-004 bis PROTO-006): Begrüßung, Attach mit Replay und
//! Live-Events, Kommandos.

use std::time::Duration;

use beton_core::event::Event;
use beton_core::id::SessionId;
pub use beton_proto::ws::ServerMsg;
use beton_proto::ws::{ClientInfo, ClientMsg, PROTOCOL_MAJOR, PROTOCOL_MINOR, SUBPROTOCOL};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::{Client, Error, Result};

/// Wie lange auf `welcome` gewartet wird.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Eine geöffnete, begrüßte WebSocket-Verbindung.
pub struct Connection {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    url: String,
    /// Close-Code des Servers, sobald die Verbindung zu ist.
    close_code: Option<u16>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Connection {
    pub(crate) async fn open(client: &Client) -> Result<Self> {
        let base = client.base_url();
        let url = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}/v1/ws")
        } else {
            format!("ws://{}/v1/ws", base.trim_start_matches("http://"))
        };
        let unreachable = |reason: String| Error::Unreachable {
            url: url.clone(),
            reason,
        };
        let mut req = url
            .as_str()
            .into_client_request()
            .map_err(|e| unreachable(e.to_string()))?;
        let headers = req.headers_mut();
        headers.insert(
            "authorization",
            format!("Bearer {}", client.token())
                .parse()
                .map_err(|_| Error::Decode("Token enthält ungültige Zeichen".into()))?,
        );
        headers.insert(
            "sec-websocket-protocol",
            SUBPROTOCOL
                .parse()
                .map_err(|_| Error::Decode("Subprotokoll".into()))?,
        );
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| match e {
                tokio_tungstenite::tungstenite::Error::Http(res) => {
                    let body: Value = res
                        .body()
                        .as_deref()
                        .and_then(|b| serde_json::from_slice(b).ok())
                        .unwrap_or(Value::Null);
                    crate::from_problem(res.status().as_u16(), &body)
                }
                other => unreachable(other.to_string()),
            })?;
        let mut conn = Self {
            ws,
            next_id: 0,
            url,
            close_code: None,
        };
        conn.send(&ClientMsg::Hello {
            protocol: format!("{PROTOCOL_MAJOR}.{PROTOCOL_MINOR}"),
            client: ClientInfo {
                kind: "sdk-rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        })
        .await?;
        let welcome = tokio::time::timeout(HELLO_TIMEOUT, async {
            while let Some(msg) = conn.next().await {
                match msg? {
                    ServerMsg::Welcome { .. } => return Ok(()),
                    ServerMsg::Nack { problem, .. } => {
                        return Err(crate::from_problem(400, &problem));
                    }
                    _ => {}
                }
            }
            Err(Error::Unreachable {
                url: conn.url.clone(),
                reason: "Verbindung vor `welcome` geschlossen".into(),
            })
        })
        .await;
        match welcome {
            Ok(r) => r.map(|()| conn),
            Err(_) => Err(Error::Unreachable {
                url: conn.url.clone(),
                reason: "keine Antwort auf `hello`".into(),
            }),
        }
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    async fn send(&mut self, msg: &ClientMsg) -> Result<()> {
        let text = serde_json::to_string(msg).map_err(|e| Error::Decode(e.to_string()))?;
        self.ws
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| Error::Unreachable {
                url: self.url.clone(),
                reason: e.to_string(),
            })
    }

    /// Events ab `from_seq` (exklusiv): erst Replay, dann `live`, dann Live-Events.
    pub async fn attach(&mut self, session: SessionId, from_seq: u64) -> Result<String> {
        let id = self.id("a");
        self.send(&ClientMsg::Attach {
            id: id.clone(),
            session_id: session,
            from_seq,
            transient: true,
            tail: None,
        })
        .await?;
        Ok(id)
    }

    /// Kommando mit REST-Zwilling (PROTO-006); Antwort kommt als `ack`/`nack` mit der ID.
    pub async fn cmd(
        &mut self,
        session: Option<SessionId>,
        name: &str,
        args: Value,
    ) -> Result<String> {
        let id = self.id("c");
        self.send(&ClientMsg::Cmd {
            id: id.clone(),
            session_id: session,
            name: name.to_owned(),
            args,
            idempotency_key: None,
        })
        .await?;
        Ok(id)
    }

    /// Nächste Nachricht; `None`, wenn der Server die Verbindung schließt.
    pub async fn next(&mut self) -> Option<Result<ServerMsg>> {
        loop {
            match self.ws.next().await? {
                Ok(Message::Text(text)) => {
                    return Some(
                        ServerMsg::from_json(&text).map_err(|e| Error::Decode(e.to_string())),
                    );
                }
                Ok(Message::Close(frame)) => {
                    self.close_code = Some(frame.map_or(1005, |f| u16::from(f.code)));
                    return None;
                }
                Ok(_) => {}
                Err(e) => {
                    return Some(Err(Error::Unreachable {
                        url: self.url.clone(),
                        reason: e.to_string(),
                    }));
                }
            }
        }
    }

    /// Close-Code, mit dem der Server die Verbindung beendet hat.
    pub fn close_code(&self) -> Option<u16> {
        self.close_code
    }

    /// Verbindung geordnet schließen.
    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

/// Was ein [`Subscription`] liefert.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// Neue Events (dauerhafte ohne Duplikate, transiente unverändert).
    Events(Vec<Event>),
    /// Replay abgeschlossen; ab jetzt live.
    Live { head_seq: u64 },
    /// Verbindung verloren; nächster Versuch nach `delay`.
    Reconnecting { attempt: u32, delay: Duration },
    /// Der Server kennt weniger Events als der Client (`seq_ahead`, z. B. nach einer
    /// Wiederherstellung): der Client baut seinen Zustand ab `seq` 0 neu auf.
    Reset,
}

/// Höchstwartezeit nach Close `4503` (Server fährt geordnet herunter): sofort, aber
/// gleichverteilt über dieses Fenster, damit nicht alle Clients gleichzeitig kommen
/// (PROTO-009 AC2).
pub const SHUTDOWN_JITTER: Duration = Duration::from_secs(2);

/// Close-Codes, nach denen ein Reconnect nichts bringt.
const FATAL_CLOSE: [u16; 3] = [
    beton_proto::ws::close::UNAUTHENTICATED,
    beton_proto::ws::close::FORBIDDEN,
    beton_proto::ws::close::UNKNOWN_SESSION,
];

/// Event-Strom einer Session mit automatischem Reconnect (PROTO-005, PROTO-009): nach einem
/// Abbruch wartet es laut [`beton_proto::ws::Backoff`] (nach `4503` nur Jitter), verbindet
/// neu und attacht ab der zuletzt gesehenen `seq`. `overflow` und `seq_ahead` behandelt es
/// selbst. Dauerhafte Events kommen genau einmal und lückenlos.
pub struct Subscription {
    client: Client,
    session: SessionId,
    last_seq: u64,
    conn: Option<Connection>,
    backoff: beton_proto::ws::Backoff,
    attempt: u32,
    pending: Option<Duration>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("session", &self.session)
            .field("last_seq", &self.last_seq)
            .finish_non_exhaustive()
    }
}

impl Subscription {
    /// Verbindet und attacht ab `from_seq` (exklusiv). Scheitert schon der erste Versuch,
    /// liefert es den Fehler.
    pub async fn open(client: &Client, session: SessionId, from_seq: u64) -> Result<Self> {
        let mut conn = client.connect_ws().await?;
        conn.attach(session, from_seq).await?;
        Ok(Self {
            client: client.clone(),
            session,
            last_seq: from_seq,
            conn: Some(conn),
            backoff: beton_proto::ws::Backoff::default(),
            attempt: 0,
            pending: None,
        })
    }

    /// Zuletzt gesehene dauerhafte `seq`.
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    fn schedule(&mut self, close_code: Option<u16>) -> Update {
        self.attempt += 1;
        let delay = if close_code == Some(beton_proto::ws::close::SHUTTING_DOWN) {
            SHUTDOWN_JITTER.mul_f64(fastrand::f64())
        } else {
            self.backoff.next_delay()
        };
        self.pending = Some(delay);
        Update::Reconnecting {
            attempt: self.attempt,
            delay,
        }
    }

    /// Nächste Aktualisierung; `None` nie (der Strom endet nur mit einem Fehler).
    pub async fn next(&mut self) -> Result<Update> {
        loop {
            if self.conn.is_none() {
                if let Some(delay) = self.pending.take() {
                    tokio::time::sleep(delay).await;
                }
                let attached = async {
                    let mut conn = self.client.connect_ws().await?;
                    conn.attach(self.session, self.last_seq).await?;
                    Ok::<_, Error>(conn)
                }
                .await;
                match attached {
                    Ok(conn) => {
                        self.conn = Some(conn);
                        self.backoff.reset();
                        self.attempt = 0;
                    }
                    Err(Error::Problem { status, .. }) if status == 401 || status == 403 => {
                        return Err(Error::Unreachable {
                            url: self.client.base_url().to_owned(),
                            reason: format!("Anmeldung abgelehnt ({status})"),
                        });
                    }
                    Err(_) => return Ok(self.schedule(None)),
                }
            }
            let Some(conn) = self.conn.as_mut() else {
                continue;
            };
            match conn.next().await {
                Some(Ok(ServerMsg::Events { events, .. })) => {
                    let mut fresh = Vec::with_capacity(events.len());
                    for e in events {
                        if e.transient {
                            fresh.push(e);
                        } else if e.seq > self.last_seq {
                            self.last_seq = e.seq;
                            fresh.push(e);
                        }
                    }
                    if !fresh.is_empty() {
                        return Ok(Update::Events(fresh));
                    }
                }
                Some(Ok(ServerMsg::Live { head_seq, .. })) => {
                    return Ok(Update::Live { head_seq });
                }
                Some(Ok(ServerMsg::Overflow { .. })) => {
                    // Ab dem zuletzt Gesehenen neu anfordern (PROTO-008).
                    let from = self.last_seq;
                    conn.attach(self.session, from).await?;
                }
                Some(Ok(ServerMsg::Nack { problem, .. })) => {
                    if problem["code"] == "seq_ahead" {
                        self.last_seq = 0;
                        conn.attach(self.session, 0).await?;
                        return Ok(Update::Reset);
                    }
                    return Err(crate::from_problem(400, &problem));
                }
                Some(Ok(_)) => {}
                Some(Err(Error::Decode(e))) => return Err(Error::Decode(e)),
                Some(Err(_)) | None => {
                    let code = self.conn.take().and_then(|c| c.close_code());
                    if code.is_some_and(|c| FATAL_CLOSE.contains(&c)) {
                        return Err(Error::Unreachable {
                            url: self.client.base_url().to_owned(),
                            reason: format!("Verbindung mit Code {code:?} beendet"),
                        });
                    }
                    return Ok(self.schedule(code));
                }
            }
        }
    }
}
