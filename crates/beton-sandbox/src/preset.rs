//! Eingebaute Presets (SBX-012): `default`, `dev`, `readonly`.
//!
//! Ein Preset ist eine Konfigurationsebene unterhalb aller eigenen Ebenen; eigene Felder
//! überschreiben bzw. ergänzen es nach den üblichen Merge-Regeln (SBX-003).

use crate::spec::{Preset, SandboxSpec};

/// Die Ebene eines Presets.
pub fn layer(preset: Preset) -> SandboxSpec {
    let _ = preset;
    SandboxSpec::default()
}
