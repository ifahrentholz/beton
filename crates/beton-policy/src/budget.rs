//! Budgets und Modell-Routing (POL-011 `spend_cap`, POL-012 `daily_budget`, POL-013
//! `model_route`).

use beton_core::event::BudgetAction;

use crate::engine::{Decision, EffectiveSet};
use crate::load::RuleRef;

/// Kostenbereich einer Budget-Regel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetScope {
    Session,
    Run,
    Subtree,
    User,
}

/// Eine Budget-Regel des effektiven Sets.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetRule {
    pub rule: RuleRef,
    pub scope: BudgetScope,
}

/// Ein erreichtes Limit in einer Entscheidung.
#[derive(Debug, Clone, PartialEq)]
pub struct Exhausted {
    pub rule: RuleRef,
    pub scope: BudgetScope,
    pub action: BudgetAction,
}

/// Budget-Regeln des Sets.
pub fn budget_rules(_set: &EffectiveSet) -> Vec<BudgetRule> {
    Vec::new()
}

/// Erreichte Limits einer Entscheidung.
pub fn exhausted(_decision: &Decision, _set: &EffectiveSet) -> Vec<Exhausted> {
    Vec::new()
}
