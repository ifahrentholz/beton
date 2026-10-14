//! Auswertungskontext: die typisierten, read-only Variablen einer Phase (POL-004).
//!
//! Der Aufrufer liefert den Kontext als JSON-Objekt mit den Variablen der obersten Ebene
//! (`session`, `tool`, …). Jeder Wert wird gegen die Deklaration in [`crate::types`]
//! konvertiert (z. B. ganze Zahl → `double` für `session.cost_usd`). Unbekannte Variablen
//! oder Felder und falsche Typen sind ein Fehler; die Auswertung lehnt dann ab (fail closed).

use std::collections::BTreeMap;
use std::sync::Arc;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Phase;
use crate::cel::ext::{LexicalResolver, PathResolver};
use crate::types::{DECLS, Ty};
use crate::value::{Key, Value};

/// Fehler beim Aufbau des Kontexts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Kontext `{path}`: {message}")]
pub struct ContextError {
    pub path: String,
    pub message: String,
}

/// Variablen einer Auswertung.
#[derive(Clone)]
pub struct Activation {
    phase: Phase,
    vars: BTreeMap<String, Value>,
    resolver: Arc<dyn PathResolver>,
}

impl std::fmt::Debug for Activation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Activation")
            .field("phase", &self.phase)
            .field("vars", &self.vars.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Activation {
    /// Baut den Kontext aus einem JSON-Objekt `{ "session": {…}, "tool": {…} }`.
    pub fn new(phase: Phase, json: serde_json::Value) -> Result<Self, ContextError> {
        let serde_json::Value::Object(map) = json else {
            return Err(err("", "Kontext muss ein JSON-Objekt sein"));
        };
        let mut vars = BTreeMap::new();
        for (name, value) in map {
            let ty = DECLS
                .var(&name, phase)
                .map_err(|_| err(&name, &format!("in Phase {phase} nicht deklariert")))?;
            vars.insert(name.clone(), convert(&value, ty, &name)?);
        }
        Ok(Self {
            phase,
            vars,
            resolver: Arc::new(LexicalResolver),
        })
    }

    /// Ersetzt die Pfadauflösung für `under()` (Default: rein lexikalisch, ohne IO).
    pub fn with_resolver(mut self, resolver: Arc<dyn PathResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn resolver(&self) -> &dyn PathResolver {
        self.resolver.as_ref()
    }

    /// Wert einer Variable der obersten Ebene; nicht gesetzte deklarierte Variablen sind `null`.
    pub fn var(&self, name: &str) -> Value {
        self.vars.get(name).cloned().unwrap_or(Value::Null)
    }

    /// Feld `object.field` (z. B. `request.model`), sofern gesetzt.
    pub fn field(&self, object: &str, field: &str) -> Option<Value> {
        match self.vars.get(object)? {
            Value::Map(m) => m.get(&Key::String(field.into())).cloned(),
            _ => None,
        }
    }

    /// Setzt `object.field` (für das modifizierte Subjekt in Pass 2 der Kombination).
    /// Der Wert wird gegen die Deklaration konvertiert.
    pub fn set_field(
        &mut self,
        object: &str,
        field: &str,
        value: &serde_json::Value,
    ) -> Result<(), ContextError> {
        let obj_ty = DECLS
            .var(object, self.phase)
            .map_err(|_| err(object, "in dieser Phase nicht deklariert"))?
            .clone();
        let Ty::Object(obj_name) = obj_ty else {
            return Err(err(object, "kein Objekt"));
        };
        let path = format!("{object}.{field}");
        let ty = DECLS
            .field(obj_name, field)
            .ok_or_else(|| err(&path, "unbekanntes Feld"))?;
        let converted = convert(value, ty, &path)?;
        let mut map = match self.vars.get(object) {
            Some(Value::Map(m)) => (**m).clone(),
            _ => BTreeMap::new(),
        };
        map.insert(Key::String(field.into()), converted);
        self.vars
            .insert(object.to_owned(), Value::Map(Arc::new(map)));
        Ok(())
    }
}

fn err(path: &str, message: &str) -> ContextError {
    ContextError {
        path: path.to_owned(),
        message: message.to_owned(),
    }
}

/// Konvertiert JSON in einen Wert des deklarierten Typs.
pub(crate) fn convert(v: &serde_json::Value, ty: &Ty, path: &str) -> Result<Value, ContextError> {
    use serde_json::Value as J;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let mismatch = || err(path, &format!("erwartet {ty}, gefunden {v}"));
    Ok(match ty {
        Ty::Dyn => Value::from_json(v),
        Ty::Bool => Value::Bool(v.as_bool().ok_or_else(mismatch)?),
        Ty::String => Value::String(v.as_str().ok_or_else(mismatch)?.into()),
        Ty::Int => Value::Int(v.as_i64().ok_or_else(mismatch)?),
        Ty::Uint => Value::Uint(v.as_u64().ok_or_else(mismatch)?),
        Ty::Double => Value::Double(v.as_f64().ok_or_else(mismatch)?),
        Ty::Null => return Err(mismatch()),
        Ty::Bytes => match v {
            J::String(s) => Value::Bytes(s.as_bytes().into()),
            _ => return Err(mismatch()),
        },
        Ty::Timestamp => {
            let s = v.as_str().ok_or_else(mismatch)?;
            Value::Timestamp(OffsetDateTime::parse(s, &Rfc3339).map_err(|_| mismatch())?)
        }
        Ty::Duration => match v {
            J::Number(n) => Value::Duration(time::Duration::seconds_f64(
                n.as_f64().ok_or_else(mismatch)?,
            )),
            J::String(s) => {
                Value::Duration(crate::cel::ext::parse_duration(s).map_err(|e| err(path, &e))?)
            }
            _ => return Err(mismatch()),
        },
        Ty::List(elem) => {
            let items = v.as_array().ok_or_else(mismatch)?;
            Value::list(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, x)| convert(x, elem, &format!("{path}[{i}]")))
                    .collect::<Result<_, _>>()?,
            )
        }
        Ty::Map(_, val) => {
            let map = v.as_object().ok_or_else(mismatch)?;
            Value::Map(Arc::new(
                map.iter()
                    .map(|(k, x)| {
                        Ok((
                            Key::String(k.as_str().into()),
                            convert(x, val, &format!("{path}.{k}"))?,
                        ))
                    })
                    .collect::<Result<_, ContextError>>()?,
            ))
        }
        Ty::Object(name) => {
            let map = v.as_object().ok_or_else(mismatch)?;
            let mut out = BTreeMap::new();
            for (k, x) in map {
                let field_path = format!("{path}.{k}");
                let fty = DECLS
                    .field(name, k)
                    .ok_or_else(|| err(&field_path, "unbekanntes Feld"))?;
                out.insert(
                    Key::String(k.as_str().into()),
                    convert(x, fty, &field_path)?,
                );
            }
            Value::Map(Arc::new(out))
        }
    })
}
