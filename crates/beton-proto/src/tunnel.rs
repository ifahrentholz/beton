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

impl TunnelUp {
    /// Liest eine Nachricht. Wie `serde_json::from_str`, nur dass Events mit `raw` gelingen
    /// (siehe `tagged`): Event-Pushes werden direkt als Struct gelesen.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Push {
            session_id: SessionId,
            epoch: u64,
            batch: Vec<RseqEvent>,
        }
        #[derive(Deserialize)]
        struct Transient {
            session_id: SessionId,
            events: Vec<Event>,
        }
        match crate::tagged::tag(text)?.as_str() {
            "events.push" => {
                let p: Push = serde_json::from_str(text)?;
                Ok(Self::EventsPush {
                    session_id: p.session_id,
                    epoch: p.epoch,
                    batch: p.batch,
                })
            }
            "transient.push" => {
                let p: Transient = serde_json::from_str(text)?;
                Ok(Self::TransientPush {
                    session_id: p.session_id,
                    events: p.events,
                })
            }
            _ => serde_json::from_str(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_015_events_with_raw_survive_the_tunnel() {
        use beton_core::event::{Actor, EventPayload, Notice, RawJson};
        let session: SessionId = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap();
        let mut event = Event::new(
            session,
            0,
            Actor::default(),
            EventPayload::Notice(Notice::default()),
        );
        event.raw =
            Some(RawJson::from_string(r#"{"type":"assistant","b":1,"a":2}"#.into()).unwrap());
        let up = TunnelUp::EventsPush {
            session_id: session,
            epoch: 1,
            batch: vec![RseqEvent { rseq: 1, event }],
        };
        let text = serde_json::to_string(&up).unwrap();
        let back = TunnelUp::from_json(&text).unwrap();
        assert_eq!(back, up);
        // Der Originaltext bleibt byte-genau.
        let TunnelUp::EventsPush { batch, .. } = back else {
            panic!()
        };
        assert_eq!(
            batch[0].event.raw.as_ref().unwrap().get(),
            r#"{"type":"assistant","b":1,"a":2}"#
        );
    }

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
