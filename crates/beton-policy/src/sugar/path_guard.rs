//! `path_guard` (POL-016): Lese- und Schreibregeln auf normalisierten Pfaden.
//!
//! Entzuckerung zu bis zu zwei Regeln in Phase `tool_call` mit der Aktion `on_violation`
//! (`deny` oder `ask`):
//!
//! - `<id>/read`: ein Pfad aus `tool.paths` verletzt `read` – auch ein geschriebener, weil
//!   Schreiben oft Lesen einschließt (`mv` liest die Quelle);
//! - `<id>/write`: ein Pfad aus `tool.write_paths` verletzt `write`.
//!
//! Ein Pfad verletzt eine Seite, wenn er auf ein `deny`-Muster passt oder `allow` nicht leer
//! ist und er auf keines davon passt. Verglichen wird der aufgelöste Pfad (`resolve()`,
//! `under()`: Symlinks über die Pfadauflösung des Runners). Fehlt ein Feld, gilt der Standard:
//! Schreiben nur im Worktree, sensible Muster weder lesen noch schreiben.
//!
//! Muster sind Globs (`*` ohne `/`, `**` auch über `/`): absolut, mit `**/` am Anfang oder
//! mit `~/`, `${worktree}`, `${home}` bzw. `${project_root}` (Kontext `session.worktree`,
//! `host.home`, `session.project_root`). Fehlt der Wert einer Variable, scheitert die Regel
//! und lehnt ab (fail closed). Die Pfaderkennung in Shell-Kommandos ist best effort; die
//! verbindliche Durchsetzung leistet die Sandbox (SBX-003, SBX-004).

use serde_json::Value;

use super::{ParamError, Params, Sugar, SugarRule, check_glob, lit, strings_at};
use crate::Phase;
use crate::doc::Action;

const PARAMS: [&str; 3] = ["read", "write", "on_violation"];
const SIDE: [&str; 2] = ["allow", "deny"];

/// Sensible Muster, die ohne eigene `deny`-Liste weder gelesen noch geschrieben werden.
pub(crate) const SENSITIVE: [&str; 4] = ["**/.env*", "**/.git/config", "~/.ssh/**", "~/.aws/**"];

/// Standard für `write.allow`.
const WRITE_ALLOW: [&str; 1] = ["${worktree}/**"];

/// Variablen in Mustern und ihre Kontextfelder.
const VARS: [(&str, &str); 3] = [
    ("worktree", "session.worktree"),
    ("home", "host.home"),
    ("project_root", "session.project_root"),
];

pub(super) fn desugar(params: &Params) -> Result<Sugar, ParamError> {
    params.only(&PARAMS)?;
    let action = match params.text("on_violation")?.as_deref() {
        None | Some("deny") => Action::Deny,
        Some("ask") => Action::Ask,
        Some(other) => {
            return Err(ParamError::new(
                "on_violation",
                format!("`on_violation: {other}` ist ungültig (deny, ask)"),
            ));
        }
    };
    let sensitive: Vec<String> = SENSITIVE.iter().map(|s| (*s).to_owned()).collect();
    let (read_allow, read_deny) = side(params, "read", Vec::new(), sensitive.clone())?;
    let write_default: Vec<String> = WRITE_ALLOW.iter().map(|s| (*s).to_owned()).collect();
    let (write_allow, write_deny) = side(params, "write", write_default, sensitive)?;

    let mut rules = Vec::new();
    if let Some(violation) = violation(&read_allow, &read_deny, "read")? {
        rules.push(SugarRule {
            part: Some("read"),
            phase: Phase::ToolCall,
            when: format!("tool.paths.exists(p, {violation})"),
            action,
            reason: "Lesen eines geschützten bzw. sensiblen Pfads (path_guard)".into(),
            kinds: Some(vec![
                "file_read".into(),
                "search".into(),
                "shell".into(),
                "file_write".into(),
                "file_edit".into(),
            ]),
            notify: None,
        });
    }
    if let Some(violation) = violation(&write_allow, &write_deny, "write")? {
        rules.push(SugarRule {
            part: Some("write"),
            phase: Phase::ToolCall,
            when: format!("tool.write_paths.exists(p, {violation})"),
            action,
            reason: "Schreiben außerhalb der erlaubten Pfade (Standard: nur im Worktree) bzw. auf \
                     einen sensiblen Pfad (path_guard)"
                .into(),
            kinds: Some(vec![
                "file_write".into(),
                "file_edit".into(),
                "shell".into(),
            ]),
            notify: None,
        });
    }
    Ok(Sugar {
        rules,
        defaults: Vec::new(),
    })
}

