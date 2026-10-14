//! Default-Masken (SBX-004) und die Credential-/State-Pfade der Vendor-CLIs.

/// Credential- und Konfigurationspfade, die in Stufe 1 und 2 immer maskiert sind, auch
/// innerhalb freigegebener Wurzeln (z. B. `read_paths: ["~"]`). Ein `*` im letzten Segment
/// ist ein Namensmuster.
pub const DEFAULT_MASKS: [&str; 17] = [
    "~/.ssh",
    "~/.aws",
    "~/.azure",
    "~/.config/gcloud",
    "~/.config/gh",
    "~/.config/glab-cli",
    "~/.netrc",
    "~/.git-credentials",
    "~/.docker/config.json",
    "~/.kube",
    "~/.gnupg",
    "~/.npmrc",
    "~/.pypirc",
    "~/.cargo/credentials*",
    "~/.beton",
    "~/Library/Keychains",
    // Platzhalter für `$XDG_RUNTIME_DIR` (aus dem Environment des Daemons).
    "$XDG_RUNTIME_DIR",
];

/// Die Keychain-Dateien (macOS); in Stufe 1 mit `keychain: true` nicht maskiert.
pub const KEYCHAINS: &str = "~/Library/Keychains";

/// Workspace-Masken, wenn keine Ebene `masks` setzt.
pub const DEFAULT_WORKSPACE_MASKS: [&str; 2] = [".env", ".env.*"];

/// Was ein Harness-Adapter für Stufe 1 deklariert (SBX-002, HAR-002): eigene
/// Credential-Dateien (lesen), State-Verzeichnisse (lesen/schreiben) und Keychain-Bedarf. In
/// Stufe 2 sind die Pfade **aller** Adapter maskiert.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarnessGrants {
    pub harness: String,
    pub credential_paths: Vec<String>,
    pub state_paths: Vec<String>,
    /// Mach-Services des Keychains (macOS) bzw. Secret Service (Linux).
    pub keychain: bool,
    /// Präfixe der Environment-Variablen des Harness (z. B. `CLAUDE`, `ANTHROPIC_`), die
    /// Stufe 1 aus dem Daemon übernimmt; Stufe 2 nie.
    pub env_families: Vec<String>,
    /// Vendor-Hosts für den Passthrough der Stufe 1 (PRX-005).
    pub passthrough: Vec<String>,
}

/// Bekannte Vendor-CLIs, verifiziert gegen claude 2.1.296 und codex-cli 0.161.0 (WP-45, ohne
/// Dateien der CLIs zu lesen: Doku, `--help`, Verhalten, Netzverbindungen über einen lokalen
/// CONNECT-Mitschnitt).
///
/// - Claude Code: Login unter macOS im Keychain (`Claude Code-credentials`), unter Linux in
///   `~/.claude/.credentials.json`; State in `~/.claude` und `~/.claude.json`. Modell-API
///   `api.anthropic.com`, Token-Erneuerung `platform.claude.com`, Konto `claude.ai`,
///   claude.ai-Connectoren `mcp-proxy.anthropic.com` (Telemetrie und Auto-Update nicht).
/// - Codex: `cli_auth_credentials_store = "file"` (Default) → `~/.codex/auth.json`; ChatGPT-Login
///   über `chatgpt.com` und `ab.chatgpt.com`, Token-Erneuerung `auth.openai.com`, API-Key
///   `api.openai.com`.
pub fn builtin_harness_grants() -> Vec<HarnessGrants> {
    let list = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    vec![
        HarnessGrants {
            harness: "claude".into(),
            credential_paths: list(&["~/.claude/.credentials.json"]),
            state_paths: list(&["~/.claude", "~/.claude.json"]),
            keychain: cfg!(target_os = "macos"),
            env_families: list(&["CLAUDE", "ANTHROPIC_"]),
            passthrough: list(&[
                "api.anthropic.com",
                "platform.claude.com",
                "claude.ai",
                "mcp-proxy.anthropic.com",
            ]),
        },
        HarnessGrants {
            harness: "codex".into(),
            credential_paths: list(&["~/.codex/auth.json"]),
            state_paths: list(&["~/.codex"]),
            keychain: false,
            env_families: list(&["CODEX_", "OPENAI_"]),
            passthrough: list(&[
                "chatgpt.com",
                "ab.chatgpt.com",
                "auth.openai.com",
                "api.openai.com",
            ]),
        },
    ]
}

/// Die eingebauten Grants eines Harness (`claude`, `codex`).
pub fn harness_grants(harness: &str) -> Option<HarnessGrants> {
    builtin_harness_grants()
        .into_iter()
        .find(|g| g.harness == harness)
}
