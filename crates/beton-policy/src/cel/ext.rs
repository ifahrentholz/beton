//! beton-Erweiterungsfunktionen für CEL (POL-002) und gemeinsame Helfer.
//!
//! `glob`, `under`, `domain_matches`, `bytes_human`, `shell.has_command`, `shell.has_flag`,
//! `url.host`, `url.path` sowie `duration()`-Parsing.

use regex::{Regex, RegexBuilder};

/// Obergrenze für kompilierte Regexe (Speicher), damit ein Muster die Engine nicht aufbläht.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Kompiliert ein Regex mit Größenlimit (Dialekt des `regex`-Crates, RE2-kompatibel).
pub fn regex(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("ungültiger Regex `{pattern}`: {e}"))
}

/// Glob → verankertes Regex. `*` passt auf alles außer `/`, `**` auch über `/` hinweg
/// (`**/` darf null Verzeichnisse überspringen), `?` auf ein Zeichen außer `/`.
pub fn glob_regex(pattern: &str) -> Result<Regex, String> {
    let mut out = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out.push('$');
    regex(&out)
}

/// `domain_matches(host, "*.example.com")`: Subdomains beliebiger Tiefe, nicht die Domain
/// selbst; ohne `*.` exakter Vergleich. Groß-/Kleinschreibung und ein abschließender Punkt
/// zählen nicht. `*` allein passt auf jeden Host.
pub fn domain_matches(host: &str, pattern: &str) -> bool {
    let norm = |s: &str| s.trim_end_matches('.').to_ascii_lowercase();
    let host = norm(host);
    let pattern = norm(pattern);
    if pattern == "*" {
        return !host.is_empty();
    }
    match pattern.strip_prefix("*.") {
        Some(suffix) => {
            host.len() > suffix.len() + 1
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
        }
        None => host == pattern,
    }
}

/// Lexikalische Normalisierung eines absoluten Pfads: `.` und `..` auflösen, doppelte
/// Trenner entfernen; `..` über die Wurzel hinaus bleibt an der Wurzel. `None` für
/// relative Pfade.
pub fn normalize_path(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

/// Pfadauflösung für `under()`. Die Engine ist rein; der Default normalisiert nur
/// lexikalisch. Der Runner kann eine Auflösung mit Symlinks einsetzen (WP-35); ein Fehler
/// macht die Auswertung zum `error` (fail closed).
pub trait PathResolver: Send + Sync {
    fn resolve(&self, path: &str) -> Result<String, String>;
}

/// Rein lexikalische Auflösung (ohne Dateisystem).
#[derive(Debug, Clone, Copy, Default)]
pub struct LexicalResolver;

impl PathResolver for LexicalResolver {
    fn resolve(&self, path: &str) -> Result<String, String> {
        normalize_path(path).ok_or_else(|| format!("Pfad `{path}` ist nicht absolut"))
    }
}

/// `under(path, root)`: liegt `path` (normalisiert) in `root` oder ist er `root`?
pub fn under(resolver: &dyn PathResolver, path: &str, root: &str) -> Result<bool, String> {
    let path = resolver.resolve(path)?;
    let root = resolver.resolve(root)?;
    if root == "/" {
        return Ok(true);
    }
    Ok(path == root || path.starts_with(&format!("{root}/")))
}

/// `bytes_human(n)`: `512 B`, `1.5 KiB`, `5.0 MiB` …
pub fn bytes_human(n: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `url.host(u)`: Host nach WHATWG-URL-Regeln (wie ihn ein HTTP-Client auflöst), klein
/// geschrieben; Fehler, wenn `u` keine URL mit Host ist.
pub fn url_host(u: &str) -> Result<String, String> {
    let parsed = url::Url::parse(u).map_err(|e| format!("keine URL `{u}`: {e}"))?;
    parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("URL `{u}` hat keinen Host"))
}

/// `url.path(u)`: Pfad der URL (ohne Query und Fragment).
pub fn url_path(u: &str) -> Result<String, String> {
    let parsed = url::Url::parse(u).map_err(|e| format!("keine URL `{u}`: {e}"))?;
    Ok(parsed.path().to_owned())
}

/// Dauer im CEL-Format: `1h30m`, `90s`, `1.5h`, `250ms`, `-5m`.
pub fn parse_duration(s: &str) -> Result<time::Duration, String> {
    let bad = || format!("ungültige Dauer `{s}` (z. B. `30m`, `1h30m`, `250ms`)");
    let (neg, mut rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if rest == "0" {
        return Ok(time::Duration::ZERO);
    }
    if rest.is_empty() {
        return Err(bad());
    }
    let mut total_ns: f64 = 0.0;
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(bad)?;
        if num_len == 0 {
            return Err(bad());
        }
        let number: f64 = rest[..num_len].parse().map_err(|_| bad())?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "h" => 3.6e12,
            "m" => 6e10,
            "s" => 1e9,
            "ms" => 1e6,
            "us" | "µs" => 1e3,
            "ns" => 1.0,
            _ => return Err(bad()),
        };
        rest = &rest[unit_len..];
        total_ns += number * factor;
    }
    if !total_ns.is_finite() || total_ns.abs() > 3.15e20 {
        return Err(bad());
    }
    let d = time::Duration::nanoseconds(total_ns as i64);
    Ok(if neg { -d } else { d })
}
