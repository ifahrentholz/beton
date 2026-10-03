//! Golden-Transcript-Tests (HAR-025, Gate und Drift-Prozess: QA-003).
//!
//! Ein Fall ist ein Verzeichnis:
//!
//! ```text
//! <fall>/
//!   meta.yaml              # harness, cli_version, recorded_at, scenario, platform
//!   script.yaml            # Eingaben und Gate-Entscheidungen (deterministisch)
//!   raw.jsonl              # Vendor-stdout als RawLine ({ms, after_stdin?, out})
//!   expected.stdin.jsonl   # erwartete Nachrichten von beton an die CLI (normalisiert)
//!   expected.events.jsonl  # erwartete normalisierte Events
//! ```
//!
//! [`run_case`] spielt `raw.jsonl` als simulierten Prozess ab, treibt den Adapter mit
//! `script.yaml` und vergleicht beide Richtungen. Mit `BETON_BLESS=1` werden die Erwartungen
//! neu geschrieben (der Diff erscheint dann im PR).

mod replay;
pub mod secrets;

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use self::replay::{RawLine, ReplayLauncher};
use crate::adapter::{
    AdapterContext, Gate, GateDecision, GateRequest, HarnessAdapter, HostEnv, NormalizedEvent,
    SessionSpec, Shutdown, UserInput,
};
use crate::capabilities::Capabilities;

/// Arbeitsverzeichnis, das Golden-Tests dem Adapter vorgeben (wird zu `<workdir>`).
pub const GOLDEN_WORKDIR: &str = "/beton-golden/workdir";

/// Umgebungsvariable, mit der Erwartungen neu geschrieben werden (`--bless`).
pub const BLESS_ENV: &str = "BETON_BLESS";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub harness: String,
    pub cli_version: String,
    pub recorded_at: String,
    pub scenario: String,
    pub platform: String,
    /// Exit-Code des aufgezeichneten Prozesses.
    #[serde(default)]
    pub exit_code: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub inputs: Vec<ScriptInput>,
    /// Gate-Entscheidungen in Reihenfolge; fehlt eine, wird abgelehnt (fail closed).
    #[serde(default)]
    pub gate: Vec<ScriptDecision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<String>,
    /// Unterbricht den laufenden Turn, sobald `after_events` Events dieses Turns kamen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt_after_events: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptDecision {
    Allow,
    Deny,
}

/// Ein geladener Fall.
#[derive(Debug, Clone)]
pub struct Case {
    pub dir: PathBuf,
    pub meta: Meta,
    pub script: Script,
    pub raw: Vec<RawLine>,
    pub expected_stdin: Option<String>,
    pub expected_events: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GoldenError {
    #[error("{path}: {message}")]
    Load { path: String, message: String },
    #[error("Golden-Fall {case} weicht ab:\n{diff}")]
    Mismatch { case: String, diff: String },
    #[error("Adapter-Fehler in {case}: {message}")]
    Adapter { case: String, message: String },
    #[error("Aufnahme enthält nach dem Scrubbing noch Secrets, nichts geschrieben:\n{0}")]
    SecretsFound(String),
    #[error("E/A-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

fn load_err(path: &Path, message: impl std::fmt::Display) -> GoldenError {
    GoldenError::Load {
        path: path.display().to_string(),
        message: message.to_string(),
    }
}

impl Case {
    pub fn load(dir: &Path) -> Result<Self, GoldenError> {
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| load_err(&path, e))
        };
        let optional = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        let meta: Meta = serde_yaml_ng::from_str(&read("meta.yaml")?)
            .map_err(|e| load_err(&dir.join("meta.yaml"), e))?;
        let script: Script = serde_yaml_ng::from_str(&read("script.yaml")?)
            .map_err(|e| load_err(&dir.join("script.yaml"), e))?;
        let raw = read("raw.jsonl")?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| {
                serde_json::from_str(l)
                    .map_err(|e| load_err(&dir.join("raw.jsonl"), format!("Zeile {}: {e}", i + 1)))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            dir: dir.to_path_buf(),
            meta,
            script,
            raw,
            expected_stdin: optional("expected.stdin.jsonl"),
            expected_events: optional("expected.events.jsonl"),
        })
    }

    pub fn name(&self) -> String {
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Alle Fälle unter einem `golden/`-Verzeichnis, sortiert.
pub fn cases(root: &Path) -> Result<Vec<PathBuf>, GoldenError> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("meta.yaml").is_file())
        .collect();
    out.sort();
    Ok(out)
}

