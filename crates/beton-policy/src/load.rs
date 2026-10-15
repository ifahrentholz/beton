//! Laden und Prüfen von Policy-Sets (POL-001): YAML lesen, gegen das Format prüfen, alle
//! CEL-Ausdrücke kompilieren und typprüfen. Jeder Fehler trägt Datei, Zeile und Spalte.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use beton_core::event::PolicySetInvalid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Phase;
use crate::cel::{CompileError, Program, compile_bool, compile_expr, ext};
use crate::doc::{
    Action, ApprovalDoc, DefaultAction, Mode, PolicySetDoc, Remember, RuleDoc, RuleType,
    SPEC_VERSION, TimeoutAction, modifiable,
};
use crate::types::Ty;
use crate::yaml::{self, Document, Style};

/// Größte Policy-Datei (1 MiB).
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// Modifikationen, die nur Policies ab der User-Ebene vornehmen dürfen: Ein Repo bzw. ein Agent
/// könnte damit sonst Schutz lockern (#231: Permission-Mode) oder Befehle in Tool-Calls
/// einschleusen (#234: Argumente).
pub const USER_ONLY_MODIFY: [&str; 2] = ["permission_mode", "args"];

/// Kanonische Tool-Klassen (POL-005), gültig in `kinds:`.
use crate::enforcement::KINDS;

/// Ebene eines Policy-Sets; die Reihenfolge ist die Autorität (`Org` zuerst).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "org" => Scope::Org,
            "team" => Scope::Team,
            "user" => Scope::User,
            "project" => Scope::Project,
            "agent" => Scope::Agent,
            _ => return None,
        })
    }
}

/// Format des Inhalts, aus dem ein Set geladen wurde (Snapshot im Blob-Store, POL-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    /// Policy-Datei (YAML-Text).
    Yaml,
    /// Inline-Regeln (z. B. `policies:` im `agent.yaml`) als kanonisches JSON-Array.
    Inline,
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
    /// Eingebauter Regeltyp ohne Entzuckerung. `require_approval` (POL-014) wird beim Laden
    /// zu einer `ask`-Regel, `spend_cap`, `daily_budget` und `model_route` (POL-011 bis
    /// POL-013) zu mehreren CEL-Regeln derselben ID ([`crate::budget`]); die übrigen folgen mit
    /// ihren Regeltyp-Paketen (WP-42) und gelten bis dahin in ihren Phasen als Fehler (fail
    /// closed).
    Builtin(RuleType),
}

/// Eine geladene, geprüfte Regel.
#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub id: String,
    pub kind: RuleKind,
    pub doc: RuleDoc,
    /// Ausdrücke in `notify.message` (`{{ … }}`, in Reihenfolge), geprüft beim Laden.
    pub notify_exprs: Vec<Program>,
    /// Ausdruck je Eintrag in `effects` (`value`, sofern gesetzt).
    pub effect_values: Vec<Option<Program>>,
    /// Fingerprint für gemerkte Freigaben (POL-010).
    pub remember_key: Option<Program>,
}

