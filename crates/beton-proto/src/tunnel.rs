//! Tunnel-Protokoll Runner/Host → Server (PROTO-015).
//!
//! WebSocket mit Subprotokoll `beton.tunnel.v1`; lokal über den Unix-Socket
//! `~/.beton/run/tunnel.sock`. Runner liefern Events ohne `seq` mit eigener Folgenummer
//! `rseq`; der Home-Knoten vergibt `seq`, persistiert und bestätigt per `events.ack`.

use beton_core::event::Event;
use beton_core::id::SessionId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const SUBPROTOCOL: &str = "beton.tunnel.v1";
pub const PROTOCOL: &str = "1.0";

/// Wer sich verbindet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PeerKind {
    Host,
    Runner,
    Node,
}

/// Ein Event mit Runner-Folgenummer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct RseqEvent {
    pub rseq: u64,
    pub event: Event,
}

/// Nachrichten an den Server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum TunnelUp {
    Hello {
        kind: PeerKind,
        version: String,
        protocol: String,
        #[serde(default)]
        harnesses: Vec<String>,
    },
    /// Anmeldung bzw. Wiederaufnahme für eine Session.
    #[serde(rename = "session.bind")]
    SessionBind {
        session_id: SessionId,
        epoch: u64,
        last_acked_rseq: u64,
    },
    #[serde(rename = "events.push")]
    EventsPush {
        session_id: SessionId,
        epoch: u64,
        batch: Vec<RseqEvent>,
    },
    #[serde(rename = "transient.push")]
    TransientPush {
        session_id: SessionId,
        events: Vec<Event>,
    },
    #[serde(rename = "cmd.result")]
    CmdResult {
        cmd_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        problem: Option<Value>,
    },
}

/// Nachrichten an Runner bzw. Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum TunnelDown {
    Welcome {
        server_version: String,
        protocol: String,
    },
    Bound {
        session_id: SessionId,
        epoch: u64,
        head_seq: u64,
        /// Höchste `rseq`, die der Server bereits gespeichert hat.
        acked_rseq: u64,
    },
    #[serde(rename = "events.ack")]
    EventsAck {
        session_id: SessionId,
        upto_rseq: u64,
        /// Vergebene `seq` (erste, letzte); leer, wenn alles schon gespeichert war.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        seq_range: Option<(u64, u64)>,
    },
    #[serde(rename = "cmd.deliver")]
    CmdDeliver {
        cmd_id: String,
        name: String,
        #[serde(default)]
        args: Value,
    },
    /// Runner beenden (z. B. Idle-Timeout, RUN-003 AC2).
    #[serde(rename = "runner.stop")]
    RunnerStop { session_id: SessionId, grace_s: u64 },
    /// Fehler als RFC-9457-Problem, z. B. `stale_epoch`.
    Problem { problem: Value },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_messages_use_dotted_names() {
        let v = serde_json::to_value(TunnelUp::SessionBind {
            session_id: "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap(),
            epoch: 1,
            last_acked_rseq: 0,
        })
        .unwrap();
        assert_eq!(v["t"], "session.bind");
        let ack: TunnelDown = serde_json::from_value(serde_json::json!({
            "t": "events.ack", "session_id": "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C", "upto_rseq": 5, "seq_range": [2, 6]
        }))
        .unwrap();
        assert!(matches!(
            ack,
            TunnelDown::EventsAck {
                upto_rseq: 5,
                seq_range: Some((2, 6)),
                ..
            }
        ));
    }
}
