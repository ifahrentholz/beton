//! Deklarative Policy-Tests (POL-026, ADR-0031): Testformat und Ausführung von
//! `beton policy test`.
//!
//! Eine Testdatei (`*.policytest.yaml`) simuliert die Ebenen (`layers`: Policy-Dateien bzw.
//! `agent.yaml` mit `policies:`-Block, Pfade relativ zur Testdatei, `~/` = Home), setzt
//! Fixtures für den Kontext und beschreibt Fälle: eine Eingabe in einer Phase und die erwartete
//! Entscheidung, oder mehrere Schritte, die Zustand (`effects`), Zähler, Grants, Uhr und Kosten
//! von Schritt zu Schritt tragen.
//!
//! - Tool-Calls werden wie im Runner normalisiert (POL-005): `tool.name` ist der native Name
//!   des Harness (`harness`, Default `claude`), `argv`, `command`, `paths` … entstehen aus den
//!   Argumenten; ausdrücklich angegebene Felder gelten vorrangig.
//! - `harness` bestimmt außerdem Enforcement-Matrix und `modify`-Fähigkeiten (POL-024), sofern
//!   der Aufrufer eine Matrix liefert ([`TestOptions::matrix`]); sonst gilt `full`.
//! - Die Uhr ist simuliert (`fixtures.now`, Default 2026-01-01T12:00:00Z); Policy-Tests messen
//!   keine Laufzeit, das Zeitbudget je Auswertung ist deshalb großzügig.
//! - `tool.repeat_count` zählt identische Calls (Klasse und Argumente) unter den letzten 10
//!   Tool-Calls des Falls einschließlich des aktuellen, `session.tool_calls` und
//!   `tool.call_index` alle Tool-Calls des Falls.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Phase;
use crate::approval::plan;
use crate::audit::{DecisionMeta, decision_event};
use crate::cel::Limits;
use crate::context::Activation;
use crate::doc::{EffectOp, StateScope};
use crate::enforcement::{Level, Matrix};
use crate::engine::{Decision, EffectiveSet, EvalOptions, Outcome, Reason};
use crate::explain::{context_values, explain, render as render_explain};
use crate::grants::SessionGrants;
use crate::load::{LoadedSet, MAX_FILE_BYTES, Scope, inline_set, load_file};
use crate::normalize::{ToolSource, harness_family, normalize_source};
use crate::state::{FinalOutcome, PolicyState, effects_due, next_value};
use crate::yaml;

/// Endung der Testdateien.
pub const SUFFIX: &str = ".policytest.yaml";
/// Start der simulierten Uhr ohne `fixtures.now`.
pub const DEFAULT_NOW: &str = "2026-01-01T12:00:00Z";
/// Arbeitsverzeichnis für Tool-Calls ohne `tool.cwd` und `session.worktree`.
pub const DEFAULT_CWD: &str = "/repo";
/// Harness ohne Angabe.
pub const DEFAULT_HARNESS: &str = "claude";
/// Fenster für `tool.repeat_count`.
const REPEAT_WINDOW: usize = 10;

/// Liefert die Enforcement-Matrix eines Harness (im CLI aus den Harness-Capabilities).
pub type MatrixFn<'a> = &'a dyn Fn(&str) -> Result<Matrix, String>;

/// Optionen eines Testlaufs.
#[derive(Clone, Default)]
pub struct TestOptions<'a> {
    /// Home für `~/` in `layers` (Default: `$HOME`).
    pub home: Option<PathBuf>,
    /// Matrix je Harness; ohne Angabe gilt `full` mit allen Modifikationen.
    pub matrix: Option<MatrixFn<'a>>,
}

