//! HTTP-Server von beton: REST unter `/v1`, OpenAPI, Fehler nach RFC 9457 und die
//! Sicherheitsschicht des lokalen Modus (WP-04: PROTO-010, PROTO-011, API-001, API-002,
//! AUTH-001 … AUTH-004).
//!
//! Spec: `docs/spec/05-security-identity.md`, `06-data-sync-protocol.md`, `08-clients.md`.
//! WebSocket (PROTO-004 ff.) und Session-Endpunkte folgen mit WP-05 bzw. WP-09.

// `Problem` ist der Fehlertyp der API und bewusst ein vollständiges Objekt; Fehler sind der
// seltene Pfad, ein `Box` brächte nur Umwege.
#![allow(clippy::result_large_err)]

pub mod api;
pub mod api_sessions;
pub mod app;
pub mod commands;
pub mod config;
pub mod extract;
pub mod hub;
pub mod idempotency;
pub mod local_auth;
pub mod problem;
pub mod queue;
pub mod security;
pub mod serve;
pub mod sessions;
pub mod tunnel;
pub mod web;
pub mod ws;

pub use crate::app::openapi;
pub use crate::config::{DEFAULT_PORT, ServerConfig};
pub use crate::problem::{Problem, ProblemCode};
pub use crate::serve::{Daemon, StartError, start, start_with};
