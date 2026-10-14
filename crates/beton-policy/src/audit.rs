//! Audit jeder Auswertung als `policy.decision` (POL-025).
//!
//! Eingaben werden nicht dupliziert: Das Event verweist über `subject_ref` auf das Subjekt
//! (z. B. `call_id` und `event_id` des `tool.call.requested`) und trägt nur dessen Hash
//! (`subject_hash`), nie Tool-Argumente im Klartext. Dry-Run-Treffer stehen mit
//! `dry_run: true` in `matched` und beeinflussen `outcome` nicht.

use beton_core::event::{PolicyDecision, PolicyOutcome};
use beton_core::id::ApprovalId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::enforcement::Level;
use crate::engine::{Decision, Match, Outcome};

/// Neue Entscheidungs-ID (`pd_<ULID>`).
pub fn new_decision_id() -> String {
    format!("pd_{}", ulid::Ulid::generate())
}

/// `sha256:<hex>` über das kanonische JSON (Schlüssel sortiert) des Subjekts.
pub fn subject_hash(subject: &Value) -> String {
    let bytes = serde_json::to_vec(subject).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
}

/// Was neben der Entscheidung ins Event gehört.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionMeta {
    pub decision_id: String,
    /// Erreichte Durchsetzung dieser Auswertung (Enforcement-Matrix).
    pub enforcement: Level,
    pub subject_ref: Value,
    pub subject_hash: Option<String>,
    /// Approval-Card einer `ask`-Entscheidung.
    pub approval_id: Option<ApprovalId>,
}

impl DecisionMeta {
    pub fn new(enforcement: Level) -> Self {
        Self {
            decision_id: new_decision_id(),
            enforcement,
            subject_ref: Value::Null,
            subject_hash: None,
            approval_id: None,
        }
    }
}

fn matched(m: &Match) -> Value {
    let mut v = json!({
        "rule_id": m.rule.rule_id,
        "scope": m.rule.scope.as_str(),
        "set": m.rule.set,
        "set_version": m.rule.set_version,
        "verdict": m.verdict.as_str(),
        "dry_run": m.dry_run,
    });
    if let Some(reason) = &m.reason {
        v["reason"] = Value::String(reason.clone());
    }
    // Teilausdrücke mit Wahrheitswerten (Explain, POL-027 AC1); keine Kontextwerte.
    if !m.trace.is_empty() {
        v["trace"] = serde_json::to_value(&m.trace).unwrap_or(Value::Null);
    }
    v
}

/// Baut das `policy.decision`-Event.
pub fn decision_event(decision: &Decision, meta: DecisionMeta) -> PolicyDecision {
    let outcome = match decision.outcome {
        Outcome::Allow => PolicyOutcome::Allow,
        Outcome::Deny { .. } => PolicyOutcome::Deny,
        Outcome::Ask { .. } => PolicyOutcome::Ask,
    };
    PolicyDecision {
        decision_id: meta.decision_id,
        phase: decision.phase.as_str().to_owned(),
        subject_ref: meta.subject_ref,
        subject_hash: meta.subject_hash,
        outcome,
        modified: (!decision.modified.is_empty())
            .then(|| serde_json::to_value(&decision.modified).unwrap_or(Value::Null)),
        approval_id: meta.approval_id,
        matched: decision
            .matched
            .iter()
            .chain(decision.would.iter())
            .map(matched)
            .collect(),
        defaults_applied: decision
            .defaults_applied
            .iter()
            .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
            .collect(),
        conflicts: decision
            .conflicts
            .iter()
            .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
            .collect(),
        errors: decision
            .errors
            .iter()
            .map(|e| {
                json!({
                    "rule_id": e.rule.rule_id,
                    "scope": e.rule.scope.as_str(),
                    "set": e.rule.set,
                    "error": e.error,
                })
            })
            .collect(),
        grants_used: decision
            .grants_used
            .iter()
            .map(|g| Value::String(g.clone()))
            .collect(),
        enforcement: meta.enforcement.as_str().to_owned(),
        policy_set_hash: decision.policy_set_hash.clone(),
        eval_us: decision.eval_us,
    }
}
