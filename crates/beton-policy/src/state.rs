//! Zustand und Zähler aus `effects` (POL-009).
//!
//! Regeln pflegen Zustand über `effects` mit den Operationen `set`, `increment`, `append`
//! (Liste, höchstens [`APPEND_MAX`] Elemente) und `delete` in den Scopes `session`, `run`,
//! `user_day` und `user`. Werte sind höchstens [`VALUE_MAX_BYTES`] groß (als JSON).
//!
//! - `session`/`run` liegen im Session-Log als `policy.state_changed { scope, key, op, value }`;
//!   `value` ist der Wert **nach** der Operation (`null` nach `delete`). Damit ist der Zustand
//!   replay- und fork-fähig: ein Fork übernimmt alle Änderungen bis `up_to_seq`
//!   ([`PolicyState::replay`]).
//! - `user_day`/`user` liegen im Store; `user_day` gilt für den Kalendertag in der Zeitzone
//!   des Users ([`day_key`]), an der Tagesgrenze beginnt ein leerer Zustand.
//!
//! Wann Effects angewendet werden, entscheidet [`effects_due`] (Kombinationsalgorithmus,
//! Schritt 7): bei `allow`/`deny`, wenn die Endentscheidung in `effects_on` liegt; bei `ask`
//! erst nach der Auflösung, bei Ablauf (`via: timeout`) nie.

use std::collections::BTreeMap;

use beton_core::event::PolicyStateChanged;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};

use crate::doc::{DefaultAction, EffectOp, StateScope};

/// Höchstzahl der Elemente einer Liste aus `append`; ältere fallen heraus.
pub const APPEND_MAX: usize = 100;
/// Höchstgröße eines Werts (kanonisches JSON) je Key.
pub const VALUE_MAX_BYTES: usize = 4 * 1024;

/// Eine ausgewertete Zustandsänderung einer Regel (Wert aus `effects[].value` berechnet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateOp {
    pub scope: StateScope,
    pub key: String,
    pub op: EffectOp,
    /// Ausgewerteter Wert (`set`, `append`, optional `increment`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Eine angewendete Änderung: der Wert nach der Operation (`None` nach `delete`).
#[derive(Debug, Clone, PartialEq)]
pub struct StateChange {
    pub scope: StateScope,
    pub key: String,
    pub op: EffectOp,
    pub value: Option<Value>,
}

impl StateChange {
    /// Das Event `policy.state_changed`.
    pub fn to_event(&self) -> PolicyStateChanged {
        PolicyStateChanged {
            scope: scope_str(self.scope).to_owned(),
            key: self.key.clone(),
            op: op_str(self.op).to_owned(),
            value: self.value.clone().unwrap_or(Value::Null),
        }
    }

    /// Aus einem Event (unbekannte Scopes oder Operationen: `None`).
    pub fn from_event(e: &PolicyStateChanged) -> Option<Self> {
        let scope = match e.scope.as_str() {
            "session" => StateScope::Session,
            "run" => StateScope::Run,
            "user_day" => StateScope::UserDay,
            "user" => StateScope::User,
            _ => return None,
        };
        let op = match e.op.as_str() {
            "set" => EffectOp::Set,
            "increment" => EffectOp::Increment,
            "append" => EffectOp::Append,
            "delete" => EffectOp::Delete,
            _ => return None,
        };
        Some(Self {
            scope,
            key: e.key.clone(),
            op,
            value: (op != EffectOp::Delete && !e.value.is_null()).then(|| e.value.clone()),
        })
    }
}

pub fn scope_str(scope: StateScope) -> &'static str {
    match scope {
        StateScope::Session => "session",
        StateScope::Run => "run",
        StateScope::UserDay => "user_day",
        StateScope::User => "user",
    }
}

pub fn op_str(op: EffectOp) -> &'static str {
    match op {
        EffectOp::Set => "set",
        EffectOp::Increment => "increment",
        EffectOp::Append => "append",
        EffectOp::Delete => "delete",
    }
}

/// Fehler einer Zustandsänderung; die Änderung unterbleibt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StateError(pub String);

/// Neuer Wert eines Keys nach `op` (`None` = gelöscht).
pub fn next_value(
    current: Option<&Value>,
    op: EffectOp,
    value: Option<Value>,
) -> Result<Option<Value>, StateError> {
    let next = match op {
        EffectOp::Delete => return Ok(None),
        EffectOp::Set => Some(value.ok_or_else(|| StateError("`set` braucht `value`".into()))?),
        EffectOp::Increment => {
            let by = value.unwrap_or_else(|| Value::from(1));
            let cur = current.cloned().unwrap_or_else(|| Value::from(0));
            Some(add(&cur, &by)?)
        }
        EffectOp::Append => {
            let item = value.ok_or_else(|| StateError("`append` braucht `value`".into()))?;
            let mut list = match current {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(other) => {
                    return Err(StateError(format!(
                        "`append` auf einen Wert ohne Liste ({})",
                        kind(other)
                    )));
                }
            };
            list.push(item);
            if list.len() > APPEND_MAX {
                let drop = list.len() - APPEND_MAX;
                list.drain(..drop);
            }
            Some(Value::Array(list))
        }
    };
    if let Some(v) = &next {
        let size = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX);
        if size > VALUE_MAX_BYTES {
            return Err(StateError(format!(
                "Wert zu groß ({size} Bytes, höchstens {VALUE_MAX_BYTES})"
            )));
        }
    }
    Ok(next)
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "Zahl",
        Value::String(_) => "Text",
        Value::Array(_) => "Liste",
        Value::Object(_) => "Objekt",
    }
}

