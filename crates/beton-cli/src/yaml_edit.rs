//! Änderungen an YAML-Konfigurationsdateien, die Kommentare und Reihenfolge erhalten (CLI-008,
//! #72): `config set/unset` ersetzt bzw. entfernt nur die betroffene Zeile samt ihrem Block.
//!
//! Unterstützt werden Block-Zuordnungen (`schlüssel:` mit eingerückten Kindern). Alles andere
//! (Flow-Stil `{…}`, Anker, mehrzeilige Skalare unter dem Pfad) liefert `None`; der Aufrufer
//! schreibt dann wie bisher neu. Das Ergebnis prüft der Aufrufer, indem er es wieder einliest.

use serde_json::Value;

#[derive(Debug, Clone)]
struct Line<'a> {
    text: &'a str,
    indent: usize,
    /// Schlüssel einer Zeile `schlüssel: …` (ohne Anführungszeichen).
    key: Option<String>,
    /// Rest nach `:` (ohne Kommentar), leer bei einem Block.
    rest: String,
}

fn is_content(l: &Line<'_>) -> bool {
    let t = l.text.trim();
    !t.is_empty() && !t.starts_with('#')
}

fn parse_line(text: &str) -> Line<'_> {
    let indent = text.len() - text.trim_start_matches(' ').len();
    let t = text.trim_start();
    let mut key = None;
    let mut rest = String::new();
    if !t.starts_with('#') && !t.starts_with('-') {
        let (k, r) = if let Some(stripped) = t.strip_prefix('"') {
            match stripped.find("\":") {
                Some(end) => (Some(stripped[..end].to_owned()), &stripped[end + 2..]),
                None => (None, ""),
            }
        } else if let Some(stripped) = t.strip_prefix('\'') {
            match stripped.find("':") {
                Some(end) => (Some(stripped[..end].to_owned()), &stripped[end + 2..]),
                None => (None, ""),
            }
        } else {
            match t.find(':') {
                Some(end) if t[end + 1..].is_empty() || t[end + 1..].starts_with(' ') => {
                    (Some(t[..end].to_owned()), &t[end + 1..])
                }
                _ => (None, ""),
            }
        };
        if let Some(k) = k {
            key = Some(k);
            rest = strip_comment(r).trim().to_owned();
        }
    }
    Line {
        text,
        indent,
        key,
        rest,
    }
}

/// Kommentar am Zeilenende (` #`) außerhalb von Anführungszeichen entfernen.
fn strip_comment(s: &str) -> &str {
    let (mut single, mut double) = (false, false);
    let bytes = s.as_bytes();
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double && (i == 0 || bytes[i - 1] == b' ') => return &s[..i],
            _ => {}
        }
    }
    s
}

/// Kommentar am Zeilenende samt Leerraum davor (zum Erhalten), falls vorhanden.
fn trailing_comment(text: &str) -> Option<&str> {
    let t = text.trim_start();
    let body = strip_comment(t);
    (body.len() < t.len()).then(|| &t[body.trim_end().len()..])
}