impl LoadedRule {
    /// Alle Hilfsausdrücke (`notify.message`, `effects[].value`, `remember_key`).
    pub fn aux(&self) -> impl Iterator<Item = &Program> {
        self.notify_exprs
            .iter()
            .chain(self.effect_values.iter().flatten())
            .chain(self.remember_key.iter())
    }
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
    /// Inhalt, aus dem das Set geladen wurde; `version` ist sein SHA-256 (POL-027 AC2).
    pub format: SourceFormat,
    pub source: Arc<[u8]>,
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
    /// Name des Sets (Zustands-Keys der Regeltypen, POL-011/POL-012).
    set: &'a str,
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
        set: "",
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
    let loc = Locator {
        set: &set.name,
        ..loc
    };
    let rules = check_rules(scope, &set.rules, &loc)?;
    Ok(LoadedSet {
        scope,
        name: set.name,
        origin: origin.to_owned(),
        version: version_of(text.as_bytes()),
        mode: set.mode,
        defaults: set.defaults,
        rules,
        format: SourceFormat::Yaml,
        source: Arc::from(text.as_bytes()),
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
        set: name,
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
    let checked = check_rules_with(scope, &docs, &loc, |i| format!("[{i}]"))?;
    let canonical = serde_json::to_vec(rules).unwrap_or_default();
    Ok(LoadedSet {
        scope,
        name: name.to_owned(),
        origin: origin.to_owned(),
        version: version_of(&canonical),
        mode: Mode::Enforce,
        defaults: BTreeMap::new(),
        rules: checked,
        format: SourceFormat::Inline,
        source: Arc::from(canonical),
    })
}

fn check_rules(
    scope: Scope,
    rules: &[RuleDoc],
    loc: &Locator,
) -> Result<Vec<LoadedRule>, LoadError> {
    check_rules_with(scope, rules, loc, |i| format!("rules[{i}]"))
}

fn check_rules_with(
    scope: Scope,
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
        // Zustands-Keys der Regeltypen schreiben nur diese selbst (#268).
        for (j, effect) in rule.effects.iter().enumerate() {
            if let Some(t) = crate::budget::reserved_prefix(&effect.key) {
                return Err(loc.at(
                    &field(&format!("effects[{j}].key")),
                    format!(
                        "Key `{}` ist für den Regeltyp {} reserviert ({}/…)",
                        effect.key,
                        t.as_str(),
                        t.as_str()
                    ),
                ));
            }
        }
        if let Some(parts) = desugar_parts(scope, rule, loc, &at)? {
            out.extend(parts);
            continue;
        }
        out.push(check_rule(scope, rule, loc, &at)?);
    }
    Ok(out)
}

fn check_rule(
    scope: Scope,
    rule: &RuleDoc,
    loc: &Locator,
    at: &str,
) -> Result<LoadedRule, LoadError> {
    if let Some(rule_type) = rule.rule_type {
        check_type_fields(rule, rule_type, loc, at)?;
        if rule_type == RuleType::RequireApproval {
            return require_approval(rule, loc, at);
        }
        return Ok(LoadedRule {
            id: rule.id.clone(),
            kind: RuleKind::Builtin(rule_type),
            doc: rule.clone(),
            notify_exprs: Vec::new(),
            effect_values: Vec::new(),
            remember_key: None,
        });
    }
    check_cel_rule(scope, rule, loc, at)
}

/// Ein Regeltyp (`type` + `params`) schließt eigene Bedingung, Aktion und deren Felder aus.
fn check_type_fields(
    rule: &RuleDoc,
    rule_type: RuleType,
    loc: &Locator,
    at: &str,
) -> Result<(), LoadError> {
    let field = |f: &str| format!("{at}.{f}");
    {
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
    }
    Ok(())
}

/// Regeltypen, die zu mehreren CEL-Regeln derselben ID werden (POL-011 bis POL-013,
/// [`crate::budget`]). Jeder Teil durchläuft die Prüfung einer handgeschriebenen Regel seiner
/// Ebene; `doc` behält Typ und Parameter (Explain, Hot-Reload).
fn desugar_parts(
    scope: Scope,
    rule: &RuleDoc,
    loc: &Locator,
    at: &str,
) -> Result<Option<Vec<LoadedRule>>, LoadError> {
    let Some(rule_type) = rule.rule_type else {
        return Ok(None);
    };
    check_type_fields(rule, rule_type, loc, at)?;
    let Some(parts) = crate::budget::desugar(rule_type, rule, scope, loc.set, loc.file) else {
        return Ok(None);
    };
    let params = |path: &str| {
        if path.is_empty() {
            format!("{at}.params")
        } else {
            format!("{at}.params.{path}")
        }
    };
    let parts = parts.map_err(|e| match e {
        crate::budget::ParamError::At { path, message } => loc.at(&params(&path), message),
        crate::budget::ParamError::Cel { path, error } => loc.cel(&params(&path), &error),
    })?;
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let mut loaded = check_cel_rule(scope, &part, loc, at).map_err(|e| LoadError {
            message: format!(
                "{}: Entzuckerung von {} fehlgeschlagen: {}",
                loc.path(&params("")),
                rule_type.as_str(),
                e.message
            ),
            ..e
        })?;
        loaded.doc.rule_type = Some(rule_type);
        loaded.doc.params.clone_from(&rule.params);
        out.push(loaded);
    }
    Ok(Some(out))
}

