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
    let valid = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Some("kein gültiger Variablenname");
    }
    if crate::spec::env_denied(name) {
        return Some("gelangt nie in eine Sandbox (SBX-005)");
    }
    if crate::resolve::controlled_env(name) {
        return Some("setzt beton je Stufe selbst");
    }
    let family = crate::masks::builtin_harness_grants()
        .iter()
        .flat_map(|g| g.env_families.clone())
        .any(|f| name.starts_with(f.as_str()));
    if family {
        return Some("gehört zur Stufe 1 eines Harness");
    }
    None
}

/// Einstieg von `bt-exec` bzw. `beton __exec`: `Some(exit)`, wenn dieses Programm als Shim
/// aufgerufen wurde (Name `bt-exec` bzw. `bt-exec-<shell>` oder erstes Argument `__exec`).
pub fn exec_dispatch() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os();
    let argv0 = args.next()?;
    let base = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let rest: Vec<std::ffi::OsString> = args.collect();
    let code = if base == SHIM || base.starts_with("bt-exec-") {
        client::run(&argv0, rest)
    } else if rest.first().map(|a| a.as_os_str()) == Some(std::ffi::OsStr::new(EXEC_ARG)) {
        client::run(&std::ffi::OsString::from(SHIM), rest[1..].to_vec())
    } else {
        return None;
    };
    Some(std::process::ExitCode::from(
        u8::try_from(code).unwrap_or(126),
    ))
}

/// Vergleich in konstanter Zeit (Token).
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn sbx_002_token_comparison_needs_equal_non_empty_input() {
        assert!(super::same(b"abc", b"abc"));
        assert!(!super::same(b"abc", b"abd"));
        assert!(!super::same(b"abc", b"ab"));
        assert!(!super::same(b"", b""));
    }
}
