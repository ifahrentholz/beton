//! Gemeinsamer JSON-RPC-2.0-Transport über stdio für Codex `app-server` (HAR-006) und ACP
//! (HAR-007), ADR-0005: ein Transport, nicht zwei.
//!
//! Eine Nachricht pro Zeile. Ausgehend setzt beton immer `"jsonrpc": "2.0"`; eingehend ist das
//! Feld optional, weil `codex app-server` es weglässt (verifiziert gegen codex-cli 0.153.2).
//!
//! Antworten auf [`RpcClient::request`] löst der Lese-Task sofort auf (für Handshakes und
//! Steuerbefehle). Antworten auf [`RpcClient::send_request`] reiht er dagegen als
//! [`Incoming::Response`] in denselben geordneten Strom wie Notifications ein, damit ein
//! Adapter z. B. das Turn-Ende erst nach den vorangehenden Updates abbildet.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use beton_core::event::RawJson;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::adapter::HarnessError;

/// JSON-RPC-Fehlercodes, die beton selbst sendet.
pub mod codes {
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// Fehlerobjekt einer Antwort bzw. Transportfehler.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RpcError {
    #[error("JSON-RPC-Fehler {code}: {message}")]
    Remote {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("Verbindung zum Harness geschlossen")]
    Closed,
    #[error("keine Antwort innerhalb von {0:?}")]
    Timeout(Duration),
}

impl RpcError {
    fn from_value(v: &Value) -> Self {
        Self::Remote {
            code: v["code"].as_i64().unwrap_or(0),
            message: v["message"].as_str().unwrap_or_default().to_owned(),
            data: v.get("data").cloned().filter(|d| !d.is_null()),
        }
    }
}

impl From<RpcError> for HarnessError {
    fn from(e: RpcError) -> Self {
        match e {
            RpcError::Closed => HarnessError::Closed,
            other => HarnessError::Protocol(other.to_string()),
        }
    }
}

/// Eine eingehende Nachricht in Lesereihenfolge. `raw` ist die Original-Zeile (HAR-001 AC3).
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
        raw: Option<RawJson>,
    },
    /// Anfrage des Harness an beton; muss mit [`RpcClient::respond`] beantwortet werden.
    Request {
        id: Value,
        method: String,
        params: Value,
        raw: Option<RawJson>,
    },
    /// Antwort auf eine mit [`RpcClient::send_request`] gesendete Anfrage.
    Response {
        id: u64,
        result: Result<Value, RpcError>,
        raw: Option<RawJson>,
    },
    /// Zeile, die kein JSON-RPC ist (ungültiges JSON oder unbekannte Form).
    Invalid { line: String },
}

type Writer = Arc<Mutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>>;
type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// Ausstehende Antwort auf eine mit [`RpcClient::request_detached`] gesendete Anfrage.
#[derive(Debug)]
pub struct PendingReply(oneshot::Receiver<Result<Value, RpcError>>);

impl PendingReply {
    /// Wartet auf die Antwort; endet die Verbindung vorher, [`RpcError::Closed`].
    pub async fn wait(self) -> Result<Value, RpcError> {
        self.0.await.unwrap_or(Err(RpcError::Closed))
    }

    /// Wie [`Self::wait`], mit Frist.
    pub async fn wait_timeout(self, timeout: Duration) -> Result<Value, RpcError> {
        tokio::time::timeout(timeout, self.wait())
            .await
            .unwrap_or(Err(RpcError::Timeout(timeout)))
    }
}

/// Client-Seite einer JSON-RPC-Verbindung; günstig klonbar.
#[derive(Clone)]
pub struct RpcClient {
    writer: Writer,
    next_id: Arc<AtomicU64>,
    pending: Pending,
    routed: Arc<std::sync::Mutex<HashSet<u64>>>,
}