fn add(a: &Value, b: &Value) -> Result<Value, StateError> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(x), Some(y)) = (x.as_i64(), y.as_i64())
                && let Some(sum) = x.checked_add(y)
            {
                return Ok(Value::from(sum));
            }
            match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) => serde_json::Number::from_f64(x + y)
                    .map(Value::Number)
                    .ok_or_else(|| StateError("Ergebnis ist keine endliche Zahl".into())),
                _ => Err(StateError("Zahl außerhalb des Wertebereichs".into())),
            }
        }
        _ => Err(StateError(format!(
            "`increment` braucht Zahlen ({} + {})",
            kind(a),
            kind(b)
        ))),
    }
}

/// Zustand der Scopes `session` und `run` einer Session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolicyState {
    session: BTreeMap<String, Value>,
    run: BTreeMap<String, Value>,
}

impl PolicyState {
    fn map(&self, scope: StateScope) -> Option<&BTreeMap<String, Value>> {
        match scope {
            StateScope::Session => Some(&self.session),
            StateScope::Run => Some(&self.run),
            StateScope::UserDay | StateScope::User => None,
        }
    }

    fn map_mut(&mut self, scope: StateScope) -> Option<&mut BTreeMap<String, Value>> {
        match scope {
            StateScope::Session => Some(&mut self.session),
            StateScope::Run => Some(&mut self.run),
            StateScope::UserDay | StateScope::User => None,
        }
    }

    pub fn get(&self, scope: StateScope, key: &str) -> Option<&Value> {
        self.map(scope)?.get(key)
    }

    /// Wendet eine Operation an und liefert die Änderung fürs Log. Scopes `user_day`/`user`
    /// liegen im Store, nicht hier.
    pub fn apply(
        &mut self,
        scope: StateScope,
        key: &str,
        op: EffectOp,
        value: Option<Value>,
    ) -> Result<StateChange, StateError> {
        let map = self.map_mut(scope).ok_or_else(|| {
            StateError(format!(
                "Scope `{}` liegt im Store, nicht in der Session",
                scope_str(scope)
            ))
        })?;
        let next = next_value(map.get(key), op, value)?;
        match &next {
            Some(v) => map.insert(key.to_owned(), v.clone()),
            None => map.remove(key),
        };
        Ok(StateChange {
            scope,
            key: key.to_owned(),
            op,
            value: next,
        })
    }

    /// Übernimmt eine geloggte Änderung (Replay).
    pub fn restore(&mut self, change: &StateChange) {
        if let Some(map) = self.map_mut(change.scope) {
            match &change.value {
                Some(v) => map.insert(change.key.clone(), v.clone()),
                None => map.remove(&change.key),
            };
        }
    }

    /// Zustand aus geloggten Änderungen (z. B. alle bis `up_to_seq` eines Forks).
    pub fn replay<'a>(changes: impl IntoIterator<Item = &'a StateChange>) -> Self {
        let mut state = Self::default();
        for c in changes {
            state.restore(c);
        }
        state
    }

    /// `state.session` bzw. `state.run` für den Auswertungskontext.
    pub fn snapshot(&self, scope: StateScope) -> BTreeMap<String, Value> {
        self.map(scope).cloned().unwrap_or_default()
    }

    /// Der `run`-Zustand beginnt mit jedem Run neu.
    pub fn reset_run(&mut self) {
        self.run.clear();
    }
}

/// Endentscheidung, nach der Effects angewendet werden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalOutcome {
    /// `allow`, auch nach Freigabe durch einen Menschen.
    Allow,
    /// `deny`, auch nach Ablehnung durch einen Menschen.
    Deny,
    /// Approval abgelaufen oder verworfen (`via: timeout|system`): keine Effects.
    Expired,
}

/// Werden die Effects einer Regel mit `effects_on` bei dieser Endentscheidung angewendet?
pub fn effects_due(effects_on: &[DefaultAction], outcome: FinalOutcome) -> bool {
    match outcome {
        FinalOutcome::Allow => effects_on.contains(&DefaultAction::Allow),
        FinalOutcome::Deny => effects_on.contains(&DefaultAction::Deny),
        FinalOutcome::Expired => false,
    }
}

/// Kalendertag (`JJJJ-MM-TT`) von `now` in der Zeitzone des Users (`user_day`).
pub fn day_key(now: OffsetDateTime, offset: UtcOffset) -> String {
    let d = now.to_offset(offset).date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}
