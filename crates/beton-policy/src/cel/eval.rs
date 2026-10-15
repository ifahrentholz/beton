//! Auswertung des Baums mit CEL-Semantik, Error-Absorption und Kostenbudget.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::check::{Compr, Func, Node};
use super::{EvalError, Meter, ext};
use crate::context::Activation;
use crate::normalize::{git, shell};
use crate::value::{Key, Value, numeric_cmp};

type Res = Result<Value, EvalError>;

fn fail(msg: impl Into<String>) -> EvalError {
    EvalError::Error(msg.into())
}

/// Kosten für Operationen auf Text: 1 je angefangene 64 Bytes.
fn text_cost(len: usize) -> u64 {
    (len as u64) / 64
}

struct Ctx<'a, 'm> {
    act: &'a Activation,
    meter: &'a mut Meter<'m>,
    /// Laufvariablen; der Akkumulator kann einen (absorbierbaren) Fehler halten.
    locals: Vec<(Arc<str>, Result<Value, EvalError>)>,
}

pub(crate) fn eval(node: &Node, act: &Activation, meter: &mut Meter) -> Res {
    let mut ctx = Ctx {
        act,
        meter,
        locals: Vec::new(),
    };
    ctx.eval(node)
}

impl Ctx<'_, '_> {
    fn eval(&mut self, node: &Node) -> Res {
        self.meter.charge(1)?;
        match node {
            Node::Lit(v) => Ok(v.clone()),
            Node::Var(name) => Ok(self.act.var(name)),
            Node::Local(name) => self
                .locals
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| Err(fail(format!("unbekannte Laufvariable `{name}`")))),
            Node::Select {
                operand,
                field,
                declared,
                test,
            } => {
                let target = self.eval(operand)?;
                // `has(state.<scope>)` ohne geladenen Zustand ist ein Fehler, kein `false`:
                // sonst griffe eine Regel `has(state.session) && …` nie (fail open, POL-009).
                if *test
                    && matches!(target, Value::Null)
                    && matches!(operand.as_ref(), Node::Var(v) if v.as_ref() == "state")
                {
                    return Err(fail("Policy-Zustand nicht geladen"));
                }
                select(&target, field, *declared, *test)
            }
            Node::And(a, b) => self.logic(a, b, false),
            Node::Or(a, b) => self.logic(a, b, true),
            Node::Not(a) => match self.eval(a)? {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                other => Err(fail(format!("`!` auf {}", other.type_name()))),
            },
            Node::NotStrictlyFalse(a) => match self.eval(a) {
                Ok(Value::Bool(b)) => Ok(Value::Bool(b)),
                Err(e) if e.is_fatal() => Err(e),
                _ => Ok(Value::Bool(true)),
            },
            Node::Cond(c, a, b) => match self.eval(c)? {
                Value::Bool(true) => self.eval(a),
                Value::Bool(false) => self.eval(b),
                other => Err(fail(format!("Bedingung ist {}", other.type_name()))),
            },
            Node::List(items) => {
                self.meter.charge(items.len() as u64)?;
                let values = items
                    .iter()
                    .map(|n| self.eval(n))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::list(values))
            }
            Node::Map(entries) => {
                self.meter.charge(entries.len() as u64)?;
                let mut map = BTreeMap::new();
                for (k, v) in entries {
                    let key = self.eval(k)?;
                    let key = Key::from_value(&key).ok_or_else(|| {
                        fail(format!("{} ist kein Map-Schlüssel", key.type_name()))
                    })?;
                    let value = self.eval(v)?;
                    if map.insert(key, value).is_some() {
                        return Err(fail("doppelter Schlüssel im Map-Literal"));
                    }
                }
                Ok(Value::Map(Arc::new(map)))
            }
            Node::Compr(c) => self.comprehension(c),
            Node::Call(func, args) => {
                let mut values = Vec::with_capacity(args.len());
                for a in args {
                    values.push(self.eval(a)?);
                }
                self.call(func, &values)
            }
        }
    }

    /// `&&` (`short = false`) bzw. `||` (`short = true`) mit kommutativer Error-Absorption.
    fn logic(&mut self, a: &Node, b: &Node, short: bool) -> Res {
        let left = self.eval(a);
        match &left {
            Ok(Value::Bool(v)) if *v == short => return Ok(Value::Bool(short)),
            Err(e) if e.is_fatal() => return left,
            _ => {}
        }
        let right = self.eval(b);
        match (&left, &right) {
            (_, Err(e)) if e.is_fatal() => right,
            (_, Ok(Value::Bool(v))) if *v == short => Ok(Value::Bool(short)),
            (Ok(Value::Bool(_)), Ok(Value::Bool(_))) => Ok(Value::Bool(!short)),
            (Err(_), _) => left,
            (_, Err(_)) => right,
            _ => Err(fail("logischer Operator auf Nicht-bool")),
        }
    }

    fn comprehension(&mut self, c: &Compr) -> Res {
        let range = self.eval(&c.range)?;
        let items: Vec<Value> = match &range {
            Value::List(items) => items.as_ref().clone(),
            Value::Map(map) => map.keys().map(Key::to_value).collect(),
            other => {
                return Err(fail(format!(
                    "über {} kann nicht iteriert werden",
                    other.type_name()
                )));
            }
        };
        let mut accu = self.eval(&c.init);
        if let Err(e) = &accu
            && e.is_fatal()
        {
            return accu;
        }
        for item in items {
            self.meter.charge(1)?;
            self.locals.push((c.accu_var.clone(), accu.clone()));
            self.locals.push((c.iter_var.clone(), Ok(item)));
            let cond = self.eval(&c.cond);
            let next = match cond {
                Ok(Value::Bool(false)) => None,
                Err(e) if e.is_fatal() => {
                    self.locals.truncate(self.locals.len() - 2);
                    return Err(e);
                }
                _ => Some(self.eval(&c.step)),
            };
            self.locals.truncate(self.locals.len() - 2);
            match next {
                None => break,
                Some(Err(e)) if e.is_fatal() => return Err(e),
                Some(v) => accu = v,
            }
        }
        self.locals.push((c.accu_var.clone(), accu));
        let result = self.eval(&c.result);
        self.locals.pop();
        result
    }

    fn call(&mut self, func: &Func, args: &[Value]) -> Res {
        let s = |i: usize| -> Result<&str, EvalError> {
            match args.get(i) {
                Some(Value::String(s)) => Ok(s),
                Some(other) => Err(fail(format!(
                    "erwartet string, gefunden {}",
                    other.type_name()
                ))),
                None => Err(fail("Argument fehlt")),
            }
        };
        match func {
            Func::Eq => Ok(Value::Bool(args[0] == args[1])),
            Func::Ne => Ok(Value::Bool(args[0] != args[1])),
            Func::Lt | Func::Le | Func::Gt | Func::Ge => {
                let ord = compare(&args[0], &args[1])?;
                Ok(Value::Bool(match func {
                    Func::Lt => ord == Ordering::Less,
                    Func::Le => ord != Ordering::Greater,
                    Func::Gt => ord == Ordering::Greater,
                    _ => ord != Ordering::Less,
                }))
            }
            Func::Add => self.add(&args[0], &args[1]),
            Func::Sub => sub(&args[0], &args[1]),
            Func::Mul | Func::Div | Func::Mod => muldiv(func, &args[0], &args[1]),
            Func::Neg => match &args[0] {
                Value::Int(i) => i
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or_else(|| fail("Überlauf")),
                Value::Double(d) => Ok(Value::Double(-d)),
                Value::Duration(d) => Ok(Value::Duration(-*d)),
                other => Err(fail(format!("`-` auf {}", other.type_name()))),
            },
            Func::Index => index(&args[0], &args[1]),
            Func::In => match &args[1] {
                Value::List(items) => {
                    self.meter.charge(items.len() as u64)?;
                    Ok(Value::Bool(items.iter().any(|x| *x == args[0])))
                }
                Value::Map(map) => Ok(Value::Bool(
                    Key::from_value(&args[0]).is_some_and(|k| map.contains_key(&k)),
                )),
                other => Err(fail(format!("`in` auf {}", other.type_name()))),
            },
            Func::Size => Ok(Value::Int(match &args[0] {
                Value::String(s) => {
                    self.meter.charge(text_cost(s.len()))?;
                    s.chars().count() as i64
                }
                Value::Bytes(b) => b.len() as i64,
                Value::List(l) => l.len() as i64,
                Value::Map(m) => m.len() as i64,
                other => return Err(fail(format!("size() auf {}", other.type_name()))),
            })),
            Func::Matches(pre) => {
                let text = s(0)?;
                self.meter.charge(1 + text_cost(text.len() * 4))?;
                let matched = match pre {
                    Some(re) => re.is_match(text),
                    None => {
                        let pattern = s(1)?;
                        self.meter.charge(10 + text_cost(pattern.len() * 16))?;
                        ext::regex(pattern).map_err(fail)?.is_match(text)
                    }
                };
                Ok(Value::Bool(matched))
            }
            Func::StartsWith | Func::EndsWith | Func::Contains => {
                let (text, needle) = (s(0)?, s(1)?);
                self.meter.charge(text_cost(text.len()))?;
                Ok(Value::Bool(match func {
                    Func::StartsWith => text.starts_with(needle),
                    Func::EndsWith => text.ends_with(needle),
                    _ => text.contains(needle),
                }))
            }
            Func::Duration => ext::parse_duration(s(0)?)
                .map(Value::Duration)
                .map_err(fail),
            Func::Timestamp => OffsetDateTime::parse(s(0)?, &Rfc3339)
                .map(Value::Timestamp)
                .map_err(|e| fail(format!("ungültiger Zeitstempel: {e}"))),
            Func::ToInt => to_int(&args[0]).map(Value::Int),
            Func::ToUint => match &args[0] {
                Value::Uint(u) => Ok(Value::Uint(*u)),
                other => {
                    let i = to_int(other)?;
                    u64::try_from(i)
                        .map(Value::Uint)
                        .map_err(|_| fail("negativ für uint"))
                }
            },
            Func::ToDouble => match &args[0] {
                Value::Int(i) => Ok(Value::Double(*i as f64)),
                Value::Uint(u) => Ok(Value::Double(*u as f64)),
                Value::Double(d) => Ok(Value::Double(*d)),
                Value::String(t) => t.parse().map(Value::Double).map_err(|_| fail("keine Zahl")),
                other => Err(fail(format!("double() auf {}", other.type_name()))),
            },
            Func::ToString => Ok(Value::String(match &args[0] {
                Value::Null => return Err(fail("string() auf null")),
                other => other.to_string().into(),
            })),
            Func::Glob(pre) => {
                let text = s(0)?;
                self.meter.charge(1 + text_cost(text.len() * 4))?;
                let matched = match pre {
                    Some(re) => re.is_match(text),
                    None => {
                        let pattern = s(1)?;
                        self.meter.charge(10 + text_cost(pattern.len() * 16))?;
                        ext::glob_regex(pattern).map_err(fail)?.is_match(text)
                    }
                };
                Ok(Value::Bool(matched))
            }
            Func::Under => ext::under(self.act.resolver(), s(0)?, s(1)?)
                .map(Value::Bool)
                .map_err(fail),
            Func::Resolve => {
                let path = s(0)?;
                self.meter.charge(10 + text_cost(path.len()))?;
                self.act
                    .resolver()
                    .resolve(path)
                    .map(|p| Value::String(p.into()))
                    .map_err(fail)
            }
            Func::DomainMatches => Ok(Value::Bool(ext::domain_matches(s(0)?, s(1)?))),
            Func::BytesHuman => match &args[0] {
                Value::Int(i) => u64::try_from(*i)
                    .map(|n| Value::String(ext::bytes_human(n).into()))
                    .map_err(|_| fail("bytes_human() mit negativer Zahl")),
                Value::Uint(u) => Ok(Value::String(ext::bytes_human(*u).into())),
                other => Err(fail(format!("bytes_human() auf {}", other.type_name()))),
            },
            Func::HasCommand | Func::HasFlag => {
                let argv = argv(&args[0])?;
                self.meter
                    .charge(argv.iter().map(|c| c.len() as u64).sum::<u64>())?;
                let refs: Vec<Vec<&str>> = argv
                    .iter()
                    .map(|c| c.iter().map(String::as_str).collect())
                    .collect();
                Ok(Value::Bool(match func {
                    Func::HasCommand => {
                        let sub = if args.len() > 2 { Some(s(2)?) } else { None };
                        shell::has_command(&refs, s(1)?, sub)
                    }
                    _ => shell::has_flag(&refs, s(1)?),
                }))
            }
            Func::GitOps | Func::GitPushTargets => {
                let argv = argv(&args[0])?;
                self.meter
                    .charge(argv.iter().map(|c| c.len() as u64).sum::<u64>())?;
                let refs: Vec<Vec<&str>> = argv
                    .iter()
                    .map(|c| c.iter().map(String::as_str).collect())
                    .collect();
                let items: Vec<Value> = match func {
                    Func::GitOps => git::git_ops(&refs)
                        .into_iter()
                        .map(|op| Value::String(op.into()))
                        .collect(),
                    _ => git::git_push_targets(&refs)
                        .into_iter()
                        .map(|b| Value::String(b.into()))
                        .collect(),
                };
                Ok(Value::list(items))
            }
            Func::UrlHost => ext::url_host(s(0)?)
                .map(|h| Value::String(h.into()))
                .map_err(fail),
            Func::UrlPath => ext::url_path(s(0)?)
                .map(|p| Value::String(p.into()))
                .map_err(fail),
        }
    }

    fn add(&mut self, a: &Value, b: &Value) -> Res {
        use Value::*;
        Ok(match (a, b) {
            (Int(x), Int(y)) => Int(x.checked_add(*y).ok_or_else(|| fail("Überlauf"))?),
            (Uint(x), Uint(y)) => Uint(x.checked_add(*y).ok_or_else(|| fail("Überlauf"))?),
            (Double(x), Double(y)) => Double(x + y),
            (String(x), String(y)) => {
                self.meter.charge(text_cost(x.len() + y.len()))?;
                String(format!("{x}{y}").into())
            }
            (Bytes(x), Bytes(y)) => Bytes([x.as_ref(), y.as_ref()].concat().into()),
            (List(x), List(y)) => {
                self.meter.charge((x.len() + y.len()) as u64)?;
                let mut out = x.as_ref().clone();
                out.extend(y.iter().cloned());
                Value::list(out)
            }
            (Timestamp(t), Duration(d)) | (Duration(d), Timestamp(t)) => {
                Timestamp(t.checked_add(*d).ok_or_else(|| fail("Überlauf"))?)
            }
            (Duration(x), Duration(y)) => {
                Duration(x.checked_add(*y).ok_or_else(|| fail("Überlauf"))?)
            }
            _ => {
                return Err(fail(format!(
                    "`+` für {} und {}",
                    a.type_name(),
                    b.type_name()
                )));
            }
        })
    }
}