/// `allow` und `deny` einer Seite (`read`/`write`), sonst die Standards.
fn side(
    params: &Params,
    name: &str,
    allow: Vec<String>,
    deny: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), ParamError> {
    let Some(v) = params.get(name) else {
        return Ok((allow, deny));
    };
    let Value::Object(map) = v else {
        return Err(ParamError::new(name, "Objekt `{ allow, deny }` erwartet"));
    };
    if let Some(unknown) = map.keys().find(|k| !SIDE.contains(&k.as_str())) {
        return Err(ParamError::new(
            format!("{name}.{unknown}"),
            format!("unbekannter Parameter `{unknown}` (erlaubt: allow, deny)"),
        ));
    }
    let allow = strings_at(map.get("allow"), &format!("{name}.allow"))?.unwrap_or(allow);
    let deny = strings_at(map.get("deny"), &format!("{name}.deny"))?.unwrap_or(deny);
    Ok((allow, deny))
}

/// Bedingung „`p` verletzt diese Seite“; `None`, wenn nichts eingeschränkt ist.
fn violation(allow: &[String], deny: &[String], name: &str) -> Result<Option<String>, ParamError> {
    let any = |patterns: &[String], field: &str| -> Result<String, ParamError> {
        let parts = patterns
            .iter()
            .enumerate()
            .map(|(i, p)| matcher(p, &format!("{name}.{field}[{i}]")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(parts.join(" || "))
    };
    let mut parts = Vec::new();
    if !deny.is_empty() {
        parts.push(any(deny, "deny")?);
    }
    if !allow.is_empty() {
        parts.push(format!("!({})", any(allow, "allow")?));
    }
    Ok((!parts.is_empty()).then(|| parts.join(" || ")))
}

/// Bedingung „`p` passt auf `pattern`“ (aufgelöste Pfade).
fn matcher(pattern: &str, field: &str) -> Result<String, ParamError> {
    let expanded = if pattern == "~" {
        "${home}".to_owned()
    } else if let Some(rest) = pattern.strip_prefix("~/") {
        format!("${{home}}/{rest}")
    } else {
        pattern.to_owned()
    };
    if let Some(after) = expanded.strip_prefix("${") {
        let Some((var, rest)) = after.split_once('}') else {
            return Err(ParamError::new(field, format!("`}}` fehlt in `{pattern}`")));
        };
        let Some((_, root)) = VARS.iter().find(|(name, _)| *name == var) else {
            return Err(ParamError::new(
                field,
                format!(
                    "unbekannte Variable `${{{var}}}` (erlaubt: ${{worktree}}, ${{home}}, ${{project_root}})"
                ),
            ));
        };
        if rest.is_empty() {
            return Ok(format!("resolve(p) == resolve({root})"));
        }
        if !rest.starts_with('/') {
            return Err(ParamError::new(
                field,
                format!("nach `${{{var}}}` muss `/` folgen: `{pattern}`"),
            ));
        }
        check_glob(rest, field)?;
        // Verzeichnis ohne Platzhalter plus `/**`: genau `under()`.
        if let Some(dir) = rest.strip_suffix("/**")
            && !dir.contains(['*', '?'])
        {
            return Ok(if dir.is_empty() {
                format!("under(p, {root})")
            } else {
                format!("under(p, {root} + {})", lit(dir))
            });
        }
        // `under()` legt die Wurzel fest; der Glob prüft den Rest.
        return Ok(format!(
            "(under(p, {root}) && glob(resolve(p), resolve({root}) + {}))",
            lit(rest)
        ));
    }
    if !(expanded.starts_with('/') || expanded.starts_with("**")) {
        return Err(ParamError::new(
            field,
            format!(
                "Muster `{pattern}` muss absolut sein oder mit `**/`, `~/`, `${{worktree}}`, \
                 `${{home}}` bzw. `${{project_root}}` beginnen"
            ),
        ));
    }
    check_glob(&expanded, field)?;
    Ok(format!("glob(resolve(p), {})", lit(&expanded)))
}
