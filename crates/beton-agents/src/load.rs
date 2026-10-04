//! `agent.yaml` lesen: YAML mit Positionen, Spec-Version prüfen, `x-`-Felder abtrennen,
//! typisiert deserialisieren und Schemafehler als [`Diagnostic`] mit Zeile/Spalte melden.

use serde_json::Value;

use crate::diag::{Diagnostic, Severity, code, suggest};
use crate::spec::{AgentSpec, SPEC_VERSION};
use crate::yaml::{self, Document, Pos, Seg, path_string};

/// Ergebnis des Lesens einer `agent.yaml`.
#[derive(Debug, Clone)]
pub struct Parsed {
    /// Gesetzt, wenn die Datei (ggf. nach Entfernen unbekannter Felder) dem Schema
    /// entspricht. Gültig ist sie nur ohne Fehler in `diagnostics`.
    pub spec: Option<AgentSpec>,
    pub doc: Option<Document>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Bekannte Harness-Arten für Vorschläge bei Tippfehlern.
const HARNESS_KINDS: [&str; 5] = ["claude", "codex", "fake", "acp:", "direct:"];

pub(crate) fn diag(
    file: &str,
    path: &[Seg],
    pos: Pos,
    code: &'static str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        file: file.to_owned(),
        path: path_string(path),
        line: pos.line,
        column: pos.column,
        code,
        message: message.into(),
        severity,
    }
}

/// Teile zwischen Backticks in einer serde-Meldung.
fn ticks(msg: &str) -> Vec<&str> {
    msg.split('`').skip(1).step_by(2).collect()
}

fn to_segments(path: &serde_path_to_error::Path) -> Vec<Seg> {
    use serde_path_to_error::Segment;
    path.iter()
        .filter_map(|s| match s {
            Segment::Seq { index } => Some(Seg::Index(*index)),
            Segment::Map { key } => Some(Seg::Key(key.clone())),
            Segment::Enum { variant } => Some(Seg::Key(variant.clone())),
            Segment::Unknown => None,
        })
        .collect()
}

/// Übersetzt einen serde-Fehler in einen Befund. Liefert außerdem den Pfad eines
/// unbekannten Felds, das für einen weiteren Durchlauf entfernt werden kann.
fn schema_error(
    file: &str,
    doc: &Document,
    err: &serde_path_to_error::Error<serde_json::Error>,
) -> (Diagnostic, Option<Vec<Seg>>) {
    let mut path = to_segments(err.path());
    let raw = err.inner().to_string();
    let msg = raw.split(" at line ").next().unwrap_or(&raw).to_owned();
    let e = |path: &[Seg], pos, code, message: String| {
        diag(file, path, pos, code, Severity::Error, message)
    };
    if msg.starts_with("unknown field") {
        let t = ticks(&msg);
        let field = t.first().copied().unwrap_or_default().to_owned();
        let hint = suggest(&field, t.iter().skip(1).copied())
            .map(|s| format!(" Meintest du „{s}“?"))
            .unwrap_or_default();
        let d = e(
            &path,
            doc.pos(&path),
            code::UNKNOWN_FIELD,
            format!("Unbekanntes Feld „{field}“.{hint}"),
        );
        return (d, Some(path));
    }
    if msg.starts_with("missing field") {
        let field = ticks(&msg).first().copied().unwrap_or_default().to_owned();
        let pos = doc.pos(&path);
        path.push(Seg::Key(field.clone()));
        return (
            e(
                &path,
                pos,
                code::MISSING_FIELD,
                format!("Pflichtfeld „{field}“ fehlt"),
            ),
            None,
        );
    }
    if msg.contains("ist keine Harness-ID") {
        let value = ticks(&msg).first().copied().unwrap_or_default().to_owned();
        let hint = suggest(&value, HARNESS_KINDS)
            .map(|s| format!(" Meintest du „{s}“?"))
            .unwrap_or_else(|| " Erlaubt: claude, codex, acp:<slug>, direct:<provider>.".into());
        return (
            e(
                &path,
                doc.pos(&path),
                code::UNKNOWN_HARNESS,
                format!("„{value}“ ist unbekannt.{hint}"),
            ),
            None,
        );
    }
    if msg.starts_with("unknown variant") {
        let t = ticks(&msg);
        let value = t.first().copied().unwrap_or_default().to_owned();
        let allowed: Vec<&str> = t.iter().skip(1).copied().collect();
        let hint = suggest(&value, allowed.iter().copied())
            .map(|s| format!(" Meintest du „{s}“?"))
            .unwrap_or_default();
        return (
            e(
                &path,
                doc.pos(&path),
                code::INVALID_VALUE,
                format!(
                    "Unbekannter Wert „{value}“ (erlaubt: {}).{hint}",
                    allowed.join(", ")
                ),
            ),
            None,
        );
    }
    let message = if msg.contains("untagged enum ProjectFiles") {
        "Erwartet `auto`, `none` oder eine Liste von Dateien".to_owned()
    } else if msg.contains("untagged enum SkillSelection") {
        "Erwartet `all`, `none` oder eine Liste von Skill-Namen".to_owned()
    } else if let Some(rest) = msg.strip_prefix("invalid type: ") {
        format!("Falscher Typ: {rest}")
    } else if let Some(rest) = msg.strip_prefix("invalid value: ") {
        format!("Ungültiger Wert: {rest}")
    } else {
        msg
    };
    (e(&path, doc.pos(&path), code::INVALID_VALUE, message), None)
}

