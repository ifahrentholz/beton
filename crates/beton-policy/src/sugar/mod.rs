//! Entzuckerung eingebauter Regeltypen aus WP-42 (POL-015 bis POL-018) zu CEL-Regeln.
//!
//! Ein Regeltyp wird beim Laden zu einer oder mehreren gewöhnlichen Regeln (Phase, CEL-
//! Bedingung, Aktion) und ggf. zu einem Default der Ebene. Die Bedingungen sind CEL-Text, den
//! der Lader wie jede andere Bedingung kompiliert und typprüft; `beton policy explain` zeigt
//! sie. Mehrere Regeln eines Typs heißen `<id>/<teil>` (z. B. `git/force-push`); Regel-IDs
//! aus Policy-Dateien enthalten nie `/`, die Namen kollidieren also nicht.
//!
//! `require_approval` (POL-014) entzuckert [`crate::load`] selbst.

mod git_guard;
pub(crate) mod loop_detection;
mod path_guard;
mod tool_allowlist;

use std::collections::BTreeMap;

use serde_json::Value;

use crate::Phase;
use crate::doc::{Action, DefaultAction, NotifyDoc, RuleDoc, RuleType};

/// Ergebnis der Entzuckerung eines Regeltyps.
#[derive(Debug, Default)]
pub(crate) struct Sugar {
    pub rules: Vec<SugarRule>,
    /// Defaults der Ebene, die der Regeltyp setzt (Phase, Default, Begründung).
    pub defaults: Vec<(Phase, DefaultAction, String)>,
}

/// Eine entzuckerte Regel.
#[derive(Debug)]
pub(crate) struct SugarRule {
    /// Teil der Regel-ID nach `/`; `None` = die ID der Typ-Regel selbst.
    pub part: Option<&'static str>,
    pub phase: Phase,
    /// Bedingung (CEL).
    pub when: String,
    pub action: Action,
    /// Begründung, falls die Typ-Regel keine eigene `reason` hat.
    pub reason: String,
    /// Betroffene Tool-Klassen für die Start-Prüfung (POL-024), falls die Typ-Regel kein
    /// eigenes `kinds` hat.
    pub kinds: Option<Vec<String>>,
    pub notify: Option<NotifyDoc>,
}

/// Fehler in den Parametern: Feld relativ zu `params` (leer = `params` selbst).
#[derive(Debug)]
pub(crate) struct ParamError {
    pub field: String,
    pub message: String,
}

impl ParamError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Entzuckert `rule` (Typ `t`). `None`: Der Typ wird hier nicht entzuckert. `shadow`: Die
/// Regel läuft im Schattenbetrieb (`dry_run` der Regel oder des Sets, POL-027); ein Typ, der
/// sonst einen Default der Ebene setzt, liefert dann eine gleichwertige `deny`-Regel, damit
/// der Default nicht scharf wirkt.
pub(crate) fn desugar(
    t: RuleType,
    rule: &RuleDoc,
    shadow: bool,
) -> Option<Result<Sugar, ParamError>> {
    let empty = BTreeMap::new();
    let params = Params(rule.params.as_ref().unwrap_or(&empty));
    match t {
        RuleType::ToolAllowlist => Some(tool_allowlist::desugar(rule, &params, shadow)),
        RuleType::PathGuard => Some(path_guard::desugar(&params)),
        RuleType::GitGuard => Some(git_guard::desugar(&params)),
        RuleType::LoopDetection => Some(loop_detection::desugar(&params)),
        _ => None,
    }
}

/// Lesender Zugriff auf `params` mit Fehlern je Feld.
pub(crate) struct Params<'a>(&'a BTreeMap<String, Value>);

impl Params<'_> {
    /// Unbekannte Parameter sind ein Ladefehler.
    pub fn only(&self, known: &[&str]) -> Result<(), ParamError> {
        match self.0.keys().find(|k| !known.contains(&k.as_str())) {
            Some(unknown) => Err(ParamError::new(
                unknown.as_str(),
                format!(
                    "unbekannter Parameter `{unknown}` (erlaubt: {})",
                    known.join(", ")
                ),
            )),
            None => Ok(()),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    /// Liste von Texten (fehlt der Parameter: `None`).
    pub fn strings(&self, name: &str) -> Result<Option<Vec<String>>, ParamError> {
        strings_at(self.0.get(name), name)
    }

    /// Ganze Zahl (fehlt der Parameter: `None`).
    pub fn int(&self, name: &str) -> Result<Option<i64>, ParamError> {
        match self.0.get(name) {
            None => Ok(None),
            Some(v) => v
                .as_i64()
                .map(Some)
                .ok_or_else(|| ParamError::new(name, "ganze Zahl erwartet")),
        }
    }

    pub fn text(&self, name: &str) -> Result<Option<String>, ParamError> {
        match self.0.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(ParamError::new(name, "Text erwartet")),
        }
    }
}

/// Liste von Texten an `field`.
pub(crate) fn strings_at(
    v: Option<&Value>,
    field: &str,
) -> Result<Option<Vec<String>>, ParamError> {
    match v {
        None => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ParamError::new(format!("{field}[{i}]"), "Text erwartet"))
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(ParamError::new(field, "Liste erwartet")),
    }
}

/// CEL-Zeichenkettenliteral.
pub(crate) fn lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// CEL-Liste von Zeichenkettenliteralen.
pub(crate) fn list(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|s| lit(s)).collect();
    format!("[{}]", items.join(", "))
}

/// Prüft ein Glob-Muster beim Laden.
pub(crate) fn check_glob(pattern: &str, field: &str) -> Result<(), ParamError> {
    crate::cel::ext::glob_regex(pattern)
        .map(|_| ())
        .map_err(|e| ParamError::new(field, e))
}

/// Prüft Tool-Klassen gegen POL-005.
pub(crate) fn check_kinds(kinds: &[String], field: &str) -> Result<(), ParamError> {
    use crate::enforcement::KINDS;
    for (i, kind) in kinds.iter().enumerate() {
        if !KINDS.contains(&kind.as_str()) {
            return Err(ParamError::new(
                format!("{field}[{i}]"),
                format!(
                    "unbekannte Tool-Klasse `{kind}` (erlaubt: {})",
                    KINDS.join(", ")
                ),
            ));
        }
    }
    Ok(())
}
