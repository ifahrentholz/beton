//! Enforcement-Matrix und Prüfung beim Session-Start (POL-024).
//!
//! Die Matrix sagt je Phase (und bei Tool-Phasen je Tool-Klasse), wie stark der Harness der
//! Session eine Entscheidung durchsetzen kann: `full`, `approval_only` (nur an Freigabe-
//! Anfragen des Vendors) oder `observe_only`. Sie entsteht aus den Harness-Capabilities
//! (`beton_harness::Capabilities::enforcement`, HAR-002) und ist hier nur ein Datentyp.
//!
//! [`check_start`] prüft vor dem Start jede Regel mit `deny`/`ask`/`modify` (und jeden noch
//! nicht entzuckerten Regeltyp): Fehlt die Durchsetzung bei einer mutierenden Tool-Klasse und
//! ist die Regel `required` (Default), verweigert der Runner den Start; Lücken nur bei
//! lesenden Klassen (`file_read`, `search`, `web_fetch`) und bei `best_effort`-Regeln ergeben
//! eine Warnung (`policy.enforcement_degraded`), die Sandbox ist dann der Backstop.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::Phase;
use crate::doc::{Action, Enforcement, Mode};
use crate::engine::{Capabilities, EffectiveSet};
use crate::load::{LoadedRule, RuleKind, RuleRef};

/// Kanonische Tool-Klassen (POL-005).
pub const KINDS: [&str; 10] = [
    "shell",
    "file_read",
    "file_write",
    "file_edit",
    "search",
    "web_fetch",
    "mcp",
    "system",
    "browser",
    "other",
];

/// Nicht mutierende Klassen: Lücken dort sind nur eine Warnung (POL-024).
pub const READING_KINDS: [&str; 3] = ["file_read", "search", "web_fetch"];

/// Wie stark ein Harness eine Phase durchsetzen kann; schwächste Stufe zuerst.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Nur Audit; `deny`/`ask`/`modify` wirken nicht.
    ObserveOnly,
    /// Nur wenn der Vendor um Freigabe fragt (`deny` und `ask` wirken dort).
    ApprovalOnly,
    /// Jede Aktion passiert das Gate.
    Full,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::ObserveOnly => "observe_only",
            Level::ApprovalOnly => "approval_only",
            Level::Full => "full",
        }
    }

    /// Kann `deny`/`ask` auf dieser Stufe wirken?
    pub fn enforces(self) -> bool {
        self != Level::ObserveOnly
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreichbare Durchsetzung eines Harness je Phase und Tool-Klasse.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub session_start: Level,
    pub model_request: Level,
    /// Je Tool-Klasse; fehlt eine Klasse, gilt `observe_only` (fail closed).
    pub tool_call: BTreeMap<String, Level>,
    /// Je Tool-Klasse; fehlt eine Klasse, gilt `observe_only`.
    pub tool_result: BTreeMap<String, Level>,
    /// `browser_navigate` und `browser_action` (beton-Browser-Tools, ab M3).
    pub browser: Level,
    /// Welche Modifikationen je Phase anwendbar sind; fehlt eine Phase, keine.
    pub modify: BTreeMap<Phase, Capabilities>,
}

impl Matrix {
    /// Alles voll durchsetzbar, alle Modifikationen (Direkt-API, Fake-Harness).
    pub fn full() -> Self {
        let all = |l| KINDS.iter().map(|k| ((*k).to_owned(), l)).collect();
        Self {
            session_start: Level::Full,
            model_request: Level::Full,
            tool_call: all(Level::Full),
            tool_result: all(Level::Full),
            browser: Level::Full,
            modify: Phase::ALL
                .iter()
                .map(|p| (*p, Capabilities::default()))
                .collect(),
        }
    }

    /// Stufe einer Phase; bei Tool-Phasen ohne Klasse die schwächste über alle Klassen.
    pub fn level(&self, phase: Phase, kind: Option<&str>) -> Level {
        let per_kind = |map: &BTreeMap<String, Level>| match kind {
            Some(k) => map.get(k).copied().unwrap_or(Level::ObserveOnly),
            None => KINDS
                .iter()
                .map(|k| map.get(*k).copied().unwrap_or(Level::ObserveOnly))
                .min()
                .unwrap_or(Level::ObserveOnly),
        };
        match phase {
            Phase::SessionStart => self.session_start,
            Phase::ModelRequest => self.model_request,
            Phase::ToolCall => per_kind(&self.tool_call),
            Phase::ToolResult => per_kind(&self.tool_result),
            Phase::BrowserNavigate | Phase::BrowserAction => self.browser,
        }
    }

    /// Anwendbare Modifikationen in `phase` (für [`crate::engine::EvalOptions`]).
    pub fn capabilities(&self, phase: Phase) -> Capabilities {
        self.modify
            .get(&phase)
            .copied()
            .unwrap_or_else(Capabilities::none)
    }
}

/// Phasen und Tool-Klassen, die eine Regel betrifft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targets {
    pub phases: Vec<Phase>,
    /// `None`: nicht bestimmbar, also alle Klassen.
    pub kinds: Option<BTreeSet<String>>,
}

