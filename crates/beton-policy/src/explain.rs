//! Explain (POL-027): rekonstruiert eine Entscheidung aus dem `policy.decision`-Event und dem
//! effektiven Set, das damals galt (Snapshot, [`crate::snapshot`]).
//!
//! Gezeigt werden das effektive Set mit Herkunft je Regel, die Pass-1-Modifikationen und
//! Konflikte, die Pass-2-Verdikte mit Bedingung und ausgewertetem Teilausdruck, Defaults
//! (aufgehoben oder nicht), Fehler, Grants und das Endergebnis. Regeltexte werden nur über
//! die Version (Inhalts-Hash) des Sets zugeordnet: Ein Set einer anderen Version liefert nie
//! den Text einer Regel (AC2).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use beton_core::event::{PolicyDecision, PolicyOutcome};
use serde::Serialize;
use serde_json::Value;

use crate::Phase;
use crate::cel::TracePart;
use crate::context::Activation;
use crate::doc::Mode;
use crate::engine::EffectiveSet;
use crate::load::{LoadedRule, LoadedSet, RuleKind};

/// Wie verlässlich die Regeltexte sind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    /// Das Set hat genau den `policy_set_hash` der Entscheidung.
    Verified,
    /// Ein anderes Set: Regeltexte nur, wo die Set-Version übereinstimmt.
    Mismatch,
    /// Kein Snapshot: nur die Daten des Events.
    Missing,
}

/// Eine Regel des effektiven Sets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleView {
    pub id: String,
    /// Phasen, in denen die Regel entscheidet.
    pub phases: Vec<String>,
    /// Aktion bzw. eingebauter Regeltyp.
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_type: Option<String>,
    /// Bedingung (bei Regeltypen die entzuckerte CEL-Bedingung).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub dry_run: bool,
}

/// Ein Set mit Herkunft.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetView {
    pub scope: String,
    pub name: String,
    pub origin: String,
    pub version: String,
    pub mode: String,
    pub rules: Vec<RuleView>,
}

/// Ein Verdikt einer Regel (Pass 1: `modify`, Pass 2: alle anderen).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerdictView {
    pub pass: u8,
    pub rule_id: String,
    pub scope: String,
    pub set: String,
    pub set_version: String,
    /// Datei bzw. Herkunft des Sets (aus dem Snapshot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub verdict: String,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Bedingung der Regel in der Version der Entscheidung.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Ausgewertete Teilausdrücke.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<TracePart>,
}

/// Rekonstruierte Entscheidung.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Explanation {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub decision_id: String,
    pub phase: String,
    pub outcome: String,
    pub enforcement: String,
    pub policy_set_hash: String,
    pub snapshot: SnapshotStatus,
    pub sets: Vec<SetView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<Value>,
    pub conflicts: Vec<Value>,
    pub verdicts: Vec<VerdictView>,
    pub defaults: Vec<Value>,
    pub errors: Vec<Value>,
    pub grants: Vec<String>,
    /// Benutzte Kontextwerte (nur bei `eval` und Policy-Tests, nie aus dem Audit).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, Value>,
}

fn outcome_str(o: PolicyOutcome) -> &'static str {
    match o {
        PolicyOutcome::Allow => "allow",
        PolicyOutcome::Deny => "deny",
        PolicyOutcome::Ask => "ask",
    }
}

fn action_str(rule: &LoadedRule) -> (String, Option<String>) {
    match (&rule.kind, rule.doc.rule_type) {
        (RuleKind::Cel { action, .. }, t) => (
            serde_json::to_value(action)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            t.map(|t| t.as_str().to_owned()),
        ),
        (RuleKind::Builtin(t), _) => (t.as_str().to_owned(), Some(t.as_str().to_owned())),
    }
}

fn when_of(rule: &LoadedRule) -> Option<String> {
    match &rule.kind {
        RuleKind::Cel { when: Some(p), .. } => Some(p.source().to_owned()),
        _ => None,
    }
}

fn phases_of(rule: &LoadedRule) -> Vec<String> {
    match &rule.kind {
        RuleKind::Cel { phase, .. } => vec![phase.as_str().to_owned()],
        RuleKind::Builtin(t) => t.phases().iter().map(|p| p.as_str().to_owned()).collect(),
    }
}

