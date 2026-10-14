//! Gemerkte Freigaben einer Session (`approve_session`, POL-010).
//!
//! Ein Grant erfüllt genau eine `ask`-Regel (Ebene, Set, Regel-ID) für einen Fingerprint
//! (`remember_key` der Regel bzw. `tool.kind:tool.command` oder Hash der Argumente). Er gilt
//! unabhängig von der Version des Sets, solange die Regel dieselbe ID behält; ein anderer
//! Befehl oder eine andere Regel erzeugt wieder eine Card.

use crate::engine::Grants;
use crate::load::{RuleRef, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub scope: Scope,
    pub set: String,
    pub rule_id: String,
    pub fingerprint: String,
}

/// Grants einer Session (im Runner, aus `approval.resolved { remember: session }`).
#[derive(Debug, Clone, Default)]
pub struct SessionGrants {
    grants: Vec<Grant>,
}

impl SessionGrants {
    /// Merkt eine Freigabe und liefert die Grant-ID.
    pub fn add(&mut self, rule: &RuleRef, fingerprint: &str) -> String {
        let id = format!("grt_{}", ulid::Ulid::generate());
        self.add_with_id(rule, fingerprint, &id);
        id
    }

    /// Wie [`SessionGrants::add`] mit vorgegebener ID (Replay aus dem Log).
    pub fn add_with_id(&mut self, rule: &RuleRef, fingerprint: &str, id: &str) {
        if self.find(rule, fingerprint).is_some() {
            return;
        }
        self.grants.push(Grant {
            id: id.to_owned(),
            scope: rule.scope,
            set: rule.set.clone(),
            rule_id: rule.rule_id.clone(),
            fingerprint: fingerprint.to_owned(),
        });
    }

    fn find(&self, rule: &RuleRef, fingerprint: &str) -> Option<&Grant> {
        self.grants.iter().find(|g| {
            g.scope == rule.scope
                && g.set == rule.set
                && g.rule_id == rule.rule_id
                && g.fingerprint == fingerprint
        })
    }

    pub fn all(&self) -> &[Grant] {
        &self.grants
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

impl Grants for SessionGrants {
    fn grant(&self, rule: &RuleRef, fingerprint: &str) -> Option<String> {
        self.find(rule, fingerprint).map(|g| g.id.clone())
    }
}
