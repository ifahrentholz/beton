//! Laden und Prüfen von Policy-Sets (POL-001): YAML lesen, gegen das Format prüfen, alle
//! CEL-Ausdrücke kompilieren und typprüfen. Jeder Fehler trägt Datei, Zeile und Spalte.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use beton_core::event::PolicySetInvalid;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::Phase;
use crate::cel::{CompileError, Program, compile_bool, compile_expr, ext};
use crate::doc::{
    Action, DefaultAction, Mode, PolicySetDoc, RuleDoc, RuleType, SPEC_VERSION, modifiable,
};
use crate::types::Ty;
use crate::yaml::{self, Document, Style};

/// Größte Policy-Datei (1 MiB).
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// Kanonische Tool-Klassen (POL-005), gültig in `kinds:`.
use crate::enforcement::KINDS;

/// Ebene eines Policy-Sets; die Reihenfolge ist die Autorität (`Org` zuerst).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Org,
    Team,
    User,
    Project,
    Agent,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Org => "org",
            Scope::Team => "team",
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Agent => "agent",
        }
    }
}

/// Verweis auf eine Regel (für Gründe, Konflikte, Audit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleRef {
    pub scope: Scope,
    pub set: String,
    pub set_version: String,
    pub rule_id: String,
}

/// Was eine geladene Regel ist.
#[derive(Debug, Clone)]
pub enum RuleKind {
    /// Regel mit Phase, Bedingung und Aktion.
    Cel {
        phase: Phase,
        when: Option<Program>,
        action: Action,
    },
    /// Eingebauter Regeltyp. Die Entzuckerung folgt mit den Regeltyp-Paketen (WP-42, WP-43);
    /// bis dahin wertet die Engine die Regel in ihren Phasen als Fehler (fail closed).
    Builtin(RuleType),
}

/// Eine geladene, geprüfte Regel.
#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub id: String,
    pub kind: RuleKind,
    pub doc: RuleDoc,
    /// Ausdrücke in `notify.message` (`{{ … }}`), `effects[].value` und `remember_key`,
    /// geprüft beim Laden.
    pub aux: Vec<Program>,
}

impl LoadedRule {
    /// Gilt die Regel in `phase`?
    pub fn applies_to(&self, phase: Phase) -> bool {
        match &self.kind {
            RuleKind::Cel { phase: p, .. } => *p == phase,
            RuleKind::Builtin(t) => t.phases().contains(&phase),
        }
    }
}

/// Ein geladenes Policy-Set.
#[derive(Debug, Clone)]
pub struct LoadedSet {
    pub scope: Scope,
    pub name: String,
    /// Datei bzw. Herkunft (`agents/x/agent.yaml`).
    pub origin: String,
    /// `sha256:<hex>` über den Inhalt.
    pub version: String,
    pub mode: Mode,
    pub defaults: BTreeMap<Phase, DefaultAction>,
    pub rules: Vec<LoadedRule>,
}

impl LoadedSet {
    pub fn rule_ref(&self, rule_id: &str) -> RuleRef {
        RuleRef {
            scope: self.scope,
            set: self.name.clone(),
            set_version: self.version.clone(),
            rule_id: rule_id.to_owned(),
        }
    }
}

/// Ladefehler mit Datei und 1-basierter Position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for LoadError {}

impl LoadError {
    /// Payload für `policy.set_invalid`.
    pub fn to_event(&self) -> PolicySetInvalid {
        PolicySetInvalid {
            file: self.file.clone(),
            line: self.line,
            error: format!("{}:{}: {}", self.line, self.column, self.message),
        }
    }
}

/// Positionen für Fehler: aus der YAML-Datei oder (inline) ohne Positionen.
struct Locator<'a> {
    file: &'a str,
    text: &'a str,
    doc: Option<&'a Document>,
    /// Präfix des Feldpfads (`policies` bei Inline-Regeln aus `agent.yaml`).
    prefix: &'a str,
}

