//! Gemeinsame Registry für eingebaute Adapter und Plugins (PLG-001).
//!
//! Eingebaute Adapter (Crates wie `beton-harness-claude`) und Out-of-Process-Plugins
//! registrieren sich über dieselbe [`AdapterRegistry`]. Außerhalb der Registry
//! unterscheiden sie sich nur über [`AdapterSource`]; die API gibt das als Feld
//! `source: builtin | plugin` aus.
//!
//! Der Typ der Capabilities ist generisch, weil die konkreten Capability-Modelle mit
//! den jeweiligen Traits kommen (HAR-002 für Harnesses, RUN-001 für RunnerProvider).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Art eines Adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Harness,
    RunnerProvider,
    GitProvider,
}

/// Herkunft eines Adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterSource {
    /// Einkompiliertes Crate.
    Builtin,
    /// Out-of-Process-Plugin (PLG-002 ff.).
    Plugin,
}

/// Eindeutige ID eines Adapters innerhalb seiner Art, z. B. `claude` oder `docker`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdapterId(String);

impl AdapterId {
    /// Erzeugt eine ID. Erlaubt sind Kleinbuchstaben, Ziffern, `-` und `:`
    /// (`:` für Namensräume wie `acp:gemini-cli`); die ID muss mit einem Buchstaben beginnen.
    pub fn new(id: impl Into<String>) -> Result<Self, RegistryError> {
        let id = id.into();
        let valid = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == ':');
        if valid {
            Ok(Self(id))
        } else {
            Err(RegistryError::InvalidId(id))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Beschreibung eines registrierten Adapters, wie sie die API ausgibt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterDescriptor<C> {
    pub kind: AdapterKind,
    pub id: AdapterId,
    pub source: AdapterSource,
    pub capabilities: C,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("ungültige Adapter-ID `{0}`")]
    InvalidId(String),
    #[error("Adapter `{id}` ({kind:?}) ist bereits registriert")]
    Duplicate { kind: AdapterKind, id: AdapterId },
}

/// Registry für Adapter einer oder mehrerer Arten.
///
/// `A` ist der Adapter selbst (z. B. `Box<dyn Harness>`), `C` sein Capability-Modell.
pub struct AdapterRegistry<A, C> {
    entries: BTreeMap<(AdapterKind, AdapterId), (AdapterDescriptor<C>, A)>,
}

impl<A, C> Default for AdapterRegistry<A, C> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<A, C> AdapterRegistry<A, C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert einen Adapter. Eine zweite Registrierung mit gleicher Art und ID
    /// wird abgelehnt – ein Plugin kann einen eingebauten Adapter nicht still ersetzen.
    pub fn register(
        &mut self,
        descriptor: AdapterDescriptor<C>,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let key = (descriptor.kind, descriptor.id.clone());
        if self.entries.contains_key(&key) {
            return Err(RegistryError::Duplicate {
                kind: descriptor.kind,
                id: descriptor.id,
            });
        }
        self.entries.insert(key, (descriptor, adapter));
        Ok(())
    }

    pub fn get(&self, kind: AdapterKind, id: &AdapterId) -> Option<(&AdapterDescriptor<C>, &A)> {
        self.entries
            .get(&(kind, id.clone()))
            .map(|(descriptor, adapter)| (descriptor, adapter))
    }

    /// Alle Adapter einer Art, sortiert nach ID.
    pub fn descriptors(&self, kind: AdapterKind) -> impl Iterator<Item = &AdapterDescriptor<C>> {
        self.entries
            .iter()
            .filter(move |((k, _), _)| *k == kind)
            .map(|(_, (descriptor, _))| descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Caps {
        resume: bool,
    }

    fn descriptor(id: &str, source: AdapterSource) -> AdapterDescriptor<Caps> {
        AdapterDescriptor {
            kind: AdapterKind::Harness,
            id: AdapterId::new(id).unwrap(),
            source,
            capabilities: Caps { resume: true },
        }
    }

    #[test]
    fn plg_001_ac1_builtin_and_plugin_share_one_registry() {
        let mut registry: AdapterRegistry<&str, Caps> = AdapterRegistry::new();
        registry
            .register(
                descriptor("claude", AdapterSource::Builtin),
                "builtin-adapter",
            )
            .unwrap();
        registry
            .register(
                descriptor("acp:gemini-cli", AdapterSource::Plugin),
                "plugin-adapter",
            )
            .unwrap();

        let ids: Vec<_> = registry
            .descriptors(AdapterKind::Harness)
            .map(|d| (d.id.as_str(), d.source))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("acp:gemini-cli", AdapterSource::Plugin),
                ("claude", AdapterSource::Builtin)
            ]
        );
        let (_, adapter) = registry
            .get(AdapterKind::Harness, &AdapterId::new("claude").unwrap())
            .unwrap();
        assert_eq!(*adapter, "builtin-adapter");
    }

    #[test]
    fn plg_001_ac1_descriptor_serializes_kind_id_source_capabilities() {
        let json = serde_json::to_value(descriptor("claude", AdapterSource::Builtin)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "harness",
                "id": "claude",
                "source": "builtin",
                "capabilities": { "resume": true }
            })
        );
        let plugin = serde_json::to_value(descriptor("x", AdapterSource::Plugin)).unwrap();
        assert_eq!(plugin["source"], "plugin");
    }

    #[test]
    fn plg_001_ac1_plugin_cannot_replace_builtin() {
        let mut registry: AdapterRegistry<(), Caps> = AdapterRegistry::new();
        registry
            .register(descriptor("claude", AdapterSource::Builtin), ())
            .unwrap();
        let err = registry
            .register(descriptor("claude", AdapterSource::Plugin), ())
            .unwrap_err();
        assert!(matches!(err, RegistryError::Duplicate { .. }));
    }

    #[test]
    fn adapter_id_rejects_invalid_values() {
        for bad in ["", "Claude", "1claude", "claude code", "../x"] {
            assert!(AdapterId::new(bad).is_err(), "{bad:?} sollte ungültig sein");
        }
        for good in ["claude", "acp:gemini-cli", "provider-docker2"] {
            assert!(AdapterId::new(good).is_ok(), "{good:?} sollte gültig sein");
        }
    }
}