/// Gate, das die Entscheidungen aus `script.yaml` der Reihe nach liefert.
#[derive(Debug, Default)]
struct ScriptGate {
    decisions: Mutex<VecDeque<ScriptDecision>>,
    requests: Mutex<Vec<GateRequest>>,
}

#[async_trait]
impl Gate for ScriptGate {
    async fn decide(&self, request: GateRequest) -> GateDecision {
        if let Ok(mut r) = self.requests.lock() {
            r.push(request);
        }
        match self.decisions.lock().ok().and_then(|mut d| d.pop_front()) {
            Some(ScriptDecision::Allow) => GateDecision::Allow { updated_args: None },
            _ => GateDecision::Deny {
                reason: Some("golden: keine Entscheidung im Skript".into()),
            },
        }
    }
}

/// Ergebnis eines Durchlaufs, normalisiert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub stdin: String,
    pub events: String,
}

/// Wie lange ein Golden-Lauf höchstens auf Events wartet, bevor er abbricht.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Spielt einen Fall gegen einen Adapter ab und liefert das normalisierte Transkript.
pub async fn replay(case: &Case, adapter: &dyn HarnessAdapter) -> Result<Transcript, GoldenError> {
    let adapter_err = |message: String| GoldenError::Adapter {
        case: case.name(),
        message,
    };
    let launcher = ReplayLauncher::new(case.raw.clone(), case.meta.exit_code);
    let gate = Arc::new(ScriptGate {
        decisions: Mutex::new(case.script.gate.iter().copied().collect()),
        requests: Mutex::default(),
    });
    let ctx = AdapterContext {
        gate: gate.clone(),
        launcher: Arc::new(launcher.clone()),
        env: HostEnv::default(),
    };
    let spec = SessionSpec {
        workdir: GOLDEN_WORKDIR.into(),
        model: case.script.model.clone(),
        ..SessionSpec::default()
    };
    let mut session = adapter
        .start(spec, ctx)
        .await
        .map_err(|e| adapter_err(e.to_string()))?;
    let mut rx = session
        .events()
        .ok_or_else(|| adapter_err("Event-Strom fehlt".into()))?;
    let mut events: Vec<NormalizedEvent> = Vec::new();

    for input in &case.script.inputs {
        if let Some(text) = &input.send {
            session
                .send(UserInput { text: text.clone() })
                .await
                .map_err(|e| adapter_err(e.to_string()))?;
        }
        let mut in_turn = 0;
        loop {
            let next = tokio::time::timeout(STEP_TIMEOUT, rx.recv())
                .await
                .map_err(|_| adapter_err("Zeitüberschreitung beim Warten auf Events".into()))?;
            let Some(event) = next else { break };
            let terminal = is_turn_end(&event);
            events.push(event);
            in_turn += 1;
            if input.interrupt_after_events == Some(in_turn) {
                session
                    .interrupt()
                    .await
                    .map_err(|e| adapter_err(e.to_string()))?;
            }
            if terminal {
                break;
            }
        }
    }
    session
        .shutdown(Shutdown::Graceful {
            timeout: Duration::from_secs(1),
        })
        .await
        .map_err(|e| adapter_err(e.to_string()))?;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
        events.push(event);
    }

    let mut normalizer = Normalizer::new(GOLDEN_WORKDIR);
    let stdin = launcher
        .stdin_lines()
        .iter()
        .map(|l| normalizer.line(l))
        .collect::<Vec<_>>();
    let events = events
        .iter()
        .map(|e| normalizer.value(event_json(e)).to_string())
        .collect::<Vec<_>>();
    Ok(Transcript {
        stdin: join_lines(&stdin),
        events: join_lines(&events),
    })
}

