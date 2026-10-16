//! Tunnel-Protokoll Runner/Host → Server (PROTO-015).
//!
//! WebSocket mit Subprotokoll `beton.tunnel.v1`; lokal über den Unix-Socket
//! `~/.beton/run/tunnel.sock`. Runner liefern Events ohne `seq` mit eigener Folgenummer
//! `rseq`; der Home-Knoten vergibt `seq`, persistiert und bestätigt per `events.ack`.

use beton_core::event::{Actor, Event};
use beton_core::id::SessionId;

use crate::channel::ChannelKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const SUBPROTOCOL: &str = "beton.tunnel.v1";
/// `1.1`: Binärkanäle (`channel.*`, Binärframes, PROTO-007) und `actor` in `cmd.deliver`.
pub const PROTOCOL: &str = "1.1";

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
        /// Vorab-Verbindung vor dem Start des Harness (Policy `session_start`, #218): nur
        /// System-Calls und Events, keine Kommandos; der Server wartet auf die Hauptschleife.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        prestart: bool,
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
    /// System-Tool, das der Server ausführt, z. B. `session.spawn` (AGT-007). Gilt immer für
    /// die gebundene Session; der Server leitet Arbeitsverzeichnis und Owner daraus ab.
    #[serde(rename = "system.call")]
    SystemCall {
        call_id: String,
        tool: String,
        #[serde(default)]
        args: Value,
    },
    /// Kanal des Runners anmelden (PROTO-007), z. B. das PTY eines Terminals. Nach jedem
    /// (Wieder-)Verbinden meldet der Runner alle offenen Kanäle erneut; der Server antwortet
    /// mit Kredit (`channel.credit`), erst dann sendet der Runner Frames.
    #[serde(rename = "channel.open")]
    ChannelOpen {
        channel_id: u32,
        kind: ChannelKind,
        #[serde(rename = "ref")]
        #[ts(rename = "ref")]
        target: String,
    },
    /// Kanal des Runners beendet.
    #[serde(rename = "channel.close")]
    ChannelClose {
        channel_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reason: Option<String>,
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

/// Nachrichten an Runner bzw. Host. `Debug` zeigt keine Ergebnisse von System-Calls: Sie
/// können Credential-Werte enthalten (`credential.resolve`, PRX-006).
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
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
        /// Wer das Kommando ausgelöst hat (z. B. für `terminal.opened` bzw. `fs.changed` mit
        /// `actor: user`, SES-019, #93); fehlt bei älteren Servern.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        actor: Option<Actor>,
    },
    /// Kredit für Frames des Runners auf einem Kanal (Bytes, PROTO-007).
    #[serde(rename = "channel.credit")]
    ChannelCredit { channel_id: u32, bytes: u64 },
    /// Ein Keyframe anfordern (neuer Zuschauer): Der Runner schickt den vollständigen Zustand
    /// (Terminal: Scrollback und Bildschirm) als Frames mit `KEYFRAME`, das letzte mit `FIN`.
    #[serde(rename = "channel.keyframe")]
    ChannelKeyframe { channel_id: u32 },
    /// Ergebnis eines `system.call`: `result` oder `problem` (`{code, detail}`).
    #[serde(rename = "system.result")]
    SystemResult {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        problem: Option<Value>,
    },
    /// Runner beenden (z. B. Idle-Timeout, RUN-003 AC2).
    #[serde(rename = "runner.stop")]
    RunnerStop { session_id: SessionId, grace_s: u64 },
    /// Fehler als RFC-9457-Problem, z. B. `stale_epoch`.
    Problem { problem: Value },
}

impl std::fmt::Debug for TunnelDown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SystemResult {
                call_id, problem, ..
            } => f
                .debug_struct("SystemResult")
                .field("call_id", call_id)
                .field("result", &"***")
                .field("problem", problem)
                .finish(),
            other => f.write_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
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
            prestart: false,
        })
        .unwrap();
        assert_eq!(v["t"], "session.bind");
        // Ohne `prestart` (ältere Runner) ist es die Verbindung der Hauptschleife (#218).
        assert!(v.get("prestart").is_none());
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

    #[test]
    fn proto_007_tunnel_channel_messages() {
        let up = serde_json::to_value(TunnelUp::ChannelOpen {
            channel_id: 1,
            kind: ChannelKind::Terminal,
            target: "term_1".into(),
        })
        .unwrap();
        assert_eq!(up["t"], "channel.open");
        assert_eq!(up["ref"], "term_1");
        let down: TunnelDown = serde_json::from_value(
            serde_json::json!({"t": "channel.credit", "channel_id": 1, "bytes": 65536}),
        )
        .unwrap();
        assert!(matches!(
            down,
            TunnelDown::ChannelCredit {
                channel_id: 1,
                bytes: 65536
            }
        ));
        // `actor` fehlt bei älteren Servern.
        let cmd: TunnelDown = serde_json::from_value(
            serde_json::json!({"t": "cmd.deliver", "cmd_id": "c1", "name": "terminal.open"}),
        )
        .unwrap();
        assert!(matches!(cmd, TunnelDown::CmdDeliver { actor: None, .. }));
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn prx_006_system_results_never_show_credential_values_in_debug() {
        let down = TunnelDown::SystemResult {
            call_id: "sys_1".into(),
            result: Some(serde_json::json!({ "value_b64": "RklYVFVSRS1TRUNSRVQ=", "ttl_s": 60 })),
            problem: None,
        };
        let text = format!("{down:?}");
        assert!(text.contains("sys_1"), "{text}");
        assert!(!text.contains("RklYVFVSRS1TRUNSRVQ="), "{text}");
    }
}
