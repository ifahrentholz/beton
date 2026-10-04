//! WebSocket-Protokoll `beton.v1` (PROTO-004 … PROTO-009).
//!
//! Textframes sind JSON-Steuernachrichten mit Feld `t`. Binärframes (Kanäle, PROTO-007)
//! folgen in M3; das Framing lässt Platz dafür.

use std::time::Duration;

use beton_core::event::Event;
use beton_core::id::SessionId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// WebSocket-Subprotokoll (Major).
pub const SUBPROTOCOL: &str = "beton.v1";
/// Major-Version des Protokolls.
pub const PROTOCOL_MAJOR: u32 = 1;
/// Aktuelle Minor-Version; unterstützt wird zusätzlich die vorherige.
pub const PROTOCOL_MINOR: u32 = 0;

/// Close-Codes (siehe Spec, WebSocket-Protokoll).
pub mod close {
    pub const PROTOCOL: u16 = 4400;
    pub const UNAUTHENTICATED: u16 = 4401;
    pub const FORBIDDEN: u16 = 4403;
    pub const UNKNOWN_SESSION: u16 = 4404;
    pub const HEARTBEAT_TIMEOUT: u16 = 4408;
    pub const RATE_LIMIT: u16 = 4429;
    pub const INTERNAL: u16 = 4500;
    pub const SHUTTING_DOWN: u16 = 4503;
}

/// Grenzen für Batches und eingehende Kommandos (PROTO-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct SessionLimits {
    pub max_batch_events: u32,
    pub max_batch_bytes: u32,
    pub max_commands_per_s: u32,
    pub max_text_frame_bytes: u32,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_batch_events: 64,
            max_batch_bytes: 256 * 1024,
            max_commands_per_s: 100,
            max_text_frame_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ClientInfo {
    /// z. B. `web`, `desktop`, `cli`, `sdk-ts`, `sdk-rust`.
    pub kind: String,
    pub version: String,
}

/// Nachrichten vom Client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        /// `1.<minor>`
        protocol: String,
        client: ClientInfo,
    },
    /// Events ab `from_seq` (exklusiv), danach live (PROTO-005).
    Attach {
        id: String,
        session_id: SessionId,
        from_seq: u64,
        /// Auch transiente Events (Deltas, Presence) senden.
        #[serde(default = "yes")]
        #[ts(optional, as = "Option<bool>")]
        transient: bool,
        /// Nur die letzten N Events (plus `has_more`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        tail: Option<u32>,
    },
    Detach {
        id: String,
        session_id: SessionId,
    },
    /// Aktion mit REST-Zwilling (PROTO-006).
    Cmd {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        session_id: Option<SessionId>,
        name: String,
        #[serde(default)]
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        idempotency_key: Option<String>,
    },
}

fn yes() -> bool {
    true
}

/// Nachrichten vom Server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        protocol: String,
        server_version: String,
        session_limits: SessionLimits,
    },
    Events {
        session_id: SessionId,
        events: Vec<Event>,
        /// Nur bei `tail`: es gibt ältere Events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        has_more: Option<bool>,
    },
    /// Replay abgeschlossen; ab jetzt kommen Live-Events.
    Live {
        session_id: SessionId,
        head_seq: u64,
    },
    Ack {
        id: String,
        result: Value,
    },
    /// Fehler als RFC-9457-Problem (PROTO-011).
    Nack {
        id: String,
        problem: Value,
    },
    /// Ausgangswarteschlange übergelaufen; Client attached neu ab `resume_from` (PROTO-008).
    Overflow {
        session_id: SessionId,
        resume_from: u64,
    },
}

impl ServerMsg {
    /// Liest eine Nachricht. Wie `serde_json::from_str`, nur dass Events mit `raw` gelingen
    /// (intern getaggte Enums puffern, daraus lässt sich `RawValue` nicht lesen).
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Batch {
            session_id: SessionId,
            events: Vec<Event>,
            #[serde(default)]
            has_more: Option<bool>,
        }
        if crate::tagged::tag(text)? == "events" {
            let b: Batch = serde_json::from_str(text)?;
            return Ok(Self::Events {
                session_id: b.session_id,
                events: b.events,
                has_more: b.has_more,
            });
        }
        serde_json::from_str(text)
    }
}

/// Ergebnis der Versionsaushandlung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supported {
    pub min: String,
    pub max: String,
}