/// Eine Regel mit Phase, Bedingung und Aktion (handgeschrieben oder entzuckert).
fn check_cel_rule(
    scope: Scope,
    rule: &RuleDoc,
    loc: &Locator,
    at: &str,
) -> Result<LoadedRule, LoadError> {
    let field = |f: &str| format!("{at}.{f}");
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
                if scope > Scope::User && USER_ONLY_MODIFY.contains(&f) {
                    return Err(loc.at(
                        &field(&format!("modify.{f}")),
                        format!(
                            "`{f}` darf nur eine Policy ab der User-Ebene ändern, nicht die Ebene {}",
                            scope.as_str()
                        ),
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
            check_timeout(t).map_err(|e| loc.at(&field("approval.timeout"), e))?;
        }
    }
    if rule.notify.is_some() && action != Action::Notify {
        return Err(loc.at(&field("notify"), "`notify` nur mit action: notify"));
    }
    let mut notify_exprs = Vec::new();
    if let Some(message) = rule.notify.as_ref().and_then(|n| n.message.as_ref()) {
        for expr in template_exprs(message) {
            let path = field("notify.message");
            notify_exprs.push(compile_expr(expr, phase, Ty::Dyn).map_err(|e| loc.at(&path, &e))?);
        }
    }
    let mut effect_values = Vec::with_capacity(rule.effects.len());
    for (j, effect) in rule.effects.iter().enumerate() {
        let path = field(&format!("effects[{j}].value"));
        let program = match &effect.value {
            Some(value) => {
                Some(compile_expr(value, phase, Ty::Dyn).map_err(|e| loc.cel(&path, &e))?)
            }
            None => None,
        };
        let needs_value = matches!(
            effect.op,
            crate::doc::EffectOp::Set | crate::doc::EffectOp::Append
        );
        if needs_value && program.is_none() {
            return Err(loc.at(&path, "`value` fehlt (nötig für set und append)"));
        }
        if effect.key.is_empty() {
            return Err(loc.at(&field(&format!("effects[{j}].key")), "`key` ist leer"));
        }
        effect_values.push(program);
    }
    let remember_key = match &rule.remember_key {
        Some(key) => {
            let path = field("remember_key");
            Some(compile_expr(key, phase, Ty::String).map_err(|e| loc.cel(&path, &e))?)
        }
        None => None,
    };
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
        notify_exprs,
        effect_values,
        remember_key,
    })
}

/// Eine Approval-Frist muss eine positive Dauer sein.
fn check_timeout(text: &str) -> Result<(), String> {
    let d = ext::parse_duration(text)?;
    if d <= time::Duration::ZERO {
        return Err(format!("Frist `{text}` muss positiv sein"));
    }
    Ok(())
}

/// Parameter von `require_approval` (POL-014).
const REQUIRE_APPROVAL_PARAMS: [&str; 7] = [
    "tools",
    "kinds",
    "servers",
    "match",
    "timeout",
    "on_timeout",
    "remember",
];