impl std::fmt::Debug for RpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcClient")
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl RpcClient {
    /// Startet den Lese-Task. Der Strom endet, wenn stdout des Harness endet; offene
    /// Anfragen schlagen dann mit [`RpcError::Closed`] fehl.
    pub fn spawn(
        stdin: Box<dyn AsyncWrite + Send + Unpin>,
        stdout: Box<dyn AsyncRead + Send + Unpin>,
    ) -> (Self, mpsc::Receiver<Incoming>) {
        let client = Self {
            writer: Arc::new(Mutex::new(Some(stdin))),
            next_id: Arc::new(AtomicU64::new(1)),
            pending: Arc::default(),
            routed: Arc::default(),
        };
        let (tx, rx) = mpsc::channel(4096);
        tokio::spawn(read_loop(
            stdout,
            tx,
            client.pending.clone(),
            client.routed.clone(),
        ));
        (client, rx)
    }

    async fn write(&self, v: &Value) -> Result<(), RpcError> {
        let mut guard = self.writer.lock().await;
        let w = guard.as_mut().ok_or(RpcError::Closed)?;
        let line = format!("{v}\n");
        w.write_all(line.as_bytes())
            .await
            .map_err(|_| RpcError::Closed)?;
        w.flush().await.map_err(|_| RpcError::Closed)
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Anfrage senden und auf die Antwort warten.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.request_detached(method, params).await?.wait().await
    }

    /// Anfrage senden, ohne auf die Antwort zu warten: Kehrt der Aufruf zurück, ist die
    /// Anfrage geschrieben (Reihenfolge auf stdin wie im Aufrufer); die Antwort liefert
    /// [`PendingReply::wait`].
    pub async fn request_detached(
        &self,
        method: &str,
        params: Value,
    ) -> Result<PendingReply, RpcError> {
        let id = self.id();
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        let sent = self
            .write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        if let Err(e) = sent {
            if let Ok(mut p) = self.pending.lock() {
                p.remove(&id);
            }
            return Err(e);
        }
        Ok(PendingReply(rx))
    }

    /// Wie [`Self::request`], mit Frist.
    pub async fn request_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        tokio::time::timeout(timeout, self.request(method, params))
            .await
            .unwrap_or(Err(RpcError::Timeout(timeout)))
    }

    /// Anfrage senden; die Antwort kommt als [`Incoming::Response`] mit der gelieferten ID.
    pub async fn send_request(&self, method: &str, params: Value) -> Result<u64, RpcError> {
        let id = self.id();
        if let Ok(mut r) = self.routed.lock() {
            r.insert(id);
        }
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        Ok(id)
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    /// Antwort auf eine Anfrage des Harness.
    pub async fn respond(&self, id: &Value, result: Value) -> Result<(), RpcError> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await
    }

    pub async fn respond_error(
        &self,
        id: &Value,
        code: i64,
        message: &str,
    ) -> Result<(), RpcError> {
        self.write(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        )
        .await
    }

    /// stdin schließen (protokolleigenes Ende).
    pub async fn close(&self) {
        self.writer.lock().await.take();
    }
}

async fn read_loop(
    stdout: Box<dyn AsyncRead + Send + Unpin>,
    tx: mpsc::Sender<Incoming>,
    pending: Pending,
    routed: Arc<std::sync::Mutex<HashSet<u64>>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let incoming = classify(&line);
        let incoming = match incoming {
            Incoming::Response { id, result, raw } => {
                let is_routed = routed.lock().map(|mut r| r.remove(&id)).unwrap_or(false);
                if is_routed {
                    Incoming::Response { id, result, raw }
                } else {
                    let waiter = pending.lock().ok().and_then(|mut p| p.remove(&id));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(result);
                    }
                    continue;
                }
            }
            other => other,
        };
        if tx.send(incoming).await.is_err() {
            break;
        }
    }
    if let Ok(mut p) = pending.lock() {
        for (_, waiter) in p.drain() {
            let _ = waiter.send(Err(RpcError::Closed));
        }
    }
}

