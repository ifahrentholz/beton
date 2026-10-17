//! Terminals, die beton für einen Harness ausführt (ACP `terminal/*`, #326, HAR-007): Das
//! Kommando läuft am PTY in Stufe 2 der Session (SBX-017) und erscheint als Terminal mit
//! `kind: tool` (SES-019). Den Host stellt der Runner.

use std::path::PathBuf;

use async_trait::async_trait;

/// Ein Kommando des Harness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolTerminal {
    pub command: String,
    pub args: Vec<String>,
    /// Zusätzliche Variablen; nur, was auch `bt-exec` an Stufe 2 weitergeben darf.
    pub env: Vec<(String, String)>,
    /// Arbeitsverzeichnis (absolut, innerhalb des Workspace); Default: Workspace.
    pub cwd: Option<PathBuf>,
    /// Rohausgabe höchstens so viele Bytes (älteste fallen weg).
    pub output_limit: Option<usize>,
}

/// Ende des Prozesses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalExit {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

/// Bisherige Ausgabe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalOutput {
    pub output: String,
    pub truncated: bool,
    pub exit: Option<TerminalExit>,
}

/// Terminals in Stufe 2 für einen Harness. Fehler sind Texte für den Harness (ohne Details
/// aus der Sandbox, die er nicht sehen soll).
#[async_trait]
pub trait TerminalHost: Send + Sync {
    /// Startet das Kommando; liefert die `terminal_id`.
    async fn create(&self, req: ToolTerminal) -> Result<String, String>;
    async fn output(&self, id: &str) -> Result<TerminalOutput, String>;
    async fn wait_for_exit(&self, id: &str) -> Result<TerminalExit, String>;
    /// Prozess beenden; das Terminal bleibt für `output` bekannt.
    async fn kill(&self, id: &str) -> Result<(), String>;
    /// Prozess beenden und das Terminal freigeben.
    async fn release(&self, id: &str) -> Result<(), String>;
}
