//! Semantische Prüfung (AGT-002): Harness vorhanden, Effort passend (HAR-017), referenzierte
//! Dateien, Skills und Sub-Agents vorhanden, keine Zyklen in `agents`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use beton_harness::{HarnessId, map_effort};
use serde::Serialize;
use serde_json::Value;

use crate::diag::{Diagnostic, Severity, code};
use crate::dir::{AGENT_YAML, AgentDir};
use crate::load::{diag, parse_agent_yaml};
use crate::resolve::{AgentRef, Located, SearchPath, Source};
use crate::spec::{
    AgentSpec, Executor, Instructions, Param, ParamType, PolicyEntry, SkillSelection, Spawn,
    SubAgent, Tools,
};
use crate::yaml::{Document, Seg};

/// Harnesses dieser Installation mit ihren Effort-Stufen (aus den Capabilities, HAR-002).
#[derive(Debug, Clone, Default)]
pub struct HarnessCatalog {
    harnesses: BTreeMap<String, Vec<String>>,
}

impl HarnessCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert einen Harness mit seinen Effort-Stufen (leer: kennt keine).
    pub fn insert(&mut self, id: &HarnessId, efforts: Vec<String>) {
        self.harnesses.insert(id.as_str().to_owned(), efforts);
    }

    fn efforts(&self, id: &str) -> Option<&[String]> {
        self.harnesses.get(id).map(Vec::as_slice)
    }
}

/// Prüft Agents gegen Suchpfad, Harness-Katalog und Skill-Verzeichnisse.
#[derive(Debug, Clone)]
pub struct Validator<'a> {
    pub search: &'a SearchPath,
    pub harnesses: &'a HarnessCatalog,
    /// Skill-Verzeichnisse außerhalb des Agents in Discovery-Reihenfolge (AGT-008):
    /// Projekt `.beton/skills`, `.claude/skills`, `.agents/skills`, dann User.
    pub skill_roots: Vec<PathBuf>,
}

/// Ergebnis einer Prüfung.
#[derive(Debug, Clone)]
pub struct Report {
    pub spec: Option<AgentSpec>,
    pub doc: Option<Document>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    /// Gültig = keine Fehler (Warnungen erlaubt).
    pub fn is_valid(&self) -> bool {
        self.spec.is_some() && !self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics.len() - self.error_count()
    }
}

/// Gemeinsame Felder von Agent und Inline-Sub-Agent.
struct Body<'s> {
    executor: Option<&'s Executor>,
    instructions: Option<&'s Instructions>,
    params: &'s BTreeMap<String, Param>,
    tools: Option<&'s Tools>,
    skills: Option<&'s SkillSelection>,
    agents: &'s BTreeMap<String, SubAgent>,
    spawn: Option<&'s Spawn>,
    policies: &'s [PolicyEntry],
}

impl<'s> Body<'s> {
    fn of_spec(s: &'s AgentSpec) -> Self {
        Self {
            executor: Some(&s.executor),
            instructions: s.instructions.as_ref(),
            params: &s.params,
            tools: s.tools.as_ref(),
            skills: s.skills.as_ref(),
            agents: &s.agents,
            spawn: s.spawn.as_ref(),
            policies: &s.policies,
        }
    }

    fn of_sub(s: &'s SubAgent) -> Self {
        Self {
            executor: s.executor.as_ref(),
            instructions: s.instructions.as_ref(),
            params: &s.params,
            tools: s.tools.as_ref(),
            skills: s.skills.as_ref(),
            agents: &s.agents,
            spawn: s.spawn.as_ref(),
            policies: &s.policies,
        }
    }
}

fn key(base: &[Seg], k: &str) -> Vec<Seg> {
    let mut p = base.to_vec();
    p.push(Seg::Key(k.to_owned()));
    p
}

fn index(base: &[Seg], i: usize) -> Vec<Seg> {
    let mut p = base.to_vec();
    p.push(Seg::Index(i));
    p
}

struct Run<'v, 'a> {
    v: &'v Validator<'a>,
    root: AgentDir,
    diags: Vec<Diagnostic>,
    done: HashSet<String>,
    /// (Identität, Name) der Agents auf dem aktuellen Pfad.
    stack: Vec<(String, String)>,
}