impl std::fmt::Debug for TestOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestOptions")
            .field("home", &self.home)
            .field("matrix", &self.matrix.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestFile {
    spec_version: u64,
    #[serde(default)]
    layers: BTreeMap<String, Vec<String>>,
    harness: Option<String>,
    #[serde(default)]
    fixtures: Map<String, Value>,
    #[serde(default)]
    cases: Vec<CaseDoc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseDoc {
    name: String,
    harness: Option<String>,
    phase: Option<Phase>,
    #[serde(default)]
    given: Map<String, Value>,
    input: Option<Map<String, Value>>,
    expect: Option<Expect>,
    resolve: Option<Resolve>,
    steps: Option<Vec<StepDoc>>,
    expect_state: Option<BTreeMap<String, Map<String, Value>>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepDoc {
    phase: Option<Phase>,
    #[serde(default)]
    given: Map<String, Value>,
    input: Option<Map<String, Value>>,
    expect: Option<Expect>,
    resolve: Option<Resolve>,
    repeat: Option<u32>,
    advance_time: Option<String>,
    cost_delta_usd: Option<f64>,
}

/// Erwartete Entscheidung; alle Felder optional, Listen und Objekte als Teilmenge.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<ExpectedOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rules: Option<Vec<String>>,
    /// Treffer von Regeln im Shadow-Betrieb (`dry_run`).
    #[serde(skip_serializing_if = "Option::is_none")]
    would: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_contains: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notifications: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enforcement: Option<Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedOutcome {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResolveDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResolveRemember {
    #[default]
    None,
    Session,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolve {
    decision: ResolveDecision,
    #[serde(default)]
    remember: ResolveRemember,
}

// ---------------------------------------------------------------------------------------
// Ergebnisse
// ---------------------------------------------------------------------------------------

/// Ergebnis eines Falls.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaseReport {
    pub name: String,
    /// Zeile des Falls in der Testdatei.
    pub line: u32,
    pub passed: bool,
    /// Abweichungen (leer, wenn bestanden).
    pub failures: Vec<String>,
    /// Erwartung und tatsächliche Entscheidung des maßgeblichen Schritts.
    pub expected: Value,
    pub actual: Value,
    /// Explain-Trace des maßgeblichen Schritts.
    pub trace: String,
    pub duration_us: u64,
}

/// Ergebnis einer Testdatei.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileReport {
    pub file: String,
    /// Die Datei ließ sich nicht laden (Format, Ebenen, Policies).
    pub error: Option<String>,
    pub cases: Vec<CaseReport>,
    pub duration_us: u64,
}

/// Zusammenfassung mehrerer Dateien.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    /// Dateien mit Ladefehler.
    pub errors: usize,
}

impl Summary {
    pub fn of(reports: &[FileReport]) -> Self {
        let mut s = Summary::default();
        for r in reports {
            if r.error.is_some() {
                s.errors += 1;
            }
            for c in &r.cases {
                if c.passed {
                    s.passed += 1;
                } else {
                    s.failed += 1;
                }
            }
        }
        s
    }

    /// 0 bei Erfolg, 1 bei Fehlschlag oder Ladefehler (POL-026 AC1).
    pub fn exit_code(&self) -> i32 {
        i32::from(self.failed > 0 || self.errors > 0)
    }
}

// ---------------------------------------------------------------------------------------
// Dateien finden und laden
// ---------------------------------------------------------------------------------------

/// Testdateien unter `paths` (Dateien direkt, Verzeichnisse rekursiv), sortiert.
pub fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("{}: nicht lesbar: {e}", dir.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|e| format!("{}: nicht lesbar: {e}", dir.display()))?
                .path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.to_string_lossy().ends_with(SUFFIX) {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut out)?;
        } else if p.is_file() {
            out.push(p.clone());
        } else {
            return Err(format!("{}: nicht gefunden", p.display()));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn read_text(path: &Path) -> Result<String, String> {
    let at = |m: String| format!("{}:1:1: {m}", path.display());
    let meta = std::fs::metadata(path).map_err(|e| at(format!("nicht lesbar: {e}")))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(at(format!("Datei größer als {MAX_FILE_BYTES} Bytes")));
    }
    std::fs::read_to_string(path).map_err(|e| at(format!("nicht lesbar: {e}")))
}

fn error_path(e: &serde_path_to_error::Error<serde_json::Error>) -> String {
    let path = e.path().to_string();
    let mut path = if path == "." { String::new() } else { path };
    let message = e.inner().to_string();
    if let Some(rest) = message.strip_prefix("unknown field `")
        && let Some(field) = rest.split('`').next()
    {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(field);
    }
    path
}

struct Parsed {
    file: TestFile,
    doc: yaml::Document,
}

fn parse(path: &Path) -> Result<Parsed, String> {
    let text = read_text(path)?;
    let doc = yaml::parse(&text).map_err(|e| {
        format!(
            "{}:{}:{}: {}",
            path.display(),
            e.pos.line,
            e.pos.column,
            e.message
        )
    })?;
    let file: TestFile = serde_path_to_error::deserialize(doc.value.clone()).map_err(|e| {
        let at = error_path(&e);
        let pos = doc.pos(&at);
        format!(
            "{}:{}:{}: {at}: {}",
            path.display(),
            pos.line,
            pos.column,
            e.inner()
        )
    })?;
    if file.spec_version != 1 {
        let pos = doc.pos("spec_version");
        return Err(format!(
            "{}:{}:{}: spec_version {} wird nicht unterstützt (nur 1)",
            path.display(),
            pos.line,
            pos.column,
            file.spec_version
        ));
    }
    Ok(Parsed { file, doc })
}

fn home_dir(opts: &TestOptions) -> Option<PathBuf> {
    opts.home
        .clone()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// Pfad einer Ebene: `~/` = Home, sonst relativ zur Testdatei.
fn resolve_layer(test: &Path, entry: &str, opts: &TestOptions) -> Result<PathBuf, String> {
    let path = if let Some(rest) = entry.strip_prefix("~/") {
        home_dir(opts)
            .ok_or_else(|| format!("{entry}: kein Home-Verzeichnis bekannt"))?
            .join(rest)
    } else {
        let p = PathBuf::from(entry);
        if p.is_absolute() {
            p
        } else {
            test.parent().unwrap_or(Path::new(".")).join(p)
        }
    };
    if path.is_dir() && path.join("agent.yaml").is_file() {
        return Ok(path.join("agent.yaml"));
    }
    Ok(path)
}

fn scope_of(key: &str) -> Option<Scope> {
    Scope::parse(key)
}

/// Die Dateien aller Ebenen einer Testdatei (für die Prüfung, dass jede mitgelieferte Policy
/// einen Test hat, QA-004 AC2).
pub fn layer_files(test: &Path, opts: &TestOptions) -> Result<Vec<PathBuf>, String> {
    let parsed = parse(test)?;
    let mut out = Vec::new();
    for entries in parsed.file.layers.values() {
        for e in entries {
            out.push(resolve_layer(test, e, opts)?);
        }
    }
    Ok(out)
}

/// Regel-IDs, die eine Testdatei in `expect.rules` erwartet (alle Fälle und Schritte); für
/// die Prüfung, dass jede mitgelieferte Regel getestet ist (QA-004 AC2).
pub fn expected_rules(test: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    let parsed = parse(test)?;
    let mut out = std::collections::BTreeSet::new();
    for case in &parsed.file.cases {
        let expects = case.expect.iter().chain(
            case.steps
                .iter()
                .flatten()
                .filter_map(|s| s.expect.as_ref()),
        );
        for e in expects {
            out.extend(e.rules.iter().flatten().cloned());
        }
    }
    Ok(out)
}

/// Lädt die `policies:` eines `agent.yaml` als Inline-Set der Ebene (AGT-014).
fn load_agent(scope: Scope, path: &Path) -> Result<Option<LoadedSet>, String> {
    let text = read_text(path)?;
    let doc = yaml::parse(&text).map_err(|e| {
        format!(
            "{}:{}:{}: {}",
            path.display(),
            e.pos.line,
            e.pos.column,
            e.message
        )
    })?;
    let name = doc.value["name"]
        .as_str()
        .ok_or_else(|| format!("{}:1:1: `name` fehlt", path.display()))?;
    let rules = match &doc.value["policies"] {
        Value::Null => return Ok(None),
        Value::Array(rules) => rules.clone(),
        _ => {
            let pos = doc.pos("policies");
            return Err(format!(
                "{}:{}:{}: `policies` muss eine Liste sein",
                path.display(),
                pos.line,
                pos.column
            ));
        }
    };
    inline_set(scope, name, &path.display().to_string(), &rules)
        .map(Some)
        .map_err(|e| {
            // Inline-Fehler tragen den Feldpfad (`policies[i]…`); Position aus dem Dokument.
            let field = e.message.split(':').next().unwrap_or_default();
            let pos = doc.pos(field);
            format!(
                "{}:{}:{}: {}",
                path.display(),
                pos.line,
                pos.column,
                e.message
            )
        })
}

fn load_layers(test: &Path, file: &TestFile, opts: &TestOptions) -> Result<EffectiveSet, String> {
    let mut sets = Vec::new();
    for (key, entries) in &file.layers {
        let scope = scope_of(key).ok_or_else(|| {
            format!(
                "{}: unbekannte Ebene `{key}` (org, team, user, project, agent)",
                test.display()
            )
        })?;
        for entry in entries {
            let path = resolve_layer(test, entry, opts)?;
            if !path.is_file() {
                return Err(format!(
                    "{}: Ebene {key}: {} nicht gefunden",
                    test.display(),
                    path.display()
                ));
            }
            if path.file_name().is_some_and(|n| n == "agent.yaml") {
                if let Some(set) = load_agent(scope, &path)? {
                    sets.push(Arc::new(set));
                }
            } else {
                sets.push(Arc::new(
                    load_file(scope, &path).map_err(|e| e.to_string())?,
                ));
            }
        }
    }
    Ok(EffectiveSet::new(sets))
}

// ---------------------------------------------------------------------------------------
// Kontext
// ---------------------------------------------------------------------------------------

/// Objekte rekursiv zusammenführen; `over` gewinnt.
fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Harness-Familie für die Normalisierung (der Fake-Harness spielt Claude-Namen ab).
fn family(harness: &str) -> String {
    match harness_family(harness) {
        "fake" => "claude".into(),
        other => other.into(),
    }
}

/// Normalisiert `tool` der Eingabe wie der Runner (POL-005); angegebene Felder gelten
/// vorrangig. Liefert den Kontext und die Herkunft (für `args`-Patches).
pub fn tool_context(
    input: &Map<String, Value>,
    harness: &str,
    cwd: &str,
    home: Option<&str>,
) -> Result<(Value, ToolSource), String> {
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .ok_or("`tool.name` fehlt")?;
    let args = input.get("args").cloned().unwrap_or_else(|| json!({}));
    let mut explicit = input.clone();
    let acp_kind = explicit
        .remove("acp_kind")
        .and_then(|v| v.as_str().map(str::to_owned));
    let source = ToolSource {
        harness: family(harness),
        native_name: name.to_owned(),
        acp_kind,
        mcp_server: input
            .get("server")
            .and_then(Value::as_str)
            .filter(|s| *s != "builtin")
            .map(str::to_owned),
        cwd: input
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(cwd)
            .to_owned(),
        home: home.map(str::to_owned),
        // Policy-Tests ohne Session: keine Herkunftsprüfung reservierter Server.
        trusted_servers: None,
    };
    let normalized = normalize_source(&source, &args).map_err(|e| e.to_string())?;
    let mut ctx = normalized.to_context();
    explicit.remove("name");
    explicit.remove("args");
    merge(&mut ctx, &Value::Object(explicit));
    Ok((ctx, source))
}

/// `host.home` ist das Home des Testlaufs, solange Fixtures es nicht setzen (POL-016).
fn with_home(ctx: &mut Value, home: Option<&str>) {
    if let Some(home) = home {
        ctx["host"]["home"] = Value::String(home.to_owned());
    }
}

/// Grundwerte des Kontexts (überschreibbar durch `fixtures` und `given`).
fn defaults(harness: &str, now: &str) -> Value {
    json!({
        "now": now,
        "session": { "id": "ses_policytest", "harness": harness, "mode": "native", "cost_usd": 0.0,
                     "tokens_in": 0, "tokens_out": 0, "turns": 0, "tool_calls": 0, "worktree": DEFAULT_CWD,
                     "project_root": DEFAULT_CWD },
        "user": { "id": "usr_local", "timezone": "UTC", "daily_cost_usd": 0.0, "monthly_cost_usd": 0.0 },
        "actor": { "kind": "agent" },
        "approvals": { "granted_in_session": 0, "denied_in_session": 0 },
    })
}

/// Baut den Kontext einer Auswertung aus Fixtures (gemeinsame Variablen) und Eingabe
/// (phasenspezifisch: `tool`, `request`, `start`, `result`, `browser`). Wird auch von
/// `beton policy eval` benutzt.
pub fn activation(
    phase: Phase,
    fixtures: &Value,
    input: &Map<String, Value>,
    harness: Option<&str>,
    home: Option<&str>,
) -> Result<Activation, String> {
    let harness = harness.unwrap_or(DEFAULT_HARNESS);
    let mut ctx = defaults(harness, DEFAULT_NOW);
    with_home(&mut ctx, home);
    merge(&mut ctx, fixtures);
    let (ctx, source) = with_input(phase, ctx, input, harness, home)?;
    build(phase, ctx, source)
}

fn with_input(
    phase: Phase,
    mut ctx: Value,
    input: &Map<String, Value>,
    harness: &str,
    home: Option<&str>,
) -> Result<(Value, Option<ToolSource>), String> {
    let mut input = input.clone();
    let mut source = None;
    if matches!(phase, Phase::ToolCall | Phase::ToolResult)
        && let Some(Value::Object(tool)) = input.remove("tool")
    {
        let cwd = ctx["session"]["worktree"]
            .as_str()
            .unwrap_or(DEFAULT_CWD)
            .to_owned();
        // `~` wie `host.home` des Kontexts (Fixtures), sonst das Home des Testlaufs.
        let fixture_home = ctx["host"]["home"].as_str().map(str::to_owned);
        let home = fixture_home.as_deref().or(home);
        let (t, s) = tool_context(&tool, harness, &cwd, home)?;
        ctx["tool"] = t;
        source = Some(s);
    }
    merge(&mut ctx, &Value::Object(input));
    Ok((ctx, source))
}

fn build(phase: Phase, ctx: Value, source: Option<ToolSource>) -> Result<Activation, String> {
    let mut act = Activation::new(phase, ctx).map_err(|e| e.to_string())?;
    if let Some(s) = source {
        act = act.with_tool_source(Arc::new(s));
    }
    Ok(act)
}

/// Auswertung ohne Zeitbudget-Risiko: Policy-Tests messen keine Laufzeit.
fn test_limits() -> Limits {
    Limits {
        eval_budget_us: 10_000_000,
        ..Limits::default()
    }
}

// ---------------------------------------------------------------------------------------
// Ausführung
// ---------------------------------------------------------------------------------------

/// Zustand eines Falls über seine Schritte.
struct CaseState {
    now: OffsetDateTime,
    cost_delta: f64,
    tool_calls: i64,
    granted: i64,
    denied: i64,
    state: PolicyState,
    user_day: BTreeMap<String, Value>,
    user: BTreeMap<String, Value>,
    grants: SessionGrants,
    history: Vec<String>,
}

/// Was ein Schritt auswertet.
#[derive(Debug, Clone)]
struct Eval {
    phase: Phase,
    given: Map<String, Value>,
    input: Map<String, Value>,
    expect: Expect,
    resolve: Option<Resolve>,
}

struct Ctx<'a> {
    set: &'a EffectiveSet,
    fixtures: Value,
    harness: String,
    matrix: Matrix,
    home: Option<String>,
}

/// Ergebnis eines ausgewerteten Schritts.
struct StepResult {
    failures: Vec<String>,
    expected: Value,
    actual: Value,
    trace: String,
}

fn outcome_str(o: &Outcome) -> &'static str {
    match o {
        Outcome::Allow => "allow",
        Outcome::Deny { .. } => "deny",
        Outcome::Ask { .. } => "ask",
    }
}

