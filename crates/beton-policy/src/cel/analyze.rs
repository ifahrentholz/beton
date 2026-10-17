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

/// Vergleicht der Ausdruck rohe Pfade lexikalisch (#248)? Rohe Pfade sind `tool.paths`,
/// `tool.write_paths` und `tool.cwd`, deren Elemente (Index, Laufvariable einer Comprehension
/// darüber) – nicht aber `resolve(…)`. Lexikalisch sind `glob`, `matches`, `startsWith`,
/// `endsWith`, `contains`, `==`, `!=` und `in`.
pub(crate) fn compares_raw_paths(node: &Node) -> bool {
    walk_raw(node, &mut Vec::new())
}

fn raw_path(node: &Node, locals: &[std::sync::Arc<str>]) -> bool {
    match node {
        Node::Select {
            operand,
            field,
            test: false,
            ..
        } => {
            matches!(operand.as_ref(), Node::Var(v) if &**v == "tool")
                && matches!(&**field, "paths" | "write_paths" | "cwd")
        }
        Node::Call(Func::Index, args) => args.first().is_some_and(|a| raw_path(a, locals)),
        Node::Local(v) => locals.contains(v),
        _ => false,
    }
}

fn walk_raw(node: &Node, locals: &mut Vec<std::sync::Arc<str>>) -> bool {
    match node {
        Node::Call(f, args) => {
            let lexical = matches!(
                f,
                Func::Glob(_)
                    | Func::Matches(_)
                    | Func::StartsWith
                    | Func::EndsWith
                    | Func::Contains
                    | Func::Eq
                    | Func::Ne
                    | Func::In
            );
            (lexical && args.iter().any(|a| raw_path(a, locals)))
                || args.iter().any(|a| walk_raw(a, locals))
        }
        Node::Compr(c) => {
            if walk_raw(&c.range, locals) {
                return true;
            }
            let pushed = raw_path(&c.range, locals);
            if pushed {
                locals.push(c.iter_var.clone());
            }
            let hit = [&c.init, &c.cond, &c.step, &c.result]
                .into_iter()
                .any(|n| walk_raw(n, locals));
            if pushed {
                locals.pop();
            }
            hit
        }
        Node::Select { operand, .. } => walk_raw(operand, locals),
        Node::List(items) => items.iter().any(|a| walk_raw(a, locals)),
        Node::And(a, b) | Node::Or(a, b) => walk_raw(a, locals) || walk_raw(b, locals),
        Node::Not(a) | Node::NotStrictlyFalse(a) => walk_raw(a, locals),
        Node::Cond(a, b, c) => [a, b, c].into_iter().any(|n| walk_raw(n, locals)),
        Node::Map(entries) => entries
            .iter()
            .any(|(k, v)| walk_raw(k, locals) || walk_raw(v, locals)),
        Node::Lit(_) | Node::Var(_) | Node::Local(_) => false,
    }
}
