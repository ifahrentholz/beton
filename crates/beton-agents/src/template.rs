//! Minimal-Templating ohne Logik (AGT-001 Details, AGT-010): `{{ params.x }}`, `{{ now }}`
//! und `{{ trigger.payload.x }}`, nur in `instructions.append`, `prompt` und Schedule-Feldern.
//!
//! Es gibt keine Bedingungen, Schleifen oder Filter. Ein nicht gesetzter Parameter ergibt
//! einen leeren Text; unbekannte Ausdrücke meldet die Validierung (`invalid_param` bzw.
//! `invalid_value`), beim Rendern bleiben sie leer.

use serde_json::Value;

use crate::params::ParamValues;

/// Ein Ausdruck zwischen `{{` und `}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `params.<name>`
    Param(String),
    /// `now`: Startzeitpunkt (RFC 3339, UTC).
    Now,
    /// `trigger.payload.<pfad>` (Webhook-Payload, ab M5).
    TriggerPayload(String),
    /// Alles andere.
    Unknown(String),
}

fn parse_expr(inner: &str) -> Expr {
    let e = inner.trim();
    if e == "now" {
        return Expr::Now;
    }
    if let Some(name) = e.strip_prefix("params.")
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Expr::Param(name.to_owned());
    }
    if let Some(path) = e.strip_prefix("trigger.payload.")
        && !path.is_empty()
    {
        return Expr::TriggerPayload(path.to_owned());
    }
    Expr::Unknown(e.to_owned())
}

/// Teile eines Textes: Literal oder Ausdruck.
fn pieces(text: &str) -> Vec<Result<&str, Expr>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else {
            break;
        };
        if start > 0 {
            out.push(Ok(&rest[..start]));
        }
        out.push(Err(parse_expr(&rest[start + 2..start + 2 + len])));
        rest = &rest[start + 2 + len + 2..];
    }
    if !rest.is_empty() {
        out.push(Ok(rest));
    }
    out
}

/// Alle Ausdrücke eines Textes in Reihenfolge.
pub fn expressions(text: &str) -> Vec<Expr> {
    pieces(text).into_iter().filter_map(Result::err).collect()
}

/// Enthält der Text einen Template-Ausdruck?
pub fn has_expressions(text: &str) -> bool {
    !expressions(text).is_empty()
}

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Setzt Parameterwerte und `now` ein.
pub fn render(text: &str, params: &ParamValues, now: &str) -> String {
    pieces(text)
        .into_iter()
        .map(|p| match p {
            Ok(lit) => lit.to_owned(),
            Err(Expr::Param(name)) => params.get(&name).map(show).unwrap_or_default(),
            Err(Expr::Now) => now.to_owned(),
            Err(Expr::TriggerPayload(_) | Expr::Unknown(_)) => String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn agt_010_params_render_into_templates() {
        let params = ParamValues::from([
            ("branch".to_owned(), json!("develop")),
            ("max_attempts".to_owned(), json!(3)),
        ]);
        let out = render(
            "Branch {{ params.branch }}, höchstens {{params.max_attempts}} Versuche, {{ params.fehlt }}!",
            &params,
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(out, "Branch develop, höchstens 3 Versuche, !");
        assert_eq!(render("{{ now }}", &params, "T"), "T");
        assert_eq!(
            render("offen {{ params.x", &params, "T"),
            "offen {{ params.x"
        );
    }

    #[test]
    fn expressions_are_classified() {
        assert_eq!(
            expressions("{{ params.a }} {{ now }} {{ trigger.payload.pr.number }} {{ env.HOME }}"),
            [
                Expr::Param("a".into()),
                Expr::Now,
                Expr::TriggerPayload("pr.number".into()),
                Expr::Unknown("env.HOME".into()),
            ]
        );
        assert!(!has_expressions("keine { Klammern }"));
    }
}
