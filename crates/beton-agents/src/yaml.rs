//! YAML mit Positionen: liest ein Dokument in einen `serde_json::Value` und merkt sich für
//! jeden Feldpfad Zeile und Spalte seines Schlüssels (bzw. Listeneintrags). So tragen
//! Schema- und semantische Befunde dieselben Koordinaten (AGT-001 AC2, AGT-002 AC2).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// Ein Segment eines Feldpfads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// `executor.harness`, `policies[1].id`; leer für die Wurzel.
pub fn path_string(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(i) => {
                let _ = write!(out, "[{i}]");
            }
        }
    }
    out
}

/// 1-basierte Position in der Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

impl Pos {
    pub const START: Pos = Pos { line: 1, column: 1 };

    fn of(m: Marker) -> Self {
        // yaml-rust2: Zeile 1-basiert, Spalte 0-basiert.
        Self {
            line: m.line().max(1),
            column: m.col() + 1,
        }
    }
}

/// Ein gelesenes YAML-Dokument mit Positionsindex.
#[derive(Debug, Clone)]
pub struct Document {
    pub value: Value,
    positions: HashMap<String, Pos>,
}

impl Document {
    /// Position des Feldpfads oder seines nächsten vorhandenen Vorfahren.
    pub fn pos(&self, path: &[Seg]) -> Pos {
        let mut path = path.to_vec();
        loop {
            if let Some(p) = self.positions.get(&path_string(&path)) {
                return *p;
            }
            if path.pop().is_none() {
                return Pos::START;
            }
        }
    }

    /// Steht genau dieser Pfad in der Datei?
    pub fn has(&self, path: &str) -> Option<Pos> {
        self.positions.get(path).copied()
    }
}

/// Fehler beim Lesen (Syntax, doppelte Schlüssel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub path: String,
    pub pos: Pos,
    pub code: &'static str,
    pub message: String,
}

enum Frame {
    Map {
        path: Vec<Seg>,
        obj: Map<String, Value>,
        key: Option<String>,
        seen: HashSet<String>,
        anchor: usize,
        /// Listeneintrag, dessen Position der erste Schlüssel bestimmt.
        refine: bool,
    },
    Seq {
        path: Vec<Seg>,
        items: Vec<Value>,
        anchor: usize,
    },
}

#[derive(Default)]
struct Receiver {
    stack: Vec<Frame>,
    root: Option<Value>,
    anchors: HashMap<usize, Value>,
    positions: HashMap<String, Pos>,
    errors: Vec<YamlError>,
    fatal: bool,
}

impl Receiver {
    /// Pfad des nächsten Werts und, falls er in einer Liste steht, seine Position eintragen.
    fn child_path(&mut self, mark: Marker) -> Vec<Seg> {
        match self.stack.last() {
            None => Vec::new(),
            Some(Frame::Map { path, key, .. }) => {
                let mut p = path.clone();
                p.push(Seg::Key(key.clone().unwrap_or_default()));
                p
            }
            Some(Frame::Seq { path, items, .. }) => {
                let mut p = path.clone();
                p.push(Seg::Index(items.len()));
                self.positions.insert(path_string(&p), Pos::of(mark));
                p
            }
        }
    }

    fn insert(&mut self, value: Value) {
        match self.stack.last_mut() {
            None => self.root = Some(value),
            Some(Frame::Map { obj, key, .. }) => {
                if let Some(k) = key.take() {
                    obj.insert(k, value);
                }
            }
            Some(Frame::Seq { items, .. }) => items.push(value),
        }
    }

    fn expects_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { key: None, .. }))
    }

    fn fail(&mut self, mark: Marker, message: &str) {
        self.errors.push(YamlError {
            path: String::new(),
            pos: Pos::of(mark),
            code: crate::diag::code::YAML_SYNTAX,
            message: message.to_owned(),
        });
        self.fatal = true;
    }
}

impl MarkedEventReceiver for Receiver {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.fatal {
            return;
        }
        match ev {
            Event::Scalar(text, style, anchor, tag) => {
                if self.expects_key() {
                    let Some(Frame::Map {
                        path,
                        key,
                        seen,
                        refine,
                        ..
                    }) = self.stack.last_mut()
                    else {
                        return;
                    };
                    let mut p = path.clone();
                    p.push(Seg::Key(text.clone()));
                    let p_str = path_string(&p);
                    if *refine {
                        self.positions.insert(path_string(path), Pos::of(mark));
                        *refine = false;
                    }
                    if !seen.insert(text.clone()) {
                        self.errors.push(YamlError {
                            path: p_str.clone(),
                            pos: Pos::of(mark),
                            code: crate::diag::code::DUPLICATE_KEY,
                            message: format!("Schlüssel „{text}“ kommt mehrfach vor"),
                        });
                    }
                    *key = Some(text);
                    self.positions.insert(p_str, Pos::of(mark));
                    return;
                }
                self.child_path(mark);
                let plain =
                    style == TScalarStyle::Plain && tag.as_ref().is_none_or(|t| t.suffix != "str");
                let value = if plain {
                    resolve_plain(&text)
                } else {
                    Value::String(text)
                };
                if anchor != 0 {
                    self.anchors.insert(anchor, value.clone());
                }
                self.insert(value);
            }
            Event::MappingStart(anchor, _) | Event::SequenceStart(anchor, _) => {
                if self.expects_key() {
                    self.fail(mark, "Nur einfache Schlüssel sind erlaubt");
                    return;
                }
                let in_seq = matches!(self.stack.last(), Some(Frame::Seq { .. }));
                let path = self.child_path(mark);
                if matches!(ev, Event::MappingStart(..)) {
                    self.stack.push(Frame::Map {
                        path,
                        obj: Map::new(),
                        key: None,
                        seen: HashSet::new(),
                        anchor,
                        refine: in_seq,
                    });
                } else {
                    self.stack.push(Frame::Seq {
                        path,
                        items: Vec::new(),
                        anchor,
                    });
                }
            }
            Event::MappingEnd | Event::SequenceEnd => {
                let (value, anchor) = match self.stack.pop() {
                    Some(Frame::Map { obj, anchor, .. }) => (Value::Object(obj), anchor),
                    Some(Frame::Seq { items, anchor, .. }) => (Value::Array(items), anchor),
                    None => return,
                };
                if anchor != 0 {
                    self.anchors.insert(anchor, value.clone());
                }
                self.insert(value);
            }
            Event::Alias(id) => {
                if self.expects_key() {
                    self.fail(mark, "Aliase sind als Schlüssel nicht erlaubt");
                    return;
                }
                self.child_path(mark);
                let value = self.anchors.get(&id).cloned().unwrap_or(Value::Null);
                self.insert(value);
            }
            _ => {}
        }
    }
}

