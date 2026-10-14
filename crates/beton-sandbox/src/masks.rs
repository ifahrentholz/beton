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
}

/// Bekannte Vendor-CLIs. *(Annahme, mit SBX-002 pro Adapter zu verifizieren und in die
/// Adapter-Capabilities zu verschieben.)*
pub fn builtin_harness_grants() -> Vec<HarnessGrants> {
    vec![
        HarnessGrants {
            harness: "claude".into(),
            credential_paths: vec!["~/.claude/.credentials.json".into()],
            state_paths: vec!["~/.claude".into(), "~/.claude.json".into()],
            keychain: true,
        },
        HarnessGrants {
            harness: "codex".into(),
            credential_paths: vec!["~/.codex/auth.json".into()],
            state_paths: vec!["~/.codex".into()],
            keychain: false,
        },
    ]
}

/// Die eingebauten Grants eines Harness (`claude`, `codex`).
pub fn harness_grants(harness: &str) -> Option<HarnessGrants> {
    builtin_harness_grants()
        .into_iter()
        .find(|g| g.harness == harness)
}
