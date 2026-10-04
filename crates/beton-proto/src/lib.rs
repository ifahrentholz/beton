//! Wire-Protokoll von beton: transiente Events, WebSocket-Framing, Versionierung.
//!
//! Das Event-Modell selbst liegt in `beton_core::event` (PROTO-001, PROTO-002).
//! Spec: PROTO-* in `docs/spec/06-data-sync-protocol.md`.

mod tagged;
pub mod transient;
pub mod tunnel;
pub mod ws;
