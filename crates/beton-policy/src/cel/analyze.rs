//! Statische Analyse geprüfter Ausdrücke: betroffene Tool-Klassen (POL-024) und benutzte
//! Kontextfelder (der Runner holt teure Werte wie `user.daily_cost_usd` nur bei Bedarf).

use std::collections::BTreeSet;

use super::check::{Func, Node};
use crate::value::Value;

type Kinds = Option<BTreeSet<String>>;

/// Tool-Klassen, auf die `node` die Regel beschränkt; `None` = nicht bestimmbar (alle).
pub(crate) fn tool_kinds(node: &Node) -> Kinds {
    match node {
        Node::Lit(Value::Bool(false)) => Some(BTreeSet::new()),
        Node::And(a, b) => intersect(tool_kinds(a), tool_kinds(b)),
        Node::Or(a, b) => union(tool_kinds(a), tool_kinds(b)),
        Node::Call(Func::Eq, args) => match args.as_slice() {
            [a, b] => eq(a, b).or_else(|| eq(b, a)),
            _ => None,
        },
        Node::Call(Func::In, args) => match args.as_slice() {
            [field, Node::List(items)] => {
                let field = tool_field(field)?;
                let mut out = BTreeSet::new();
                for item in items {
                    out.extend(kinds_for(field, string_lit(item)?)?);
                }
                Some(out)
            }
            _ => None,
        },
        _ => None,
    }
}

/// `tool.kind == "x"` bzw. `tool.name == "Bash"`.
fn eq(field: &Node, lit: &Node) -> Kinds {
    kinds_for(tool_field(field)?, string_lit(lit)?)
}

#[derive(Clone, Copy)]
enum ToolField {
    Kind,
    Name,
}

fn tool_field(node: &Node) -> Option<ToolField> {
    match node {
        Node::Select {
            operand,
            field,
            test: false,
            ..
        } if matches!(operand.as_ref(), Node::Var(v) if &**v == "tool") => match &**field {
            "kind" => Some(ToolField::Kind),
            "name" => Some(ToolField::Name),
            _ => None,
        },
        _ => None,
    }
}

fn string_lit(node: &Node) -> Option<&str> {
    match node {
        Node::Lit(Value::String(s)) => Some(s),
        _ => None,
    }
}

fn kinds_for(field: ToolField, value: &str) -> Kinds {
    match field {
        ToolField::Kind => Some(BTreeSet::from([value.to_owned()])),
        ToolField::Name => {
            let kinds = crate::normalize::tool_map().ok()?.kinds_of_name(value);
            // Unbekannter Name: könnte jedes (MCP-)Tool sein.
            (!kinds.is_empty()).then_some(kinds)
        }
    }
}

fn intersect(a: Kinds, b: Kinds) -> Kinds {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
    }
}

fn union(a: Kinds, b: Kinds) -> Kinds {
    match (a, b) {
        (Some(mut a), Some(b)) => {
            a.extend(b);
            Some(a)
        }
        _ => None,
    }
}

/// Alle Felder `objekt.feld` (eine Ebene unter einer Variable) und benutzten Variablen.
pub(crate) fn fields(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::Var(v) => {
            out.insert(v.to_string());
        }
        Node::Select { operand, field, .. } => {
            if let Node::Var(v) = operand.as_ref() {
                out.insert(format!("{v}.{field}"));
            }
            fields(operand, out);
        }
        Node::Call(_, args) | Node::List(args) => args.iter().for_each(|a| fields(a, out)),
        Node::And(a, b) | Node::Or(a, b) => {
            fields(a, out);
            fields(b, out);
        }
        Node::Not(a) | Node::NotStrictlyFalse(a) => fields(a, out),
        Node::Cond(a, b, c) => {
            fields(a, out);
            fields(b, out);
            fields(c, out);
        }
        Node::Map(entries) => {
            for (k, v) in entries {
                fields(k, out);
                fields(v, out);
            }
        }
        Node::Compr(c) => {
            for n in [&c.range, &c.init, &c.cond, &c.step, &c.result] {
                fields(n, out);
            }
        }
        Node::Lit(_) | Node::Local(_) => {}
    }
}