fn remove_path(value: &mut Value, path: &[Seg]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let mut cur = value;
    for seg in parents {
        cur = match (seg, cur) {
            (Seg::Key(k), Value::Object(m)) => match m.get_mut(k) {
                Some(v) => v,
                None => return false,
            },
            (Seg::Index(i), Value::Array(a)) => match a.get_mut(*i) {
                Some(v) => v,
                None => return false,
            },
            _ => return false,
        };
    }
    match (last, cur) {
        (Seg::Key(k), Value::Object(m)) => m.remove(k).is_some(),
        _ => false,
    }
}

/// Liest den Text einer `agent.yaml`. `file` erscheint in den Befunden.
pub fn parse_agent_yaml(text: &str, file: &str) -> Parsed {
    let (doc, yaml_errors) = yaml::parse(text);
    let mut diagnostics: Vec<Diagnostic> = yaml_errors
        .into_iter()
        .map(|e| Diagnostic {
            file: file.to_owned(),
            path: e.path,
            line: e.pos.line,
            column: e.pos.column,
            code: e.code,
            message: e.message,
            severity: Severity::Error,
        })
        .collect();
    let Some(doc) = doc else {
        return Parsed {
            spec: None,
            doc: None,
            diagnostics,
        };
    };
    let fail = |mut diagnostics: Vec<Diagnostic>, d: Diagnostic, doc: Document| {
        diagnostics.push(d);
        Parsed {
            spec: None,
            doc: Some(doc),
            diagnostics,
        }
    };
    let Value::Object(map) = &doc.value else {
        let d = diag(
            file,
            &[],
            Pos::START,
            code::INVALID_VALUE,
            Severity::Error,
            "agent.yaml muss ein YAML-Objekt sein",
        );
        return fail(diagnostics, d, doc);
    };
    // Erst die Version: ein Dokument einer anderen Version soll nicht an unbekannten Feldern
    // scheitern, sondern klar abgelehnt werden (AGT-001 AC4).
    if let Some(v) = map.get("spec_version")
        && let Some(n) = v
            .as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        && n != SPEC_VERSION
    {
        let path = [Seg::Key("spec_version".into())];
        let d = diag(
            file,
            &path,
            doc.pos(&path),
            code::UNSUPPORTED_SPEC_VERSION,
            Severity::Error,
            format!(
                "Nicht unterstützte Spec-Version {n}: diese beton-Version kennt nur spec_version: {SPEC_VERSION}"
            ),
        );
        return fail(diagnostics, d, doc);
    }
    let mut value = doc.value.clone();
    let mut extensions = std::collections::BTreeMap::new();
    if let Value::Object(m) = &mut value {
        let keys: Vec<String> = m.keys().filter(|k| k.starts_with("x-")).cloned().collect();
        for k in keys {
            if let Some(v) = m.remove(&k) {
                extensions.insert(k, v);
            }
        }
    }
    // Unbekannte Felder werden gemeldet und für einen weiteren Durchlauf entfernt, damit
    // mehrere Tippfehler auf einmal erscheinen.
    let mut spec = None;
    for _ in 0..32 {
        match serde_path_to_error::deserialize::<_, AgentSpec>(value.clone()) {
            Ok(mut s) => {
                s.extensions = extensions;
                spec = Some(s);
                break;
            }
            Err(err) => {
                let (d, removable) = schema_error(file, &doc, &err);
                diagnostics.push(d);
                match removable {
                    Some(path) if remove_path(&mut value, &path) => continue,
                    _ => break,
                }
            }
        }
    }
    // Gelang ein späterer Durchlauf, bleibt `spec` für die semantischen Prüfungen gesetzt;
    // ungültig ist der Agent wegen der Befunde trotzdem.
    Parsed {
        spec,
        doc: Some(doc),
        diagnostics,
    }
}
