//! Ladeorte und Hot-Reload (POL-001).
//!
//! Der Store kennt die Policy-Verzeichnisse je Ebene (User `~/.beton/policies/`, Projekt
//! `.beton/policies/`) und Inline-Sets (Agent-Ebene, WP-49). [`PolicyStore::refresh`] liest
//! die Verzeichnisse neu und vergleicht den Inhalts-Hash jeder Datei:
//!
//! - geänderte, gültige Datei → neue Version, `policy.set_changed`
//! - fehlerhafte Datei → letzte gültige Version bleibt aktiv, `policy.set_invalid` mit Datei,
//!   Zeile und Fehler (AC3)
//! - gelöschte Datei → die letzte gültige Version bleibt für die Lebensdauer des Stores (der
//!   Session) aktiv, `policy.set_invalid` meldet die Löschung (fail closed, #234); erst eine
//!   neue Session liest den Stand ohne die Datei
//! - nicht lesbares Verzeichnis (außer „existiert nicht“) → bekannte Sets bleiben aktiv,
//!   `policy.set_invalid`; war es nie lesbar, zählt es als nie gültige Datei (Start verweigert)
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
    /// Gelöscht; die letzte gültige Fassung bleibt aktiv (#234).
    deleted: bool,
}

/// Policy-Sets aller lokalen Ebenen mit Hot-Reload. Eine Kopie erlaubt eine Auswertung mit
/// dem aktuellen Stand der Dateien, ohne Ereignisse des Originals zu verbrauchen (POL-027).
#[derive(Debug, Default, Clone)]
pub struct PolicyStore {
    dirs: Vec<(Scope, PathBuf)>,
    files: BTreeMap<PathBuf, FileEntry>,
    inline: Vec<Arc<LoadedSet>>,
    effective: Option<Arc<EffectiveSet>>,
    /// Ebenen, deren Dateien nach dem ersten Durchlauf nur verschärft werden dürfen
    /// (Re-Review R1 zu #253): lockernde Änderungen gelten erst in einer neuen Session.
    tighten: Vec<Scope>,
    /// Der erste Durchlauf ist erfolgt.
    primed: bool,
}

/// `~/.beton/policies` bzw. `<repo>/.beton/policies` zu einem beton-Verzeichnis.
pub fn policies_dir(beton_dir: &Path) -> PathBuf {
    beton_dir.join("policies")
}

