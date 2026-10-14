//! Eingebaute Presets (SBX-012): `default`, `dev`, `readonly`.
//!
//! Ein Preset ist eine Konfigurationsebene direkt unter der spezifischsten Ebene, die es setzt:
//! Es überschreibt weniger spezifische Ebenen, die eigenen Felder dieser Ebene überschreiben
//! bzw. ergänzen es nach den üblichen Merge-Regeln (SBX-003). Die Auflösung
//! wendet das Preset selbst an ([`crate::resolve()`]), damit es nie still entfällt.

use crate::spec::{Preset, SandboxSpec, WorkspaceAccess};

/// Paket-Registries und Toolchain-Downloads, die `dev` lesend (GET, HEAD) erlaubt.
pub const DEV_REGISTRIES: [&str; 10] = [
    "registry.npmjs.org",
    "registry.yarnpkg.com",
    "pypi.org",
    "files.pythonhosted.org",
    "index.crates.io",
    "static.crates.io",
    "proxy.golang.org",
    "sum.golang.org",
    "codeload.github.com",
    "objects.githubusercontent.com",
];

/// Toolchain-Caches, die `dev` lesend und schreibend freigibt (fehlende werden übersprungen).
pub const DEV_CACHES: [&str; 4] = [
    "~/.cargo/registry",
    "~/.npm",
    "~/.cache/pip",
    "~/go/pkg/mod",
];

/// Die Ebene eines Presets.
pub fn layer(preset: Preset) -> SandboxSpec {
    match preset {
        Preset::Default => SandboxSpec {
            workspace: Some(WorkspaceAccess::Rw),
            allow_network: Some(true),
            egress_rules: Some(Vec::new()),
            ..SandboxSpec::default()
        },
        Preset::Dev => SandboxSpec {
            workspace: Some(WorkspaceAccess::Rw),
            allow_network: Some(true),
            egress_rules: Some(vec![format!("GET,HEAD {}", DEV_REGISTRIES.join(" "))]),
            write_paths: Some(DEV_CACHES.iter().map(|s| (*s).to_owned()).collect()),
            ..SandboxSpec::default()
        },
        Preset::Readonly => SandboxSpec {
            workspace: Some(WorkspaceAccess::Ro),
            allow_network: Some(false),
            write_paths: Some(Vec::new()),
            ..SandboxSpec::default()
        },
    }
}