/// Ort einer Datei für Befunde.
struct Here<'d> {
    file: &'d str,
    doc: &'d Document,
    dir: &'d AgentDir,
}

impl Run<'_, '_> {
    fn push(
        &mut self,
        here: &Here<'_>,
        path: &[Seg],
        code: &'static str,
        sev: Severity,
        msg: String,
    ) {
        self.diags
            .push(diag(here.file, path, here.doc.pos(path), code, sev, msg));
    }

    fn error(&mut self, here: &Here<'_>, path: &[Seg], code: &'static str, msg: String) {
        self.push(here, path, code, Severity::Error, msg);
    }

    fn warn(&mut self, here: &Here<'_>, path: &[Seg], code: &'static str, msg: String) {
        self.push(here, path, code, Severity::Warning, msg);
    }

    fn label(&self, dir: &AgentDir) -> String {
        match dir.relative_to(&self.root) {
            Some(rel) if rel.is_empty() => AGENT_YAML.to_owned(),
            Some(rel) => format!("{rel}/{AGENT_YAML}"),
            None => format!("{}/{AGENT_YAML}", dir.display()),
        }
    }

    /// Prüft ein Agent-Verzeichnis (rekursiv mit seinen Sub-Agents).
    fn check_dir(&mut self, dir: &AgentDir) -> Option<(AgentSpec, Document)> {
        let file = self.label(dir);
        let Some(bytes) = dir.read(AGENT_YAML) else {
            self.diags.push(Diagnostic {
                file,
                path: String::new(),
                line: 1,
                column: 1,
                code: code::MISSING_AGENT_YAML,
                message: format!("{}: keine agent.yaml", dir.display()),
                severity: Severity::Error,
            });
            return None;
        };
        let text = String::from_utf8_lossy(&bytes);
        let parsed = parse_agent_yaml(&text, &file);
        self.diags.extend(parsed.diagnostics);
        let (spec, doc) = (parsed.spec?, parsed.doc?);
        let here = Here {
            file: &file,
            doc: &doc,
            dir,
        };
        let dir_name = dir.dir_name();
        if spec.name.as_str() != dir_name {
            self.warn(
                &here,
                &[Seg::Key("name".into())],
                code::NAME_MISMATCH,
                format!(
                    "Name „{}“ weicht vom Verzeichnis „{dir_name}“ ab; der Suchpfad findet den Agent unter „{dir_name}“",
                    spec.name
                ),
            );
        }
        self.stack.push((dir.identity(), spec.name.to_string()));
        self.check_body(&Body::of_spec(&spec), &here, &[]);
        let mut ids = HashSet::new();
        for (i, s) in spec.schedules.iter().enumerate() {
            let p = index(&[Seg::Key("schedules".into())], i);
            if !ids.insert(s.id.as_str()) {
                self.error(
                    &here,
                    &key(&p, "id"),
                    code::DUPLICATE_KEY,
                    format!("Schedule-ID „{}“ kommt mehrfach vor", s.id),
                );
            }
            if s.cron.split_whitespace().count() != 5 {
                self.error(
                    &here,
                    &key(&p, "cron"),
                    code::INVALID_VALUE,
                    format!("„{}“ ist kein Cron-Ausdruck mit 5 Feldern", s.cron),
                );
            }
        }
        self.stack.pop();
        Some((spec, doc))
    }