fn reasons(o: &Outcome) -> &[Reason] {
    match o {
        Outcome::Allow => &[],
        Outcome::Deny { reasons } | Outcome::Ask { reasons } => reasons,
    }
}

/// Ist `actual` eine Obermenge von `expected` (Objekte rekursiv, sonst Gleichheit)?
fn subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => e
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|x| subset(v, x))),
        (Value::Number(e), Value::Number(a)) => e.as_f64() == a.as_f64(),
        (e, a) => e == a,
    }
}

fn identity(tool: &Value) -> String {
    crate::normalize::call_identity(
        tool["kind"].as_str().unwrap_or_default(),
        tool["name"].as_str().unwrap_or_default(),
        &tool["args"],
    )
}

impl CaseState {
    fn new(fixtures: &Value) -> Result<Self, String> {
        let now_text = fixtures["now"].as_str().unwrap_or(DEFAULT_NOW);
        let now = OffsetDateTime::parse(now_text, &Rfc3339)
            .map_err(|_| format!("fixtures.now `{now_text}` ist kein RFC-3339-Zeitpunkt"))?;
        let mut s = Self {
            now,
            cost_delta: 0.0,
            tool_calls: fixtures["session"]["tool_calls"].as_i64().unwrap_or(0),
            granted: 0,
            denied: 0,
            state: PolicyState::default(),
            user_day: BTreeMap::new(),
            user: BTreeMap::new(),
            grants: SessionGrants::default(),
            history: Vec::new(),
        };
        s.set_state(&fixtures["state"]);
        Ok(s)
    }