/// Ende (exklusiv) des Blocks der Zeile `at`: bis vor die nächste Inhaltszeile mit
/// Einrückung ≤ der von `at`; Kommentare und Leerzeilen am Ende gehören nicht dazu.
fn block_end(lines: &[Line<'_>], at: usize, end: usize) -> usize {
    let indent = lines[at].indent;
    let mut last = at;
    for (i, l) in lines.iter().enumerate().take(end).skip(at + 1) {
        if is_content(l) {
            if l.indent <= indent {
                break;
            }
            last = i;
        }
    }
    last + 1
}

/// Einrückung der Kinder im Bereich (erste Inhaltszeile), sonst `parent + 2`.
fn child_indent(lines: &[Line<'_>], start: usize, end: usize, parent: Option<usize>) -> usize {
    lines[start..end]
        .iter()
        .find(|l| is_content(l))
        .map_or(parent.map_or(0, |p| p + 2), |l| l.indent)
}

fn find(lines: &[Line<'_>], start: usize, end: usize, indent: usize, key: &str) -> Option<usize> {
    (start..end).find(|&i| {
        is_content(&lines[i]) && lines[i].indent == indent && lines[i].key.as_deref() == Some(key)
    })
}

fn render_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        key.to_owned()
    } else {
        serde_norway::to_string(key)
            .map(|s| s.trim_end().to_owned())
            .unwrap_or_else(|_| format!("\"{key}\""))
    }
}

/// Zeilen für `key: value` mit Einrückung `indent`.
fn render(indent: usize, key: &str, value: &Value) -> Option<Vec<String>> {
    let pad = " ".repeat(indent);
    let key = render_key(key);
    let complex = match value {
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => false,
    };
    if !complex {
        let scalar = match value {
            Value::Object(_) => "{}".to_owned(),
            Value::Array(_) => "[]".to_owned(),
            v => serde_norway::to_string(v).ok()?.trim_end().to_owned(),
        };
        if scalar.contains('\n') {
            return None;
        }
        return Some(vec![format!("{pad}{key}: {scalar}")]);
    }
    let body = serde_norway::to_string(value).ok()?;
    let mut out = vec![format!("{pad}{key}:")];
    let inner = " ".repeat(indent + 2);
    out.extend(body.lines().map(|l| format!("{inner}{l}")));
    Some(out)
}

fn join(lines: &[String], trailing_newline: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_newline || !s.is_empty() {
        s.push('\n');
    }
    s
}

/// Setzt `path` auf `value` und behält alles andere (Kommentare, Reihenfolge).
pub fn set(text: &str, path: &[String], value: &Value) -> Option<String> {
    let (leaf, parents) = path.split_last()?;
    let lines: Vec<Line<'_>> = text.lines().map(parse_line).collect();
    if lines.iter().any(|l| {
        let t = l.text.trim();
        t.starts_with("---") || t.starts_with("...") || t.contains('&') && t.contains(": &")
    }) {
        return None;
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.text.to_owned()).collect();
    let (mut start, mut end, mut parent) = (0, lines.len(), None::<usize>);
    for (depth, seg) in parents.iter().enumerate() {
        let indent = child_indent(&lines, start, end, parent);
        match find(&lines, start, end, indent, seg) {
            Some(i) => {
                if !lines[i].rest.is_empty() {
                    return None;
                }
                let b = block_end(&lines, i, end);
                parent = Some(indent);
                start = i + 1;
                end = b;
            }
            None => {
                // Fehlende Abschnitte am Ende des Bereichs anlegen.
                let mut insert = Vec::new();
                let mut ind = indent;
                for p in &parents[depth..] {
                    insert.push(format!("{}{}:", " ".repeat(ind), render_key(p)));
                    ind += 2;
                }
                insert.extend(render(ind, leaf, value)?);
                let at = last_content(&lines, start, end).map_or(start, |i| i + 1);
                out.splice(at..at, insert);
                return Some(join(&out, true));
            }
        }
    }
    let indent = child_indent(&lines, start, end, parent);
    let mut new = render(indent, leaf, value)?;
    match find(&lines, start, end, indent, leaf) {
        Some(i) => {
            let b = block_end(&lines, i, end);
            // Kommentar am Ende einer einzeiligen Angabe bleibt.
            if new.len() == 1
                && b == i + 1
                && let Some(c) = trailing_comment(lines[i].text)
            {
                new[0].push(' ');
                new[0].push_str(c.trim_start());
            }
            out.splice(i..b, new);
        }
        None => {
            let at = last_content(&lines, start, end).map_or(start, |i| i + 1);
            out.splice(at..at, new);
        }
    }
    Some(join(&out, true))
}

fn last_content(lines: &[Line<'_>], start: usize, end: usize) -> Option<usize> {
    (start..end).rev().find(|&i| is_content(&lines[i]))
}

/// Entfernt `path` samt Block und leer gewordene Abschnitte; `None`, wenn nicht editierbar.
pub fn unset(text: &str, path: &[String]) -> Option<String> {
    let lines: Vec<Line<'_>> = text.lines().map(parse_line).collect();
    let mut out: Vec<String> = lines.iter().map(|l| l.text.to_owned()).collect();
    let (mut start, mut end, mut parent) = (0, lines.len(), None::<usize>);
    // (Zeile, Blockende) je Abschnitt auf dem Pfad.
    let mut chain: Vec<(usize, usize)> = Vec::new();
    for seg in path {
        let indent = child_indent(&lines, start, end, parent);
        let i = find(&lines, start, end, indent, seg)?;
        let b = block_end(&lines, i, end);
        chain.push((i, b));
        parent = Some(indent);
        start = i + 1;
        end = b;
    }
    let (mut i, mut b) = chain.pop()?;
    // Leere Elternabschnitte mit entfernen.
    while let Some(&(pi, pb)) = chain.last() {
        let others = (pi + 1..pb).any(|j| (j < i || j >= b) && is_content(&lines[j]));
        if others {
            break;
        }
        chain.pop();
        i = pi;
        b = pb;
    }
    out.drain(i..b);
    Some(join(&out, true))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(s: &str) -> Vec<String> {
        s.split('.').map(str::to_owned).collect()
    }

    fn parse(t: &str) -> Value {
        serde_norway::from_str(t).unwrap()
    }

    const FILE: &str = "# Meine Konfiguration\nserver:\n  # Port lokal\n  listen: [\"127.0.0.1:7420\"]\n\nsandbox:\n  backend: auto # nie none\n  egress_rules:\n    - \"GET,HEAD registry.npmjs.org/**\"\n# Ende\n";

    #[test]
    fn cli_008_set_replaces_one_value_and_keeps_comments_and_order() {
        let out = set(FILE, &p("sandbox.backend"), &json!("seatbelt")).unwrap();
        assert!(out.contains("# Meine Konfiguration"));
        assert!(out.contains("  # Port lokal"));
        assert!(out.contains("  backend: seatbelt # nie none"), "{out}");
        assert!(out.contains("# Ende"));
        assert!(out.find("server:").unwrap() < out.find("sandbox:").unwrap());
        let v = parse(&out);
        assert_eq!(v["sandbox"]["backend"], "seatbelt");
        assert_eq!(v["server"]["listen"], json!(["127.0.0.1:7420"]));
    }

    #[test]
    fn cli_008_set_adds_new_keys_and_sections_in_place() {
        let out = set(FILE, &p("sandbox.allow_network"), &json!(false)).unwrap();
        let v = parse(&out);
        assert_eq!(v["sandbox"]["allow_network"], false);
        assert!(
            out.find("allow_network").unwrap() < out.find("# Ende").unwrap(),
            "{out}"
        );
        let out = set(FILE, &p("compaction.auto"), &json!("beton")).unwrap();
        assert_eq!(parse(&out)["compaction"]["auto"], "beton");
        let out = set(
            FILE,
            &p("sandbox.egress_rules"),
            &json!(["GET a.test/**", "GET b.test/**"]),
        )
        .unwrap();
        assert_eq!(
            parse(&out)["sandbox"]["egress_rules"],
            json!(["GET a.test/**", "GET b.test/**"])
        );
        assert!(out.contains("  backend: auto # nie none"));
        let out = set("", &p("a.b"), &json!(1)).unwrap();
        assert_eq!(parse(&out), json!({"a": {"b": 1}}));
    }

    #[test]
    fn cli_008_unset_removes_the_block_and_empty_sections() {
        let out = unset(FILE, &p("sandbox.egress_rules")).unwrap();
        let v = parse(&out);
        assert!(v["sandbox"].get("egress_rules").is_none());
        assert_eq!(v["sandbox"]["backend"], "auto");
        assert!(out.contains("# Ende") && out.contains("# Port lokal"));
        let out = unset(FILE, &p("server.listen")).unwrap();
        assert!(parse(&out).get("server").is_none(), "{out}");
        assert!(unset(FILE, &p("gibt.es.nicht")).is_none());
    }

    #[test]
    fn cli_008_flow_style_parents_are_not_edited() {
        assert!(
            set(
                "sandbox: {backend: auto}\n",
                &p("sandbox.backend"),
                &json!("x")
            )
            .is_none()
        );
    }
}