fn select(target: &Value, field: &str, declared: bool, test: bool) -> Res {
    match target {
        Value::Map(map) => {
            let found = map.get(&Key::String(field.into()));
            if test {
                return Ok(Value::Bool(match found {
                    Some(Value::Null) => !declared,
                    Some(_) => true,
                    None => false,
                }));
            }
            match found {
                Some(v) => Ok(v.clone()),
                None if declared => Ok(Value::Null),
                None => Err(fail(format!("Schlüssel `{field}` fehlt"))),
            }
        }
        Value::Null if declared => {
            if test {
                Ok(Value::Bool(false))
            } else {
                Err(fail(format!("Feld `{field}` auf null")))
            }
        }
        other => Err(fail(format!("Feld `{field}` auf {}", other.type_name()))),
    }
}

fn compare(a: &Value, b: &Value) -> Result<Ordering, EvalError> {
    use Value::*;
    let ord = match (a, b) {
        (String(x), String(y)) => Some(x.cmp(y)),
        (Bytes(x), Bytes(y)) => Some(x.cmp(y)),
        (Bool(x), Bool(y)) => Some(x.cmp(y)),
        (Timestamp(x), Timestamp(y)) => Some(x.cmp(y)),
        (Duration(x), Duration(y)) => Some(x.cmp(y)),
        _ => numeric_cmp(a, b),
    };
    ord.ok_or_else(|| {
        fail(format!(
            "{} und {} sind nicht vergleichbar",
            a.type_name(),
            b.type_name()
        ))
    })
}

