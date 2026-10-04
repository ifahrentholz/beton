//! Parameterwerte eines Starts (AGT-010): gegen die Deklaration in `params` prüfen, Defaults
//! einsetzen und Texte aus CLI (`--param k=v`) typgerecht umwandeln.
//!
//! Die Prüfung läuft vor dem Start (Server), damit ein ungültiger Wert nie einen Runner
//! erreicht (AGT-010 AC1); fehlende Pflichtparameter meldet sie gesammelt (AC2), damit die UI
//! ein Formular und die CLI eine vollständige Meldung zeigen kann.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::spec::{Param, ParamType};

/// Aufgelöste Werte, sortiert nach Name.
pub type ParamValues = BTreeMap<String, Value>;

/// Art eines Parameterfehlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamErrorKind {
    /// Der Agent deklariert den Parameter nicht.
    Unknown,
    /// `required: true` ohne Default und ohne Wert.
    Missing,
    /// Typ, `minimum`/`maximum` oder `values` verletzt.
    Invalid,
}

/// Ein Fehler zu einem Parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParamError {
    pub name: String,
    pub kind: ParamErrorKind,
    pub message: String,
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.message)
    }
}

fn type_name(kind: ParamType) -> &'static str {
    match kind {
        ParamType::String => "string",
        ParamType::Integer => "integer",
        ParamType::Number => "number",
        ParamType::Boolean => "boolean",
        ParamType::Enum => "enum",
    }
}

/// Wandelt einen Text (CLI, Formular) in den deklarierten Typ; andere Werte bleiben.
fn coerce(param: &Param, value: &Value) -> Value {
    let Value::String(text) = value else {
        return value.clone();
    };
    let t = text.trim();
    match param.kind {
        ParamType::Integer => t
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| value.clone()),
        ParamType::Number => t
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| value.clone()),
        ParamType::Boolean => match t {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => value.clone(),
        },
        ParamType::String | ParamType::Enum => value.clone(),
    }
}

/// Prüft einen Wert gegen Typ, `minimum`/`maximum` und `values`; `None` = passt.
pub fn value_problem(param: &Param, value: &Value) -> Option<String> {
    let type_ok = match param.kind {
        ParamType::String => value.is_string(),
        ParamType::Integer => value.is_i64() || value.is_u64(),
        ParamType::Number => value.is_number(),
        ParamType::Boolean => value.is_boolean(),
        ParamType::Enum => value
            .as_str()
            .is_some_and(|s| param.values.iter().any(|v| v == s)),
    };
    if !type_ok {
        if param.kind == ParamType::Enum {
            return Some(format!(
                "{value} ist keiner der erlaubten Werte ({})",
                param.values.join(", ")
            ));
        }
        return Some(format!(
            "{value} passt nicht zu type: {}",
            type_name(param.kind)
        ));
    }
    let n = value.as_f64()?;
    if let Some(min) = param.minimum.filter(|min| n < *min) {
        return Some(format!("{value} liegt unter minimum {min}"));
    }
    if let Some(max) = param.maximum.filter(|max| n > *max) {
        return Some(format!("{value} liegt über maximum {max}"));
    }
    None
}

