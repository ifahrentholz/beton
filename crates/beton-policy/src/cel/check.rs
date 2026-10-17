//! Typprüfung und Übersetzung des cel-rust-AST in den beton-eigenen Auswertungsbaum.

use std::sync::Arc;

use cel::IdedExpr;
use cel::common::ast::{EntryExpr, Expr, LiteralValue, SourceInfo, operators as op};
use regex::Regex;

use super::CompileError;
use super::ext;
use crate::Phase;
use crate::types::{DECLS, Ty};
use crate::value::Value;

/// Maximale Verschachtelungstiefe beim Parsen (schützt Stack und Laufzeit).
const MAX_DEPTH: u16 = 64;

/// Maximale Klammerverschachtelung im Quelltext (vor dem Parsen geprüft).
const MAX_NESTING: usize = 32;

/// Maximale Tiefe des Auswertungsbaums. Die Auswertung läuft rekursiv auf dem Thread des
/// Aufrufers (z. B. einem Tokio-Worker mit 2 MiB Stack); die Tiefe ist deshalb begrenzt.
const MAX_AST_DEPTH: usize = 128;

/// Stack für Parser und Typprüfung: der ANTLR-Parser von cel-rust braucht im Debug-Build
/// über 100 KiB je Verschachtelungsebene.
const PARSE_STACK: usize = 64 << 20;

/// Aufgelöste Funktion bzw. Operator.
#[derive(Debug, Clone)]
pub(crate) enum Func {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Index,
    In,
    Size,
    /// Vorkompiliert, wenn das Muster ein Literal ist.
    Matches(Option<Arc<Regex>>),
    StartsWith,
    EndsWith,
    Contains,
    Duration,
    Timestamp,
    ToInt,
    ToUint,
    ToDouble,
    ToString,
    Glob(Option<Arc<Regex>>),
    Under,
    Resolve,
    DomainMatches,
    BytesHuman,
    /// `priced(model)`: Listenpreis im Preis-Katalog (POL-011).
    Priced,
    HasCommand,
    HasFlag,
    GitOps,
    GitPushTargets,
    GitElsewhere,
    UrlHost,
    UrlPath,
}

/// Knoten des Auswertungsbaums.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    Lit(Value),
    /// Variable der obersten Ebene (`session`, `tool`, …).
    Var(Arc<str>),
    /// Laufvariable einer Comprehension.
    Local(Arc<str>),
    Select {
        operand: Box<Node>,
        field: Arc<str>,
        /// Feld eines deklarierten Objekts: fehlt es, ist der Wert `null` (statt Fehler).
        declared: bool,
        /// `has(x.f)`
        test: bool,
    },
    Call(Func, Vec<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Not(Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    NotStrictlyFalse(Box<Node>),
    List(Vec<Node>),
    Map(Vec<(Node, Node)>),
    Compr(Box<Compr>),
}

#[derive(Debug, Clone)]
pub(crate) struct Compr {
    pub range: Node,
    pub iter_var: Arc<str>,
    pub accu_var: Arc<str>,
    pub init: Node,
    pub cond: Node,
    pub step: Node,
    pub result: Node,
}

struct Checker<'a> {
    phase: Phase,
    src: &'a str,
    info: &'a SourceInfo,
    locals: Vec<(String, Ty)>,
    depth: usize,
}

type Res<T> = Result<T, CompileError>;

/// Parst, prüft und übersetzt `src`. Läuft auf einem eigenen Thread mit großem Stack,
/// damit tief verschachtelte Ausdrücke nie den Prozess abbrechen (fail closed als Ladefehler).
pub(crate) fn check(src: &str, phase: Phase) -> Res<(Node, Ty)> {
    nesting(src)?;
    let spawn_failed = |msg: String| CompileError {
        line: 1,
        column: 1,
        message: format!("Ausdruck konnte nicht geprüft werden: {msg}"),
    };
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("beton-policy-cel".into())
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, || check_inner(src, phase))
            .map_err(|e| spawn_failed(e.to_string()))?
            .join()
            .unwrap_or_else(|_| Err(spawn_failed("interner Fehler".into())))
    })
}