/// Statische Analyse einer Regel: das Feld `kinds:` oder der CEL-Ausdruck (`tool.kind == …`,
/// `tool.kind in […]`, `tool.name == …` über die Abbildungstabelle, mit `&&`/`||`).
pub fn targets(rule: &LoadedRule) -> Targets {
    let phases = Phase::ALL
        .iter()
        .copied()
        .filter(|p| rule.applies_to(*p))
        .collect();
    let kinds = match (&rule.doc.kinds, &rule.kind) {
        (Some(list), _) => Some(list.iter().cloned().collect()),
        (None, RuleKind::Cel { when: Some(p), .. }) => p.tool_kinds(),
        _ => None,
    };
    Targets { phases, kinds }
}

/// Eine Regel, die der Harness nicht (voll) durchsetzen kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub rule: RuleRef,
    pub phase: Phase,
    /// Betroffene Tool-Klassen ohne Durchsetzung (leer bei anderen Phasen).
    pub kinds: Vec<String>,
    pub level: Level,
}

impl Gap {
    /// Lesbare Begründung ohne Inhalte der Policy-Datei.
    pub fn describe(&self) -> String {
        if self.kinds.is_empty() {
            format!(
                "Regel `{}` ({}, Set `{}`): Phase {} ist bei diesem Harness nur {}",
                self.rule.rule_id,
                self.rule.scope.as_str(),
                self.rule.set,
                self.phase,
                self.level
            )
        } else {
            format!(
                "Regel `{}` ({}, Set `{}`): Phase {} für {} ist bei diesem Harness nur {}",
                self.rule.rule_id,
                self.rule.scope.as_str(),
                self.rule.set,
                self.phase,
                self.kinds.join(", "),
                self.level
            )
        }
    }
}

/// Ergebnis der Prüfung beim Start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartCheck {
    /// Nicht durchsetzbare Pflichtregeln: Start verweigert.
    pub refused: Vec<Gap>,
    /// Lücken bei lesenden Klassen bzw. `best_effort`: Warnung.
    pub degraded: Vec<Gap>,
}

impl StartCheck {
    pub fn is_clean(&self) -> bool {
        self.refused.is_empty() && self.degraded.is_empty()
    }

    fn ids(gaps: &[Gap]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for g in gaps {
            if !out.contains(&g.rule.rule_id) {
                out.push(g.rule.rule_id.clone());
            }
        }
        out
    }

    /// IDs der Regeln, die den Start verhindern (in kanonischer Reihenfolge).
    pub fn refused_rules(&self) -> Vec<String> {
        Self::ids(&self.refused)
    }

    pub fn degraded_rules(&self) -> Vec<String> {
        Self::ids(&self.degraded)
    }
}

/// Prüft alle Regeln des effektiven Sets gegen die Matrix (POL-024).
pub fn check_start(set: &EffectiveSet, matrix: &Matrix) -> StartCheck {
    let mut out = StartCheck::default();
    for s in set.sets() {
        if s.mode == Mode::DryRun {
            continue;
        }
        for rule in &s.rules {
            if rule.doc.dry_run {
                continue;
            }
            let blocking = match &rule.kind {
                RuleKind::Cel { action, .. } => {
                    matches!(action, Action::Deny | Action::Ask | Action::Modify)
                }
                // Noch nicht entzuckerte Regeltypen wirken als `deny` (fail closed).
                RuleKind::Builtin(_) => true,
            };
            if !blocking {
                continue;
            }
            let required =
                rule.doc.enforcement.unwrap_or(Enforcement::Required) == Enforcement::Required;
            let t = targets(rule);
            for phase in &t.phases {
                let gap = |kinds: Vec<String>, level| Gap {
                    rule: s.rule_ref(&rule.id),
                    phase: *phase,
                    kinds,
                    level,
                };
                match phase {
                    Phase::ToolCall | Phase::ToolResult => {
                        let kinds: Vec<String> = match &t.kinds {
                            Some(k) => k.iter().cloned().collect(),
                            None => KINDS.iter().map(|k| (*k).to_owned()).collect(),
                        };
                        let mut weak: Vec<String> = kinds
                            .into_iter()
                            .filter(|k| !matrix.level(*phase, Some(k)).enforces())
                            .collect();
                        if weak.is_empty() {
                            continue;
                        }
                        weak.sort();
                        let mutating = weak.iter().any(|k| !READING_KINDS.contains(&k.as_str()));
                        let g = gap(weak, Level::ObserveOnly);
                        if required && mutating {
                            out.refused.push(g);
                        } else {
                            out.degraded.push(g);
                        }
                    }
                    _ => {
                        let level = matrix.level(*phase, None);
                        if level.enforces() {
                            continue;
                        }
                        let g = gap(Vec::new(), level);
                        if required {
                            out.refused.push(g);
                        } else {
                            out.degraded.push(g);
                        }
                    }
                }
            }
        }
    }
    out
}
