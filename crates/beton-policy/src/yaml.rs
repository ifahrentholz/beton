//! YAML mit Positionen (angelehnt an `beton-agents/src/yaml.rs`): liest ein Dokument in
//! einen `serde_json::Value` und merkt sich Zeile und Spalte jedes Schlüssels und jedes
//! Skalarwerts, damit Ladefehler auf die Stelle in der Datei zeigen (POL-001 AC3).
//!
//! Unterschiede zum Agent-Leser: doppelte Schlüssel sind ein Fehler, Aliase sind auf eine
//! Gesamtzahl von Knoten begrenzt (Schutz vor „Billion Laughs“), und für Skalare wird der
//! Stil festgehalten (für die Position von CEL-Fehlern innerhalb eines Werts).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// Höchstzahl an Knoten eines Dokuments (inklusive aufgelöster Aliase).
const MAX_NODES: usize = 100_000;

/// 1-basierte Position in der Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

impl Pos {
    pub const START: Pos = Pos { line: 1, column: 1 };

    fn of(m: Marker) -> Self {
        // yaml-rust2: Zeile 1-basiert, Spalte 0-basiert.
        Self {
            line: u32::try_from(m.line().max(1)).unwrap_or(u32::MAX),
            column: u32::try_from(m.col() + 1).unwrap_or(u32::MAX),
        }
    }
}

/// Stil eines Skalars, soweit er für Positionen innerhalb des Werts zählt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// ungequotet: der Wert beginnt an der Position
    Plain,
    /// `'…'` oder `"…"`: der Wert beginnt ein Zeichen später
    Quoted,
    /// `|` oder `>`: der Wert beginnt in der nächsten Zeile
    Block,
}

/// Feldpfad wie `rules[2].when`.
pub fn path_string(segments: &[Seg]) -> String {
    let mut out = String::new();
    for seg in segments {
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

/// Ein Segment eines Feldpfads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Ein gelesenes Dokument mit Positionsindex.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub value: Value,
    keys: HashMap<String, Pos>,
    values: HashMap<String, (Pos, Style)>,
}

impl Document {
    /// Position des Schlüssels (bzw. Listeneintrags) `path` oder seines nächsten Vorfahren.
    pub fn pos(&self, path: &str) -> Pos {
        let mut p = path.to_owned();
        loop {
            if let Some(pos) = self.keys.get(&p) {
                return *pos;
            }
            match p.rfind(['.', '[']) {
                Some(i) => p.truncate(i),
                None => return Pos::START,
            }
        }
    }

    /// Position und Stil des Skalarwerts an `path`.
    pub fn value_pos(&self, path: &str) -> Option<(Pos, Style)> {
        self.values.get(path).copied()
    }
}

/// Ein Lesefehler mit Position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub pos: Pos,
    pub message: String,
}

enum Frame {
    Map {
        path: String,
        obj: Map<String, Value>,
        key: Option<String>,
        seen: HashSet<String>,
        anchor: usize,
    },
    Seq {
        path: String,
        items: Vec<Value>,
        anchor: usize,
    },
}

#[derive(Default)]
struct Receiver {
    stack: Vec<Frame>,
    root: Option<Value>,
    anchors: HashMap<usize, (Value, usize)>,
    keys: HashMap<String, Pos>,
    values: HashMap<String, (Pos, Style)>,
    error: Option<YamlError>,
    nodes: usize,
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn count(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(count).sum::<usize>(),
        Value::Object(o) => 1 + o.values().map(count).sum::<usize>(),
        _ => 1,
    }
}

impl Receiver {
    fn child_path(&mut self, mark: Marker) -> String {
        match self.stack.last() {
            None => String::new(),
            Some(Frame::Map { path, key, .. }) => join(path, key.as_deref().unwrap_or_default()),
            Some(Frame::Seq { path, items, .. }) => {
                let p = format!("{path}[{}]", items.len());
                self.keys.insert(p.clone(), Pos::of(mark));
                p
            }
        }
    }

