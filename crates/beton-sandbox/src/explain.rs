//! Ausgabe der aufgelösten Policy (SBX-003 AC5): Wurzeln, Masken, Environment-**Namen**
//! (nie Werte) und Regeln.

use serde_json::{Value, json};

use crate::resolve::ResolvedPolicy;

/// Die aufgelöste Policy als JSON. Environment-Werte erscheinen nie, nur die Namen.
pub fn explain(policy: &ResolvedPolicy) -> Value {
    let mut v = serde_json::to_value(policy).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        let names: Vec<&String> = policy.env.keys().collect();
        map.insert("env".into(), json!(names));
    }
    v
}