    fn check_body(&mut self, b: &Body<'_>, here: &Here<'_>, base: &[Seg]) {
        if let Some(e) = b.executor {
            self.check_executor(e, here, &key(base, "executor"));
        }
        if let Some(i) = b.instructions {
            self.check_instructions(i, here, &key(base, "instructions"));
        }
        self.check_params(b.params, here, &key(base, "params"));
        if let Some(t) = b.tools {
            self.check_tools(t, here, &key(base, "tools"));
        }
        if let Some(SkillSelection::List(names)) = b.skills {
            let p = key(base, "skills");
            for (i, name) in names.iter().enumerate() {
                if !self.skill_exists(here.dir, name) {
                    self.error(
                        here,
                        &index(&p, i),
                        code::SKILL_NOT_FOUND,
                        format!("Skill „{name}“ nicht gefunden"),
                    );
                }
            }
        }
        let agents = key(base, "agents");
        for (name, sub) in b.agents {
            self.check_subagent(name, sub, here, &key(&agents, name));
        }
        if let Some(s) = b.spawn {
            let p = key(base, "spawn");
            for (i, name) in s.agents.iter().enumerate() {
                if !b.agents.contains_key(name) {
                    self.error(
                        here,
                        &index(&key(&p, "agents"), i),
                        code::UNKNOWN_SUBAGENT,
                        format!("„{name}“ ist nicht unter `agents` deklariert"),
                    );
                }
            }
            if let Some(share) = s.budget_share
                && !(share > 0.0 && share <= 1.0)
            {
                self.error(
                    here,
                    &key(&p, "budget_share"),
                    code::INVALID_VALUE,
                    format!("budget_share {share} liegt nicht in (0, 1]"),
                );
            }
        }
        self.check_policies(b.policies, here, &key(base, "policies"));
    }

    fn check_executor(&mut self, e: &Executor, here: &Here<'_>, p: &[Seg]) {
        let harness = e.harness.as_str();
        let Some(supported) = self.v.harnesses.efforts(harness) else {
            self.warn(
                here,
                &key(p, "harness"),
                code::HARNESS_UNAVAILABLE,
                format!("Harness „{harness}“ ist in dieser Installation nicht verfügbar"),
            );
            return;
        };
        if let Some(requested) = e.reasoning_effort {
            let requested = requested.as_str();
            let who = match &e.model {
                Some(m) => format!("{harness}/{m}"),
                None => harness.to_owned(),
            };
            match map_effort(requested, supported) {
                Some(eff) if eff == requested => {}
                Some(eff) => self.warn(
                    here,
                    &key(p, "reasoning_effort"),
                    code::EFFORT_MAPPED,
                    format!(
                        "„{requested}“ → „{eff}“ ({who} kennt diese Stufe nicht; effektiv „{eff}“)"
                    ),
                ),
                None => self.warn(
                    here,
                    &key(p, "reasoning_effort"),
                    code::EFFORT_MAPPED,
                    format!(
                        "„{requested}“ wird nicht angewendet ({who} kennt keine Effort-Stufen)"
                    ),
                ),
            }
        }
        if e.max_turns == Some(0) {
            self.error(
                here,
                &key(p, "max_turns"),
                code::INVALID_VALUE,
                "max_turns muss mindestens 1 sein".into(),
            );
        }
    }

    /// Prüft eine Datei relativ zum Agent-Verzeichnis.
    fn check_file(&mut self, here: &Here<'_>, p: &[Seg], rel: &str) {
        if !AgentDir::contains(rel) {
            self.error(
                here,
                p,
                code::PATH_OUTSIDE_AGENT,
                format!("{rel} liegt außerhalb des Agent-Verzeichnisses"),
            );
        } else if !here.dir.exists(rel) {
            self.error(
                here,
                p,
                code::FILE_NOT_FOUND,
                format!("{rel} existiert nicht"),
            );
        }
    }

    fn check_instructions(&mut self, i: &Instructions, here: &Here<'_>, p: &[Seg]) {
        if i.file.is_some() && i.text.is_some() {
            self.error(
                here,
                p,
                code::CONFLICTING_FIELDS,
                "`file` und `text` schließen sich aus".into(),
            );
        }
        if let Some(file) = &i.file {
            self.check_file(here, &key(p, "file"), file);
        }
    }