fn set_view(s: &LoadedSet) -> SetView {
    SetView {
        scope: s.scope.as_str().to_owned(),
        name: s.name.clone(),
        origin: s.origin.clone(),
        version: s.version.clone(),
        mode: match s.mode {
            Mode::Enforce => "enforce".into(),
            Mode::DryRun => "dry_run".into(),
        },
        rules: s
            .rules
            .iter()
            .map(|r| {
                let (action, rule_type) = action_str(r);
                RuleView {
                    id: r.id.clone(),
                    phases: phases_of(r),
                    action,
                    rule_type,
                    when: when_of(r),
                    dry_run: s.mode == Mode::DryRun || r.doc.dry_run,
                }
            })
            .collect(),
    }
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}

/// Erklärt eine Entscheidung. `set` ist das Set der Entscheidung (aus dem Snapshot) bzw.
/// das aktuelle Set; ohne Set bleiben nur die Daten des Events.
pub fn explain(event: &PolicyDecision, set: Option<&EffectiveSet>) -> Explanation {
    let snapshot = match set {
        None => SnapshotStatus::Missing,
        Some(s) if s.hash() == event.policy_set_hash => SnapshotStatus::Verified,
        Some(_) => SnapshotStatus::Mismatch,
    };
    let find = |scope: &str, name: &str, version: &str| {
        set.into_iter()
            .flat_map(|s| s.sets().iter())
            .find(|s| s.scope.as_str() == scope && s.name == name && s.version == version)
    };
    let verdicts = event
        .matched
        .iter()
        .map(|m| {
            let (scope, name, version) = (text(m, "scope"), text(m, "set"), text(m, "set_version"));
            let rule_id = text(m, "rule_id");
            let loaded = find(&scope, &name, &version);
            let rule = loaded.and_then(|s| s.rules.iter().find(|r| r.id == rule_id));
            let verdict = text(m, "verdict");
            VerdictView {
                pass: if verdict == "modify" { 1 } else { 2 },
                origin: loaded.map(|s| s.origin.clone()),
                when: rule.and_then(when_of),
                reason: m["reason"].as_str().map(str::to_owned),
                dry_run: m["dry_run"].as_bool().unwrap_or(false),
                trace: serde_json::from_value(m["trace"].clone()).unwrap_or_default(),
                rule_id,
                scope,
                set: name,
                set_version: version,
                verdict,
            }
        })
        .collect();
    Explanation {
        decision_id: event.decision_id.clone(),
        phase: event.phase.clone(),
        outcome: outcome_str(event.outcome).to_owned(),
        enforcement: event.enforcement.clone(),
        policy_set_hash: event.policy_set_hash.clone(),
        snapshot,
        sets: set
            .map(|s| s.sets().iter().map(|s| set_view(s)).collect())
            .unwrap_or_default(),
        modified: event.modified.clone(),
        conflicts: event.conflicts.clone(),
        verdicts,
        defaults: event.defaults_applied.clone(),
        errors: event.errors.clone(),
        grants: event
            .grants_used
            .iter()
            .filter_map(|g| g.as_str().map(str::to_owned))
            .collect(),
        context: BTreeMap::new(),
    }
}

/// Die Kontextwerte, die die Regeln einer Phase lesen (`session.cost_usd`, `tool.command`).
pub fn context_values(
    set: &EffectiveSet,
    phase: Phase,
    act: &Activation,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for field in set.programs(phase).flat_map(|p| p.fields()) {
        let mut parts = field.split('.');
        let Some(var) = parts.next() else { continue };
        let mut value = act.var(var).to_json();
        for part in parts {
            value = value.get(part).cloned().unwrap_or(Value::Null);
        }
        out.insert(field, value);
    }
    // Nur Blätter: `session` neben `session.cost_usd` ist überflüssig.
    let keys: Vec<String> = out.keys().cloned().collect();
    out.retain(|k, _| !keys.iter().any(|o| o.starts_with(&format!("{k}."))));
    out
}

fn short(version: &str) -> &str {
    version.get(..19).unwrap_or(version)
}

