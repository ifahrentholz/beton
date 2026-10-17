//! Effektives Policy-Set und Kombinationsalgorithmus (POL-007, 03-policies.md „Design“).
//!
//! Die Auswertung ist rein: ohne IO, deterministisch und seiteneffektfrei. Regeln werden in
//! kanonischer Reihenfolge betrachtet (Ebene `org → team → user → project → agent`, darin
//! Set-Name alphabetisch, darin Deklarationsreihenfolge); die Lade-Reihenfolge spielt keine
//! Rolle (AC5).
//!
//! 1. Pass 1 – Modifikation (erster Schreiber gewinnt; `args` je JSON-Pointer; `redact`
//!    kumulativ; `on_unsupported`).
//! 2. Pass 2 – Entscheidung auf dem modifizierten Subjekt.
//! 3. Defaults je Ebene, aufgehoben nur durch `allow` derselben Ebene.
//! 4. Fehler: `deny`/`ask`/`modify` → `deny` (fail closed), `allow` hebt nichts auf, `notify`
//!    wird protokolliert.
//! 5. Grants machen `ask` zu `allow`.
//! 6. Ergebnis: `deny` > `ask` > `allow`; `notify` immer.
//! 7. Effects werden gemeldet (Anwendung: POL-009).
//! 8. Dry-Run-Regeln werden ausgewertet und gemeldet, fließen aber nicht ein.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::Phase;
use crate::cel::{Clock, EvalError, Limits, Meter, SystemClock, TracePart};
use crate::context::Activation;
use crate::doc::{
    Action, ApprovalDoc, Channel, DefaultAction, EffectDoc, Mode, OnUnsupported, Severity,
};
use crate::load::{LoadedRule, LoadedSet, RuleKind, RuleRef, Scope};
use crate::state::StateOp;

/// Welche Modifikationen der Harness in dieser Phase anwenden kann (Enforcement-Matrix,
/// POL-024, [`crate::enforcement::Matrix::capabilities`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub model: bool,
    pub reasoning_effort: bool,
    pub permission_mode: bool,
    pub args: bool,
    pub redact: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            model: true,
            reasoning_effort: true,
            permission_mode: true,
            args: true,
            redact: true,
        }
    }
}

impl Capabilities {
    /// Keine Modifikation anwendbar.
    pub fn none() -> Self {
        Self {
            model: false,
            reasoning_effort: false,
            permission_mode: false,
            args: false,
            redact: false,
        }
    }

    fn supports(&self, field: &str) -> bool {
        match field {
            "model" => self.model,
            "reasoning_effort" => self.reasoning_effort,
            "permission_mode" => self.permission_mode,
            "args" => self.args,
            "redact" => self.redact,
            _ => false,
        }
    }
}

/// Gemerkte Freigaben (POL-010). Liefert die Grant-ID, wenn für die `ask`-Regel ein
/// gültiger Grant mit diesem Fingerprint existiert ([`crate::grants::SessionGrants`]).
pub trait Grants {
    fn grant(&self, rule: &RuleRef, fingerprint: &str) -> Option<String>;
}

/// Optionen einer Auswertung.
#[derive(Clone, Default)]
pub struct EvalOptions<'a> {
    pub capabilities: Capabilities,
    pub grants: Option<&'a dyn Grants>,
    pub limits: Limits,
    /// Uhr für das Zeitbudget (Default: Systemuhr).
    pub clock: Option<&'a dyn Clock>,
}

/// Verdikt einer Regel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Deny,
    Ask,
    Modify,
    Allow,
    Notify,
    Error,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Deny => "deny",
            Verdict::Ask => "ask",
            Verdict::Modify => "modify",
            Verdict::Allow => "allow",
            Verdict::Notify => "notify",
            Verdict::Error => "error",
        }
    }

    fn of(action: Action) -> Self {
        match action {
            Action::Allow => Verdict::Allow,
            Action::Deny => Verdict::Deny,
            Action::Ask => Verdict::Ask,
            Action::Modify => Verdict::Modify,
            Action::Notify => Verdict::Notify,
        }
    }
}

/// Eine Regel, die gegriffen hat (bzw. bei Dry-Run gegriffen hätte).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    pub rule: RuleRef,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub dry_run: bool,
    /// Teilausdrücke der Bedingung mit Ergebnis (POL-027 AC1); leer ohne `when`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<TracePart>,
    /// Die Spur ist gekürzt: Ihr Gesamtbudget war erschöpft (#353).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub trace_incomplete: bool,
}

/// Ein Grund für `deny` oder `ask`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reason {
    pub rule: RuleRef,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// Approval-Einstellungen der Regel (Kombination: POL-010).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalDoc>,
    /// Grant-Fingerprint einer merkbaren `ask`-Regel in Phase `tool_call` (POL-010):
    /// `remember_key` der Regel, sonst das JSON-Array `[kind, server, name, cwd, command]`
    /// (Shell) bzw. `[kind, server, name, "sha256:…"]` über die kanonischen Argumente.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// Endergebnis.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Allow,
    /// Alle Gründe; der erste stammt von der autoritativsten Ebene.
    Deny {
        reasons: Vec<Reason>,
    },
    /// Genau eine Approval-Card mit allen `ask`-Gründen.
    Ask {
        reasons: Vec<Reason>,
    },
}

/// Eine Modifikation unterhalb der User-Ebene, die Modell bzw. Effort verteuern würde und
/// deshalb eine Freigabe braucht (#286, #353).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostRaise {
    pub rule: RuleRef,
    /// Die verteuernden Felder (`model`, `reasoning_effort`).
    pub fields: Vec<&'static str>,
}