    fn check_params(&mut self, params: &BTreeMap<String, Param>, here: &Here<'_>, p: &[Seg]) {
        for (name, param) in params {
            let pp = key(p, name);
            let valid_name = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !valid_name {
                self.error(
                    here,
                    &pp,
                    code::INVALID_PARAM,
                    format!("Parametername „{name}“ ist ungültig (erlaubt: a-z, 0-9, _)"),
                );
            }
            let numeric = matches!(param.kind, ParamType::Integer | ParamType::Number);
            if !numeric && (param.minimum.is_some() || param.maximum.is_some()) {
                self.error(
                    here,
                    &pp,
                    code::INVALID_PARAM,
                    "minimum/maximum gibt es nur für integer und number".into(),
                );
            }
            if let (Some(min), Some(max)) = (param.minimum, param.maximum)
                && min > max
            {
                self.error(
                    here,
                    &key(&pp, "minimum"),
                    code::INVALID_PARAM,
                    format!("minimum {min} ist größer als maximum {max}"),
                );
            }
            match (param.kind, param.values.is_empty()) {
                (ParamType::Enum, true) => self.error(
                    here,
                    &pp,
                    code::INVALID_PARAM,
                    "type: enum braucht `values`".into(),
                ),
                (ParamType::Enum, false) | (_, true) => {}
                (_, false) => self.error(
                    here,
                    &key(&pp, "values"),
                    code::INVALID_PARAM,
                    "`values` gibt es nur bei type: enum".into(),
                ),
            }
            if let Some(default) = &param.default
                && let Some(problem) = default_problem(param, default)
            {
                self.error(here, &key(&pp, "default"), code::INVALID_PARAM, problem);
            }
        }
    }

    fn check_tools(&mut self, t: &Tools, here: &Here<'_>, p: &[Seg]) {
        let mcp = key(p, "mcp");
        for (name, server) in &t.mcp {
            let sp = key(&mcp, name);
            match (&server.command, &server.url) {
                (Some(_), Some(_)) => self.error(
                    here,
                    &sp,
                    code::CONFLICTING_FIELDS,
                    "MCP-Server: entweder `command` (stdio) oder `url` (HTTP)".into(),
                ),
                (None, None) => self.error(
                    here,
                    &sp,
                    code::MISSING_FIELD,
                    "MCP-Server braucht `command` (stdio) oder `url` (HTTP)".into(),
                ),
                (Some(_), None) if !server.headers.is_empty() => self.error(
                    here,
                    &key(&sp, "headers"),
                    code::CONFLICTING_FIELDS,
                    "`headers` gibt es nur bei HTTP-Servern (`url`)".into(),
                ),
                (None, Some(_)) if !server.args.is_empty() || !server.env.is_empty() => self.error(
                    here,
                    &sp,
                    code::CONFLICTING_FIELDS,
                    "`args`/`env` gibt es nur bei stdio-Servern (`command`)".into(),
                ),
                _ => {}
            }
        }
    }

    fn skill_exists(&self, dir: &AgentDir, name: &str) -> bool {
        let rel = format!("skills/{name}/SKILL.md");
        dir.exists(&rel)
            || self
                .v
                .skill_roots
                .iter()
                .any(|root| root.join(name).join("SKILL.md").is_file())
    }

    fn check_subagent(&mut self, name: &str, sub: &SubAgent, here: &Here<'_>, p: &[Seg]) {
        let Some(reference) = &sub.reference else {
            if sub.executor.is_none() {
                self.error(
                    here,
                    p,
                    code::MISSING_FIELD,
                    format!("Sub-Agent „{name}“ braucht `ref` oder inline einen `executor`"),
                );
            }
            self.check_body(&Body::of_sub(sub), here, p);
            return;
        };
        let rp = key(p, "ref");
        let inline_fields = sub.description.is_some()
            || sub.executor.is_some()
            || sub.instructions.is_some()
            || !sub.params.is_empty()
            || sub.tools.is_some()
            || sub.skills.is_some()
            || !sub.agents.is_empty()
            || sub.spawn.is_some()
            || !sub.policies.is_empty()
            || sub.sandbox.is_some();
        if inline_fields {
            self.error(
                here,
                p,
                code::CONFLICTING_FIELDS,
                format!("Sub-Agent „{name}“: `ref` und Inline-Felder schließen sich aus"),
            );
        }
        let target = match AgentRef::parse(reference) {
            Err(e) => {
                self.error(here, &rp, code::INVALID_VALUE, e);
                return;
            }
            Ok(AgentRef::Path(path)) => {
                let found = if path.is_absolute() {
                    match here.dir {
                        AgentDir::Fs(_) => Some(AgentDir::Fs(path.clone())),
                        AgentDir::Builtin { .. } => None,
                    }
                } else {
                    here.dir.join(&path.to_string_lossy())
                };
                match found {
                    Some(d) if d.has_agent_yaml() => d,
                    Some(_) => {
                        self.error(
                            here,
                            &rp,
                            code::AGENT_NOT_FOUND,
                            format!("Sub-Agent „{name}“: {reference} hat keine agent.yaml"),
                        );
                        return;
                    }
                    None => {
                        self.error(
                            here,
                            &rp,
                            code::PATH_OUTSIDE_AGENT,
                            format!("Sub-Agent „{name}“: {reference} verlässt die Built-ins"),
                        );
                        return;
                    }
                }
            }
            Ok(r) => match self.v.search.resolve(&r, Path::new(".")) {
                Ok(located) => located.dir,
                Err(e) => {
                    self.error(
                        here,
                        &rp,
                        code::AGENT_NOT_FOUND,
                        format!("Sub-Agent „{name}“: {e}"),
                    );
                    return;
                }
            },
        };
        let id = target.identity();
        if let Some(start) = self.stack.iter().position(|(i, _)| *i == id) {
            let mut chain: Vec<&str> = self.stack[start..]
                .iter()
                .map(|(_, n)| n.as_str())
                .collect();
            chain.push(self.stack[start].1.as_str());
            let msg = format!("Sub-Agent-Zyklus: {}", chain.join(" → "));
            self.error(here, &rp, code::AGENT_CYCLE, msg);
            return;
        }
        if self.done.insert(id) {
            self.check_dir(&target);
        }
    }