    /// Übernimmt `state.<scope>` aus Fixtures bzw. `given` in den Zustand des Falls.
    fn set_state(&mut self, state: &Value) {
        let Value::Object(scopes) = state else { return };
        for (scope, values) in scopes {
            let Value::Object(values) = values else {
                continue;
            };
            for (k, v) in values {
                match scope.as_str() {
                    "session" | "run" => {
                        let scope = if scope == "session" {
                            StateScope::Session
                        } else {
                            StateScope::Run
                        };
                        let _ = self.state.apply(scope, k, EffectOp::Set, Some(v.clone()));
                    }
                    "user_day" => {
                        self.user_day.insert(k.clone(), v.clone());
                    }
                    "user" => {
                        self.user.insert(k.clone(), v.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    fn get(&self, scope: StateScope) -> &BTreeMap<String, Value> {
        match scope {
            StateScope::UserDay => &self.user_day,
            _ => &self.user,
        }
    }

    fn state_value(&self, scope: StateScope, key: &str) -> Option<Value> {
        match scope {
            StateScope::Session | StateScope::Run => self.state.get(scope, key).cloned(),
            other => self.get(other).get(key).cloned(),
        }
    }

    /// Kontext eines Schritts: Grundwerte, Fixtures, `given`, dann der Zustand des Falls.
    fn context(&mut self, ctx: &Ctx, e: &Eval, harness: &str) -> Result<Activation, String> {
        if let Some(state) = e.given.get("state") {
            self.set_state(state);
        }
        let mut given = e.given.clone();
        given.remove("state");
        let now = self.now.format(&Rfc3339).map_err(|e| e.to_string())?;
        let mut vars = defaults(harness, &now);
        with_home(&mut vars, ctx.home.as_deref());
        merge(&mut vars, &ctx.fixtures);
        vars["now"] = Value::String(now.clone());
        merge(&mut vars, &Value::Object(given.clone()));
        if e.phase == Phase::ToolCall {
            self.tool_calls += 1;
        }
        // Zähler und Kosten des Falls.
        let add = |v: &mut Value, delta: f64| {
            if let Some(x) = v.as_f64() {
                *v = json!(x + delta);
            }
        };
        if self.cost_delta != 0.0 {
            add(&mut vars["session"]["cost_usd"], self.cost_delta);
            add(&mut vars["user"]["daily_cost_usd"], self.cost_delta);
            add(&mut vars["user"]["monthly_cost_usd"], self.cost_delta);
            if vars["run"]["cost_usd"].is_number() {
                add(&mut vars["run"]["cost_usd"], self.cost_delta);
            }
        }
        if given
            .get("session")
            .and_then(|s| s.get("tool_calls"))
            .is_none()
        {
            vars["session"]["tool_calls"] = json!(self.tool_calls);
        }
        vars["approvals"] = json!({
            "granted_in_session": self.granted,
            "denied_in_session": self.denied,
        });
        vars["state"] = json!({
            "session": self.state.snapshot(StateScope::Session),
            "run": self.state.snapshot(StateScope::Run),
            "user_day": self.user_day,
            "user": self.user,
        });
        let (mut vars, source) = with_input(e.phase, vars, &e.input, harness, ctx.home.as_deref())?;
        if e.phase == Phase::ToolCall && vars["tool"].is_object() {
            let id = identity(&vars["tool"]);
            self.history.push(id.clone());
            let start = self.history.len().saturating_sub(REPEAT_WINDOW);
            let repeats = self.history[start..].iter().filter(|h| **h == id).count();
            let explicit = e.input.get("tool").and_then(|t| t.get("repeat_count"));
            if explicit.is_none() {
                vars["tool"]["repeat_count"] = json!(repeats);
            }
            if e.input
                .get("tool")
                .and_then(|t| t.get("call_index"))
                .is_none()
            {
                vars["tool"]["call_index"] = json!(self.tool_calls);
            }
        }
        build(e.phase, vars, source)
    }

    /// Wendet die Effects einer Entscheidung nach der Endentscheidung an (POL-009).
    fn apply(&mut self, decision: &Decision, outcome: FinalOutcome) -> Vec<String> {
        let mut errors = Vec::new();
        for pending in &decision.effects {
            if !effects_due(&pending.effects_on, outcome) {
                continue;
            }
            for op in &pending.ops {
                let result = match op.scope {
                    StateScope::Session | StateScope::Run => self
                        .state
                        .apply(op.scope, &op.key, op.op, op.value.clone())
                        .map(|_| ()),
                    scope => {
                        let map = match scope {
                            StateScope::UserDay => &mut self.user_day,
                            _ => &mut self.user,
                        };
                        next_value(map.get(&op.key), op.op, op.value.clone()).map(|next| {
                            match next {
                                Some(v) => map.insert(op.key.clone(), v),
                                None => map.remove(&op.key),
                            };
                        })
                    }
                };
                if let Err(e) = result {
                    errors.push(format!("Effect {}: {}", pending.rule.rule_id, e.0));
                }
            }
        }
        errors
    }
}

fn level(matrix: &Matrix, phase: Phase, act: &Activation) -> Level {
    let kind = match phase {
        Phase::ToolCall | Phase::ToolResult => match act.field("tool", "kind") {
            Some(crate::value::Value::String(k)) => Some(k.to_string()),
            _ => None,
        },
        _ => None,
    };
    matrix.level(phase, kind.as_deref())
}

/// Wertet einen Schritt aus und prüft die Erwartung.
fn step(ctx: &Ctx, st: &mut CaseState, e: &Eval, harness: &str, matrix: &Matrix) -> StepResult {
    let expected = serde_json::to_value(&e.expect).unwrap_or(Value::Null);
    let act = match st.context(ctx, e, harness) {
        Ok(a) => a,
        Err(err) => {
            return StepResult {
                failures: vec![format!("Kontext ungültig: {err}")],
                expected,
                actual: Value::Null,
                trace: String::new(),
            };
        }
    };
    let opts = EvalOptions {
        capabilities: matrix.capabilities(e.phase),
        grants: Some(&st.grants),
        limits: test_limits(),
        clock: None,
    };
    let decision = ctx.set.evaluate(&act, &opts);
    let enforcement = level(matrix, e.phase, &act);
    let reasons = reasons(&decision.outcome);
    let mut rules: Vec<String> = decision
        .matched
        .iter()
        .map(|m| m.rule.rule_id.clone())
        .chain(reasons.iter().map(|r| r.rule.rule_id.clone()))
        .collect();
    rules.dedup();
    let mut seen = std::collections::BTreeSet::new();
    rules.retain(|r| seen.insert(r.clone()));
    let would: Vec<String> = decision
        .would
        .iter()
        .map(|m| m.rule.rule_id.clone())
        .collect();
    let notifications: Vec<Value> = decision
        .notifications
        .iter()
        .map(|n| {
            json!({
                "rule": n.rule.rule_id,
                "channels": n.channels,
                "message": n.message,
            })
        })
        .collect();
    let conflicts: Vec<Value> = decision
        .conflicts
        .iter()
        .map(|c| {
            json!({
                "field": c.field,
                "kept": c.kept,
                "kept_rule": c.kept_rule.rule_id,
                "discarded": c.discarded,
                "discarded_rule": c.discarded_rule.rule_id,
            })
        })
        .collect();
    let modified = serde_json::to_value(&decision.modified).unwrap_or(Value::Null);
    let actual = json!({
        "outcome": outcome_str(&decision.outcome),
        "rules": rules,
        "would": would,
        "reasons": reasons.iter().map(|r| r.reason.clone()).collect::<Vec<_>>(),
        "modified": modified,
        "notifications": notifications,
        "conflicts": conflicts,
        "enforcement": enforcement,
    });
    let mut failures = check(&e.expect, &actual, reasons);
    // Ohne erwartetes Ergebnis prüft ein Schritt nichts (QA-004).
    if e.expect.outcome.is_none() {
        failures.insert(0, "`expect.outcome` fehlt (allow, deny oder ask)".into());
    }

    // Endentscheidung, Effects, Grants und Zähler.
    let final_outcome = match (&decision.outcome, e.resolve) {
        (Outcome::Allow, _) => FinalOutcome::Allow,
        (Outcome::Deny { .. }, _) => FinalOutcome::Deny,
        (Outcome::Ask { .. }, Some(r)) if r.decision == ResolveDecision::Allow => {
            st.granted += 1;
            FinalOutcome::Allow
        }
        (Outcome::Ask { .. }, Some(_)) => {
            st.denied += 1;
            FinalOutcome::Deny
        }
        (Outcome::Ask { .. }, None) => FinalOutcome::Expired,
    };
    if let (Outcome::Ask { reasons }, Some(r)) = (&decision.outcome, e.resolve)
        && r.decision == ResolveDecision::Allow
        && r.remember == ResolveRemember::Session
        && plan(reasons, None).approve_session
    {
        for reason in reasons {
            if let Some(fp) = &reason.fingerprint {
                st.grants.add(&reason.rule, fp);
            }
        }
    }
    if e.resolve.is_some() && !matches!(decision.outcome, Outcome::Ask { .. }) {
        failures.push(format!(
            "resolve: keine Freigabe offen (Ergebnis {})",
            outcome_str(&decision.outcome)
        ));
    }
    failures.extend(st.apply(&decision, final_outcome));

    let mut ex = explain(
        &decision_event(&decision, DecisionMeta::new(enforcement)),
        Some(ctx.set),
    );
    ex.decision_id.clear();
    ex.context = context_values(ctx.set, e.phase, &act);
    StepResult {
        failures,
        expected,
        actual,
        trace: render_explain(&ex),
    }
}

fn check(expect: &Expect, actual: &Value, reasons: &[Reason]) -> Vec<String> {
    let mut f = Vec::new();
    let list = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    if let Some(o) = expect.outcome {
        let want = serde_json::to_value(o).unwrap_or(Value::Null);
        if actual["outcome"] != want {
            f.push(format!(
                "outcome: erwartet {}, tatsächlich {}",
                want.as_str().unwrap_or_default(),
                actual["outcome"].as_str().unwrap_or_default()
            ));
        }
    }
    for (field, expected) in [("rules", &expect.rules), ("would", &expect.would)] {
        if let Some(want) = expected {
            let have = list(&actual[field]);
            let missing: Vec<&String> = want.iter().filter(|r| !have.contains(r)).collect();
            if !missing.is_empty() {
                f.push(format!(
                    "{field}: erwartet {}, es fehlt {}; tatsächlich [{}]",
                    want.join(", "),
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    have.join(", ")
                ));
            }
        }
    }
    if let Some(text) = &expect.reason_contains {
        let needle = text.to_lowercase();
        if !reasons
            .iter()
            .any(|r| r.reason.to_lowercase().contains(&needle))
        {
            f.push(format!(
                "reason_contains: kein Grund enthält „{text}“; Gründe: [{}]",
                reasons
                    .iter()
                    .map(|r| r.reason.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
    }
    if let Some(want) = &expect.modified
        && !subset(want, &actual["modified"])
    {
        f.push(format!(
            "modified: erwartet {want}, tatsächlich {}",
            actual["modified"]
        ));
    }
    if let Some(want) = &expect.notifications {
        let have = actual["notifications"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for n in want {
            let pattern = match n {
                Value::String(rule) => json!({ "rule": rule }),
                other => other.clone(),
            };
            if !have.iter().any(|h| subset(&pattern, h)) {
                f.push(format!(
                    "notifications: {pattern} fehlt; tatsächlich {}",
                    Value::Array(have.clone())
                ));
            }
        }
    }
    if let Some(want) = &expect.conflicts {
        let have = actual["conflicts"].as_array().cloned().unwrap_or_default();
        if want.is_empty() && !have.is_empty() {
            f.push(format!(
                "conflicts: keine erwartet, tatsächlich {}",
                Value::Array(have.clone())
            ));
        }
        for c in want {
            if !have.iter().any(|h| subset(c, h)) {
                f.push(format!(
                    "conflicts: {c} fehlt; tatsächlich {}",
                    Value::Array(have.clone())
                ));
            }
        }
    }
    if let Some(want) = expect.enforcement {
        let want = want.as_str();
        if actual["enforcement"] != want {
            f.push(format!(
                "enforcement: erwartet {want}, tatsächlich {}",
                actual["enforcement"].as_str().unwrap_or_default()
            ));
        }
    }
    f
}

fn run_case(ctx: &Ctx, case: &CaseDoc, line: u32, opts: &TestOptions) -> CaseReport {
    let started = Instant::now();
    let mut report = CaseReport {
        name: case.name.clone(),
        line,
        passed: false,
        failures: Vec::new(),
        expected: Value::Null,
        actual: Value::Null,
        trace: String::new(),
        duration_us: 0,
    };
    let harness = case.harness.clone().unwrap_or_else(|| ctx.harness.clone());
    let matrix = if harness == ctx.harness {
        Ok(ctx.matrix.clone())
    } else {
        matrix_for(&harness, opts)
    };
    let result = matrix.and_then(|m| run_steps(ctx, case, &harness, &m, &mut report));
    if let Err(e) = result {
        report.failures.push(e);
    }
    report.passed = report.failures.is_empty();
    report.duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    report
}

fn steps_of(case: &CaseDoc) -> Result<Vec<StepDoc>, String> {
    match (&case.steps, case.phase) {
        (Some(_), Some(_)) => Err("`steps` und `phase` schließen sich aus".into()),
        (Some(steps), None) => {
            if case.input.is_some() || case.expect.is_some() || case.resolve.is_some() {
                return Err(
                    "mit `steps` stehen `input`, `expect` und `resolve` in den Schritten".into(),
                );
            }
            Ok(steps.clone())
        }
        (None, Some(phase)) => {
            if case.expect.is_none() {
                return Err("`expect` fehlt".into());
            }
            Ok(vec![StepDoc {
                phase: Some(phase),
                given: case.given.clone(),
                input: case.input.clone(),
                expect: case.expect.clone(),
                resolve: case.resolve,
                ..StepDoc::default()
            }])
        }
        (None, None) => Err("`phase` (mit `input` und `expect`) oder `steps` fehlt".into()),
    }
}

fn run_steps(
    ctx: &Ctx,
    case: &CaseDoc,
    harness: &str,
    matrix: &Matrix,
    report: &mut CaseReport,
) -> Result<(), String> {
    let steps = steps_of(case)?;
    let mut fixtures = ctx.fixtures.clone();
    if case.steps.is_some() {
        merge(&mut fixtures, &Value::Object(case.given.clone()));
    }
    let mut st = CaseState::new(&fixtures)?;
    let ctx = Ctx {
        set: ctx.set,
        fixtures,
        harness: harness.to_owned(),
        matrix: matrix.clone(),
        home: ctx.home.clone(),
    };
    let multi = steps.len() > 1 || case.expect_state.is_some();
    let mut last: Option<Eval> = None;
    for (i, s) in steps.iter().enumerate() {
        let label = |m: &str| {
            if multi {
                format!("Schritt {}: {m}", i + 1)
            } else {
                m.to_owned()
            }
        };
        if let Some(d) = &s.advance_time {
            let d = crate::cel::ext::parse_duration(d).map_err(|e| label(&e))?;
            st.now += d;
        }
        if let Some(c) = s.cost_delta_usd {
            st.cost_delta += c;
        }
        let evals: Vec<Eval> = if let Some(n) = s.repeat {
            let mut prev = last
                .clone()
                .ok_or_else(|| label("`repeat` ohne vorherigen Schritt"))?;
            if let Some(e) = &s.expect {
                prev.expect = e.clone();
            }
            if let Some(r) = s.resolve {
                prev.resolve = Some(r);
            }
            vec![prev; n as usize]
        } else if let Some(phase) = s.phase {
            vec![Eval {
                phase,
                given: s.given.clone(),
                input: s.input.clone().unwrap_or_default(),
                expect: s.expect.clone().unwrap_or_default(),
                resolve: s.resolve,
            }]
        } else if s.advance_time.is_some() || s.cost_delta_usd.is_some() {
            Vec::new()
        } else {
            return Err(label(
                "Schritt ohne `phase`, `repeat`, `advance_time` oder `cost_delta_usd`",
            ));
        };
        for e in evals {
            let r = step(&ctx, &mut st, &e, harness, matrix);
            let failed = !r.failures.is_empty();
            report.expected = r.expected;
            report.actual = r.actual;
            report.trace = if multi {
                format!("Schritt {}:\n{}", i + 1, r.trace)
            } else {
                r.trace
            };
            report.failures.extend(r.failures.iter().map(|m| label(m)));
            last = Some(e);
            if failed {
                return Ok(());
            }
        }
    }
    if let Some(want) = &case.expect_state {
        for (name, values) in want {
            let scope = match name.as_str() {
                "session" => StateScope::Session,
                "run" => StateScope::Run,
                "user_day" => StateScope::UserDay,
                "user" => StateScope::User,
                other => {
                    return Err(format!(
                        "expect_state: unbekannter Scope `{other}` (session, run, user_day, user)"
                    ));
                }
            };
            for (key, expected) in values {
                let actual = st.state_value(scope, key);
                let matches = actual.as_ref().is_some_and(|a| subset(expected, a));
                if !matches {
                    report.failures.push(format!(
                        "expect_state: state.{}.{key} erwartet {expected}, tatsächlich {}",
                        crate::state::scope_str(scope),
                        actual.map_or_else(|| "nicht gesetzt".to_owned(), |a| a.to_string())
                    ));
                }
            }
        }
    }
    Ok(())
}

fn matrix_for(harness: &str, opts: &TestOptions) -> Result<Matrix, String> {
    match opts.matrix {
        Some(f) => f(harness).map_err(|e| format!("Harness `{harness}`: {e}")),
        None => Ok(Matrix::full()),
    }
}

/// Führt eine Testdatei aus.
pub fn run_file(path: &Path, opts: &TestOptions) -> FileReport {
    let started = Instant::now();
    let mut report = FileReport {
        file: path.display().to_string(),
        error: None,
        cases: Vec::new(),
        duration_us: 0,
    };
    let result = (|| -> Result<(), String> {
        let parsed = parse(path)?;
        let set = load_layers(path, &parsed.file, opts)?;
        let harness = parsed
            .file
            .harness
            .clone()
            .unwrap_or_else(|| DEFAULT_HARNESS.to_owned());
        let matrix = matrix_for(&harness, opts)?;
        // Eine Datei ohne Fälle prüft nichts und besteht nicht (QA-004).
        if parsed.file.cases.is_empty() {
            return Err(format!(
                "{}: keine Fälle (`cases` ist leer)",
                path.display()
            ));
        }
        let mut fixtures = Value::Object(parsed.file.fixtures.clone());
        if fixtures["session"]["harness"].is_null() && parsed.file.harness.is_some() {
            fixtures["session"]["harness"] = Value::String(harness.clone());
        }
        let ctx = Ctx {
            set: &set,
            fixtures,
            harness,
            matrix,
            home: home_dir(opts).map(|h| h.display().to_string()),
        };
        for (i, case) in parsed.file.cases.iter().enumerate() {
            let line = parsed.doc.pos(&format!("cases[{i}]")).line;
            report.cases.push(run_case(&ctx, case, line, opts));
        }
        Ok(())
    })();
    if let Err(e) = result {
        report.error = Some(e);
    }
    report.duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    report
}

/// Findet und führt alle Testdateien unter `paths` aus.
pub fn run_paths(paths: &[PathBuf], opts: &TestOptions) -> Result<Vec<FileReport>, String> {
    Ok(discover(paths)?.iter().map(|p| run_file(p, opts)).collect())
}

// ---------------------------------------------------------------------------------------
// Ausgabe
// ---------------------------------------------------------------------------------------

fn ms(us: u64) -> String {
    format!("{} ms", us.div_ceil(1000))
}

fn rules_of(actual: &Value) -> String {
    let rules: Vec<&str> = actual["rules"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if rules.is_empty() {
        String::new()
    } else {
        format!(" durch {}", rules.join(", "))
    }
}

/// Textausgabe: je Datei die Fälle, bei Fehlschlag erwartete und tatsächliche Entscheidung
/// samt auslösender Regeln und Explain-Trace (POL-026 AC1, QA-004 AC1).
pub fn render(reports: &[FileReport]) -> String {
    let mut out = String::new();
    for r in reports {
        let _ = writeln!(out, "{}", r.file);
        if let Some(e) = &r.error {
            let _ = writeln!(out, "  ✗ Datei nicht ladbar: {e}");
        }
        for c in &r.cases {
            if c.passed {
                let _ = writeln!(out, "  ✓ {} ({})", c.name, ms(c.duration_us));
                continue;
            }
            let _ = writeln!(out, "  ✗ {} (Zeile {})", c.name, c.line);
            if let Some(want) = c.expected["outcome"].as_str() {
                let _ = writeln!(out, "      erwartet: {want}");
            }
            if let Some(have) = c.actual["outcome"].as_str() {
                let _ = writeln!(out, "      tatsächlich: {have}{}", rules_of(&c.actual));
            }
            for f in &c.failures {
                let _ = writeln!(out, "      - {f}");
            }
            if !c.trace.is_empty() {
                let _ = writeln!(out, "      Explain-Trace:");
                for line in c.trace.lines() {
                    let _ = writeln!(out, "        {line}");
                }
            }
        }
    }
    let s = Summary::of(reports);
    let _ = writeln!(
        out,
        "\n{} bestanden, {} fehlgeschlagen, {} Dateien mit Ladefehler",
        s.passed, s.failed, s.errors
    );
    out
}

fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // In XML 1.0 nicht erlaubte Steuerzeichen.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => out.push('?'),
            c => out.push(c),
        }
    }
    out
}

fn secs(us: u64) -> String {
    format!("{:.3}", us as f64 / 1_000_000.0)
}

/// JUnit-XML für CI (POL-026 AC2): eine `testsuite` je Datei, ein `testcase` je Fall,
/// Ladefehler einer Datei als `error`.
pub fn junit(reports: &[FileReport]) -> String {
    let s = Summary::of(reports);
    let tests: usize = reports
        .iter()
        .map(|r| r.cases.len() + usize::from(r.error.is_some()))
        .sum();
    let total: u64 = reports.iter().map(|r| r.duration_us).sum();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuites name=\"beton policy test\" tests=\"{tests}\" failures=\"{}\" errors=\"{}\" time=\"{}\">",
        s.failed,
        s.errors,
        secs(total)
    );
    for r in reports {
        let failures = r.cases.iter().filter(|c| !c.passed).count();
        let file = xml(&r.file);
        let _ = writeln!(
            out,
            "  <testsuite name=\"{file}\" tests=\"{}\" failures=\"{failures}\" errors=\"{}\" time=\"{}\">",
            r.cases.len() + usize::from(r.error.is_some()),
            usize::from(r.error.is_some()),
            secs(r.duration_us)
        );
        if let Some(e) = &r.error {
            let _ = writeln!(
                out,
                "    <testcase name=\"Datei laden\" classname=\"{file}\" time=\"0.000\">\n      <error message=\"{}\" type=\"load\">{}</error>\n    </testcase>",
                xml(e),
                xml(e)
            );
        }
        for c in &r.cases {
            let _ = write!(
                out,
                "    <testcase name=\"{}\" classname=\"{file}\" time=\"{}\"",
                xml(&c.name),
                secs(c.duration_us)
            );
            if c.passed {
                let _ = writeln!(out, "/>");
                continue;
            }
            let mut body = String::new();
            let _ = writeln!(
                body,
                "erwartet: {}\ntatsächlich: {}{}",
                c.expected["outcome"].as_str().unwrap_or("-"),
                c.actual["outcome"].as_str().unwrap_or("-"),
                rules_of(&c.actual)
            );
            for f in &c.failures {
                let _ = writeln!(body, "- {f}");
            }
            body.push_str(&c.trace);
            let _ = writeln!(
                out,
                ">\n      <failure message=\"{}\" type=\"policy\">{}</failure>\n    </testcase>",
                xml(c.failures.first().map_or("", String::as_str)),
                xml(&body)
            );
        }
        let _ = writeln!(out, "  </testsuite>");
    }
    out.push_str("</testsuites>\n");
    out
}