/// Angewendete Modifikationen.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Modifications {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Zusammengeführter JSON-Merge-Patch auf die Tool-Argumente.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Vereinigung aller Redact-Muster (wird zuletzt angewendet).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redact: Vec<String>,
}

impl Modifications {
    pub fn is_empty(&self) -> bool {
        *self == Modifications::default()
    }
}

/// Verworfene Modifikation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    /// `model`, `reasoning_effort`, `permission_mode` oder `/args/<pointer>`.
    pub field: String,
    pub kept: Value,
    pub kept_rule: RuleRef,
    pub discarded: Value,
    pub discarded_rule: RuleRef,
}

/// Default einer Ebene in dieser Phase.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DefaultApplied {
    pub scope: Scope,
    pub phase: Phase,
    pub default: DefaultAction,
    /// Durch ein `allow` derselben Ebene aufgehoben.
    pub lifted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleError {
    pub rule: RuleRef,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Notification {
    pub rule: RuleRef,
    pub channels: Vec<Channel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Effects einer Regel, die angewendet werden (bei `ask` erst nach Auflösung).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingEffects {
    pub rule: RuleRef,
    pub effects: Vec<EffectDoc>,
    pub effects_on: Vec<DefaultAction>,
    /// Die Effects mit ausgewerteten Werten (Kontext der Entscheidung, POL-009).
    pub ops: Vec<StateOp>,
}

/// Ergebnis einer Auswertung mit vollständiger Spur (Explain, Audit).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub phase: Phase,
    #[serde(flatten)]
    pub outcome: Outcome,
    pub modified: Modifications,
    pub matched: Vec<Match>,
    pub would: Vec<Match>,
    pub defaults_applied: Vec<DefaultApplied>,
    pub conflicts: Vec<Conflict>,
    pub errors: Vec<RuleError>,
    pub grants_used: Vec<String>,
    pub notifications: Vec<Notification>,
    pub effects: Vec<PendingEffects>,
    pub warnings: Vec<String>,
    /// Modifikationen unterhalb der User-Ebene, die verteuern würden und deshalb nachfragen
    /// (#286, #353).
    pub cost_raises: Vec<CostRaise>,
    pub policy_set_hash: String,
    pub eval_us: u64,
}

/// Die für eine Session geltenden Sets aller Ebenen.
#[derive(Debug, Clone)]
pub struct EffectiveSet {
    sets: Vec<Arc<LoadedSet>>,
    hash: String,
    invalid: Vec<String>,
    never_valid: Vec<beton_core::event::PolicySetInvalid>,
}

impl EffectiveSet {
    /// Sortiert die Sets kanonisch (Ebene, Name, Herkunft) und berechnet `policy_set_hash`.
    pub fn new(sets: Vec<Arc<LoadedSet>>) -> Self {
        Self::with_invalid(sets, Vec::new())
    }

    /// Wie [`EffectiveSet::new`]; `invalid` nennt Dateien, die gerade nicht geladen werden
    /// können (ihre letzte gültige Version ist enthalten, falls es eine gab).
    pub fn with_invalid(mut sets: Vec<Arc<LoadedSet>>, mut invalid: Vec<String>) -> Self {
        sets.sort_by(|a, b| {
            (a.scope, &a.name, &a.origin, &a.version)
                .cmp(&(b.scope, &b.name, &b.origin, &b.version))
        });
        let mut hasher = Sha256::new();
        // Längenpräfixe statt Trennzeichen; Herkunft und Format gehören dazu, weil sie die
        // Bedeutung eines Sets prägen (z. B. Zustands-Keys von `daily_budget`, #263).
        for s in &sets {
            let format = match s.format {
                crate::load::SourceFormat::Yaml => "yaml",
                crate::load::SourceFormat::Inline => "inline",
            };
            for part in [s.scope.as_str(), &s.name, &s.origin, &s.version, format] {
                hasher.update((part.len() as u64).to_be_bytes());
                hasher.update(part.as_bytes());
            }
        }
        invalid.sort();
        Self {
            sets,
            hash: format!("sha256:{}", hex::encode(hasher.finalize())),
            invalid,
            never_valid: Vec::new(),
        }
    }

    /// Dateien, die noch nie gültig waren, mit Ladefehler (#182: der Runner verweigert dann
    /// den Start, statt ohne diese Datei weiterzulaufen).
    pub fn with_never_valid(mut self, mut files: Vec<beton_core::event::PolicySetInvalid>) -> Self {
        files.sort_by(|a, b| a.file.cmp(&b.file));
        self.never_valid = files;
        self
    }

    /// Dateien ohne je gültige Fassung; sie tragen nichts zum Set bei.
    pub fn never_valid(&self) -> &[beton_core::event::PolicySetInvalid] {
        &self.never_valid
    }

    /// Alle Ausdrücke der Regeln in `phase` (`when` und Hilfsausdrücke, auch Dry-Run).
    pub fn programs(&self, phase: Phase) -> impl Iterator<Item = &crate::cel::Program> {
        self.sets
            .iter()
            .flat_map(|s| s.rules.iter())
            .filter(move |r| r.applies_to(phase))
            .flat_map(|r| {
                let when = match &r.kind {
                    RuleKind::Cel { when: Some(p), .. } => Some(p),
                    _ => None,
                };
                when.into_iter().chain(r.aux())
            })
    }

    /// Gibt es Regeln oder Defaults (≠ `allow`) für `phase`?
    pub fn has_phase(&self, phase: Phase) -> bool {
        self.sets.iter().any(|s| {
            s.rules.iter().any(|r| r.applies_to(phase))
                || s.defaults
                    .get(&phase)
                    .is_some_and(|d| *d != DefaultAction::Allow)
        })
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn sets(&self) -> &[Arc<LoadedSet>] {
        &self.sets
    }

    /// Dateien mit Ladefehler (für Warnungen im Runner, z. B. beim Session-Start).
    pub fn invalid(&self) -> &[String] {
        &self.invalid
    }

    /// Wertet eine Phase aus. Der Kontext bestimmt die Phase.
    pub fn evaluate(&self, ctx: &Activation, opts: &EvalOptions) -> Decision {
        let system = SystemClock::default();
        let clock: &dyn Clock = opts.clock.unwrap_or(&system);
        let started = clock.now_us();
        let mut run = Run::new(self, ctx.phase(), opts, clock);
        let outcome = run.combine(ctx);
        let mut decision = run.decision;
        decision.outcome = outcome;
        decision.eval_us = clock.now_us().saturating_sub(started);
        decision
    }
}

struct Run<'a> {
    set: &'a EffectiveSet,
    phase: Phase,
    opts: &'a EvalOptions<'a>,
    clock: &'a dyn Clock,
    meter: Meter<'a>,
    decision: Decision,
    /// Die geladene Regel je Eintrag in `decision.matched` (gleiche Reihenfolge): Regeltypen
    /// haben mehrere Teile mit derselben ID (POL-011 bis POL-013), die Effects gehören zum
    /// Teil, der gegriffen hat.
    hits: Vec<&'a LoadedRule>,
    /// Regel, deren Modell Pass 1 übernommen hat (erster Schreiber).
    model_writer: Option<RuleRef>,
    /// Spuren, die nach der Entscheidung berechnet werden (#263): (Dry-Run?, Index in
    /// `would` bzw. `matched`, Regel, Pass 2 mit modifiziertem Kontext?).
    pending_traces: Vec<(bool, usize, &'a LoadedRule, bool)>,
}

/// Ein Verdikt mit Herkunft (für die Kombination).
struct Verdicts {
    deny: Vec<Reason>,
    ask: Vec<(Reason, bool)>,
    /// Ebenen mit einer gegriffenen `allow`-Regel.
    allowed_scopes: Vec<Scope>,
}

impl<'a> Run<'a> {
    fn new(
        set: &'a EffectiveSet,
        phase: Phase,
        opts: &'a EvalOptions<'a>,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            set,
            phase,
            opts,
            clock,
            meter: Meter::new(&opts.limits, clock),
            decision: Decision {
                phase,
                outcome: Outcome::Allow,
                modified: Modifications::default(),
                matched: Vec::new(),
                would: Vec::new(),
                defaults_applied: Vec::new(),
                conflicts: Vec::new(),
                errors: Vec::new(),
                grants_used: Vec::new(),
                notifications: Vec::new(),
                effects: Vec::new(),
                warnings: Vec::new(),
                cost_raises: Vec::new(),
                policy_set_hash: set.hash.clone(),
                eval_us: 0,
            },
            hits: Vec::new(),
            model_writer: None,
            pending_traces: Vec::new(),
        }
    }

    /// Regeln dieser Phase in kanonischer Reihenfolge.
    fn rules(&self) -> impl Iterator<Item = (&'a LoadedSet, &'a LoadedRule)> + use<'a> {
        let phase = self.phase;
        self.set
            .sets
            .iter()
            .flat_map(|s| s.rules.iter().map(move |r| (s.as_ref(), r)))
            .filter(move |(_, r)| r.applies_to(phase))
    }

    /// Wertet die Bedingung aus: `Ok(true)` = gegriffen.
    fn hit(&mut self, rule: &LoadedRule, ctx: &Activation) -> Result<bool, EvalError> {
        self.meter.begin_expr()?;
        match &rule.kind {
            RuleKind::Cel { when: Some(p), .. } => p.eval_bool(ctx, &mut self.meter),
            RuleKind::Cel { when: None, .. } => Ok(true),
            RuleKind::Builtin(t) => Err(EvalError::Error(format!(
                "Regeltyp `{}` wird noch nicht ausgewertet",
                t.as_str()
            ))),
        }
    }

    /// Merkt die Spur der Bedingung einer gegriffenen Regel vor (Explain, POL-027 AC1).
    fn defer_trace(&mut self, dry: bool, rule: &'a LoadedRule, modified: bool) {
        let index = if dry {
            self.decision.would.len()
        } else {
            self.decision.matched.len()
        };
        self.pending_traces.push((dry, index, rule, modified));
    }

    /// Spuren nach der Entscheidung (#263): mit eigenem Meter, damit Explain weder das
    /// Zeitbudget der Entscheidung verbraucht noch sie ändern kann. Den Aufwand begrenzen das
    /// Kostenlimit je Teilausdruck und ein Gesamtbudget für Kosten und Zeit (#353); ist es
    /// erschöpft, bleiben die übrigen Spuren leer und alle betroffenen als unvollständig
    /// markiert.
    fn fill_traces(&mut self, original: &Activation, modified: &Activation) {
        let mut meter = Meter::for_trace(&self.opts.limits, self.clock);
        for (dry, index, rule, pass2) in std::mem::take(&mut self.pending_traces) {
            let ctx = if pass2 { modified } else { original };
            let (trace, incomplete) = match &rule.kind {
                _ if meter.exhausted() => (Vec::new(), true),
                RuleKind::Cel { when: Some(p), .. } => {
                    let trace = p.trace(ctx, &mut meter);
                    (trace, meter.exhausted())
                }
                _ => (Vec::new(), false),
            };
            let list = if dry {
                &mut self.decision.would
            } else {
                &mut self.decision.matched
            };
            if let Some(entry) = list.get_mut(index) {
                entry.trace = trace;
                entry.trace_incomplete = incomplete;
            }
        }
    }

    fn deadline_deny(&mut self) -> Outcome {
        let rule = RuleRef {
            scope: Scope::Org,
            set: String::new(),
            set_version: String::new(),
            rule_id: "engine".into(),
        };
        let reason = "Policy-Auswertung überschritt das Zeitbudget (fail closed)".to_owned();
        self.decision.errors.push(RuleError {
            rule: rule.clone(),
            error: reason.clone(),
        });
        Outcome::Deny {
            reasons: vec![Reason {
                rule,
                reason,
                severity: None,
                approval: None,
                fingerprint: None,
            }],
        }
    }

    fn combine(&mut self, original: &Activation) -> Outcome {
        let mut verdicts = Verdicts {
            deny: Vec::new(),
            ask: Vec::new(),
            allowed_scopes: Vec::new(),
        };
        // Pass 1 – Modifikation
        let mut ctx = original.clone();
        match self.pass_modify(&ctx, &mut verdicts) {
            Ok(()) => {}
            Err(_) => return self.deadline_deny(),
        }
        if let Err(e) = self.apply_modifications(&mut ctx) {
            let rule = RuleRef {
                scope: Scope::Org,
                set: String::new(),
                set_version: String::new(),
                rule_id: "engine".into(),
            };
            let reason = format!("Modifikation nicht anwendbar: {e}");
            self.decision.errors.push(RuleError {
                rule: rule.clone(),
                error: reason.clone(),
            });
            verdicts.deny.push(Reason {
                rule,
                reason,
                severity: None,
                approval: None,
                fingerprint: None,
            });
        }
        self.unpriced_route(&mut verdicts);
        // Pass 2 – Entscheidung
        if self.pass_decide(&ctx, &mut verdicts).is_err() {
            return self.deadline_deny();
        }
        // Defaults je Ebene
        self.defaults(&mut verdicts);
        // Grants und Ergebnis
        let mut ask = Vec::new();
        for (mut reason, grantable) in verdicts.ask {
            if grantable && self.phase == Phase::ToolCall {
                reason.fingerprint = self.fingerprint(&reason.rule, &ctx);
            }
            let grant = reason
                .fingerprint
                .as_deref()
                .and_then(|fp| self.opts.grants.and_then(|g| g.grant(&reason.rule, fp)));
            match grant {
                Some(id) => self.decision.grants_used.push(id),
                None => ask.push(reason),
            }
        }
        let outcome = if !verdicts.deny.is_empty() {
            Outcome::Deny {
                reasons: verdicts.deny,
            }
        } else if !ask.is_empty() {
            Outcome::Ask { reasons: ask }
        } else {
            Outcome::Allow
        };
        self.effects(&outcome, &ctx);
        self.fill_traces(original, &ctx);
        outcome
    }

    /// Eine Regel unterhalb der User-Ebene (Projekt, Agent) darf nicht auf ein Modell ohne
    /// Katalogpreis umrouten, solange ein Budget gilt: dessen Kosten wären sonst unbekannt
    /// (POL-011, POL-013; Security-Review #284).
    fn unpriced_route(&mut self, verdicts: &mut Verdicts) {
        let (Some(model), Some(writer)) = (&self.decision.modified.model, &self.model_writer)
        else {
            return;
        };
        if writer.scope <= Scope::User || crate::cel::ext::priced(model) {
            return;
        }
        // Jede Regel, die Kosten liest, wirkt als Budget (auch handgeschriebenes CEL, #297).
        let Some(budget) = crate::budget::first_budget(self.set) else {
            return;
        };
        verdicts.deny.push(Reason {
            rule: writer.clone(),
            reason: format!(
                "Umrouten auf `{model}` abgelehnt: Das Modell hat keinen Katalogpreis, und \
                 solange ein Budget gilt (`{}`), darf eine Regel der Ebene {} nicht darauf \
                 umleiten",
                budget.rule_id,
                writer.scope.as_str()
            ),
            severity: None,
            approval: None,
            fingerprint: None,
        });
    }

    /// Die geladene Regel zu einer Referenz (gleiche Version).
    fn loaded(&self, r: &RuleRef) -> Option<&'a LoadedRule> {
        self.set
            .sets
            .iter()
            .filter(|s| s.scope == r.scope && s.name == r.set && s.version == r.set_version)
            .flat_map(|s| s.rules.iter())
            .find(|rule| rule.id == r.rule_id)
    }

    /// Grant-Fingerprint einer `ask`-Regel (POL-010). `None`, wenn `remember_key` scheitert
    /// oder keinen Text liefert: dann ist die Regel nicht merkbar.
    fn fingerprint(&mut self, r: &RuleRef, ctx: &Activation) -> Option<String> {
        let rule = self.loaded(r)?;
        if let Some(program) = &rule.remember_key {
            self.meter.begin_expr().ok()?;
            return match program.eval(ctx, &mut self.meter) {
                Ok(crate::value::Value::String(s)) => Some(s.to_string()),
                Ok(_) => None,
                Err(e) => {
                    self.decision.errors.push(RuleError {
                        rule: r.clone(),
                        error: format!("remember_key: {e}"),
                    });
                    None
                }
            };
        }
        let text = |field: &str| match ctx.field("tool", field) {
            Some(crate::value::Value::String(s)) if !s.is_empty() => Some(s.to_string()),
            _ => None,
        };
        // Bindung an Klasse, Server und Tool (Security-Review B1): ein Grant gilt nie für ein
        // anderes Tool mit denselben Argumenten. Die Teile stehen als JSON-Array im
        // Fingerprint, damit kein Trennzeichen in einem Teil zwei Calls gleich macht (#242).
        let mut parts = vec![
            text("kind").unwrap_or_else(|| "other".into()),
            text("server").unwrap_or_else(|| "builtin".into()),
            text("name").unwrap_or_default(),
        ];
        if let Some(command) = text("command") {
            // Shell: der ausgeführte Befehl im selben Arbeitsverzeichnis.
            parts.push(text("cwd").unwrap_or_default());
            parts.push(command);
        } else {
            let args = ctx
                .field("tool", "args")
                .map(|a| a.to_json())
                .unwrap_or(Value::Null);
            let canonical = serde_json::to_vec(&canonical_json(&args)).unwrap_or_default();
            parts.push(format!(
                "sha256:{}",
                hex::encode(Sha256::digest(&canonical))
            ));
        }
        serde_json::to_string(&parts).ok()
    }

    fn is_dry(set: &LoadedSet, rule: &LoadedRule) -> bool {
        set.mode == Mode::DryRun || rule.doc.dry_run
    }

    fn reason(set: &LoadedSet, rule: &LoadedRule, fallback: &str) -> Reason {
        Reason {
            rule: set.rule_ref(&rule.id),
            reason: rule
                .doc
                .reason
                .clone()
                .unwrap_or_else(|| fallback.to_owned()),
            severity: rule.doc.severity,
            approval: rule.doc.approval.clone(),
            fingerprint: None,
        }
    }

    /// `notify.message` mit ausgewerteten `{{ … }}`-Ausdrücken (POL-002). Ein fehlerhafter
    /// Ausdruck erscheint als `?` und wird als Fehler protokolliert.
    fn render(
        &mut self,
        set: &LoadedSet,
        rule: &LoadedRule,
        text: &str,
        ctx: &Activation,
    ) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut exprs = rule.notify_exprs.iter();
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else { break };
            out.push_str(&rest[..start]);
            let value = exprs.next().map(|p| {
                self.meter
                    .begin_expr()
                    .and_then(|()| p.eval(ctx, &mut self.meter))
            });
            match value {
                Some(Ok(v)) => out.push_str(&display(&v)),
                Some(Err(e)) => {
                    self.record_error(set, rule, &e);
                    out.push('?');
                }
                None => out.push('?'),
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn record_error(&mut self, set: &LoadedSet, rule: &LoadedRule, e: &EvalError) {
        self.decision.errors.push(RuleError {
            rule: set.rule_ref(&rule.id),
            error: e.to_string(),
        });
    }

    fn pass_modify(&mut self, ctx: &Activation, verdicts: &mut Verdicts) -> Result<(), EvalError> {
        let mut written: BTreeMap<&'static str, (Value, RuleRef)> = BTreeMap::new();
        let mut leaves: Vec<(String, Value, RuleRef)> = Vec::new();
        for (set, rule) in self.rules().collect::<Vec<_>>() {
            let RuleKind::Cel {
                action: Action::Modify,
                ..
            } = &rule.kind
            else {
                continue;
            };
            let dry = Self::is_dry(set, rule);
            let hit = match self.hit(rule, ctx) {
                Ok(h) => h,
                Err(e) if e == EvalError::Deadline => return Err(e),
                Err(e) => {
                    self.record_error(set, rule, &e);
                    if !dry {
                        // Eine fehlerhafte Modifikation ist nicht sicher anwendbar: fail closed.
                        verdicts.deny.push(Self::reason(
                            set,
                            rule,
                            "Modifikations-Regel fehlerhaft",
                        ));
                    }
                    continue;
                }
            };
            if !hit {
                continue;
            }
            self.defer_trace(dry, rule, false);
            let entry = Match {
                rule: set.rule_ref(&rule.id),
                verdict: Verdict::Modify,
                reason: rule.doc.reason.clone(),
                dry_run: dry,
                trace: Vec::new(),
                trace_incomplete: false,
            };
            if dry {
                self.decision.would.push(entry);
                continue;
            }
            self.decision.matched.push(entry);
            self.hits.push(rule);
            let Some(m) = &rule.doc.modify else { continue };
            // Auf den ohnehin geltenden Wert zu ändern ist keine Änderung und braucht keine
            // Fähigkeit des Harness (#283); solche Felder entfallen.
            let noop: Vec<&str> = m
                .fields()
                .into_iter()
                .filter(|f| !self.opts.capabilities.supports(f) && self.unchanged(m, f, ctx))
                .collect();
            let unsupported: Vec<&str> = m
                .fields()
                .into_iter()
                .filter(|f| !self.opts.capabilities.supports(f) && !noop.contains(f))
                .collect();
            if !unsupported.is_empty() {
                let what = format!(
                    "Harness kann `{}` in Phase {} nicht ändern",
                    unsupported.join("`, `"),
                    self.phase
                );
                // Die Begründung der Regel allein erklärt nicht, warum abgelehnt wird.
                let explained = |mut r: Reason| {
                    if r.reason != what {
                        r.reason = format!("{} ({what})", r.reason);
                    }
                    r
                };
                match rule.doc.on_unsupported.unwrap_or(OnUnsupported::Deny) {
                    OnUnsupported::Deny => verdicts
                        .deny
                        .push(explained(Self::reason(set, rule, &what))),
                    OnUnsupported::Ask => verdicts
                        .ask
                        .push((explained(Self::reason(set, rule, &what)), true)),
                    OnUnsupported::Skip => self
                        .decision
                        .warnings
                        .push(format!("{}: {what}; Modifikation verworfen", rule.id)),
                }
                continue;
            }
            let me = set.rule_ref(&rule.id);
            // Unterhalb der User-Ebene nur verschärfen (POL-007, #286): Ein teureres Modell oder
            // ein höherer Effort gilt erst nach Freigabe.
            if set.scope > Scope::User
                && let Some((why, fields)) = self.raises_cost(set.scope, m, &written, ctx)
            {
                let mut r = Self::reason(set, rule, &why);
                if r.reason != why {
                    r.reason = format!("{} ({why})", r.reason);
                }
                verdicts.ask.push((r, false));
                self.decision.cost_raises.push(CostRaise {
                    rule: me.clone(),
                    fields,
                });
            }
            for (field, value) in [
                ("model", &m.model),
                ("reasoning_effort", &m.reasoning_effort),
                ("permission_mode", &m.permission_mode),
            ] {
                let Some(value) = value else { continue };
                if noop.contains(&field) {
                    continue;
                }
                let value = Value::String(value.clone());
                match written.get(field) {
                    None => {
                        written.insert(field, (value, me.clone()));
                    }
                    Some((kept, _)) if *kept == value => {}
                    Some((kept, kept_rule)) => self.decision.conflicts.push(Conflict {
                        field: field.into(),
                        kept: kept.clone(),
                        kept_rule: kept_rule.clone(),
                        discarded: value,
                        discarded_rule: me.clone(),
                    }),
                }
            }
            if let Some(patch) = &m.args {
                let mut mine = Vec::new();
                flatten("", patch, &mut mine);
                for (pointer, value) in mine {
                    let overlap = leaves.iter().find(|(p, _, _)| overlaps(p, &pointer));
                    match overlap {
                        None => leaves.push((pointer, value, me.clone())),
                        Some((p, v, _)) if *p == pointer && *v == value => {}
                        Some((_, kept, kept_rule)) => self.decision.conflicts.push(Conflict {
                            field: format!("/args{pointer}"),
                            kept: kept.clone(),
                            kept_rule: kept_rule.clone(),
                            discarded: value,
                            discarded_rule: me.clone(),
                        }),
                    }
                }
            }
            for pattern in m.redact.iter().flatten() {
                if !self.decision.modified.redact.contains(pattern) {
                    self.decision.modified.redact.push(pattern.clone());
                }
            }
        }
        self.model_writer = written.get("model").map(|(_, r)| r.clone());
        let text = |f: &str| {
            written
                .get(f)
                .and_then(|(v, _)| v.as_str().map(str::to_owned))
        };
        self.decision.modified.model = text("model");
        self.decision.modified.reasoning_effort = text("reasoning_effort");
        self.decision.modified.permission_mode = text("permission_mode");
        if !leaves.is_empty() {
            let mut patch = Value::Object(Map::new());
            for (pointer, value, _) in &leaves {
                set_pointer(&mut patch, pointer, value.clone());
            }
            self.decision.modified.args = Some(patch);
        }
        Ok(())
    }

    /// Überträgt Pass-1-Ergebnisse in den Kontext (Pass 2 sieht das modifizierte Subjekt).
    /// Würde die Modifikation einer Regel Modell oder Effort gegenüber der Anfrage verteuern
    /// (#286)? Felder, die schon eine autoritativere Regel geschrieben hat, zählen nicht (sie
    /// werden ohnehin verworfen). `Some(begründung)`, wenn nicht nachweisbar günstiger.
    /// Gilt der Wert der Modifikation für `field` (`model`, `reasoning_effort`,
    /// `permission_mode`) ohnehin schon (`start.*` bzw. `request.*`)? Ohne bekannten Wert
    /// nie.
    fn unchanged(&self, m: &crate::doc::ModifyDoc, field: &str, ctx: &Activation) -> bool {
        let object = match self.phase {
            Phase::SessionStart => "start",
            Phase::ModelRequest => "request",
            _ => return false,
        };
        let wanted = match field {
            "model" => &m.model,
            "reasoning_effort" => &m.reasoning_effort,
            "permission_mode" => &m.permission_mode,
            _ => return false,
        };
        match (wanted, ctx.field(object, field)) {
            (Some(w), Some(crate::value::Value::String(current))) => {
                !w.is_empty() && **w == *current.as_ref()
            }
            _ => false,
        }
    }

    fn raises_cost(
        &self,
        scope: Scope,
        m: &crate::doc::ModifyDoc,
        written: &BTreeMap<&'static str, (Value, RuleRef)>,
        ctx: &Activation,
    ) -> Option<(String, Vec<&'static str>)> {
        let object = match self.phase {
            Phase::SessionStart => "start",
            Phase::ModelRequest => "request",
            _ => return None,
        };
        let mut fields = Vec::new();
        let current = |field: &str| match ctx.field(object, field) {
            Some(crate::value::Value::String(s)) if !s.is_empty() => Some(s.to_string()),
            _ => None,
        };
        let shown = |v: &Option<String>| v.as_deref().unwrap_or("unbekannt").to_owned();
        let mut why = Vec::new();
        if let Some(to) = &m.model
            && !written.contains_key("model")
        {
            let from = current("model");
            if !crate::budget::model_not_more_expensive(from.as_deref(), to) {
                fields.push("model");
                why.push(format!(
                    "Modell `{}` → `{to}` ist laut Preis-Katalog nicht nachweisbar günstiger",
                    shown(&from)
                ));
            }
        }
        if let Some(to) = &m.reasoning_effort
            && !written.contains_key("reasoning_effort")
        {
            let from = current("reasoning_effort");
            if !crate::budget::effort_not_higher(from.as_deref(), to) {
                fields.push("reasoning_effort");
                why.push(format!(
                    "Effort `{}` → `{to}` ist nicht nachweisbar niedriger",
                    shown(&from)
                ));
            }
        }
        (!why.is_empty()).then(|| {
            (
                format!(
                    "Regel der Ebene {} würde verteuern: {}; Freigabe nötig",
                    scope.as_str(),
                    why.join("; ")
                ),
                fields,
            )
        })
    }

    fn apply_modifications(&self, ctx: &mut Activation) -> Result<(), String> {
        let m = &self.decision.modified;
        let object = match self.phase {
            Phase::SessionStart => Some("start"),
            Phase::ModelRequest => Some("request"),
            _ => None,
        };
        if let Some(object) = object {
            for (field, value) in [
                ("model", &m.model),
                ("reasoning_effort", &m.reasoning_effort),
                ("permission_mode", &m.permission_mode),
            ] {
                if let Some(v) = value {
                    ctx.set_field(object, field, &Value::String(v.clone()))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        if let Some(patch) = &m.args {
            let mut args = ctx
                .field("tool", "args")
                .map(|v| v.to_json())
                .filter(Value::is_object)
                .unwrap_or_else(|| Value::Object(Map::new()));
            merge_patch(&mut args, patch);
            ctx.set_field("tool", "args", &args)
                .map_err(|e| e.to_string())?;
            // Abgeleitete Felder aus den neuen Argumenten (#182); ohne Herkunft (z. B. in
            // Policy-Tests) bleiben sie, wie sie sind.
            if let Some(source) = ctx.tool_source().cloned() {
                let tool = crate::normalize::normalize_source(&source, &args)
                    .map_err(|e| e.to_string())?;
                for (field, value) in [
                    ("command", serde_json::json!(tool.command)),
                    ("argv", serde_json::json!(tool.argv)),
                    ("cwd", serde_json::json!(tool.cwd)),
                    ("paths", serde_json::json!(tool.paths)),
                    ("write_paths", serde_json::json!(tool.write_paths)),
                    ("urls", serde_json::json!(tool.urls)),
                    ("parse_uncertain", serde_json::json!(tool.parse_uncertain)),
                ] {
                    ctx.set_field("tool", field, &value)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }

    fn pass_decide(&mut self, ctx: &Activation, verdicts: &mut Verdicts) -> Result<(), EvalError> {
        for (set, rule) in self.rules().collect::<Vec<_>>() {
            let action = match &rule.kind {
                RuleKind::Cel {
                    action: Action::Modify,
                    ..
                } => continue,
                RuleKind::Cel { action, .. } => *action,
                // Noch nicht entzuckerte Regeltypen gelten als deny-Regel (fail closed).
                RuleKind::Builtin(_) => Action::Deny,
            };
            let dry = Self::is_dry(set, rule);
            let hit = match self.hit(rule, ctx) {
                Ok(h) => h,
                Err(e) if e == EvalError::Deadline => return Err(e),
                Err(e) => {
                    self.record_error(set, rule, &e);
                    if dry {
                        continue;
                    }
                    match action {
                        Action::Deny | Action::Ask => verdicts.deny.push(Self::reason(
                            set,
                            rule,
                            &format!("Regel `{}` fehlerhaft: {e}", rule.id),
                        )),
                        _ => {}
                    }
                    continue;
                }
            };
            if !hit {
                continue;
            }
            self.defer_trace(dry, rule, true);
            let entry = Match {
                rule: set.rule_ref(&rule.id),
                verdict: Verdict::of(action),
                reason: rule.doc.reason.clone(),
                dry_run: dry,
                trace: Vec::new(),
                trace_incomplete: false,
            };
            if dry {
                self.decision.would.push(entry);
                continue;
            }
            self.decision.matched.push(entry);
            self.hits.push(rule);
            match action {
                Action::Deny => verdicts.deny.push(Self::reason(
                    set,
                    rule,
                    &format!("Regel `{}` lehnt ab", rule.id),
                )),
                Action::Ask => verdicts.ask.push((
                    Self::reason(set, rule, &format!("Regel `{}` verlangt Freigabe", rule.id)),
                    true,
                )),
                Action::Allow => {
                    if !verdicts.allowed_scopes.contains(&set.scope) {
                        verdicts.allowed_scopes.push(set.scope);
                    }
                }
                Action::Notify => {
                    let notify = rule.doc.notify.clone();
                    let message = notify
                        .as_ref()
                        .and_then(|n| n.message.as_deref())
                        .map(|m| self.render(set, rule, m, ctx));
                    self.decision.notifications.push(Notification {
                        rule: set.rule_ref(&rule.id),
                        channels: notify
                            .as_ref()
                            .map(|n| n.channels.clone())
                            .filter(|c| !c.is_empty())
                            .unwrap_or_else(|| vec![Channel::Inbox]),
                        message,
                    });
                }
                Action::Modify => {}
            }
        }
        Ok(())
    }

    fn defaults(&mut self, verdicts: &mut Verdicts) {
        // Strengster Default je Ebene (über alle Sets der Ebene, ohne Dry-Run-Sets).
        let mut per_scope: BTreeMap<Scope, (DefaultAction, &LoadedSet)> = BTreeMap::new();
        for set in &self.set.sets {
            if set.mode == Mode::DryRun {
                continue;
            }
            let Some(d) = set.defaults.get(&self.phase) else {
                continue;
            };
            match per_scope.get(&set.scope) {
                Some((existing, _)) if existing >= d => {}
                _ => {
                    per_scope.insert(set.scope, (*d, set));
                }
            }
        }
        for (scope, (default, set)) in per_scope {
            if default == DefaultAction::Allow {
                continue;
            }
            let lifted = verdicts.allowed_scopes.contains(&scope);
            self.decision.defaults_applied.push(DefaultApplied {
                scope,
                phase: self.phase,
                default,
                lifted,
            });
            if lifted {
                continue;
            }
            // Begründung eines Regeltyps (`deny_reason` einer Allowlist, POL-015), sonst
            // allgemein.
            let text = set.type_defaults.get(&self.phase).map(|d| d.reason.clone());
            let reason = Reason {
                rule: set.rule_ref(&format!("defaults.{}", self.phase)),
                reason: text.unwrap_or_else(|| {
                    format!(
                        "Default der Ebene {} für {}: {}",
                        scope.as_str(),
                        self.phase,
                        if default == DefaultAction::Deny {
                            "deny"
                        } else {
                            "ask"
                        }
                    )
                }),
                severity: None,
                approval: None,
                fingerprint: None,
            };
            if default == DefaultAction::Deny {
                verdicts.deny.push(reason);
            } else {
                verdicts.ask.push((reason, false));
            }
        }
        // Gründe kanonisch ordnen: Ebene, Set, Reihenfolge der Regeln (Defaults zuletzt).
        let order = |r: &Reason| (r.rule.scope, r.rule.set.clone());
        verdicts.deny.sort_by_key(order);
        verdicts.ask.sort_by_key(|(r, _)| order(r));
    }

    fn effects(&mut self, outcome: &Outcome, ctx: &Activation) {
        let final_action = match outcome {
            Outcome::Allow => DefaultAction::Allow,
            Outcome::Deny { .. } => DefaultAction::Deny,
            Outcome::Ask { .. } => DefaultAction::Ask,
        };
        let matched: Vec<(RuleRef, &'a LoadedRule)> = self
            .decision
            .matched
            .iter()
            .map(|m| m.rule.clone())
            .zip(self.hits.iter().copied())
            .collect();
        for (rule_ref, rule) in matched {
            if rule.doc.effects.is_empty() {
                continue;
            }
            let effects_on = rule
                .doc
                .effects_on
                .clone()
                .unwrap_or_else(|| vec![DefaultAction::Allow, DefaultAction::Deny]);
            // Bei `ask` entscheidet erst die Auflösung (POL-009).
            if final_action == DefaultAction::Ask || effects_on.contains(&final_action) {
                let mut ops = Vec::with_capacity(rule.doc.effects.len());
                for (effect, program) in rule.doc.effects.iter().zip(&rule.effect_values) {
                    let value = match program {
                        None => None,
                        Some(p) => {
                            let evaluated = self
                                .meter
                                .begin_expr()
                                .and_then(|()| p.eval(ctx, &mut self.meter));
                            match evaluated {
                                Ok(v) => Some(v.to_json()),
                                Err(e) => {
                                    // Ohne Wert unterbleibt die Änderung; die Entscheidung
                                    // bleibt davon unberührt.
                                    self.decision.errors.push(RuleError {
                                        rule: rule_ref.clone(),
                                        error: format!("effects.{}: {e}", effect.key),
                                    });
                                    continue;
                                }
                            }
                        }
                    };
                    ops.push(StateOp {
                        scope: effect.scope,
                        key: effect.key.clone(),
                        op: effect.op,
                        value,
                    });
                }
                self.decision.effects.push(PendingEffects {
                    rule: rule_ref,
                    effects: rule.doc.effects.clone(),
                    effects_on,
                    ops,
                });
            }
        }
    }
}

/// Darstellung eines Werts in `notify.message`: Text ohne Anführungszeichen, sonst JSON.
fn display(v: &crate::value::Value) -> String {
    match v.to_json() {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

/// Kanonisches JSON: Objekt-Schlüssel sortiert (unabhängig von der Eingabereihenfolge).
fn canonical_json(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<String, Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), canonical_json(v)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// Zerlegt einen Merge-Patch in Blätter (`/a/b` → Wert); Objekte werden rekursiv zerlegt,
/// alles andere (auch `null` = löschen, Listen) ist ein Blatt.
fn flatten(prefix: &str, patch: &Value, out: &mut Vec<(String, Value)>) {
    match patch {
        Value::Object(map) if !prefix.is_empty() || !map.is_empty() => {
            if map.is_empty() {
                out.push((prefix.to_owned(), patch.clone()));
            }
            for (k, v) in map {
                let key = k.replace('~', "~0").replace('/', "~1");
                flatten(&format!("{prefix}/{key}"), v, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

/// Berühren sich zwei Pointer (gleich oder einer ist Präfix des anderen)?
fn overlaps(a: &str, b: &str) -> bool {
    a == b || b.starts_with(&format!("{a}/")) || a.starts_with(&format!("{b}/"))
}

fn set_pointer(target: &mut Value, pointer: &str, value: Value) {
    let mut cur = target;
    let parts: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|p| p.replace("~1", "/").replace("~0", "~"))
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let Value::Object(map) = cur else { return };
        if i == parts.len() - 1 {
            map.insert(part.clone(), value);
            return;
        }
        cur = map
            .entry(part.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

/// JSON-Merge-Patch (RFC 7396).
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch_map) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(map) = target else { return };
    for (k, v) in patch_map {
        if v.is_null() {
            map.remove(k);
        } else {
            merge_patch(map.entry(k.clone()).or_insert(Value::Null), v);
        }
    }
}
