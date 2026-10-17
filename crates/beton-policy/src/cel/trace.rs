//! Spur einer Bedingung für Explain (POL-027 AC1): Eine Bedingung der Form `a && b && c` bzw.
//! `a || b` wird in ihre Teilausdrücke der obersten Ebene zerlegt; jeder Teil wird einzeln
//! ausgewertet. So zeigt Explain, welcher Teil eine Regel greifen ließ.
//!
//! Die Spur enthält nur den Quelltext der Policy und Wahrheitswerte, nie Werte aus dem
//! Kontext (POL-025 AC2: keine Tool-Argumente im Audit).

use serde::{Deserialize, Serialize};

use super::check::Node;
use super::{Meter, eval};
use crate::context::Activation;
use crate::value::Value;

/// Ein Teilausdruck einer Bedingung und sein Ergebnis (`None`: Fehler oder kein `bool`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracePart {
    pub expr: String,
    pub value: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    And,
    Or,
}

impl Op {
    fn token(self) -> &'static str {
        match self {
            Op::And => "&&",
            Op::Or => "||",
        }
    }
}

/// Teilknoten derselben Verknüpfung in Quelltext-Reihenfolge.
fn flatten<'a>(node: &'a Node, op: Op, out: &mut Vec<&'a Node>) {
    match (node, op) {
        (Node::And(a, b), Op::And) | (Node::Or(a, b), Op::Or) => {
            flatten(a, op, out);
            flatten(b, op, out);
        }
        _ => out.push(node),
    }
}

/// Zerlegt `src` an `token` außerhalb von Klammern und Zeichenketten. Zeichenketten in allen
/// CEL-Formen: `"…"`, `'…'`, Triple-Quotes (`"""…"""`, `'''…'''`) und mit Präfix `r`/`R`
/// (Raw, ohne Escape), auch kombiniert mit `b`/`B` (#263).
fn split_top_level<'a>(src: &'a str, token: &str) -> Vec<&'a str> {
    let bytes = src.as_bytes();
    let mut parts = Vec::new();
    let (mut depth, mut start, mut i) = (0usize, 0usize, 0usize);
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' | b'\'' => {
                i = string_end(bytes, i, raw_prefix(bytes, i));
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && bytes[i..].starts_with(token.as_bytes()) => {
                parts.push(src[start..i].trim());
                i += token.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(src[start..].trim());
    parts
}

fn ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Steht vor dem Quote an `quote` ein Raw-Präfix (`r`, `R`, `rb`, `br`, …) als eigenes Token?
fn raw_prefix(bytes: &[u8], quote: usize) -> bool {
    let before = |n: usize| quote.checked_sub(n).map(|k| bytes[k]);
    let boundary = |n: usize| before(n).is_none_or(|c| !ident(c));
    let is_r = |c: Option<u8>| matches!(c, Some(b'r' | b'R'));
    let is_b = |c: Option<u8>| matches!(c, Some(b'b' | b'B'));
    (is_r(before(1)) && boundary(2))
        || (is_r(before(1)) && is_b(before(2)) && boundary(3))
        || (is_b(before(1)) && is_r(before(2)) && boundary(3))
}

/// Index nach dem Ende der Zeichenkette, die an `open` beginnt (bzw. das Ende von `bytes`).
fn string_end(bytes: &[u8], open: usize, raw: bool) -> usize {
    let q = bytes[open];
    let triple = bytes[open..].starts_with(&[q, q, q]);
    let (mut i, close): (usize, &[u8]) = if triple {
        (open + 3, &[q, q, q][..])
    } else {
        (open + 1, &[q][..])
    };
    while i < bytes.len() {
        if !raw && bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(close) {
            return i + close.len();
        }
        i += 1;
    }
    bytes.len()
}

fn value_of(r: Result<Value, super::EvalError>) -> Option<bool> {
    match r {
        Ok(Value::Bool(b)) => Some(b),
        _ => None,
    }
}

/// Spur von `node` (Quelltext `src`).
pub(crate) fn trace(src: &str, node: &Node, act: &Activation, meter: &mut Meter) -> Vec<TracePart> {
    let op = match node {
        Node::And(..) => Some(Op::And),
        Node::Or(..) => Some(Op::Or),
        _ => None,
    };
    let mut nodes = Vec::new();
    let texts = match op {
        Some(op) => {
            flatten(node, op, &mut nodes);
            split_top_level(src, op.token())
        }
        None => Vec::new(),
    };
    if nodes.len() < 2 || texts.len() != nodes.len() {
        // Keine Verknüpfung auf oberster Ebene (oder nicht eindeutig zuzuordnen): der ganze
        // Ausdruck ist der Teilausdruck.
        if meter.begin_expr().is_err() {
            return vec![TracePart {
                expr: src.trim().to_owned(),
                value: None,
            }];
        }
        return vec![TracePart {
            expr: src.trim().to_owned(),
            value: value_of(eval::eval(node, act, meter)),
        }];
    }
    let mut out = Vec::with_capacity(nodes.len());
    for (part, text) in nodes.into_iter().zip(texts) {
        let value = match meter.begin_expr() {
            Ok(()) => value_of(eval::eval(part, act, meter)),
            Err(_) => None,
        };
        out.push(TracePart {
            expr: text.to_owned(),
            value,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_top_level;

    #[test]
    fn pol_027_split_respects_strings_and_brackets() {
        assert_eq!(
            split_top_level(r#"a && (b && c) && d.matches("x && y") && e['&&']"#, "&&"),
            vec!["a", "(b && c)", r#"d.matches("x && y")"#, "e['&&']"]
        );
        assert_eq!(
            split_top_level(r#"s == "a\"&&" || t"#, "||"),
            vec![r#"s == "a\"&&""#, "t"]
        );
        assert_eq!(split_top_level("x", "&&"), vec!["x"]);
        // #263: Raw-Strings kennen kein Escape, Triple-Quotes enthalten einfache Quotes.
        assert_eq!(
            split_top_level(r#"a == r"x\" && b"#, "&&"),
            vec![r#"a == r"x\""#, "b"]
        );
        assert_eq!(
            split_top_level(r"a == R'x\' && b", "&&"),
            vec![r"a == R'x\'", "b"]
        );
        assert_eq!(
            split_top_level(r#"a == """x " && y""" && b"#, "&&"),
            vec![r#"a == """x " && y""""#, "b"]
        );
        assert_eq!(
            split_top_level("a == '''it's && no''' || b", "||"),
            vec!["a == '''it's && no'''", "b"]
        );
        assert_eq!(
            split_top_level(r#"a == br"\" && b"#, "&&"),
            vec![r#"a == br"\""#, "b"]
        );
        // Ein `r` am Ende eines Bezeichners ist kein Raw-String-Präfix.
        assert_eq!(
            split_top_level(r#"bar == "a\" && c" && d"#, "&&"),
            vec![r#"bar == "a\" && c""#, "d"]
        );
        assert_eq!(
            split_top_level("ä == \"ö\" && ü", "&&"),
            vec!["ä == \"ö\"", "ü"]
        );
    }
}
