//! MCP-Injektion und System-Tools (HAR-009, AGT-006, AGT-007, AGT-008).
//!
//! - [`config`]: MCP-Server aus User-, Projekt- und Agent-Ebene (`mcp.yaml`, `tools.mcp`).
//! - [`skills`]: Skills im `SKILL.md`-Format: Discovery, Auswahl, `skill_load`/`skill_read_file`.
//! - [`system`]: der eingebaute MCP-Server `beton` mit den System-Tools.
//! - [`hub`]: Relay-Hub im Runner mit session-gebundenem Token; leitet an `beton` oder an
//!   konfigurierte stdio-/HTTP-Server weiter und filtert deren Tools laut `allow`.
//! - [`relay`]: das stdio-Relay `beton mcp serve|proxy`, das der Harness startet.
//! - [`plan`]: was eine Session bekommt (Server, System-Tools, Skills, Sub-Agents).
//!
//! Spec: `docs/spec/01-harnesses.md` (HAR-009), `docs/spec/02-agents.md` (AGT-006 bis AGT-008).

pub mod config;
mod http;
pub mod hub;
pub mod plan;
pub mod protocol;
pub mod relay;
pub mod skills;
pub mod system;