impl Locator<'_> {
    fn path(&self, path: &str) -> String {
        if self.prefix.is_empty() {
            path.to_owned()
        } else if path.starts_with('[') {
            format!("{}{path}", self.prefix)
        } else {
            format!("{}.{path}", self.prefix)
        }
    }

    fn at(&self, path: &str, message: impl fmt::Display) -> LoadError {
        let pos = self.doc.map_or(yaml::Pos::START, |d| d.pos(path));
        LoadError {
            file: self.file.to_owned(),
            line: pos.line,
            column: pos.column,
            message: format!("{}: {message}", self.path(path)),
        }
    }

    /// Fehler in einem CEL-Ausdruck am Feld `path`: Position in der Datei plus Position im
    /// Ausdruck.
    fn cel(&self, path: &str, e: &CompileError) -> LoadError {
        let message = format!("{}: Zeichen {}: {}", self.path(path), e.column, e.message);
        let Some((pos, style)) = self.doc.and_then(|d| d.value_pos(path)) else {
            let pos = self.doc.map_or(yaml::Pos::START, |d| d.pos(path));
            return LoadError {
                file: self.file.to_owned(),
                line: pos.line,
                column: pos.column,
                message,
            };
        };
        let (line, column) = match style {
            Style::Plain | Style::Quoted => {
                let shift = u32::from(style == Style::Quoted);
                if e.line <= 1 {
                    (pos.line, pos.column + shift + e.column - 1)
                } else {
                    (pos.line + e.line - 1, e.column)
                }
            }
            Style::Block => {
                let line = pos.line + e.line;
                let indent = self
                    .text
                    .lines()
                    .nth(line as usize - 1)
                    .map_or(0, |l| l.len() - l.trim_start().len());
                (line, u32::try_from(indent).unwrap_or(0) + e.column)
            }
        };
        LoadError {
            file: self.file.to_owned(),
            line,
            column,
            message,
        }
    }
}

/// Feldpfad eines Formatfehlers; bei unbekannten Feldern der Pfad des Felds selbst.
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

fn version_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

/// Liest ein Policy-Set aus YAML-Text. `origin` ist der Dateipfad für Fehlermeldungen.
pub fn parse_set(scope: Scope, origin: &str, text: &str) -> Result<LoadedSet, LoadError> {
    let doc = yaml::parse(text).map_err(|e| LoadError {
        file: origin.to_owned(),
        line: e.pos.line,
        column: e.pos.column,
        message: e.message,
    })?;
    let loc = Locator {
        file: origin,
        text,
        doc: Some(&doc),
        prefix: "",
    };
    let set: PolicySetDoc = serde_path_to_error::deserialize(doc.value.clone())
        .map_err(|e| loc.at(&error_path(&e), e.inner()))?;
    if set.spec_version != SPEC_VERSION {
        return Err(loc.at(
            "spec_version",
            format!(
                "spec_version {} wird nicht unterstützt (nur 1)",
                set.spec_version
            ),
        ));
    }
    if !valid_id(&set.name) {
        return Err(loc.at(
            "name",
            format!("name `{}` muss [a-z0-9-] sein (max. 64)", set.name),
        ));
    }
    let rules = check_rules(&set.rules, &loc)?;
    Ok(LoadedSet {
        scope,
        name: set.name,
        origin: origin.to_owned(),
        version: version_of(text.as_bytes()),
        mode: set.mode,
        defaults: set.defaults,
        rules,
    })
}