    fn check_policies(&mut self, policies: &[PolicyEntry], here: &Here<'_>, p: &[Seg]) {
        for (i, entry) in policies.iter().enumerate() {
            let pp = index(p, i);
            match (&entry.reference, &entry.id) {
                (Some(r), None) => {
                    if !entry.rule.is_empty() {
                        self.error(
                            here,
                            &pp,
                            code::CONFLICTING_FIELDS,
                            "Policy-Verweis (`ref`) hat keine Regelfelder".into(),
                        );
                    }
                    self.check_file(here, &key(&pp, "ref"), r);
                }
                (None, Some(id)) => {
                    let ok = !id.is_empty()
                        && id.len() <= 64
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                    if !ok {
                        self.error(
                            here,
                            &key(&pp, "id"),
                            code::INVALID_VALUE,
                            format!("Regel-ID „{id}“ ist ungültig (a-z, 0-9, -; max. 64)"),
                        );
                    }
                }
                (Some(_), Some(_)) => self.error(
                    here,
                    &pp,
                    code::CONFLICTING_FIELDS,
                    "Policy-Eintrag: entweder `ref` (Datei) oder `id` (Inline-Regel)".into(),
                ),
                (None, None) => self.error(
                    here,
                    &pp,
                    code::MISSING_FIELD,
                    "Policy-Eintrag braucht `ref` (Datei) oder `id` (Inline-Regel)".into(),
                ),
            }
        }
    }
}

fn default_problem(param: &Param, default: &Value) -> Option<String> {
    let type_ok = match param.kind {
        ParamType::String => default.is_string(),
        ParamType::Integer => default.is_i64() || default.is_u64(),
        ParamType::Number => default.is_number(),
        ParamType::Boolean => default.is_boolean(),
        ParamType::Enum => default
            .as_str()
            .is_some_and(|s| param.values.iter().any(|v| v == s)),
    };
    if !type_ok {
        let kind = serde_json::to_value(param.kind).unwrap_or(Value::Null);
        let kind = kind.as_str().unwrap_or_default();
        return Some(format!("Default {default} passt nicht zu type: {kind}"));
    }
    let n = default.as_f64()?;
    if param.minimum.is_some_and(|min| n < min) || param.maximum.is_some_and(|max| n > max) {
        return Some(format!(
            "Default {default} liegt außerhalb von minimum/maximum"
        ));
    }
    None
}

