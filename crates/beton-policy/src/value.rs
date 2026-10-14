//! Laufzeitwerte der CEL-Auswertung.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use time::OffsetDateTime;

/// Ein CEL-Wert. Listen und Maps sind geteilt (`Arc`), Kopien sind billig.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Double(f64),
    String(Arc<str>),
    Bytes(Arc<[u8]>),
    List(Arc<Vec<Value>>),
    Map(Arc<BTreeMap<Key, Value>>),
    Timestamp(OffsetDateTime),
    Duration(time::Duration),
}

/// Schlüssel einer Map.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Bool(bool),
    Int(i64),
    Uint(u64),
    String(Arc<str>),
}

impl Key {
    pub fn to_value(&self) -> Value {
        match self {
            Key::Bool(b) => Value::Bool(*b),
            Key::Int(i) => Value::Int(*i),
            Key::Uint(u) => Value::Uint(*u),
            Key::String(s) => Value::String(s.clone()),
        }
    }

    pub fn from_value(v: &Value) -> Option<Key> {
        match v {
            Value::Bool(b) => Some(Key::Bool(*b)),
            Value::Int(i) => Some(Key::Int(*i)),
            Value::Uint(u) => Some(Key::Uint(*u)),
            Value::String(s) => Some(Key::String(s.clone())),
            _ => None,
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Uint(_) => "uint",
            Value::Double(_) => "double",
            Value::String(_) => "string",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Timestamp(_) => "timestamp",
            Value::Duration(_) => "duration",
        }
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Arc::new(items))
    }

    /// Ein JSON-Wert ohne Typdeklaration (`dyn`): ganze Zahlen als `int` (bzw. `uint`
    /// jenseits von `i64`), Brüche als `double`.
    pub fn from_json(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::Uint(u)
                } else {
                    Value::Double(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            serde_json::Value::String(s) => Value::String(s.as_str().into()),
            serde_json::Value::Array(items) => {
                Value::list(items.iter().map(Value::from_json).collect())
            }
            serde_json::Value::Object(map) => Value::Map(Arc::new(
                map.iter()
                    .map(|(k, v)| (Key::String(k.as_str().into()), Value::from_json(v)))
                    .collect(),
            )),
        }
    }

    /// Zurück nach JSON (für Explain und Modifikationen). Zeitwerte als RFC 3339 bzw.
    /// Sekunden.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Int(i) => J::from(*i),
            Value::Uint(u) => J::from(*u),
            Value::Double(d) => serde_json::Number::from_f64(*d).map_or(J::Null, J::Number),
            Value::String(s) => J::String(s.to_string()),
            Value::Bytes(b) => J::Array(b.iter().map(|x| J::from(*x)).collect()),
            Value::List(items) => J::Array(items.iter().map(Value::to_json).collect()),
            Value::Map(map) => J::Object(
                map.iter()
                    .map(|(k, v)| {
                        let key = match k {
                            Key::String(s) => s.to_string(),
                            other => other.to_value().to_string(),
                        };
                        (key, v.to_json())
                    })
                    .collect(),
            ),
            Value::Timestamp(t) => J::String(
                t.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
            ),
            Value::Duration(d) => J::from(d.as_seconds_f64()),
        }
    }
}

impl PartialEq for Value {
    /// CEL-Gleichheit, Zahlen typübergreifend.
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) => true,
            (Bool(a), Bool(b)) => a == b,
            (String(a), String(b)) => a == b,
            (Bytes(a), Bytes(b)) => a == b,
            (List(a), List(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            (Timestamp(a), Timestamp(b)) => a == b,
            (Duration(a), Duration(b)) => a == b,
            (a, b) => matches!(numeric_cmp(a, b), Some(std::cmp::Ordering::Equal)),
        }
    }
}

/// Typübergreifender Zahlenvergleich (`int`, `uint`, `double`).
pub fn numeric_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use Value::*;
    match (a, b) {
        (Int(x), Int(y)) => Some(x.cmp(y)),
        (Uint(x), Uint(y)) => Some(x.cmp(y)),
        (Double(x), Double(y)) => x.partial_cmp(y),
        (Int(x), Uint(y)) => Some(i128::from(*x).cmp(&i128::from(*y))),
        (Uint(x), Int(y)) => Some(i128::from(*x).cmp(&i128::from(*y))),
        (Int(x), Double(y)) => (*x as f64).partial_cmp(y),
        (Double(x), Int(y)) => x.partial_cmp(&(*y as f64)),
        (Uint(x), Double(y)) => (*x as f64).partial_cmp(y),
        (Double(x), Uint(y)) => x.partial_cmp(&(*y as f64)),
        _ => None,
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.into())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => f.write_str(s),
            Value::Double(d) => write!(f, "{d}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Uint(u) => write!(f, "{u}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Null => f.write_str("null"),
            other => write!(f, "{}", other.to_json()),
        }
    }
}