fn is_turn_end(e: &NormalizedEvent) -> bool {
    matches!(
        e.payload.type_name(),
        "turn.completed" | "turn.failed" | "turn.interrupted" | "harness.exited"
    )
}

/// Vergleichsform eines Events: Typ, Nutzlast, Turn und ob `raw` mitkam.
fn event_json(e: &NormalizedEvent) -> Value {
    let mut v = serde_json::to_value(&e.payload).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        if let Some(turn) = e.turn_id {
            obj.insert("turn_id".into(), json!(turn.to_string()));
        }
        obj.insert("has_raw".into(), json!(e.raw.is_some()));
    }
    v
}

fn join_lines(lines: &[String]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Prüft einen Fall (HAR-025 AC1). Mit `BETON_BLESS=1` werden fehlende oder abweichende
/// Erwartungen geschrieben statt gemeldet (QA-003 AC1).
pub async fn run_case(dir: &Path, adapter: &dyn HarnessAdapter) -> Result<(), GoldenError> {
    let bless = std::env::var(BLESS_ENV).is_ok_and(|v| v == "1");
    check_case(dir, adapter, bless).await
}

pub async fn check_case(
    dir: &Path,
    adapter: &dyn HarnessAdapter,
    bless: bool,
) -> Result<(), GoldenError> {
    let case = Case::load(dir)?;
    let actual = replay(&case, adapter).await?;
    if bless {
        std::fs::write(dir.join("expected.stdin.jsonl"), &actual.stdin)?;
        std::fs::write(dir.join("expected.events.jsonl"), &actual.events)?;
        return Ok(());
    }
    let mut diff = String::new();
    for (name, expected, got) in [
        ("expected.stdin.jsonl", &case.expected_stdin, &actual.stdin),
        (
            "expected.events.jsonl",
            &case.expected_events,
            &actual.events,
        ),
    ] {
        match expected {
            None => {
                let _ = writeln!(diff, "{name} fehlt (mit {BLESS_ENV}=1 erzeugen)");
            }
            Some(expected) if expected != got => diff.push_str(&line_diff(name, expected, got)),
            Some(_) => {}
        }
    }
    if diff.is_empty() {
        Ok(())
    } else {
        Err(GoldenError::Mismatch {
            case: case.name(),
            diff: format!("{diff}\nAbsichtliche Änderung? Mit {BLESS_ENV}=1 neu schreiben."),
        })
    }
}

/// Lesbarer zeilenweiser Diff (nur abweichende Zeilen mit Nummer).
pub fn line_diff(name: &str, expected: &str, got: &str) -> String {
    let (e, g): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), got.lines().collect());
    let mut out = format!("--- {name} (erwartet)\n+++ {name} (tatsächlich)\n");
    for i in 0..e.len().max(g.len()) {
        match (e.get(i), g.get(i)) {
            (Some(a), Some(b)) if a == b => {}
            (a, b) => {
                if let Some(a) = a {
                    let _ = writeln!(out, "{:>4} - {a}", i + 1);
                }
                if let Some(b) = b {
                    let _ = writeln!(out, "{:>4} + {b}", i + 1);
                }
            }
        }
    }
    out
}

/// Normalisierung vor dem Vergleich: IDs → `<id:n>`, Zeitstempel → `<ts>`,
/// Arbeitsverzeichnis → `<workdir>`.
#[derive(Debug)]
pub struct Normalizer {
    workdir: String,
    ids: BTreeMap<String, usize>,
}

static ID_RE: std::sync::LazyLock<Option<Regex>> = std::sync::LazyLock::new(|| {
    Regex::new(concat!(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        r"|\b[a-z]{2,4}_[0-9A-HJKMNP-TV-Z]{26}\b",
        r"|\b(?:msg|toolu|call|req|srvtoolu)_[A-Za-z0-9]{12,}\b"
    ))
    .ok()
});

static TS_RE: std::sync::LazyLock<Option<Regex>> = std::sync::LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})").ok()
});

impl Normalizer {
    pub fn new(workdir: &str) -> Self {
        Self {
            workdir: workdir.to_owned(),
            ids: BTreeMap::new(),
        }
    }