impl Validator<'_> {
    /// Prüft einen gefundenen Agent samt Sub-Agents.
    pub fn validate(&self, located: &Located) -> Report {
        let mut run = Run {
            v: self,
            root: located.dir.clone(),
            diags: Vec::new(),
            done: HashSet::from([located.dir.identity()]),
            stack: Vec::new(),
        };
        let checked = run.check_dir(&located.dir);
        let mut diagnostics = run.diags;
        diagnostics.sort_by(|a, b| {
            (a.file != AGENT_YAML, &a.file, a.line, a.column).cmp(&(
                b.file != AGENT_YAML,
                &b.file,
                b.line,
                b.column,
            ))
        });
        let (spec, doc) = checked.map_or((None, None), |(s, d)| (Some(s), Some(d)));
        Report {
            spec,
            doc,
            diagnostics,
        }
    }
}

/// Der aufgelöste Agent für `beton agent show` (AGT-013 AC2).
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedAgent {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: Source,
    /// Verzeichnis bzw. `builtin:<name>`.
    pub path: String,
    /// Inhalts-Hash über alle Dateien des Agents.
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadows: Option<String>,
    /// Die Definition mit Defaults.
    pub spec: Value,
    /// Herkunft jedes Felds: `agent.yaml:<zeile>` oder `default`.
    pub origins: BTreeMap<String, String>,
    pub warnings: Vec<Diagnostic>,
}

/// Entfernt `null`, leere Listen und leere Objekte.
fn prune(v: Value) -> Option<Value> {
    match v {
        Value::Null => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Object(m) => {
            let m: serde_json::Map<String, Value> = m
                .into_iter()
                .filter_map(|(k, v)| prune(v).map(|v| (k, v)))
                .collect();
            (!m.is_empty()).then_some(Value::Object(m))
        }
        other => Some(other),
    }
}

fn leaves(v: &Value, path: &mut Vec<Seg>, out: &mut Vec<Vec<Seg>>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                path.push(Seg::Key(k.clone()));
                leaves(child, path, out);
                path.pop();
            }
        }
        Value::Array(a) if a.iter().any(Value::is_object) => {
            for (i, child) in a.iter().enumerate() {
                path.push(Seg::Index(i));
                leaves(child, path, out);
                path.pop();
            }
        }
        _ => out.push(path.clone()),
    }
}

impl ResolvedAgent {
    /// Alle Felder als (Pfad, Wert, Herkunft) in Schema-Reihenfolge der JSON-Ausgabe.
    pub fn fields(&self) -> Vec<(String, &Value, &str)> {
        let mut paths = Vec::new();
        leaves(&self.spec, &mut Vec::new(), &mut paths);
        paths
            .into_iter()
            .filter_map(|p| {
                let mut v = &self.spec;
                for seg in &p {
                    v = match seg {
                        Seg::Key(k) => v.get(k)?,
                        Seg::Index(i) => v.get(*i)?,
                    };
                }
                let s = crate::yaml::path_string(&p);
                let origin = self.origins.get(&s).map_or("default", String::as_str);
                Some((s, v, origin))
            })
            .collect()
    }

    /// Baut den aufgelösten Agent aus einem gültigen Prüfbericht.
    pub fn new(located: &Located, report: &Report) -> Option<Self> {
        let spec = report.spec.as_ref()?;
        let doc = report.doc.as_ref()?;
        let mut json = prune(serde_json::to_value(spec).ok()?).unwrap_or(Value::Null);
        if let Some(exec) = json.get_mut("executor").and_then(Value::as_object_mut) {
            exec.entry("mode")
                .or_insert_with(|| Value::String("native".into()));
        }
        if let Value::Object(m) = &mut json {
            for (k, v) in &spec.extensions {
                m.insert(k.clone(), v.clone());
            }
        }
        let mut paths = Vec::new();
        leaves(&json, &mut Vec::new(), &mut paths);
        let origins = paths
            .into_iter()
            .map(|p| {
                let s = crate::yaml::path_string(&p);
                let origin = doc.has(&s).map_or_else(
                    || "default".to_owned(),
                    |pos| format!("{AGENT_YAML}:{}", pos.line),
                );
                (s, origin)
            })
            .collect();
        Some(Self {
            name: spec.name.to_string(),
            version: spec.version.clone(),
            description: spec.description.clone(),
            source: located.source,
            path: located.dir.display(),
            hash: located.dir.hash(),
            shadows: located.shadows.clone(),
            spec: json,
            origins,
            warnings: report.diagnostics.clone(),
        })
    }
}