    fn insert(&mut self, value: Value, nodes: usize) {
        self.nodes += nodes;
        if self.nodes > MAX_NODES {
            self.error.get_or_insert(YamlError {
                pos: Pos::START,
                message: format!("Dokument hat mehr als {MAX_NODES} Knoten"),
            });
            return;
        }
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

    fn fail(&mut self, mark: Marker, message: String) {
        self.error.get_or_insert(YamlError {
            pos: Pos::of(mark),
            message,
        });
    }
}

impl MarkedEventReceiver for Receiver {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::Scalar(text, style, anchor, tag) => {
                if self.expects_key() {
                    let Some(Frame::Map {
                        path, key, seen, ..
                    }) = self.stack.last_mut()
                    else {
                        return;
                    };
                    let p = join(path, &text);
                    if !seen.insert(text.clone()) {
                        self.fail(mark, format!("Schlüssel `{text}` kommt mehrfach vor"));
                        return;
                    }
                    *key = Some(text);
                    self.keys.insert(p, Pos::of(mark));
                    return;
                }
                let path = self.child_path(mark);
                let shown = match style {
                    TScalarStyle::Plain => Style::Plain,
                    TScalarStyle::SingleQuoted | TScalarStyle::DoubleQuoted => Style::Quoted,
                    _ => Style::Block,
                };
                self.values.insert(path, (Pos::of(mark), shown));
                let plain =
                    style == TScalarStyle::Plain && tag.as_ref().is_none_or(|t| t.suffix != "str");
                let value = if plain {
                    resolve_plain(&text)
                } else {
                    Value::String(text)
                };
                if anchor != 0 {
                    self.anchors.insert(anchor, (value.clone(), 1));
                }
                self.insert(value, 1);
            }
            Event::MappingStart(anchor, _) | Event::SequenceStart(anchor, _) => {
                if self.expects_key() {
                    self.fail(mark, "Nur einfache Schlüssel sind erlaubt".into());
                    return;
                }
                let path = self.child_path(mark);
                if matches!(ev, Event::MappingStart(..)) {
                    self.stack.push(Frame::Map {
                        path,
                        obj: Map::new(),
                        key: None,
                        seen: HashSet::new(),
                        anchor,
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
                    let n = count(&value);
                    self.anchors.insert(anchor, (value.clone(), n));
                }
                // Die Kinder sind bereits gezählt.
                self.insert(value, 1);
            }
            Event::Alias(id) => {
                if self.expects_key() {
                    self.fail(mark, "Aliase sind als Schlüssel nicht erlaubt".into());
                    return;
                }
                self.child_path(mark);
                let (value, n) = self.anchors.get(&id).cloned().unwrap_or((Value::Null, 1));
                self.insert(value, n);
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
    if is_float(text)
        && let Ok(f) = text.parse::<f64>()
        && let Some(n) = Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::String(text.to_owned())
}

/// Liest das erste Dokument.
pub fn parse(text: &str) -> Result<Document, YamlError> {
    let mut recv = Receiver::default();
    if let Err(e) = Parser::new_from_str(text).load(&mut recv, false) {
        return Err(YamlError {
            pos: Pos::of(*e.marker()),
            message: format!("YAML-Syntaxfehler: {}", e.info()),
        });
    }
    if let Some(e) = recv.error {
        return Err(e);
    }
    Ok(Document {
        value: recv.root.unwrap_or(Value::Null),
        keys: recv.keys,
        values: recv.values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_and_positions() {
        let src = "name: x\nrules:\n  - id: a\n    when: 'tool.kind == \"shell\"'\n  - id: b\n    when: |\n      true\nn: 1.5\n";
        let doc = parse(src).unwrap();
        assert_eq!(
            doc.value,
            json!({"name": "x", "rules": [{"id": "a", "when": "tool.kind == \"shell\""},
                   {"id": "b", "when": "true\n"}], "n": 1.5})
        );
        assert_eq!(doc.pos("rules[1].id"), Pos { line: 5, column: 5 });
        assert_eq!(doc.pos("rules[1].missing"), doc.pos("rules[1]"));
        assert_eq!(doc.pos("rules[1]").line, 5);
        assert_eq!(
            doc.value_pos("rules[0].when"),
            Some((
                Pos {
                    line: 4,
                    column: 11
                },
                Style::Quoted
            ))
        );
        assert_eq!(
            doc.value_pos("rules[1].when").map(|v| v.1),
            Some(Style::Block)
        );
        assert_eq!(doc.pos("unbekannt"), Pos::START);
    }

    #[test]
    fn duplicate_keys_and_syntax_errors() {
        let err = parse("name: a\nname: b\n").unwrap_err();
        assert_eq!(err.pos, Pos { line: 2, column: 1 });
        assert!(parse("name: [a\n").is_err());
        assert!(parse("? [a]\n: b\n").is_err());
    }

    #[test]
    fn aliases_are_resolved_and_bounded() {
        let doc = parse("a: &x {k: 1}\nb: *x\n").unwrap();
        assert_eq!(doc.value, json!({"a": {"k": 1}, "b": {"k": 1}}));
        let mut bomb = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..8 {
            let refs = vec![format!("*a{}", i - 1); 10].join(", ");
            bomb.push_str(&format!("a{i}: &a{i} [{refs}]\n"));
        }
        let err = parse(&bomb).unwrap_err();
        assert!(err.message.contains("Knoten"), "{err:?}");
    }

    #[test]
    fn plain_scalars_follow_the_core_schema() {
        let doc = parse("a: ~\nb: TRUE\nc: -3\nd: 1e3\ne: '1'\nf: 18446744073709551615\n").unwrap();
        assert_eq!(
            doc.value,
            json!({"a": null, "b": true, "c": -3, "d": 1000.0, "e": "1", "f": 18446744073709551615u64})
        );
    }
}