/// Löst die Werte eines Starts auf: Defaults einsetzen, Texte umwandeln, alles prüfen.
/// Liefert alle Fehler auf einmal, sortiert nach Name.
pub fn resolve(
    declared: &BTreeMap<String, Param>,
    given: &BTreeMap<String, Value>,
) -> Result<ParamValues, Vec<ParamError>> {
    let mut errors = Vec::new();
    let mut out = ParamValues::new();
    for name in given.keys().filter(|n| !declared.contains_key(*n)) {
        let known: Vec<&str> = declared.keys().map(String::as_str).collect();
        errors.push(ParamError {
            name: name.clone(),
            kind: ParamErrorKind::Unknown,
            message: if known.is_empty() {
                "Der Agent deklariert keine Parameter".into()
            } else {
                format!("unbekannter Parameter (deklariert: {})", known.join(", "))
            },
        });
    }
    for (name, param) in declared {
        match given.get(name) {
            Some(value) => {
                let value = coerce(param, value);
                match value_problem(param, &value) {
                    Some(message) => errors.push(ParamError {
                        name: name.clone(),
                        kind: ParamErrorKind::Invalid,
                        message,
                    }),
                    None => {
                        out.insert(name.clone(), value);
                    }
                }
            }
            None => match &param.default {
                Some(default) => {
                    out.insert(name.clone(), default.clone());
                }
                None if param.required == Some(true) => errors.push(ParamError {
                    name: name.clone(),
                    kind: ParamErrorKind::Missing,
                    message: format!(
                        "Pflichtparameter ohne Default fehlt (type: {})",
                        type_name(param.kind)
                    ),
                }),
                None => {}
            },
        }
    }
    errors.sort_by(|a, b| a.name.cmp(&b.name));
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// `k=v` aus der CLI (`--param`); der Wert bleibt Text und wird bei der Auflösung umgewandelt.
pub fn parse_assignment(raw: &str) -> Result<(String, Value), String> {
    let (k, v) = raw
        .split_once('=')
        .ok_or_else(|| format!("`{raw}`: erwartet NAME=WERT"))?;
    let k = k.trim();
    if k.is_empty() {
        return Err(format!("`{raw}`: Parametername fehlt"));
    }
    Ok((k.to_owned(), Value::String(v.to_owned())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    fn param(kind: ParamType) -> Param {
        Param {
            kind,
            description: None,
            default: None,
            minimum: None,
            maximum: None,
            required: None,
            values: Vec::new(),
        }
    }

    fn declared() -> BTreeMap<String, Param> {
        BTreeMap::from([
            (
                "branch".to_owned(),
                Param {
                    default: Some(json!("main")),
                    ..param(ParamType::String)
                },
            ),
            (
                "max_attempts".to_owned(),
                Param {
                    default: Some(json!(3)),
                    minimum: Some(1.0),
                    maximum: Some(10.0),
                    ..param(ParamType::Integer)
                },
            ),
            (
                "ticket".to_owned(),
                Param {
                    required: Some(true),
                    ..param(ParamType::String)
                },
            ),
            (
                "level".to_owned(),
                Param {
                    values: vec!["low".into(), "high".into()],
                    ..param(ParamType::Enum)
                },
            ),
            ("dry_run".to_owned(), param(ParamType::Boolean)),
        ])
    }

    fn given(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect()
    }

    #[test]
    fn agt_010_ac1_value_above_maximum_is_rejected() {
        let err = resolve(
            &declared(),
            &given(&[("max_attempts", "20"), ("ticket", "T-1")]),
        )
        .unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].name, "max_attempts");
        assert_eq!(err[0].kind, ParamErrorKind::Invalid);
        assert!(err[0].message.contains("maximum 10"), "{}", err[0].message);
    }

    #[test]
    fn agt_010_ac2_missing_required_param_without_default_is_reported() {
        let err = resolve(&declared(), &BTreeMap::new()).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].name, "ticket");
        assert_eq!(err[0].kind, ParamErrorKind::Missing);
    }

    #[test]
    fn cli_texts_are_coerced_and_defaults_filled() {
        let values = resolve(
            &declared(),
            &given(&[
                ("max_attempts", "7"),
                ("ticket", "T-1"),
                ("level", "high"),
                ("dry_run", "true"),
            ]),
        )
        .unwrap();
        assert_eq!(values["max_attempts"], json!(7));
        assert_eq!(values["branch"], json!("main"));
        assert_eq!(values["level"], json!("high"));
        assert_eq!(values["dry_run"], json!(true));
        // Typisierte Werte (API, session_spawn) gehen ebenso.
        let mut typed = given(&[("ticket", "T-2")]);
        typed.insert("max_attempts".into(), json!(2));
        assert_eq!(resolve(&declared(), &typed).unwrap()["max_attempts"], 2);
    }

    #[test]
    fn unknown_params_wrong_types_and_enum_values_are_errors() {
        let err = resolve(
            &declared(),
            &given(&[
                ("ticket", "T"),
                ("nope", "1"),
                ("max_attempts", "viele"),
                ("level", "mittel"),
                ("dry_run", "ja"),
            ]),
        )
        .unwrap_err();
        let kinds: Vec<(&str, ParamErrorKind)> =
            err.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("dry_run", ParamErrorKind::Invalid),
                ("level", ParamErrorKind::Invalid),
                ("max_attempts", ParamErrorKind::Invalid),
                ("nope", ParamErrorKind::Unknown),
            ]
        );
    }

    #[test]
    fn assignments_split_at_the_first_equals_sign() {
        assert_eq!(
            parse_assignment("q=a=b").unwrap(),
            ("q".into(), json!("a=b"))
        );
        assert!(parse_assignment("nur").is_err());
        assert!(parse_assignment("=x").is_err());
    }
}