fn parse_version(v: &str) -> Option<(u32, u32)> {
    let (major, minor) = v.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Versionsaushandlung (PROTO-004): `min(client, server)`, sofern das ≥ `max − 1` ist.
pub fn negotiate(client: &str, server_minor: u32) -> Result<String, Supported> {
    let supported = Supported {
        min: format!("{PROTOCOL_MAJOR}.{}", server_minor.saturating_sub(1)),
        max: format!("{PROTOCOL_MAJOR}.{server_minor}"),
    };
    let Some((major, client_minor)) = parse_version(client) else {
        return Err(supported);
    };
    if major != PROTOCOL_MAJOR {
        return Err(supported);
    }
    let lo = client_minor.min(server_minor);
    let hi = client_minor.max(server_minor);
    if lo + 1 >= hi {
        Ok(format!("{PROTOCOL_MAJOR}.{lo}"))
    } else {
        Err(supported)
    }
}

/// Reconnect-Backoff (PROTO-009): 0,5 s → 30 s exponentiell, ±20 % Jitter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub const BASE: Duration = Duration::from_millis(500);
    pub const MAX: Duration = Duration::from_secs(30);

    /// Wartezeit für den nächsten Versuch; `jitter` ∈ [-1, 1] skaliert ±20 %.
    pub fn next_with(&mut self, jitter: f64) -> Duration {
        let base = Self::BASE.as_secs_f64() * 2f64.powi(self.attempt.min(16) as i32);
        let capped = base.min(Self::MAX.as_secs_f64());
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_secs_f64((capped * (1.0 + 0.2 * jitter.clamp(-1.0, 1.0))).max(0.0))
    }

    /// Mit Zufalls-Jitter.
    pub fn next_delay(&mut self) -> Duration {
        self.next_with(fastrand::f64() * 2.0 - 1.0)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn proto_005_event_batches_with_raw_can_be_read() {
        use beton_core::event::{Actor, EventPayload, Notice, RawJson};
        let session: SessionId = "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap();
        let mut event = Event::new(
            session,
            1,
            Actor::default(),
            EventPayload::Notice(Notice::default()),
        );
        event.raw = Some(RawJson::from_string(r#"{"z":1,"a":[2]}"#.into()).unwrap());
        let msg = ServerMsg::Events {
            session_id: session,
            events: vec![event],
            has_more: None,
        };
        let text = serde_json::to_string(&msg).unwrap();
        assert!(
            serde_json::from_str::<ServerMsg>(&text).is_err(),
            "serde allein scheitert"
        );
        assert_eq!(ServerMsg::from_json(&text).unwrap(), msg);
        let live = r#"{"t":"live","session_id":"ses_01JB8Y2D0M3K4J5H6G7F8E9D0C","head_seq":3}"#;
        assert!(matches!(
            ServerMsg::from_json(live).unwrap(),
            ServerMsg::Live { head_seq: 3, .. }
        ));
    }

    use super::*;

    #[test]
    fn proto_004_ac1_version_negotiation() {
        assert_eq!(negotiate("1.4", 3), Ok("1.3".into()));
        assert_eq!(negotiate("1.3", 4), Ok("1.3".into()));
        assert_eq!(negotiate("1.4", 4), Ok("1.4".into()));
        assert_eq!(
            negotiate("1.2", 4),
            Err(Supported {
                min: "1.3".into(),
                max: "1.4".into()
            })
        );
        assert!(negotiate("2.0", 0).is_err());
        assert!(negotiate("kaputt", 0).is_err());
    }

    #[test]
    fn messages_use_t_tag() {
        let m: ClientMsg = serde_json::from_value(serde_json::json!({
            "t": "attach", "id": "r1", "session_id": "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C", "from_seq": 120
        }))
        .unwrap();
        assert!(matches!(
            m,
            ClientMsg::Attach {
                transient: true,
                tail: None,
                ..
            }
        ));
        let s = serde_json::to_value(ServerMsg::Live {
            session_id: "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap(),
            head_seq: 184,
        })
        .unwrap();
        assert_eq!(s["t"], "live");
    }

    #[test]
    fn proto_009_backoff_grows_to_30s_with_jitter() {
        let mut b = Backoff::default();
        assert_eq!(b.next_with(0.0), Duration::from_millis(500));
        assert_eq!(b.next_with(0.0), Duration::from_secs(1));
        for _ in 0..20 {
            b.next_with(0.0);
        }
        assert_eq!(b.next_with(0.0), Duration::from_secs(30));
        assert_eq!(b.next_with(1.0), Duration::from_secs(36));
        assert_eq!(b.next_with(-1.0), Duration::from_secs(24));
        b.reset();
        assert_eq!(b.next_with(0.0), Duration::from_millis(500));
    }
}