/// Liest eine Policy-Datei (höchstens [`MAX_FILE_BYTES`]).
pub fn load_file(scope: Scope, path: &Path) -> Result<LoadedSet, LoadError> {
    let origin = path.display().to_string();
    let io = |message: String| LoadError {
        file: origin.clone(),
        line: 1,
        column: 1,
        message,
    };
    let meta = std::fs::metadata(path).map_err(|e| io(format!("nicht lesbar: {e}")))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(io(format!("Datei größer als {MAX_FILE_BYTES} Bytes")));
    }
    let bytes = std::fs::read(path).map_err(|e| io(format!("nicht lesbar: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| io("keine gültige UTF-8-Datei".into()))?;
    parse_set(scope, &origin, &text)
}

/// Lade-Schnittstelle für Inline-Regeln, z. B. den `policies:`-Block im `agent.yaml`
/// (AGT-014, Anbindung mit WP-49). Fehler nennen den Feldpfad `policies[i]…`.
pub fn inline_set(
    scope: Scope,
    name: &str,
    origin: &str,
    rules: &[serde_json::Value],
) -> Result<LoadedSet, LoadError> {
    let loc = Locator {
        file: origin,
        text: "",
        doc: None,
        prefix: "policies",
    };
    let docs: Vec<RuleDoc> = rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            serde_path_to_error::deserialize(r.clone()).map_err(|e| {
                let path = error_path(&e);
                let sub = if path.is_empty() {
                    String::new()
                } else {
                    format!(".{path}")
                };
                loc.at(&format!("[{i}]{sub}"), e.inner())
            })
        })
        .collect::<Result<_, _>>()?;
    let checked = check_rules_with(&docs, &loc, |i| format!("[{i}]"))?;
    let canonical = serde_json::to_vec(rules).unwrap_or_default();
    Ok(LoadedSet {
        scope,
        name: name.to_owned(),
        origin: origin.to_owned(),
        version: version_of(&canonical),
        mode: Mode::Enforce,
        defaults: BTreeMap::new(),
        rules: checked,
    })
}

fn check_rules(rules: &[RuleDoc], loc: &Locator) -> Result<Vec<LoadedRule>, LoadError> {
    check_rules_with(rules, loc, |i| format!("rules[{i}]"))
}

fn check_rules_with(
    rules: &[RuleDoc],
    loc: &Locator,
    base: impl Fn(usize) -> String,
) -> Result<Vec<LoadedRule>, LoadError> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rules.len());
    for (i, rule) in rules.iter().enumerate() {
        let at = base(i);
        let field = |f: &str| format!("{at}.{f}");
        if !valid_id(&rule.id) {
            return Err(loc.at(
                &field("id"),
                format!("id `{}` muss [a-z0-9-] sein (max. 64)", rule.id),
            ));
        }
        if !seen.insert(rule.id.as_str()) {
            return Err(loc.at(&field("id"), format!("Regel-ID `{}` ist doppelt", rule.id)));
        }
        out.push(check_rule(rule, loc, &at)?);
    }
    Ok(out)
}