impl PolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert ein Verzeichnis mit `*.yaml`- bzw. `*.yml`-Policy-Sets (nicht rekursiv;
    /// `*.policytest.yaml` sind Tests und keine Sets).
    pub fn add_dir(&mut self, scope: Scope, dir: impl Into<PathBuf>) {
        self.dirs.push((scope, dir.into()));
        self.effective = None;
    }

    /// Nach dem ersten [`PolicyStore::refresh`] übernimmt der Store Änderungen an Dateien
    /// dieser Ebene nur, wenn sie verschärfen ([`only_tightens`]); sonst bleibt die bisherige
    /// Fassung aktiv und `policy.set_invalid` meldet die lockernde Änderung.
    pub fn tighten_only(&mut self, scope: Scope) {
        if !self.tighten.contains(&scope) {
            self.tighten.push(scope);
        }
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
        // Verzeichnisse, die gerade nicht lesbar sind: ihre Dateien gelten nicht als gelöscht.
        let mut unreadable: Vec<PathBuf> = Vec::new();
        for (scope, dir) in &self.dirs {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    // Fail closed (#234): nie still verschwinden lassen.
                    let error = PolicySetInvalid {
                        file: dir.display().to_string(),
                        line: 0,
                        error: format!("Policy-Verzeichnis nicht lesbar: {e}"),
                    };
                    let known = self.files.keys().any(|p| p.starts_with(dir));
                    let entry = self.files.entry(dir.clone()).or_insert(FileEntry {
                        scope: *scope,
                        seen: [0; 32],
                        valid: None,
                        invalid: false,
                        error: None,
                        deleted: false,
                    });
                    if !entry.invalid {
                        events.push(StoreEvent::Invalid(error.clone()));
                        self.effective = None;
                    }
                    entry.invalid = true;
                    // Nur ohne bekannte Sets zählt es als nie gültig (Start verweigert).
                    entry.error = (!known).then_some(error);
                    unreadable.push(dir.clone());
                    present.push((*scope, dir.clone()));
                    continue;
                }
            };
            let mut files: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| is_policy_file(p))
                .collect();
            files.sort();
            present.extend(files.into_iter().map(|p| (*scope, p)));
        }

        // Gelöschte Dateien: Die letzte gültige Fassung bleibt aktiv (fail closed, #234).
        let gone: Vec<PathBuf> = self
            .files
            .keys()
            .filter(|p| !present.iter().any(|(_, q)| q == *p))
            .filter(|p| !unreadable.iter().any(|d| p.starts_with(d)))
            .cloned()
            .collect();
        for path in gone {
            let keep = self.files.get(&path).is_some_and(|e| e.valid.is_some());
            if !keep {
                self.files.remove(&path);
                self.effective = None;
                continue;
            }
            if let Some(entry) = self.files.get_mut(&path)
                && !entry.deleted
            {
                entry.deleted = true;
                entry.seen = [0; 32];
                events.push(StoreEvent::Invalid(PolicySetInvalid {
                    file: path.display().to_string(),
                    line: 0,
                    error: "Policy-Datei gelöscht; die letzte gültige Fassung bleibt bis zum Ende der Session aktiv".into(),
                }));
            }
        }

        for (scope, path) in present {
            if unreadable.contains(&path) {
                continue;
            }
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
            let tighten_now = self.primed && self.tighten.contains(&scope);
            let entry = self.files.entry(path).or_insert(FileEntry {
                scope,
                seen: hash,
                valid: None,
                invalid: false,
                error: None,
                deleted: false,
            });
            entry.seen = hash;
            entry.scope = scope;
            entry.deleted = false;
            let check = match (&result, tighten_now) {
                (Ok(set), true) => only_tightens(entry.valid.as_deref(), set).err(),
                _ => None,
            };
            if let Some(why) = check {
                events.push(StoreEvent::Invalid(PolicySetInvalid {
                    file: origin.clone(),
                    line: 0,
                    error: format!(
                        "Die Änderung lockert die Policy ({why}); sie gilt erst in einer neuen Session"
                    ),
                }));
                continue;
            }
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
        self.primed = true;
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

/// Verschärft `new` gegenüber `old` (ohne `old`: gegenüber einem leeren Set)? Erlaubt sind:
/// unveränderte Regeln, neue Regeln mit `deny`, `ask` oder `notify` (bzw. noch nicht
/// entzuckerte Regeltypen) ohne `effects` und ohne eigenen Freigabe-Timeout bzw.
/// `on_timeout: allow`, Dry-Run-Regeln und strengere Defaults. `Err` nennt den ersten
/// lockernden Punkt.
pub fn only_tightens(old: Option<&LoadedSet>, new: &LoadedSet) -> Result<(), String> {
    use crate::doc::{Action, DefaultAction, Mode, TimeoutAction};
    let active = |s: &LoadedSet| s.mode == Mode::Enforce;
    let old_rules: Vec<&crate::load::LoadedRule> = old
        .filter(|o| active(o))
        .map(|o| o.rules.iter().filter(|r| !r.doc.dry_run).collect())
        .unwrap_or_default();
    let old_defaults = old
        .filter(|o| active(o))
        .map(|o| o.defaults.clone())
        .unwrap_or_default();
    if !active(new) {
        if old_rules.is_empty() && old_defaults.is_empty() {
            return Ok(());
        }
        return Err("mode: dry_run".into());
    }
    for (phase, was) in &old_defaults {
        let now = new
            .defaults
            .get(phase)
            .copied()
            .unwrap_or(DefaultAction::Allow);
        if now < *was {
            return Err(format!("Default für {phase} milder"));
        }
    }
    for rule in &old_rules {
        if !new.rules.iter().any(|r| r.doc == rule.doc) {
            return Err(format!("Regel `{}` entfernt oder geändert", rule.id));
        }
    }
    for rule in &new.rules {
        if rule.doc.dry_run || old_rules.iter().any(|r| r.doc == rule.doc) {
            continue;
        }
        // Wirksame Aktion: entzuckerte Regeltypen (z. B. `require_approval` → `ask`, die Teile
        // von `spend_cap`/`daily_budget`/`model_route` → `deny`/`ask`/`modify`) über ihre
        // CEL-Fassung; noch nicht entzuckerte wertet die Engine fail closed als `deny`.
        // Umrouten (`modify`) verschärft nicht: Es gilt erst in einer neuen Session. Eine
        // `tool_allowlist` bringt `allow`-Regeln mit (sie heben Defaults anderer Sets derselben
        // Ebene auf) und lockert damit (POL-015).
        let action = match &rule.kind {
            crate::load::RuleKind::Cel { action, .. } => *action,
            crate::load::RuleKind::Builtin(_) => Action::Deny,
        };
        if !matches!(action, Action::Deny | Action::Ask | Action::Notify) {
            return Err(format!("Regel `{}` neu mit allow oder modify", rule.id));
        }
        // Effects können Zähler anderer Regeln zurücksetzen (#253 N1). Ausgenommen sind die
        // eigenen Schwellen-Keys eines Budget-Regeltyps (`spend_cap/<ebene>/<set>/<id>/…`), die
        // handgeschriebene Regeln nicht schreiben dürfen.
        let own = |key: &str| {
            rule.doc.rule_type.is_some_and(|t| {
                crate::budget::own_key(t, new.scope, &new.name, &new.origin, &rule.id, key)
            })
        };
        if !rule.doc.effects.iter().all(|e| own(&e.key)) {
            return Err(format!("Regel `{}` neu mit effects", rule.id));
        }
        // Der kürzeste Timeout gilt für die ganze Card; `on_timeout: allow` erlaubt (N2).
        if let Some(a) = &rule.doc.approval
            && (a.timeout.is_some() || a.on_timeout == Some(TimeoutAction::Allow))
        {
            return Err(format!(
                "Regel `{}` neu mit eigenem Freigabe-Timeout bzw. on_timeout: allow",
                rule.id
            ));
        }
    }
    Ok(())
}

fn is_policy_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    // `.yml` wie `.yaml`: eine Datei mit der anderen Endung bliebe sonst still wirkungslos.
    path.is_file()
        && (name.ends_with(".yaml") || name.ends_with(".yml"))
        && !name.ends_with(".policytest.yaml")
        && !name.ends_with(".policytest.yml")
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
