//! CEL-Typen und die Deklaration der Kontext-Variablen je Phase (POL-004).
//!
//! Die Deklaration ist die Grundlage der Typprüfung beim Laden (POL-002 AC1): Jede Variable
//! und jedes Feld hat einen Typ; phasenspezifische Variablen sind nur in ihren Phasen
//! sichtbar. Die Befüllung zur Laufzeit folgt mit der Runner-Anbindung (WP-35).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use crate::Phase;

/// Ein CEL-Typ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Int,
    Uint,
    Double,
    String,
    Bytes,
    Null,
    Timestamp,
    Duration,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    /// Ein deklariertes Objekt (z. B. `session`) mit festen Feldern.
    Object(&'static str),
    /// Dynamisch, erst zur Laufzeit geprüft (z. B. `tool.args`).
    Dyn,
}

impl Ty {
    pub fn list(t: Ty) -> Ty {
        Ty::List(Box::new(t))
    }

    pub fn map(k: Ty, v: Ty) -> Ty {
        Ty::Map(Box::new(k), Box::new(v))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Uint | Ty::Double)
    }

    /// Kann ein Wert dieses Typs dort stehen, wo `other` erwartet wird?
    pub fn assignable_to(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Dyn, _) | (_, Ty::Dyn) => true,
            (Ty::List(a), Ty::List(b)) => a.assignable_to(b),
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => ka.assignable_to(kb) && va.assignable_to(vb),
            (a, b) => a == b,
        }
    }

    /// Gemeinsamer Typ zweier Zweige (Ternär, Listen-Literale); `Dyn`, wenn sie abweichen.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (a, b) if a == b => a.clone(),
            (Ty::Null, b) | (b, Ty::Null) => b.clone(),
            (Ty::List(a), Ty::List(b)) => Ty::list(a.join(b)),
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => Ty::map(ka.join(kb), va.join(vb)),
            _ => Ty::Dyn,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Bool => f.write_str("bool"),
            Ty::Int => f.write_str("int"),
            Ty::Uint => f.write_str("uint"),
            Ty::Double => f.write_str("double"),
            Ty::String => f.write_str("string"),
            Ty::Bytes => f.write_str("bytes"),
            Ty::Null => f.write_str("null"),
            Ty::Timestamp => f.write_str("timestamp"),
            Ty::Duration => f.write_str("duration"),
            Ty::List(t) => write!(f, "list({t})"),
            Ty::Map(k, v) => write!(f, "map({k}, {v})"),
            Ty::Object(name) => f.write_str(name),
            Ty::Dyn => f.write_str("dyn"),
        }
    }
}

/// Eine Kontext-Variable der obersten Ebene.
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: &'static str,
    pub ty: Ty,
    /// Phasen, in denen die Variable sichtbar ist; leer = alle Phasen.
    pub phases: Vec<Phase>,
}

/// Alle Deklarationen: Variablen und Objekttypen.
#[derive(Debug)]
pub struct Decls {
    pub vars: Vec<VarDecl>,
    pub objects: BTreeMap<&'static str, Vec<(&'static str, Ty)>>,
}

impl Decls {
    /// Typ der Variable `name` in `phase`. `Err` mit den Phasen, in denen sie gilt.
    pub fn var(&self, name: &str, phase: Phase) -> Result<&Ty, Option<&[Phase]>> {
        let mut other = None;
        for v in self.vars.iter().filter(|v| v.name == name) {
            if v.phases.is_empty() || v.phases.contains(&phase) {
                return Ok(&v.ty);
            }
            other = Some(v.phases.as_slice());
        }
        Err(other)
    }

    /// Typ des Felds `field` im Objekt `object`.
    pub fn field(&self, object: &str, field: &str) -> Option<&Ty> {
        self.objects
            .get(object)?
            .iter()
            .find(|(n, _)| *n == field)
            .map(|(_, t)| t)
    }
}

fn s() -> Ty {
    Ty::String
}
fn d() -> Ty {
    Ty::Double
}
fn i() -> Ty {
    Ty::Int
}
fn b() -> Ty {
    Ty::Bool
}
fn ts() -> Ty {
    Ty::Timestamp
}
fn strings() -> Ty {
    Ty::list(Ty::String)
}

