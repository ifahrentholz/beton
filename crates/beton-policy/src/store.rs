//! Ladeorte und Hot-Reload (POL-001).
//!
//! Der Store kennt die Policy-Verzeichnisse je Ebene (User `~/.beton/policies/`, Projekt
//! `.beton/policies/`) und Inline-Sets (Agent-Ebene, WP-49). [`PolicyStore::refresh`] liest
//! die Verzeichnisse neu und vergleicht den Inhalts-Hash jeder Datei:
//!
//! - geänderte, gültige Datei → neue Version, `policy.set_changed`
//! - fehlerhafte Datei → letzte gültige Version bleibt aktiv, `policy.set_invalid` mit Datei,
//!   Zeile und Fehler (AC3)
//! - gelöschte Datei → Set entfällt, `policy.set_changed` mit Version `deleted`
//!
//! Der Runner ruft `refresh` vor jeder Auswertung auf (WP-35). Das kostet einen
//! Verzeichnis-Scan und das Hashen weniger kleiner Dateien und garantiert, dass die nächste
//! Auswertung die neue Version nutzt (AC2) – unabhängig von der Latenz eines
//! Dateisystem-Watchers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beton_core::event::{PolicySetChanged, PolicySetInvalid};
use sha2::{Digest, Sha256};

use crate::engine::EffectiveSet;
use crate::load::{LoadError, LoadedSet, MAX_FILE_BYTES, Scope, parse_set};

/// Ereignis eines Reloads (wird in alle betroffenen aktiven Sessions geschrieben).
#[derive(Debug, Clone, PartialEq)]
pub enum StoreEvent {
    Changed(PolicySetChanged),
    Invalid(PolicySetInvalid),
}

#[derive(Debug, Clone)]
struct FileEntry {
    scope: Scope,
    /// Hash des zuletzt gelesenen Inhalts (gültig oder nicht).
    seen: [u8; 32],
    /// Letzte gültige Version.
    valid: Option<Arc<LoadedSet>>,
    /// Ist der aktuelle Inhalt fehlerhaft?
    invalid: bool,
    /// Letzter Ladefehler (für Dateien ohne gültige Fassung, #182).
    error: Option<PolicySetInvalid>,
}

/// Policy-Sets aller lokalen Ebenen mit Hot-Reload. Eine Kopie erlaubt eine Auswertung mit
/// dem aktuellen Stand der Dateien, ohne Ereignisse des Originals zu verbrauchen (POL-027).
#[derive(Debug, Default, Clone)]
pub struct PolicyStore {
    dirs: Vec<(Scope, PathBuf)>,
    files: BTreeMap<PathBuf, FileEntry>,
    inline: Vec<Arc<LoadedSet>>,
    effective: Option<Arc<EffectiveSet>>,
}

/// `~/.beton/policies` bzw. `<repo>/.beton/policies` zu einem beton-Verzeichnis.
pub fn policies_dir(beton_dir: &Path) -> PathBuf {
    beton_dir.join("policies")
}

impl PolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert ein Verzeichnis mit `*.yaml`-Policy-Sets (nicht rekursiv;
    /// `*.policytest.yaml` sind Tests und keine Sets).
    pub fn add_dir(&mut self, scope: Scope, dir: impl Into<PathBuf>) {
        self.dirs.push((scope, dir.into()));
        self.effective = None;
    }

    /// Ersetzt die Inline-Sets einer Ebene (z. B. `policies:` des Agents).
    pub fn set_inline(&mut self, scope: Scope, sets: Vec<LoadedSet>) {
        self.inline.retain(|s| s.scope != scope);
        self.inline.extend(sets.into_iter().map(Arc::new));
        self.effective = None;
    }

    /// Liest alle Verzeichnisse neu. Liefert die Ereignisse dieses Durchlaufs.
    pub fn refresh(&mut self) -> Vec<StoreEvent> {
        let mut events = Vec::new();
        let mut present: Vec<(Scope, PathBuf)> = Vec::new();
        for (scope, dir) in &self.dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue; // Verzeichnis existiert (noch) nicht
            };
            let mut files: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| is_policy_file(p))
                .collect();
            files.sort();
            present.extend(files.into_iter().map(|p| (*scope, p)));
        }

        // Gelöschte Dateien
        let gone: Vec<PathBuf> = self
            .files
            .keys()
            .filter(|p| !present.iter().any(|(_, q)| q == *p))
            .cloned()
            .collect();
        for path in gone {
            if let Some(entry) = self.files.remove(&path)
                && let Some(set) = entry.valid
            {
                events.push(StoreEvent::Changed(PolicySetChanged {
                    scope: entry.scope.as_str().into(),
                    set: set.name.clone(),
                    version: "deleted".into(),
                }));
            }
            self.effective = None;
        }

        for (scope, path) in present {
            let origin = path.display().to_string();
            let read = read_limited(&path);
            let hash: [u8; 32] = match &read {
                Ok(text) => Sha256::digest(text.as_bytes()).into(),
                Err(e) => Sha256::digest(e.as_bytes()).into(),
            };
            if let Some(entry) = self.files.get(&path)
                && entry.seen == hash
                && entry.scope == scope
            {
                continue;
            }
            self.effective = None;
            let result = match read {
                Ok(text) => parse_set(scope, &origin, &text),
                Err(message) => Err(LoadError {
                    file: origin.clone(),
                    line: 1,
                    column: 1,
                    message,
                }),
            };
            let entry = self.files.entry(path).or_insert(FileEntry {
                scope,
                seen: hash,
                valid: None,
                invalid: false,
                error: None,
            });
            entry.seen = hash;
            entry.scope = scope;
            match result {
                Ok(set) => {
                    events.push(StoreEvent::Changed(PolicySetChanged {
                        scope: scope.as_str().into(),
                        set: set.name.clone(),
                        version: set.version.clone(),
                    }));
                    entry.valid = Some(Arc::new(set));
                    entry.invalid = false;
                    entry.error = None;
                }
                Err(e) => {
                    let event = e.to_event();
                    events.push(StoreEvent::Invalid(event.clone()));
                    entry.invalid = true;
                    entry.error = Some(event);
                }
            }
        }
        events
    }

    /// Das aktuelle effektive Set (gecacht bis zur nächsten Änderung).
    pub fn effective(&mut self) -> Arc<EffectiveSet> {
        if let Some(e) = &self.effective {
            return e.clone();
        }
        let sets: Vec<Arc<LoadedSet>> = self
            .files
            .values()
            .filter_map(|f| f.valid.clone())
            .chain(self.inline.iter().cloned())
            .collect();
        let invalid: Vec<String> = self
            .files
            .iter()
            .filter(|(_, f)| f.invalid)
            .map(|(p, _)| p.display().to_string())
            .collect();
        let never_valid: Vec<PolicySetInvalid> = self
            .files
            .values()
            .filter(|f| f.invalid && f.valid.is_none())
            .filter_map(|f| f.error.clone())
            .collect();
        let effective =
            Arc::new(EffectiveSet::with_invalid(sets, invalid).with_never_valid(never_valid));
        self.effective = Some(effective.clone());
        effective
    }
}

fn is_policy_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    path.is_file()
        && name.ends_with(".yaml")
        && !name.ends_with(".policytest.yaml")
        && !name.starts_with('.')
}

fn read_limited(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("nicht lesbar: {e}"))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("Datei größer als {MAX_FILE_BYTES} Bytes"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("nicht lesbar: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "keine gültige UTF-8-Datei".into())
}
