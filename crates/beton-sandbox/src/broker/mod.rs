//! Exec-Broker (SBX-002): Stufe-0-Dienst im Runner, der Kommandos aus Stufe 1 in einer frischen
//! Stufe-2-Sandbox startet, und sein Client `bt-exec`.
//!
//! ```text
//!  Stufe 1 (Sandbox)                 Stufe 0 (Runner)              Stufe 2 (frische Sandbox)
//!  claude ─Bash─► bt-exec ─unix─►  Broker ──Sandbox::spawn──►  /bin/zsh -c "npm test"
//!                   ▲   Request {argv, cwd, env_overrides}, stdio per SCM_RIGHTS   │
//!                   └── started {pid} · exited {code, signal} ◄────────────────────┘
//! ```
//!
//! Der Broker prüft jede Anfrage gegen die Stufe-2-Policy (fail closed): Token der Session,
//! `cwd` innerhalb der Wurzeln der Stufe 2, keine gesperrten oder von beton gesetzten
//! Variablen, kein PTY (SBX-017, M3). Erst dann startet er das Kommando; Exit-Code und Signal
//! gehen zurück, `bt-exec` reicht Signale (SIGINT, SIGTERM, SIGHUP, SIGQUIT) weiter. Bricht die
//! Verbindung ab, beendet der Broker den Prozessbaum.

pub mod client;
pub mod proto;

#[cfg(unix)]
mod fdpass;
#[cfg(unix)]
pub mod server;

#[cfg(unix)]
pub use server::Broker;

/// Socket des Brokers im Environment der Stufe 1.
pub const ENV_SOCKET: &str = "BETON_EXEC_SOCKET";
/// Token der Session im Environment der Stufe 1.
pub const ENV_TOKEN: &str = "BETON_EXEC_TOKEN";
/// Name des Shims; `bt-exec-zsh`, `bt-exec-bash` und `bt-exec-sh` starten die jeweilige Shell
/// in Stufe 2 (Claude Code: `CLAUDE_CODE_SHELL`).
pub const SHIM: &str = "bt-exec";
/// `beton __exec <programm> [args…]` entspricht `bt-exec`.
pub const EXEC_ARG: &str = "__exec";

/// Warum der Broker eine Variable aus `env_overrides` ablehnt (bzw. `bt-exec` sie nicht
/// weitergibt): gesperrt (SBX-005), von beton je Stufe gesetzt, Familie eines Harness oder
/// kein gültiger Name. `None`: erlaubt.
pub fn override_rejected(name: &str) -> Option<&'static str> {
    let _ = name;
    None
}

/// Einstieg von `bt-exec` bzw. `beton __exec`: `Some(exit)`, wenn dieses Programm als Shim
/// aufgerufen wurde.
pub fn exec_dispatch() -> Option<std::process::ExitCode> {
    None
}