    pub fn text(&mut self, s: &str) -> String {
        let mut out = s.replace(&self.workdir, "<workdir>");
        if let Some(re) = TS_RE.as_ref() {
            out = re.replace_all(&out, "<ts>").into_owned();
        }
        if let Some(re) = ID_RE.as_ref() {
            let ids = &mut self.ids;
            out = re
                .replace_all(&out, |c: &regex::Captures<'_>| {
                    let next = ids.len() + 1;
                    let n = *ids.entry(c[0].to_owned()).or_insert(next);
                    format!("<id:{n}>")
                })
                .into_owned();
        }
        out
    }

    pub fn value(&mut self, v: Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.text(&s)),
            Value::Array(a) => Value::Array(a.into_iter().map(|x| self.value(x)).collect()),
            Value::Object(o) => {
                Value::Object(o.into_iter().map(|(k, x)| (k, self.value(x))).collect())
            }
            other => other,
        }
    }

    /// Eine JSON-Zeile normalisieren; Nicht-JSON bleibt Text.
    pub fn line(&mut self, line: &str) -> String {
        match serde_json::from_str::<Value>(line) {
            Ok(v) => self.value(v).to_string(),
            Err(_) => self.text(line),
        }
    }
}

/// Schreibt eine Aufnahme (für `beton dev record-golden`): erst Scrubbing, dann Secret-Scan.
/// Bleibt ein Fund, wird nichts geschrieben (HAR-025 AC2).
pub fn write_recording(
    dir: &Path,
    meta: &Meta,
    script: &Script,
    raw: &[RawLine],
    known_secrets: &[String],
) -> Result<(), GoldenError> {
    let yaml_err = |e: serde_yaml_ng::Error| load_err(dir, e);
    let files = [
        (
            "meta.yaml",
            serde_yaml_ng::to_string(meta).map_err(yaml_err)?,
        ),
        (
            "script.yaml",
            serde_yaml_ng::to_string(script).map_err(yaml_err)?,
        ),
        (
            "raw.jsonl",
            join_lines(
                &raw.iter()
                    .map(|l| serde_json::to_string(l).unwrap_or_default())
                    .collect::<Vec<_>>(),
            ),
        ),
    ];
    let mut scrubbed = Vec::new();
    let mut findings = String::new();
    for (name, content) in files {
        let clean = secrets::scrub(&content, known_secrets);
        for f in secrets::find_secrets(&clean) {
            let _ = writeln!(findings, "{name} {f}");
        }
        scrubbed.push((name, clean));
    }
    if !findings.is_empty() {
        return Err(GoldenError::SecretsFound(findings));
    }
    std::fs::create_dir_all(dir)?;
    for (name, content) in scrubbed {
        std::fs::write(dir.join(name), content)?;
    }
    Ok(())
}

/// Abgleich Katalog ↔ Aufnahmen (HAR-025 AC4, QA-003 AC3): Jede Aufnahme liegt im
/// `version_range`, und für jede als getestet deklarierte Version gibt es eine Aufnahme.
pub fn check_versions(
    caps: &Capabilities,
    tested_versions: &[&str],
    golden_root: &Path,
) -> Result<Vec<String>, GoldenError> {
    let mut problems = Vec::new();
    let req = caps
        .version_range
        .as_deref()
        .map(semver::VersionReq::parse)
        .transpose()
        .map_err(|e| load_err(golden_root, e))?;
    let mut recorded = Vec::new();
    for dir in cases(golden_root)? {
        let case = Case::load(&dir)?;
        let version = case.meta.cli_version.clone();
        match (semver::Version::parse(&version), &req) {
            (Err(e), _) => problems.push(format!("{}: cli_version {version}: {e}", case.name())),
            (Ok(v), Some(req)) if !req.matches(&v) => problems.push(format!(
                "{}: cli_version {version} liegt nicht in {req}",
                case.name()
            )),
            _ => {}
        }
        recorded.push(version);
    }
    for v in tested_versions {
        if !recorded.iter().any(|r| r == v) {
            problems.push(format!("keine Aufnahme für die unterstützte Version {v}"));
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests;