fn sub(a: &Value, b: &Value) -> Res {
    use Value::*;
    Ok(match (a, b) {
        (Int(x), Int(y)) => Int(x.checked_sub(*y).ok_or_else(|| fail("Überlauf"))?),
        (Uint(x), Uint(y)) => Uint(x.checked_sub(*y).ok_or_else(|| fail("Überlauf"))?),
        (Double(x), Double(y)) => Double(x - y),
        (Timestamp(x), Timestamp(y)) => Duration(*x - *y),
        (Timestamp(t), Duration(d)) => {
            Timestamp(t.checked_sub(*d).ok_or_else(|| fail("Überlauf"))?)
        }
        (Duration(x), Duration(y)) => Duration(x.checked_sub(*y).ok_or_else(|| fail("Überlauf"))?),
        _ => {
            return Err(fail(format!(
                "`-` für {} und {}",
                a.type_name(),
                b.type_name()
            )));
        }
    })
}

fn muldiv(func: &Func, a: &Value, b: &Value) -> Res {
    use Value::*;
    let zero = || fail("Division durch null");
    let over = || fail("Überlauf");
    Ok(match (func, a, b) {
        (Func::Mul, Int(x), Int(y)) => Int(x.checked_mul(*y).ok_or_else(over)?),
        (Func::Mul, Uint(x), Uint(y)) => Uint(x.checked_mul(*y).ok_or_else(over)?),
        (Func::Mul, Double(x), Double(y)) => Double(x * y),
        (Func::Div, Int(_), Int(0)) | (Func::Mod, Int(_), Int(0)) => return Err(zero()),
        (Func::Div, Uint(_), Uint(0)) | (Func::Mod, Uint(_), Uint(0)) => return Err(zero()),
        (Func::Div, Int(x), Int(y)) => Int(x.checked_div(*y).ok_or_else(over)?),
        (Func::Div, Uint(x), Uint(y)) => Uint(x / y),
        (Func::Div, Double(x), Double(y)) => Double(x / y),
        (Func::Mod, Int(x), Int(y)) => Int(x.checked_rem(*y).ok_or_else(over)?),
        (Func::Mod, Uint(x), Uint(y)) => Uint(x % y),
        _ => {
            return Err(fail(format!(
                "Operator für {} und {} nicht definiert",
                a.type_name(),
                b.type_name()
            )));
        }
    })
}

