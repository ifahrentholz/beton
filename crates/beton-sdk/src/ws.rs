//! WebSocket-Stream `/v1/ws` (PROTO-004 bis PROTO-006): Begrüßung, Attach mit Replay und
//! Live-Events, Kommandos.

use std::time::Duration;

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
                Ok(Message::Close(_)) => return None,
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

    /// Verbindung geordnet schließen.
    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}
