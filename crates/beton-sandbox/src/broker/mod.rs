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
pub mod git_shim;
pub mod proto;

#[cfg(unix)]
mod fdpass;
#[cfg(unix)]
pub mod server;

#[cfg(unix)]
pub use server::{Broker, GitCheck};

/// Socket des Brokers im Environment der Stufe 1.
pub const ENV_SOCKET: &str = "BETON_EXEC_SOCKET";
/// Token der Session im Environment der Stufe 1.
pub const ENV_TOKEN: &str = "BETON_EXEC_TOKEN";
/// Name des Shims; `bt-exec-zsh`, `bt-exec-bash` und `bt-exec-sh` starten die jeweilige Shell
/// in Stufe 2 (Claude Code: `CLAUDE_CODE_SHELL`).
pub const SHIM: &str = "bt-exec";
/// `beton __exec <programm> [args…]` entspricht `bt-exec`.
pub const EXEC_ARG: &str = "__exec";

/// Namen der Variablen, die `bt-exec` zusätzlich weitergibt (im Environment der Stufe 1, durch
/// Komma getrennt, nur Namen).
pub const ENV_FORWARD: &str = "BETON_EXEC_ENV";

/// Basis- und Werkzeug-Variablen, die `bt-exec` von Stufe 1 an Stufe 2 weitergibt (#259):
/// Shell- und Terminal-Zustand, Editoren, Git-Identität (keine Credential-Helfer). Dazu kommen
/// die Locale-Kategorien ([`crate::resolve::LOCALE_ENV`]) und die konfigurierten Namen der
/// Stufe 2 ([`ENV_FORWARD`]).
pub const TOOL_ENV: [&str; 34] = [
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "TZ",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "COLUMNS",
    "LINES",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "CLICOLOR",
    "EDITOR",
    "VISUAL",
    "PAGER",
    "LESS",
    "CI",
    "GIT_EDITOR",
    "GIT_PAGER",
    "GIT_SEQUENCE_EDITOR",
    "GIT_TERMINAL_PROMPT",
    "GIT_MERGE_AUTOEDIT",
    "GIT_OPTIONAL_LOCKS",
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_AUTHOR_DATE",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_COMMITTER_DATE",
    "GIT_FLUSH",
];

/// Warum der Broker eine Variable aus `env_overrides` ablehnt (bzw. `bt-exec` sie nicht
/// weitergibt). Allowlist (SBX-005, #259): erlaubt sind nur [`TOOL_ENV`], die Locale-Kategorien und die
/// konfigurierten Namen der Stufe 2 (`configured`) – nie gesperrte (SBX-005), von beton je
/// Stufe gesetzte oder Variablen einer Harness-Familie. `None`: erlaubt.
pub fn override_rejected(name: &str, configured: &[String]) -> Option<&'static str> {
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
    let allowed = TOOL_ENV.contains(&name)
        || crate::resolve::LOCALE_ENV.contains(&name)
        || configured.iter().any(|c| c == name);
    if !allowed {
        return Some(
            "nicht in der Allowlist (Basis-, Werkzeug- und konfigurierte Variablen der Stufe 2)",
        );
    }
    None
}

/// Nur die absoluten Einträge eines `PATH` (`bt-exec` vor der Weitergabe, SBX-002).
pub fn absolute_path(path: &str) -> String {
    path.split(':')
        .filter(|d| d.starts_with('/'))
        .collect::<Vec<_>>()
        .join(":")
}

/// Besteht ein `PATH` nur aus absoluten Einträgen? Ein relativer oder leerer Eintrag hinge
/// vom Arbeitsverzeichnis ab, also vom Workspace (SBX-002).
pub fn path_ok(path: &str) -> bool {
    !path.is_empty() && path.split(':').all(|d| d.starts_with('/'))
}

/// Konfigurierte Namen der Stufe 2, die Stufe 1 überschreiben darf: das Environment der
/// Policy ohne von beton gesetzte, gesperrte, Harness- und Credential-Variablen.
pub fn forward_names(policy: &crate::ResolvedPolicy) -> Vec<String> {
    policy
        .env
        .keys()
        .filter(|k| !policy.credential_env.contains(k))
        .filter(|k| {
            !TOOL_ENV.contains(&k.as_str()) && !crate::resolve::LOCALE_ENV.contains(&k.as_str())
        })
        .filter(|k| override_rejected(k, std::slice::from_ref(*k)).is_none())
        .cloned()
        .collect()
}

/// Einstieg von `bt-exec` bzw. `beton __exec` und des Git-Shims: `Some(exit)`, wenn dieses
/// Programm als Shim aufgerufen wurde (Name `bt-exec` bzw. `bt-exec-<shell>`, erstes Argument
/// `__exec` oder Name `git`, SBX-003).
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
    } else if base == git_shim::NAME {
        git_shim::run(&argv0, rest)
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