fn json_short(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Textdarstellung für `beton policy explain` und die Fehlschläge von `beton policy test`.
pub fn render(ex: &Explanation) -> String {
    let mut out = String::new();
    let id = if ex.decision_id.is_empty() {
        String::new()
    } else {
        format!("{} · ", ex.decision_id)
    };
    let _ = writeln!(
        out,
        "Entscheidung {id}Phase {} · Ergebnis: {} · Durchsetzung: {}",
        ex.phase, ex.outcome, ex.enforcement
    );
    let status = match ex.snapshot {
        SnapshotStatus::Verified => "Snapshot geprüft",
        SnapshotStatus::Mismatch => {
            "abweichendes Set: Regeltexte nur, wo die Set-Version übereinstimmt"
        }
        SnapshotStatus::Missing => "kein Snapshot gespeichert, nur Daten der Entscheidung",
    };
    let _ = writeln!(out, "Policy-Set {} ({status})", ex.policy_set_hash);
    if !ex.sets.is_empty() {
        let _ = writeln!(out, "\nEffektives Set:");
        for s in &ex.sets {
            let mode = if s.mode == "dry_run" {
                " [dry_run]"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "  {:<8} {}{mode}  {}  {}",
                s.scope, s.name, s.origin, s.version
            );
            for r in &s.rules {
                let dry = if r.dry_run { " [dry_run]" } else { "" };
                // Entzuckerte Regeltypen nennen ihren Typ (`deny [git_guard]`).
                let typ = match &r.rule_type {
                    Some(t) if *t != r.action => format!(" [{t}]"),
                    _ => String::new(),
                };
                let when = r
                    .when
                    .as_deref()
                    .map(|w| format!("  when: {w}"))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "    - {} ({}) {}{typ}{dry}{when}",
                    r.id,
                    r.phases.join(", "),
                    r.action
                );
            }
        }
    }
    let _ = writeln!(out, "\nPass 1 – Modifikationen:");
    let modifies: Vec<&VerdictView> = ex.verdicts.iter().filter(|v| v.pass == 1).collect();
    match &ex.modified {
        Some(m) => {
            let _ = writeln!(out, "  angewendet: {m}");
        }
        None if modifies.is_empty() => {
            let _ = writeln!(out, "  keine");
        }
        None => {
            let _ = writeln!(out, "  nichts angewendet");
        }
    }
    for v in modifies {
        render_verdict(&mut out, v);
    }
    for c in &ex.conflicts {
        let _ = writeln!(
            out,
            "  Konflikt {}: behalten {} ({}), verworfen {} ({})",
            text(c, "field"),
            json_short(&c["kept"]),
            text(&c["kept_rule"], "rule_id"),
            json_short(&c["discarded"]),
            text(&c["discarded_rule"], "rule_id"),
        );
    }
    let _ = writeln!(out, "\nPass 2 – Verdikte:");
    let decides: Vec<&VerdictView> = ex.verdicts.iter().filter(|v| v.pass == 2).collect();
    if decides.is_empty() {
        let _ = writeln!(out, "  keine Regel hat gegriffen");
    }
    for v in decides {
        render_verdict(&mut out, v);
    }
    if !ex.defaults.is_empty() {
        let _ = writeln!(out, "\nDefaults:");
        for d in &ex.defaults {
            let lifted = if d["lifted"].as_bool().unwrap_or(false) {
                "aufgehoben durch allow derselben Ebene"
            } else {
                "nicht aufgehoben"
            };
            let _ = writeln!(
                out,
                "  {} {}: {} ({lifted})",
                text(d, "scope"),
                text(d, "phase"),
                text(d, "default")
            );
        }
    }
    if !ex.errors.is_empty() {
        let _ = writeln!(out, "\nFehler (fail closed bei deny/ask/modify):");
        for e in &ex.errors {
            let _ = writeln!(
                out,
                "  {}/{}/{}: {}",
                text(e, "scope"),
                text(e, "set"),
                text(e, "rule_id"),
                text(e, "error")
            );
        }
    }
    if !ex.grants.is_empty() {
        let _ = writeln!(out, "\nGrants: {}", ex.grants.join(", "));
    }
    if !ex.context.is_empty() {
        let _ = writeln!(out, "\nKontext:");
        for (k, v) in &ex.context {
            let _ = writeln!(out, "  {k} = {v}");
        }
    }
    let _ = writeln!(out, "\nErgebnis: {}", ex.outcome);
    out
}

fn render_verdict(out: &mut String, v: &VerdictView) {
    let verdict = if v.dry_run {
        format!("would_{} [dry_run]", v.verdict)
    } else {
        v.verdict.clone()
    };
    let origin = v
        .origin
        .as_deref()
        .map(|o| format!("  {o}"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "  {verdict}  {}/{}/{}{origin}  ({})",
        v.scope,
        v.set,
        v.rule_id,
        short(&v.set_version)
    );
    if let Some(r) = &v.reason {
        let _ = writeln!(out, "      Grund: {r}");
    }
    if let Some(w) = &v.when {
        let _ = writeln!(out, "      when: {w}");
    }
    for p in &v.trace {
        let value = match p.value {
            Some(b) => b.to_string(),
            None => "Fehler".into(),
        };
        let _ = writeln!(out, "      {} → {value}", p.expr);
    }
}