fn check_rule(rule: &RuleDoc, loc: &Locator, at: &str) -> Result<LoadedRule, LoadError> {
    let field = |f: &str| format!("{at}.{f}");
    if let Some(rule_type) = rule.rule_type {
        let forbidden = [
            ("on", rule.on.is_some()),
            ("when", rule.when.is_some()),
            ("action", rule.action.is_some()),
            ("modify", rule.modify.is_some()),
            ("notify", rule.notify.is_some()),
            ("approval", rule.approval.is_some()),
            ("effects", !rule.effects.is_empty()),
            ("remember_key", rule.remember_key.is_some()),
        ];
        if let Some((name, _)) = forbidden.iter().find(|(_, set)| *set) {
            return Err(loc.at(
                &field(name),
                format!(
                    "`{name}` ist mit `type` ({}) nicht erlaubt",
                    rule_type.as_str()
                ),
            ));
        }
        return Ok(LoadedRule {
            id: rule.id.clone(),
            kind: RuleKind::Builtin(rule_type),
            doc: rule.clone(),
            aux: Vec::new(),
        });
    }
    if rule.params.is_some() {
        return Err(loc.at(&field("params"), "`params` gibt es nur mit `type`"));
    }
    let phase = rule.on.ok_or_else(|| loc.at(at, "`on` (Phase) fehlt"))?;
    let action = rule.action.ok_or_else(|| loc.at(at, "`action` fehlt"))?;
    let when = match &rule.when {
        Some(src) => Some(compile_bool(src, phase).map_err(|e| loc.cel(&field("when"), &e))?),
        None => None,
    };
    if phase == Phase::ToolResult && action == Action::Ask {
        return Err(loc.at(
            &field("action"),
            "`ask` ist in Phase tool_result nicht möglich",
        ));
    }
    match (&rule.modify, action) {
        (None, Action::Modify) => return Err(loc.at(at, "`modify` fehlt für action: modify")),
        (Some(_), a) if a != Action::Modify => {
            return Err(loc.at(&field("modify"), "`modify` nur mit action: modify"));
        }
        (Some(m), _) => {
            let fields = m.fields();
            if fields.is_empty() {
                return Err(loc.at(&field("modify"), "`modify` ändert kein Feld"));
            }
            for f in fields {
                if !modifiable(phase).contains(&f) {
                    return Err(loc.at(
                        &field(&format!("modify.{f}")),
                        format!("`{f}` ist in Phase {phase} nicht modifizierbar"),
                    ));
                }
            }
            if let Some(args) = &m.args
                && !args.is_object()
            {
                return Err(loc.at(
                    &field("modify.args"),
                    "`args` muss ein JSON-Merge-Patch (Objekt) sein",
                ));
            }
            for (j, pattern) in m.redact.iter().flatten().enumerate() {
                ext::regex(pattern)
                    .map_err(|e| loc.at(&field(&format!("modify.redact[{j}]")), e))?;
            }
        }
        _ => {}
    }
    if rule.on_unsupported.is_some() && action != Action::Modify {
        return Err(loc.at(
            &field("on_unsupported"),
            "`on_unsupported` nur mit action: modify",
        ));
    }
    if let Some(approval) = &rule.approval {
        if action != Action::Ask {
            return Err(loc.at(&field("approval"), "`approval` nur mit action: ask"));
        }
        if let Some(t) = &approval.timeout {
            ext::parse_duration(t).map_err(|e| loc.at(&field("approval.timeout"), e))?;
        }
    }
    if rule.notify.is_some() && action != Action::Notify {
        return Err(loc.at(&field("notify"), "`notify` nur mit action: notify"));
    }
    let mut aux = Vec::new();
    if let Some(message) = rule.notify.as_ref().and_then(|n| n.message.as_ref()) {
        for expr in template_exprs(message) {
            let path = field("notify.message");
            aux.push(compile_expr(expr, phase, Ty::Dyn).map_err(|e| loc.at(&path, &e))?);
        }
    }
    for (j, effect) in rule.effects.iter().enumerate() {
        if let Some(value) = &effect.value {
            let path = field(&format!("effects[{j}].value"));
            aux.push(compile_expr(value, phase, Ty::Dyn).map_err(|e| loc.cel(&path, &e))?);
        }
    }
    if let Some(key) = &rule.remember_key {
        let path = field("remember_key");
        aux.push(compile_expr(key, phase, Ty::String).map_err(|e| loc.cel(&path, &e))?);
    }
    for (j, kind) in rule.kinds.iter().flatten().enumerate() {
        if !KINDS.contains(&kind.as_str()) {
            return Err(loc.at(
                &field(&format!("kinds[{j}]")),
                format!(
                    "unbekannte Tool-Klasse `{kind}` (erlaubt: {})",
                    KINDS.join(", ")
                ),
            ));
        }
    }
    Ok(LoadedRule {
        id: rule.id.clone(),
        kind: RuleKind::Cel {
            phase,
            when,
            action,
        },
        doc: rule.clone(),
        aux,
    })
}

/// Ausdrücke in `{{ … }}`.
fn template_exprs(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push(after[..end].trim());
        rest = &after[end + 2..];
    }
    out
}
