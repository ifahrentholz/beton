//! Agents als validierte YAML-Definitionen (AGT-001, AGT-002, AGT-003, AGT-013).
//!
//! - [`spec`]: Agent-Format v1, Quelle des JSON-Schemas `schemas/v1/agent.schema.json`.
//! - [`load`]: `agent.yaml` lesen, Schemafehler mit Datei, Zeile und Spalte.
//! - [`validate`]: semantische Prüfung und der aufgelöste Agent für `beton agent show`.
//! - [`resolve`]: Agent-Refs, Suchpfad (Projekt → User → Built-ins) und Liste.
//! - [`dir`]: Agent-Verzeichnisse, eingebettete Built-ins (`agents/` im Repo), Inhalts-Hash.
//! - [`scaffold`]: Gerüst für `beton agent new`.
//! - [`snapshot`]: Agent-Snapshot pro Session mit Inhalts-Hash (AGT-004).
//! - [`instructions`]: Instructions samt Projektdateien zusammensetzen (AGT-005).
//! - [`params`], [`template`]: Parameterwerte prüfen und in Templates einsetzen (AGT-010).
//!
//! Spec: `docs/spec/02-agents.md`.

pub mod diag;
pub mod dir;
pub mod instructions;
pub mod load;
pub mod params;
pub mod resolve;
pub mod scaffold;
pub mod snapshot;
pub mod spec;
pub mod template;
pub mod validate;
pub mod yaml;

pub use diag::{Diagnostic, Severity};
pub use dir::{AgentDir, Builtins};
pub use resolve::{AgentRef, ListEntry, Located, ResolveError, SearchPath, Source};
pub use snapshot::{AgentSnapshot, Overrides};
pub use spec::{AgentSpec, schema_json};
pub use validate::{HarnessCatalog, Report, ResolvedAgent, Validator};
