//! Wire-Protokoll von beton: transiente Events, WebSocket-Framing, Binärkanäle, Versionierung.
//!
//! Das Event-Modell selbst liegt in `beton_core::event` (PROTO-001, PROTO-002).
//! Spec: PROTO-* in `docs/spec/06-data-sync-protocol.md`.

pub mod channel;
mod tagged;
pub mod transient;
pub mod tunnel;
pub mod ws;