/// CEL-Zeichenkettenliteral.
fn cel_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `require_approval` (POL-014): Zucker für `ask` in Phase `tool_call`. Die Kriterien
/// `tools` (Glob gegen `tool.name` bzw. `mcp:<server>/<tool>`), `kinds`, `servers` und `match`
/// (CEL) sind ODER-verknüpft; `timeout`, `on_timeout` und `remember` landen in `approval`.
fn require_approval(rule: &RuleDoc, loc: &Locator, at: &str) -> Result<LoadedRule, LoadError> {
    let field = |f: &str| format!("{at}.params.{f}");
    let empty = BTreeMap::new();
    let params = rule.params.as_ref().unwrap_or(&empty);
    if let Some(unknown) = params
        .keys()
        .find(|k| !REQUIRE_APPROVAL_PARAMS.contains(&k.as_str()))
    {
        return Err(loc.at(
            &field(unknown),
            format!(
                "unbekannter Parameter `{unknown}` (erlaubt: {})",
                REQUIRE_APPROVAL_PARAMS.join(", ")
            ),
        ));
    }
    let strings = |name: &str| -> Result<Vec<String>, LoadError> {
        match params.get(name) {
            None => Ok(Vec::new()),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| loc.at(&format!("{}[{i}]", field(name)), "Text erwartet"))
                })
                .collect(),
            Some(_) => Err(loc.at(&field(name), "Liste erwartet")),
        }
    };
    let text = |name: &str| -> Result<Option<String>, LoadError> {
        match params.get(name) {
            None => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(loc.at(&field(name), "Text erwartet")),
        }
    };
    let tools = strings("tools")?;
    let kinds = strings("kinds")?;
    let servers = strings("servers")?;
    let matcher = text("match")?;

    let mut parts: Vec<String> = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        if !KINDS.contains(&kind.as_str()) {
            return Err(loc.at(
                &format!("{}[{i}]", field("kinds")),
                format!(
                    "unbekannte Tool-Klasse `{kind}` (erlaubt: {})",
                    KINDS.join(", ")
                ),
            ));
        }
    }
    if !kinds.is_empty() {
        let list: Vec<String> = kinds.iter().map(|k| cel_string(k)).collect();
        parts.push(format!("tool.kind in [{}]", list.join(", ")));
    }
    if !servers.is_empty() {
        let list: Vec<String> = servers.iter().map(|s| cel_string(s)).collect();
        parts.push(format!("tool.server in [{}]", list.join(", ")));
    }
    for pattern in &tools {
        if pattern.starts_with("mcp:") {
            parts.push(format!(
                "glob(\"mcp:\" + tool.server + \"/\" + tool.name, {})",
                cel_string(pattern)
            ));
        } else {
            parts.push(format!("glob(tool.name, {})", cel_string(pattern)));
        }
    }
    if let Some(m) = &matcher {
        compile_bool(m, Phase::ToolCall).map_err(|e| loc.cel(&field("match"), &e))?;
        parts.push(format!("({m})"));
    }
    if parts.is_empty() {
        return Err(loc.at(
            &format!("{at}.params"),
            "require_approval braucht mindestens eines von tools, kinds, servers oder match",
        ));
    }

    let timeout = text("timeout")?;
    if let Some(t) = &timeout {
        check_timeout(t).map_err(|e| loc.at(&field("timeout"), e))?;
    }
    let on_timeout = match text("on_timeout")?.as_deref() {
        None => None,
        Some("deny") => Some(TimeoutAction::Deny),
        Some("allow") => Some(TimeoutAction::Allow),
        Some("abort") => Some(TimeoutAction::Abort),
        Some(other) => {
            return Err(loc.at(
                &field("on_timeout"),
                format!("`on_timeout: {other}` ist ungültig (deny, allow, abort)"),
            ));
        }
    };
    let remember = match text("remember")?.as_deref() {
        None => None,
        Some("session") => Some(Remember::Session),
        Some("none") => Some(Remember::None),
        Some(other) => {
            return Err(loc.at(
                &field("remember"),
                format!("`remember: {other}` ist ungültig (session, none)"),
            ));
        }
    };

    let source = parts.join(" || ");
    let when =
        compile_bool(&source, Phase::ToolCall).map_err(|e| loc.cel(&format!("{at}.params"), &e))?;
    let mut doc = rule.clone();
    doc.approval = Some(ApprovalDoc {
        timeout,
        on_timeout,
        remember,
        allow_on_timeout: None,
    });
    doc.reason.get_or_insert_with(|| {
        format!(
            "Regel `{}` verlangt deine Freigabe (require_approval)",
            rule.id
        )
    });
    Ok(LoadedRule {
        id: rule.id.clone(),
        kind: RuleKind::Cel {
            phase: Phase::ToolCall,
            when: Some(when),
            action: Action::Ask,
        },
        doc,
        notify_exprs: Vec::new(),
        effect_values: Vec::new(),
        remember_key: None,
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