/// Klammerverschachtelung außerhalb von String-Literalen.
fn nesting(src: &str) -> Res<()> {
    let (mut depth, mut line, mut column) = (0usize, 1u32, 0u32);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in src.chars() {
        column += 1;
        if c == '\n' {
            line += 1;
            column = 0;
        }
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => {
                depth += 1;
                if depth > MAX_NESTING {
                    return Err(CompileError {
                        line,
                        column,
                        message: format!(
                            "Ausdruck ist tiefer als {MAX_NESTING} Ebenen verschachtelt"
                        ),
                    });
                }
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn check_inner(src: &str, phase: Phase) -> Res<(Node, Ty)> {
    let parser = cel::parser::Parser::new().max_recursion_depth(MAX_DEPTH);
    let (expr, info) = parser.parse_with_source_info(src).map_err(|errs| {
        let first = errs.errors.first();
        let (line, col) = first.map_or((1, 0), |e| e.pos);
        CompileError {
            line: u32::try_from(line.max(1)).unwrap_or(1),
            column: u32::try_from(col.max(0) + 1).unwrap_or(1),
            message: format!(
                "Syntaxfehler: {}",
                first.map_or_else(|| "unbekannt".to_owned(), |e| e.msg.clone())
            ),
        }
    })?;
    let mut checker = Checker {
        phase,
        src,
        info: &info,
        locals: Vec::new(),
        depth: 0,
    };
    checker.expr(&expr)
}

impl Checker<'_> {
    fn err(&self, id: u64, message: impl Into<String>) -> CompileError {
        let offset = self
            .info
            .offset_for(id)
            .map_or(0, |(start, _)| start as usize);
        let (mut line, mut column) = (1u32, 1u32);
        for (i, c) in self.src.chars().enumerate() {
            if i >= offset {
                break;
            }
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        CompileError {
            line,
            column,
            message: message.into(),
        }
    }

    fn local(&self, name: &str) -> Option<&Ty> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
    }

    fn expr(&mut self, e: &IdedExpr) -> Res<(Node, Ty)> {
        self.depth += 1;
        if self.depth > MAX_AST_DEPTH {
            return Err(self.err(
                e.id,
                format!("Ausdruck ist zu tief geschachtelt (mehr als {MAX_AST_DEPTH} Ebenen)"),
            ));
        }
        let result = self.expr_inner(e);
        self.depth -= 1;
        result
    }

    fn expr_inner(&mut self, e: &IdedExpr) -> Res<(Node, Ty)> {
        match &e.expr {
            Expr::Literal(lit) => Ok(literal(lit)),
            Expr::Ident(name) => self.ident(e.id, name),
            Expr::Select(sel) => {
                let (operand, oty) = self.expr(&sel.operand)?;
                let field: Arc<str> = sel.field.as_str().into();
                let (ty, declared) = match &oty {
                    Ty::Object(obj) => match DECLS.field(obj, &sel.field) {
                        Some(_) if let Some(why) = crate::vars::unavailable(obj, &sel.field) => {
                            return Err(
                                self.err(e.id, unavailable_msg(&sel.operand, &sel.field, why))
                            );
                        }
                        Some(t) => (t.clone(), true),
                        None => {
                            return Err(self.err(
                                e.id,
                                format!("unbekanntes Feld `{}` in `{obj}`", sel.field),
                            ));
                        }
                    },
                    Ty::Map(k, v) if k.assignable_to(&Ty::String) => ((**v).clone(), false),
                    Ty::Dyn => (Ty::Dyn, false),
                    other => {
                        return Err(self.err(
                            e.id,
                            format!("Feldzugriff `.{}` auf {other} nicht möglich", sel.field),
                        ));
                    }
                };
                let node = Node::Select {
                    operand: Box::new(operand),
                    field,
                    declared,
                    test: sel.test,
                };
                Ok((node, if sel.test { Ty::Bool } else { ty }))
            }
            Expr::Call(call) => self.call(e.id, call),
            Expr::List(list) => {
                if !list.optional_indices.is_empty() {
                    return Err(self.err(e.id, "optionale Listeneinträge werden nicht unterstützt"));
                }
                let mut nodes = Vec::new();
                let mut elem = None::<Ty>;
                for item in &list.elements {
                    let (n, t) = self.expr(item)?;
                    elem = Some(match elem {
                        None => t,
                        Some(prev) => prev.join(&t),
                    });
                    nodes.push(n);
                }
                Ok((Node::List(nodes), Ty::list(elem.unwrap_or(Ty::Dyn))))
            }
            Expr::Map(map) => {
                let mut entries = Vec::new();
                let (mut kty, mut vty) = (None::<Ty>, None::<Ty>);
                for entry in &map.entries {
                    let EntryExpr::MapEntry(me) = &entry.expr else {
                        return Err(self.err(entry.id, "ungültiger Map-Eintrag"));
                    };
                    if me.optional {
                        return Err(
                            self.err(entry.id, "optionale Map-Einträge werden nicht unterstützt")
                        );
                    }
                    let (k, kt) = self.expr(&me.key)?;
                    if !matches!(kt, Ty::String | Ty::Int | Ty::Uint | Ty::Bool | Ty::Dyn) {
                        return Err(self.err(entry.id, format!("{kt} ist kein Map-Schlüssel")));
                    }
                    let (v, vt) = self.expr(&me.value)?;
                    kty = Some(kty.map_or(kt.clone(), |p| p.join(&kt)));
                    vty = Some(vty.map_or(vt.clone(), |p| p.join(&vt)));
                    entries.push((k, v));
                }
                Ok((
                    Node::Map(entries),
                    Ty::map(kty.unwrap_or(Ty::Dyn), vty.unwrap_or(Ty::Dyn)),
                ))
            }
            Expr::Comprehension(c) => {
                if c.iter_var2.is_some() {
                    return Err(self.err(
                        e.id,
                        "Comprehensions mit zwei Variablen werden nicht unterstützt",
                    ));
                }
                let (range, rty) = self.expr(&c.iter_range)?;
                let elem = match &rty {
                    Ty::List(t) => (**t).clone(),
                    Ty::Map(k, _) => (**k).clone(),
                    Ty::Dyn => Ty::Dyn,
                    other => {
                        return Err(self.err(
                            c.iter_range.id,
                            format!("über {other} kann nicht iteriert werden"),
                        ));
                    }
                };
                let (init, ity) = self.expr(&c.accu_init)?;
                self.locals.push((c.accu_var.clone(), ity.clone()));
                self.locals.push((c.iter_var.clone(), elem));
                let cond = self.expr(&c.loop_cond);
                let step = self.expr(&c.loop_step);
                self.locals.pop();
                let (cond, cty) = cond?;
                let (step, sty) = step?;
                if !cty.assignable_to(&Ty::Bool) {
                    return Err(self.err(
                        c.loop_cond.id,
                        format!("Bedingung liefert {cty} statt bool"),
                    ));
                }
                // Der Akkumulator hat den gemeinsamen Typ aus Start und Schritt.
                let accu_ty = ity.join(&sty);
                if let Some(slot) = self.locals.last_mut() {
                    slot.1 = accu_ty;
                }
                let result = self.expr(&c.result);
                self.locals.pop();
                let (result, rty) = result?;
                Ok((
                    Node::Compr(Box::new(Compr {
                        range,
                        iter_var: c.iter_var.as_str().into(),
                        accu_var: c.accu_var.as_str().into(),
                        init,
                        cond,
                        step,
                        result,
                    })),
                    rty,
                ))
            }
            Expr::Struct(_) => Err(self.err(e.id, "Struct-Literale werden nicht unterstützt")),
            Expr::Unspecified => Err(self.err(e.id, "leerer Ausdruck")),
        }
    }

    fn ident(&self, id: u64, name: &str) -> Res<(Node, Ty)> {
        if let Some(t) = self.local(name) {
            return Ok((Node::Local(name.into()), t.clone()));
        }
        match DECLS.var(name, self.phase) {
            Ok(Ty::Object(obj)) if let Some(why) = crate::vars::unavailable(obj, "") => Err(self
                .err(
                    id,
                    format!("`{name}` ist noch nicht verfügbar: {why} (POL-004)"),
                )),
            Ok(t) => Ok((Node::Var(name.into()), t.clone())),
            Err(Some(phases)) => {
                let list: Vec<&str> = phases.iter().map(|p| p.as_str()).collect();
                Err(self.err(
                    id,
                    format!(
                        "`{name}` gibt es in Phase {} nicht, nur in {}",
                        self.phase,
                        list.join(", ")
                    ),
                ))
            }
            Err(None) => Err(self.err(id, format!("unbekannte Variable `{name}`"))),
        }
    }

    fn call(&mut self, id: u64, call: &cel::common::ast::CallExpr) -> Res<(Node, Ty)> {
        let name = call.func_name.as_str();
        // Namensraum-Funktionen `shell.*` und `url.*`
        if let Some(target) = &call.target
            && let Expr::Ident(ns) = &target.expr
            && (ns == "shell" || ns == "url")
            && self.local(ns).is_none()
        {
            let qualified = format!("{ns}.{name}");
            let args = self.args(&call.args)?;
            return self.typed(id, &qualified, args);
        }
        match name {
            op::LOGICAL_AND | op::LOGICAL_OR => {
                let [(a, at), (b, bt)] = self.fixed::<2>(id, name, &call.args)?;
                for t in [&at, &bt] {
                    if !t.assignable_to(&Ty::Bool) {
                        return Err(self.err(
                            id,
                            format!("`{}` erwartet bool, gefunden {t}", symbol(name)),
                        ));
                    }
                }
                let node = if name == op::LOGICAL_AND {
                    Node::And(Box::new(a), Box::new(b))
                } else {
                    Node::Or(Box::new(a), Box::new(b))
                };
                Ok((node, Ty::Bool))
            }
            op::LOGICAL_NOT | op::NOT_STRICTLY_FALSE => {
                let [(a, at)] = self.fixed::<1>(id, name, &call.args)?;
                if !at.assignable_to(&Ty::Bool) {
                    return Err(self.err(id, format!("`!` erwartet bool, gefunden {at}")));
                }
                let node = if name == op::LOGICAL_NOT {
                    Node::Not(Box::new(a))
                } else {
                    Node::NotStrictlyFalse(Box::new(a))
                };
                Ok((node, Ty::Bool))
            }
            op::CONDITIONAL => {
                let [(c, ct), (a, at), (b, bt)] = self.fixed::<3>(id, name, &call.args)?;
                if !ct.assignable_to(&Ty::Bool) {
                    return Err(self.err(id, format!("Bedingung von `?:` ist {ct} statt bool")));
                }
                Ok((
                    Node::Cond(Box::new(c), Box::new(a), Box::new(b)),
                    at.join(&bt),
                ))
            }
            _ => {
                let mut args = Vec::new();
                if let Some(t) = &call.target {
                    args.push(self.expr(t)?);
                }
                args.extend(self.args(&call.args)?);
                let qualified = if call.target.is_some() {
                    format!(".{name}")
                } else {
                    name.to_owned()
                };
                self.typed(id, &qualified, args)
            }
        }
    }

    fn args(&mut self, args: &[IdedExpr]) -> Res<Vec<(Node, Ty)>> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    fn fixed<const N: usize>(
        &mut self,
        id: u64,
        name: &str,
        args: &[IdedExpr],
    ) -> Res<[(Node, Ty); N]> {
        let checked = self.args(args)?;
        checked
            .try_into()
            .map_err(|_| self.err(id, format!("`{}` erwartet {N} Argumente", symbol(name))))
    }

    /// Typprüfung der Operatoren und Funktionen. `name` ist `f` (global), `.f` (Methode,
    /// Empfänger als erstes Argument) oder `ns.f`.
    fn typed(&self, id: u64, name: &str, args: Vec<(Node, Ty)>) -> Res<(Node, Ty)> {
        let types: Vec<Ty> = args.iter().map(|(_, t)| t.clone()).collect();
        let lits: Vec<Option<String>> = args
            .iter()
            .map(|(n, _)| match n {
                Node::Lit(Value::String(s)) => Some(s.to_string()),
                _ => None,
            })
            .collect();
        let nodes: Vec<Node> = args.into_iter().map(|(n, _)| n).collect();
        let shown = name.trim_start_matches('.');
        let mismatch = || {
            let list: Vec<String> = types.iter().map(ToString::to_string).collect();
            self.err(
                id,
                format!(
                    "`{}` ist für ({}) nicht definiert",
                    symbol(shown),
                    list.join(", ")
                ),
            )
        };
        let is = |i: usize, t: &Ty| types.get(i).is_some_and(|x| x.assignable_to(t));
        let n = types.len();
        let any = |t: &Ty| *t == Ty::Dyn;
        let (func, ret) = match name {
            op::EQUALS | op::NOT_EQUALS => {
                let (a, b) = (&types[0], &types[1]);
                let ok = a.assignable_to(b)
                    || b.assignable_to(a)
                    || (a.is_numeric() && b.is_numeric())
                    || *a == Ty::Null
                    || *b == Ty::Null;
                if !ok {
                    return Err(mismatch());
                }
                (
                    if name == op::EQUALS {
                        Func::Eq
                    } else {
                        Func::Ne
                    },
                    Ty::Bool,
                )
            }
            op::LESS | op::LESS_EQUALS | op::GREATER | op::GREATER_EQUALS => {
                let (a, b) = (&types[0], &types[1]);
                let ordered = |t: &Ty| {
                    matches!(
                        t,
                        Ty::String | Ty::Bytes | Ty::Timestamp | Ty::Duration | Ty::Bool
                    )
                };
                let ok = any(a)
                    || any(b)
                    || (a.is_numeric() && b.is_numeric())
                    || (a == b && ordered(a));
                if !ok {
                    return Err(mismatch());
                }
                let f = match name {
                    op::LESS => Func::Lt,
                    op::LESS_EQUALS => Func::Le,
                    op::GREATER => Func::Gt,
                    _ => Func::Ge,
                };
                (f, Ty::Bool)
            }
            op::ADD | op::SUBSTRACT | op::MULTIPLY | op::DIVIDE | op::MODULO => {
                let ret = arith(name, &types[0], &types[1]).ok_or_else(mismatch)?;
                let f = match name {
                    op::ADD => Func::Add,
                    op::SUBSTRACT => Func::Sub,
                    op::MULTIPLY => Func::Mul,
                    op::DIVIDE => Func::Div,
                    _ => Func::Mod,
                };
                (f, ret)
            }
            op::NEGATE => match &types[0] {
                t @ (Ty::Int | Ty::Double | Ty::Duration | Ty::Dyn) => (Func::Neg, t.clone()),
                _ => return Err(mismatch()),
            },
            op::INDEX => {
                let ret = match (&types[0], &types[1]) {
                    (Ty::List(t), Ty::Int | Ty::Uint | Ty::Dyn) => (**t).clone(),
                    (Ty::Map(k, v), key) if key.assignable_to(k) => (**v).clone(),
                    (Ty::Dyn, _) => Ty::Dyn,
                    _ => return Err(mismatch()),
                };
                (Func::Index, ret)
            }
            op::IN => {
                let ok = match &types[1] {
                    Ty::List(t) => {
                        types[0].assignable_to(t) || (types[0].is_numeric() && t.is_numeric())
                    }
                    Ty::Map(k, _) => types[0].assignable_to(k),
                    Ty::Dyn => true,
                    _ => false,
                };
                if !ok {
                    return Err(mismatch());
                }
                (Func::In, Ty::Bool)
            }
            "size" | ".size" if n == 1 => {
                if !matches!(
                    types[0],
                    Ty::String | Ty::Bytes | Ty::List(_) | Ty::Map(..) | Ty::Dyn
                ) {
                    return Err(mismatch());
                }
                (Func::Size, Ty::Int)
            }
            "matches" | ".matches" if n == 2 => {
                if !(is(0, &Ty::String) && is(1, &Ty::String)) {
                    return Err(mismatch());
                }
                let re = match lits[1].as_deref() {
                    Some(p) => Some(Arc::new(ext::regex(p).map_err(|e| self.err(id, e))?)),
                    None => None,
                };
                (Func::Matches(re), Ty::Bool)
            }
            ".startsWith" | ".endsWith" | ".contains" if n == 2 => {
                if !(is(0, &Ty::String) && is(1, &Ty::String)) {
                    return Err(mismatch());
                }
                let f = match name {
                    ".startsWith" => Func::StartsWith,
                    ".endsWith" => Func::EndsWith,
                    _ => Func::Contains,
                };
                (f, Ty::Bool)
            }
            "duration" | "timestamp" if n == 1 => {
                if !is(0, &Ty::String) {
                    return Err(mismatch());
                }
                if let Some(lit) = lits[0].as_deref() {
                    let valid = if name == "duration" {
                        ext::parse_duration(lit).map(|_| ())
                    } else {
                        time::OffsetDateTime::parse(
                            lit,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .map(|_| ())
                        .map_err(|e| format!("ungültiger Zeitstempel `{lit}`: {e}"))
                    };
                    valid.map_err(|e| self.err(id, e))?;
                }
                if name == "duration" {
                    (Func::Duration, Ty::Duration)
                } else {
                    (Func::Timestamp, Ty::Timestamp)
                }
            }
            "int" | "uint" | "double" if n == 1 => {
                if !matches!(
                    types[0],
                    Ty::Int | Ty::Uint | Ty::Double | Ty::String | Ty::Timestamp | Ty::Dyn
                ) {
                    return Err(mismatch());
                }
                match name {
                    "int" => (Func::ToInt, Ty::Int),
                    "uint" => (Func::ToUint, Ty::Uint),
                    _ => (Func::ToDouble, Ty::Double),
                }
            }
            "string" if n == 1 => (Func::ToString, Ty::String),
            "glob" | "under" | "domain_matches" if n == 2 => {
                if !(is(0, &Ty::String) && is(1, &Ty::String)) {
                    return Err(mismatch());
                }
                match name {
                    "glob" => {
                        let re = match lits[1].as_deref() {
                            Some(p) => {
                                Some(Arc::new(ext::glob_regex(p).map_err(|e| self.err(id, e))?))
                            }
                            None => None,
                        };
                        (Func::Glob(re), Ty::Bool)
                    }
                    "under" => (Func::Under, Ty::Bool),
                    _ => (Func::DomainMatches, Ty::Bool),
                }
            }
            "priced" if n == 1 => {
                if !is(0, &Ty::String) {
                    return Err(mismatch());
                }
                (Func::Priced, Ty::Bool)
            }
            "resolve" if n == 1 => {
                if !is(0, &Ty::String) {
                    return Err(mismatch());
                }
                (Func::Resolve, Ty::String)
            }
            "bytes_human" if n == 1 => {
                if !matches!(types[0], Ty::Int | Ty::Uint | Ty::Dyn) {
                    return Err(mismatch());
                }
                (Func::BytesHuman, Ty::String)
            }
            "shell.has_command" if n == 2 || n == 3 => {
                let argv = Ty::list(Ty::list(Ty::String));
                if !(is(0, &argv) && (1..n).all(|i| is(i, &Ty::String))) {
                    return Err(mismatch());
                }
                (Func::HasCommand, Ty::Bool)
            }
            "shell.has_flag" if n == 2 => {
                if !(is(0, &Ty::list(Ty::list(Ty::String))) && is(1, &Ty::String)) {
                    return Err(mismatch());
                }
                (Func::HasFlag, Ty::Bool)
            }
            "shell.git_ops" | "shell.git_push_targets" | "shell.git_elsewhere"
                if n == 1 || (n == 2 && name == "shell.git_push_targets") =>
            {
                if !is(0, &Ty::list(Ty::list(Ty::String))) || (n == 2 && !is(1, &Ty::Bool)) {
                    return Err(mismatch());
                }
                match name {
                    "shell.git_ops" => (Func::GitOps, Ty::list(Ty::String)),
                    "shell.git_push_targets" => (Func::GitPushTargets, Ty::list(Ty::String)),
                    _ => (Func::GitElsewhere, Ty::Bool),
                }
            }
            "url.host" | "url.path" if n == 1 => {
                if !is(0, &Ty::String) {
                    return Err(mismatch());
                }
                let f = if name == "url.host" {
                    Func::UrlHost
                } else {
                    Func::UrlPath
                };
                (f, Ty::String)
            }
            known @ ("size"
            | ".size"
            | "matches"
            | ".matches"
            | ".startsWith"
            | ".endsWith"
            | ".contains"
            | "duration"
            | "timestamp"
            | "int"
            | "uint"
            | "double"
            | "string"
            | "glob"
            | "under"
            | "resolve"
            | "domain_matches"
            | "bytes_human"
            | "priced"
            | "shell.has_command"
            | "shell.has_flag"
            | "shell.git_ops"
            | "shell.git_push_targets"
            | "shell.git_elsewhere"
            | "url.host"
            | "url.path") => {
                return Err(self.err(
                    id,
                    format!(
                        "`{}` mit {n} Argumenten ist nicht definiert",
                        known.trim_start_matches('.')
                    ),
                ));
            }
            other => {
                return Err(self.err(
                    id,
                    format!("unbekannte Funktion `{}`", other.trim_start_matches('.')),
                ));
            }
        };
        Ok((Node::Call(func, nodes), ret))
    }
}

/// Ergebnistyp arithmetischer Operatoren (CEL: kein Mischen von Zahlentypen).
fn arith(name: &str, a: &Ty, b: &Ty) -> Option<Ty> {
    use Ty::*;
    if *a == Dyn || *b == Dyn {
        let other = if *a == Dyn { b } else { a };
        return Some(if other.is_numeric() {
            other.clone()
        } else {
            Dyn
        });
    }
    match name {
        op::ADD => match (a, b) {
            (Int, Int) => Some(Int),
            (Uint, Uint) => Some(Uint),
            (Double, Double) => Some(Double),
            (String, String) => Some(String),
            (Bytes, Bytes) => Some(Bytes),
            (List(x), List(y)) => Some(Ty::list(x.join(y))),
            (Timestamp, Duration) | (Duration, Timestamp) => Some(Timestamp),
            (Duration, Duration) => Some(Duration),
            _ => None,
        },
        op::SUBSTRACT => match (a, b) {
            (Int, Int) => Some(Int),
            (Uint, Uint) => Some(Uint),
            (Double, Double) => Some(Double),
            (Timestamp, Timestamp) => Some(Duration),
            (Timestamp, Duration) => Some(Timestamp),
            (Duration, Duration) => Some(Duration),
            _ => None,
        },
        op::MULTIPLY | op::DIVIDE => match (a, b) {
            (Int, Int) => Some(Int),
            (Uint, Uint) => Some(Uint),
            (Double, Double) => Some(Double),
            _ => None,
        },
        _ => match (a, b) {
            (Int, Int) => Some(Int),
            (Uint, Uint) => Some(Uint),
            _ => None,
        },
    }
}

fn literal(lit: &LiteralValue) -> (Node, Ty) {
    let (v, t) = match lit {
        LiteralValue::Boolean(b) => (Value::Bool(*b.inner()), Ty::Bool),
        LiteralValue::Int(i) => (Value::Int(*i.inner()), Ty::Int),
        LiteralValue::UInt(u) => (Value::Uint(*u.inner()), Ty::Uint),
        LiteralValue::Double(d) => (Value::Double(*d.inner()), Ty::Double),
        LiteralValue::String(s) => (Value::String(s.inner().into()), Ty::String),
        LiteralValue::Bytes(b) => (Value::Bytes(b.inner().into()), Ty::Bytes),
        LiteralValue::Null => (Value::Null, Ty::Null),
    };
    (Node::Lit(v), t)
}

/// Lesbares Symbol eines Operators.
fn symbol(name: &str) -> &str {
    match name {
        op::LOGICAL_AND => "&&",
        op::LOGICAL_OR => "||",
        op::LOGICAL_NOT => "!",
        op::CONDITIONAL => "?:",
        op::EQUALS => "==",
        op::NOT_EQUALS => "!=",
        op::LESS => "<",
        op::LESS_EQUALS => "<=",
        op::GREATER => ">",
        op::GREATER_EQUALS => ">=",
        op::ADD => "+",
        op::SUBSTRACT => "-",
        op::MULTIPLY => "*",
        op::DIVIDE => "/",
        op::MODULO => "%",
        op::NEGATE => "-",
        op::INDEX => "[]",
        op::IN => "in",
        other => other,
    }
}

/// Meldung für ein deklariertes, aber noch nicht verfügbares Feld (#196): Pfad wie im Ausdruck
/// (`start.sandbox.profile`), dazu der Grund.
fn unavailable_msg(operand: &IdedExpr, field: &str, why: &str) -> String {
    fn path(e: &IdedExpr) -> Option<String> {
        match &e.expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::Select(sel) => Some(format!("{}.{}", path(&sel.operand)?, sel.field)),
            _ => None,
        }
    }
    let full = path(operand).map_or_else(|| field.to_owned(), |p| format!("{p}.{field}"));
    format!("`{full}` ist noch nicht verfügbar: {why}; eine Regel darauf griffe nie (POL-004)")
}