/// Die Deklarationen aus POL-004 (03-policies.md).
pub static DECLS: LazyLock<Decls> = LazyLock::new(|| {
    use Phase::*;
    let mut objects = BTreeMap::new();
    objects.insert(
        "session",
        vec![
            ("id", s()),
            ("title", s()),
            ("harness", s()),
            ("mode", s()),
            ("model", s()),
            ("reasoning_effort", s()),
            ("permission_mode", s()),
            ("cost_usd", d()),
            ("tokens_in", i()),
            ("tokens_out", i()),
            ("turns", i()),
            ("tool_calls", i()),
            ("started_at", ts()),
            ("duration_s", d()),
            ("is_async", b()),
            ("attached_clients", i()),
            ("depth", i()),
            ("parent_id", s()),
            ("project", s()),
            ("worktree", s()),
            ("branch", s()),
            ("labels", strings()),
        ],
    );
    objects.insert(
        "run",
        vec![
            ("id", s()),
            ("trigger", s()),
            ("cost_usd", d()),
            ("subtree_cost_usd", d()),
            ("duration_s", d()),
            ("turns", i()),
            ("budget_usd", d()),
        ],
    );
    objects.insert(
        "user",
        vec![
            ("id", s()),
            ("email", s()),
            ("roles", strings()),
            ("teams", strings()),
            ("timezone", s()),
            ("daily_cost_usd", d()),
            ("monthly_cost_usd", d()),
        ],
    );
    objects.insert("org", vec![("id", s())]);
    objects.insert("team", vec![("ids", strings())]);
    objects.insert(
        "agent",
        vec![
            ("name", s()),
            ("version", s()),
            ("source", s()),
            ("hash", s()),
        ],
    );
    objects.insert(
        "host",
        vec![
            ("id", s()),
            ("os", s()),
            ("arch", s()),
            ("labels", strings()),
            ("local", b()),
        ],
    );
    objects.insert("actor", vec![("kind", s()), ("id", s())]);
    objects.insert(
        "budget",
        vec![
            ("online", b()),
            ("lease_remaining_usd", d()),
            ("lease_expires_at", ts()),
        ],
    );
    let state = Ty::map(Ty::String, Ty::Dyn);
    objects.insert(
        "state",
        vec![
            ("session", state.clone()),
            ("run", state.clone()),
            ("user_day", state.clone()),
            ("user", state),
        ],
    );
    objects.insert(
        "approvals",
        vec![("granted_in_session", i()), ("denied_in_session", i())],
    );
    objects.insert(
        "start",
        vec![
            ("trigger", s()),
            ("harness", s()),
            ("model", s()),
            ("reasoning_effort", s()),
            ("permission_mode", s()),
            ("sandbox", Ty::Object("start_sandbox")),
            ("network", Ty::Object("start_network")),
            ("worktree", s()),
            ("fork_of", s()),
            ("agent_ref", s()),
        ],
    );
    objects.insert(
        "start_sandbox",
        vec![("enabled", b()), ("backend", s()), ("profile", s())],
    );
    objects.insert("start_network", vec![("proxy_enabled", b())]);
    objects.insert(
        "request",
        vec![
            ("model", s()),
            ("provider", s()),
            ("reasoning_effort", s()),
            ("granularity", s()),
            ("prompt", s()),
            ("messages", Ty::list(Ty::Dyn)),
            ("tools", strings()),
            ("estimated_input_tokens", i()),
            ("context_used_pct", d()),
        ],
    );
    objects.insert(
        "tool",
        vec![
            ("name", s()),
            ("native_name", s()),
            ("server", s()),
            ("kind", s()),
            ("args", Ty::map(Ty::String, Ty::Dyn)),
            ("command", s()),
            ("argv", Ty::list(strings())),
            ("cwd", s()),
            ("paths", strings()),
            ("urls", strings()),
            ("call_index", i()),
            ("repeat_count", i()),
            ("consecutive_errors", i()),
            ("is_subagent_spawn", b()),
            ("parse_uncertain", b()),
        ],
    );
    objects.insert(
        "result",
        vec![
            ("ok", b()),
            ("exit_code", i()),
            ("error", s()),
            ("text", s()),
            ("bytes", i()),
            ("duration_ms", i()),
            ("truncated", b()),
        ],
    );
    objects.insert(
        "browser_navigate",
        vec![
            ("url", s()),
            ("scheme", s()),
            ("host", s()),
            ("path", s()),
            ("initiator", s()),
            ("tab_id", s()),
            ("profile", s()),
        ],
    );
    objects.insert(
        "browser_action",
        vec![
            ("action", s()),
            ("url", s()),
            ("host", s()),
            ("selector", s()),
            ("element_role", s()),
            ("is_form_submit", b()),
            ("is_password_field", b()),
            ("text_len", i()),
            ("file_name", s()),
        ],
    );

    let all = Vec::new;
    let var = |name, ty, phases| VarDecl { name, ty, phases };
    let vars = vec![
        var("now", ts(), all()),
        var("session", Ty::Object("session"), all()),
        var("run", Ty::Object("run"), all()),
        var("user", Ty::Object("user"), all()),
        var("org", Ty::Object("org"), all()),
        var("team", Ty::Object("team"), all()),
        var("agent", Ty::Object("agent"), all()),
        var("host", Ty::Object("host"), all()),
        var("actor", Ty::Object("actor"), all()),
        var("budget", Ty::Object("budget"), all()),
        var("state", Ty::Object("state"), all()),
        var("approvals", Ty::Object("approvals"), all()),
        var("start", Ty::Object("start"), vec![SessionStart]),
        var("request", Ty::Object("request"), vec![ModelRequest]),
        var("tool", Ty::Object("tool"), vec![ToolCall, ToolResult]),
        var("result", Ty::Object("result"), vec![ToolResult]),
        var(
            "browser",
            Ty::Object("browser_navigate"),
            vec![BrowserNavigate],
        ),
        var("browser", Ty::Object("browser_action"), vec![BrowserAction]),
    ];
    Decls { vars, objects }
});