fn index(target: &Value, key: &Value) -> Res {
    match target {
        Value::List(items) => {
            let i = match key {
                Value::Int(i) => usize::try_from(*i).ok(),
                Value::Uint(u) => usize::try_from(*u).ok(),
                Value::Double(d) if d.fract() == 0.0 && *d >= 0.0 => Some(*d as usize),
                other => return Err(fail(format!("Listenindex {}", other.type_name()))),
            };
            i.and_then(|i| items.get(i).cloned())
                .ok_or_else(|| fail("Index außerhalb der Liste"))
        }
        Value::Map(map) => Key::from_value(key)
            .and_then(|k| map.get(&k).cloned())
            // Ohne den Schlüssel selbst: er kann aus den Tool-Argumenten stammen (#234).
            .ok_or_else(|| fail(format!("Schlüssel ({}) fehlt", key.type_name()))),
        other => Err(fail(format!("Index auf {}", other.type_name()))),
    }
}

fn to_int(v: &Value) -> Result<i64, EvalError> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Uint(u) => i64::try_from(*u).map_err(|_| fail("Überlauf")),
        Value::Double(d) if d.is_finite() && d.abs() < 9.2e18 => Ok(d.trunc() as i64),
        Value::String(s) => s.parse().map_err(|_| fail("String ist keine ganze Zahl")),
        Value::Timestamp(t) => Ok(t.unix_timestamp()),
        other => Err(fail(format!("int() auf {}", other.type_name()))),
    }
}

fn argv(v: &Value) -> Result<Vec<Vec<String>>, EvalError> {
    let bad = || fail("argv muss eine Liste von String-Listen sein");
    let Value::List(cmds) = v else {
        return Err(bad());
    };
    cmds.iter()
        .map(|c| match c {
            Value::List(words) => words
                .iter()
                .map(|w| match w {
                    Value::String(s) => Ok(s.to_string()),
                    _ => Err(bad()),
                })
                .collect(),
            _ => Err(bad()),
        })
        .collect()
}