fn is_digits(s: &str, radix: u32) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_digit(radix))
}

fn is_float(s: &str) -> bool {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((a, b)) => {
            (a.is_empty() || is_digits(a, 10))
                && (b.is_empty() || is_digits(b, 10))
                && !(a.is_empty() && b.is_empty())
        }
        None => is_digits(mantissa, 10),
    };
    let exp_ok = exp.is_none_or(|e| is_digits(e.strip_prefix(['-', '+']).unwrap_or(e), 10));
    mantissa_ok && exp_ok
}

/// Typauflösung ungequoteter Skalare (YAML 1.2 Core-Schema).
fn resolve_plain(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if is_digits(unsigned, 10) {
        if let Ok(n) = text.parse::<i64>() {
            return Value::Number(n.into());
        }
        if let Ok(n) = text.trim_start_matches('+').parse::<u64>() {
            return Value::Number(n.into());
        }
    }
    if let Some(hex) = text.strip_prefix("0x")
        && let Ok(n) = u64::from_str_radix(hex, 16)
    {
        return Value::Number(n.into());
    }
    if let Some(oct) = text.strip_prefix("0o")
        && let Ok(n) = u64::from_str_radix(oct, 8)
    {
        return Value::Number(n.into());
    }
    if is_float(text)
        && let Ok(f) = text.parse::<f64>()
        && let Some(n) = Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::String(text.to_owned())
}

/// Liest das erste Dokument. Syntaxfehler sind fatal (`None`); doppelte Schlüssel werden
/// gemeldet, das Dokument bleibt nutzbar.
pub fn parse(text: &str) -> (Option<Document>, Vec<YamlError>) {
    let mut recv = Receiver::default();
    if let Err(e) = Parser::new_from_str(text).load(&mut recv, false) {
        recv.errors.push(YamlError {
            path: String::new(),
            pos: Pos::of(*e.marker()),
            code: crate::diag::code::YAML_SYNTAX,
            message: format!("YAML-Syntaxfehler: {}", e.info()),
        });
        return (None, recv.errors);
    }
    if recv.fatal {
        return (None, recv.errors);
    }
    let doc = Document {
        value: recv.root.unwrap_or(Value::Null),
        positions: recv.positions,
    };
    (Some(doc), recv.errors)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;

    #[test]
    fn values_and_positions_of_keys_and_items() {
        let src = "name: x\nexecutor:\n  harness: claude\n  max_turns: 20\nlist: [a, \"1\"]\npolicies:\n  - id: a\n    type: b\n  - ref: ./p.yaml\nversion: 0.3.0\nratio: 0.5\nnone: ~\n";
        let (doc, errors) = parse(src);
        assert!(errors.is_empty(), "{errors:?}");
        let doc = doc.unwrap();
        assert_eq!(
            doc.value,
            json!({"name": "x", "executor": {"harness": "claude", "max_turns": 20},
                   "list": ["a", "1"], "policies": [{"id": "a", "type": "b"}, {"ref": "./p.yaml"}],
                   "version": "0.3.0", "ratio": 0.5, "none": null})
        );
        assert_eq!(
            doc.has("executor.harness"),
            Some(Pos { line: 3, column: 3 })
        );
        assert_eq!(doc.has("policies[1]"), Some(Pos { line: 9, column: 5 }));
        assert_eq!(doc.has("policies[1].ref"), Some(Pos { line: 9, column: 5 }));
        assert_eq!(
            doc.has("list[1]"),
            Some(Pos {
                line: 5,
                column: 11
            })
        );
        let missing = [Seg::Key("executor".into()), Seg::Key("model".into())];
        assert_eq!(doc.pos(&missing), Pos { line: 2, column: 1 });
    }

    #[test]
    fn duplicate_keys_and_syntax_errors_are_reported_with_position() {
        let (doc, errors) = parse("name: a\nname: b\n");
        assert!(doc.is_some());
        assert_eq!(errors[0].code, "duplicate_key");
        assert_eq!(errors[0].pos, Pos { line: 2, column: 1 });
        let (doc, errors) = parse("name: [a\n");
        assert!(doc.is_none());
        assert_eq!(errors[0].code, "yaml_syntax");
    }

    #[test]
    fn anchors_and_aliases_are_resolved() {
        let (doc, _) = parse("a: &x {k: 1}\nb: *x\n");
        assert_eq!(doc.unwrap().value, json!({"a": {"k": 1}, "b": {"k": 1}}));
    }
}