/// Ordnet eine Zeile ein.
pub fn classify(line: &str) -> Incoming {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Incoming::Invalid {
            line: line.to_owned(),
        };
    };
    let raw = RawJson::from_string(line.to_owned()).ok();
    let method = v["method"].as_str().map(str::to_owned);
    let id = v.get("id").filter(|id| !id.is_null()).cloned();
    match (method, id) {
        (Some(method), Some(id)) => Incoming::Request {
            id,
            method,
            params: v.get("params").cloned().unwrap_or(Value::Null),
            raw,
        },
        (Some(method), None) => Incoming::Notification {
            method,
            params: v.get("params").cloned().unwrap_or(Value::Null),
            raw,
        },
        (None, Some(id)) => match id.as_u64() {
            Some(id) => Incoming::Response {
                id,
                result: match v.get("error").filter(|e| !e.is_null()) {
                    Some(e) => Err(RpcError::from_value(e)),
                    None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                },
                raw,
            },
            None => Incoming::Invalid {
                line: line.to_owned(),
            },
        },
        (None, None) => Incoming::Invalid {
            line: line.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_without_jsonrpc_field_are_accepted() {
        // So antwortet `codex app-server` (0.153.2).
        let r = classify(r#"{"id":1,"result":{"userAgent":"x/1.0"}}"#);
        assert!(matches!(
            r,
            Incoming::Response {
                id: 1,
                result: Ok(_),
                ..
            }
        ));
        let e = classify(r#"{"error":{"code":-32600,"message":"Not initialized"},"id":1}"#);
        assert!(matches!(
            e,
            Incoming::Response {
                result: Err(RpcError::Remote { code: -32600, .. }),
                ..
            }
        ));
        let n = classify(r#"{"method":"turn/started","params":{"threadId":"t"}}"#);
        assert!(matches!(n, Incoming::Notification { ref method, .. } if method == "turn/started"));
        let q = classify(
            r#"{"jsonrpc":"2.0","id":"a","method":"session/request_permission","params":{}}"#,
        );
        assert!(matches!(q, Incoming::Request { ref id, .. } if id == "a"));
        assert!(matches!(classify("kein json"), Incoming::Invalid { .. }));
    }

    #[tokio::test]
    async fn awaited_and_routed_responses() {
        let (client_stdin, mut peer_in) = tokio::io::duplex(4096);
        let (mut peer_out, client_stdout) = tokio::io::duplex(4096);
        let (client, mut rx) = RpcClient::spawn(Box::new(client_stdin), Box::new(client_stdout));
        let peer = tokio::spawn(async move {
            let mut lines = BufReader::new(&mut peer_in).lines();
            let first: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(first["jsonrpc"], "2.0");
            let second: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            // Erst eine Notification, dann die Antwort auf die geroutete Anfrage.
            let out = format!(
                "{}\n{}\n{}\n",
                json!({"method": "update", "params": {"n": 1}}),
                json!({"id": second["id"], "result": {"stop": "end"}}),
                json!({"id": first["id"], "result": {"ok": true}}),
            );
            peer_out.write_all(out.as_bytes()).await.unwrap();
        });
        let awaited = client.request("initialize", json!({}));
        let routed_id = {
            // Reihenfolge der Zeilen: erst `initialize`, dann die geroutete Anfrage.
            let c = client.clone();
            let h = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                c.send_request("prompt", json!({})).await.unwrap()
            });
            let result = awaited.await.unwrap();
            assert_eq!(result["ok"], true);
            h.await.unwrap()
        };
        peer.await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(Incoming::Notification { .. })
        ));
        match rx.recv().await {
            Some(Incoming::Response { id, result, .. }) => {
                assert_eq!(id, routed_id);
                assert_eq!(result.unwrap()["stop"], "end");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn pending_requests_fail_when_the_harness_ends() {
        let (client_stdin, _peer_in) = tokio::io::duplex(4096);
        let (peer_out, client_stdout) = tokio::io::duplex(4096);
        let (client, _rx) = RpcClient::spawn(Box::new(client_stdin), Box::new(client_stdout));
        let req = tokio::spawn({
            let c = client.clone();
            async move { c.request("x", json!({})).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(peer_out);
        assert_eq!(req.await.unwrap(), Err(RpcError::Closed));
    }
}
