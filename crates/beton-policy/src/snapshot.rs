//! Set-Snapshots für Explain (POL-027 AC2, DATA-006).
//!
//! Ein effektives Set wird als **Manifest** (Ebene, Name, Herkunft und Version je Set plus
//! `policy_set_hash`) und als **Inhalte** gespeichert. Die Inhalte sind inhaltsadressiert: die
//! Version eines Sets ist der SHA-256 seines gespeicherten Inhalts (YAML-Text bzw.
//! kanonisches JSON der Inline-Regeln). Der Runner legt sie pro Version im Blob-Store der
//! Session ab; Explain baut daraus das Set der damaligen Entscheidung wieder auf.
//!
//! [`rebuild`] prüft jede Version und den Gesamt-Hash: Ein manipulierter oder fehlender
//! Inhalt ist ein Fehler, nie ein stillschweigend anderes Set.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::EffectiveSet;
use crate::load::{LoadedSet, Scope, SourceFormat, inline_set, parse_set};

/// Zusammensetzung eines effektiven Sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub policy_set_hash: String,
    pub sets: Vec<ManifestSet>,
}

/// Ein Set im Manifest; sein Inhalt liegt unter `version` im Blob-Store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSet {
    pub scope: Scope,
    pub name: String,
    /// Datei bzw. Herkunft (`agents/x/agent.yaml`).
    pub origin: String,
    /// `sha256:<hex>` des Inhalts.
    pub version: String,
    pub format: SourceFormat,
}

/// Fehler beim Wiederaufbau.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("Inhalt von Set `{set}` ({version}) fehlt")]
    Missing { set: String, version: String },
    #[error("Inhalt von Set `{set}` passt nicht zur Version {version}")]
    Corrupt { set: String, version: String },
    #[error("Set `{set}` lässt sich nicht laden: {error}")]
    Invalid { set: String, error: String },
    #[error("Policy-Set-Hash weicht ab: erwartet {expected}, berechnet {actual}")]
    HashMismatch { expected: String, actual: String },
}

impl EffectiveSet {
    /// Manifest des effektiven Sets (kanonische Reihenfolge).
    pub fn manifest(&self) -> Manifest {
        Manifest {
            policy_set_hash: self.hash().to_owned(),
            sets: self
                .sets()
                .iter()
                .map(|s| ManifestSet {
                    scope: s.scope,
                    name: s.name.clone(),
                    origin: s.origin.clone(),
                    version: s.version.clone(),
                    format: s.format,
                })
                .collect(),
        }
    }

    /// Inhalte je Version (für den Blob-Store); doppelte Versionen nur einmal.
    pub fn sources(&self) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        for s in self.sets() {
            if !out.iter().any(|(v, _)| *v == s.version) {
                out.push((s.version.clone(), s.source.to_vec()));
            }
        }
        out
    }
}

fn version_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Baut das effektive Set aus Manifest und Inhalten wieder auf. `read` liefert den Inhalt zu
/// einer Version.
pub fn rebuild(
    manifest: &Manifest,
    read: impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<EffectiveSet, SnapshotError> {
    let mut sets = Vec::with_capacity(manifest.sets.len());
    for m in &manifest.sets {
        let bytes = read(&m.version).ok_or_else(|| SnapshotError::Missing {
            set: m.name.clone(),
            version: m.version.clone(),
        })?;
        if version_of(&bytes) != m.version {
            return Err(SnapshotError::Corrupt {
                set: m.name.clone(),
                version: m.version.clone(),
            });
        }
        let invalid = |error: String| SnapshotError::Invalid {
            set: m.name.clone(),
            error,
        };
        let set: LoadedSet = match m.format {
            SourceFormat::Yaml => {
                let text = String::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
                parse_set(m.scope, &m.origin, &text).map_err(|e| invalid(e.to_string()))?
            }
            SourceFormat::Inline => {
                let rules: Vec<serde_json::Value> =
                    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
                inline_set(m.scope, &m.name, &m.origin, &rules)
                    .map_err(|e| invalid(e.to_string()))?
            }
        };
        if set.version != m.version || set.name != m.name {
            return Err(SnapshotError::Corrupt {
                set: m.name.clone(),
                version: m.version.clone(),
            });
        }
        sets.push(Arc::new(set));
    }
    let effective = EffectiveSet::new(sets);
    if effective.hash() != manifest.policy_set_hash {
        return Err(SnapshotError::HashMismatch {
            expected: manifest.policy_set_hash.clone(),
            actual: effective.hash().to_owned(),
        });
    }
    Ok(effective)
}
