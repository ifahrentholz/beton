//! Pfadregeln (SBX-003): nur `~` wird expandiert, verbotene Grants sind Fehler.
//!
//! Zwei Prüfungen: [`validate_spec`] beim Laden (lexikalisch, ohne Dateisystem) und
//! [`Forbidden`] bei der Auflösung auf kanonischen Pfaden (nach `realpath`), damit ein
//! Symlink in eine verbotene Wurzel als [`SandboxError::PathEscape`] scheitert.

use std::path::{Path, PathBuf};

use crate::error::SandboxError;
use crate::spec::{SandboxSpec, env_denied};

/// Grants in oder unter diesen Pfaden sind immer verboten (lexikalisch, `~` = Home).
pub const FORBIDDEN_ROOTS: [&str; 8] = [
    "~/.beton",
    "~/.ssh",
    "/proc",
    "/sys",
    "/var/run/docker.sock",
    "/private/var/run/docker.sock",
    "/run/docker.sock",
    "~/.docker/run/docker.sock",
];

/// Schreib-Wurzeln dürfen diese Pfade weder sein noch enthalten (Persistenz außerhalb des
/// Workspace, SBX-007 AC4).
pub const WRITE_PROTECTED: [&str; 2] = ["~", "~/Library/LaunchAgents"];

/// `~` bzw. `~/…` relativ zu `home`; alles andere unverändert.
pub fn expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// Lexikalisch normalisiert: doppelte `/`, `.` und abschließendes `/` entfernt.
fn normalize(raw: &str) -> String {
    let (prefix, rest) = match raw.strip_prefix('~') {
        Some(rest) => ("~", rest),
        None => ("", raw),
    };
    let parts: Vec<&str> = rest
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() {
        return if prefix.is_empty() {
            "/".into()
        } else {
            "~".into()
        };
    }
    format!("{prefix}/{}", parts.join("/"))
}

/// Form eines Pfads in `read_paths`, `write_paths` oder absoluten Masken.
pub(crate) fn check_syntax(field: &str, raw: &str) -> Result<(), SandboxError> {
    if raw.is_empty() {
        return Err(SandboxError::invalid(field, "leerer Pfad"));
    }
    if raw.contains('\0') {
        return Err(SandboxError::invalid(field, "Pfad enthält ein NUL-Byte"));
    }
    if !(raw == "~" || raw.starts_with("~/") || raw.starts_with('/')) {
        let message = if raw.starts_with('~') {
            format!("`{raw}`: nur `~` und `~/…` werden expandiert (kein `~user`)")
        } else {
            format!(
                "`{raw}` ist ein relativer Pfad; erlaubt sind absolute Pfade und `~/…` \
                 (Variablen wie `$HOME` werden nicht expandiert)"
            )
        };
        return Err(SandboxError::invalid(field, message));
    }
    if raw.split('/').any(|c| c == "..") {
        return Err(SandboxError::invalid(
            field,
            format!("`{raw}`: `..` ist nicht erlaubt"),
        ));
    }
    Ok(())
}

/// Lexikalische Verbote (ohne Dateisystem, `~` als Platzhalter für das Home).
pub(crate) fn lexical_forbidden(raw: &str, write: bool) -> Option<String> {
    let norm = normalize(raw);
    if norm == "/" {
        return Some("`/` ist als Grant verboten".into());
    }
    for f in FORBIDDEN_ROOTS {
        if norm == f || norm.starts_with(&format!("{f}/")) {
            return Some(format!("`{f}` ist als Grant verboten"));
        }
    }
    if write {
        for p in WRITE_PROTECTED {
            if norm == p || p.starts_with(&format!("{norm}/")) {
                return Some(format!("`{p}` ist als Schreib-Wurzel verboten"));
            }
        }
    }
    None
}

/// Ein Namensmuster für Masken: genau ein Pfadsegment mit `*`/`?`, optional `/` am Ende.
pub(crate) fn check_pattern(field: &str, raw: &str) -> Result<(), SandboxError> {
    let seg = raw.strip_suffix('/').unwrap_or(raw);
    if seg.is_empty() || seg == "." || seg == ".." {
        return Err(SandboxError::invalid(
            field,
            format!("ungültiges Muster `{raw}`"),
        ));
    }
    if seg.contains('/') {
        return Err(SandboxError::invalid(
            field,
            format!(
                "`{raw}`: Muster umfassen genau ein Pfadsegment (gilt in jeder Tiefe); \
                 für feste Pfade `~/…` oder `/…` verwenden"
            ),
        ));
    }
    if seg.contains('\0') {
        return Err(SandboxError::invalid(field, "Muster enthält ein NUL-Byte"));
    }
    Ok(())
}

fn check_env_name(field: &str, name: &str, allow_prefix: bool) -> Result<(), SandboxError> {
    let base = if allow_prefix {
        name.strip_suffix('*').unwrap_or(name)
    } else {
        name
    };
    let valid = !name.is_empty() && base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(SandboxError::invalid(
            field,
            format!("`{name}` ist kein gültiger Variablenname"),
        ));
    }
    if env_denied(name) {
        return Err(SandboxError::invalid(
            field,
            format!("`{name}` gelangt nie in eine Sandbox (SBX-005)"),
        ));
    }
    Ok(())
}

/// Prüft eine Konfigurationsebene lexikalisch (beim Laden).
pub(crate) fn validate_spec(spec: &SandboxSpec) -> Result<(), SandboxError> {
    for (name, list, write) in [
        ("read_paths", &spec.read_paths, false),
        ("write_paths", &spec.write_paths, true),
    ] {
        for (i, raw) in list.iter().flatten().enumerate() {
            let field = format!("{name}[{i}]");
            check_syntax(&field, raw)?;
            if let Some(reason) = lexical_forbidden(raw, write) {
                return Err(SandboxError::invalid(field, format!("`{raw}`: {reason}")));
            }
        }
    }
    for (i, raw) in spec.masks.iter().flatten().enumerate() {
        let field = format!("masks[{i}]");
        if raw.starts_with('~') || raw.starts_with('/') {
            check_syntax(&field, raw)?;
        } else {
            check_pattern(&field, raw)?;
        }
    }
    for (i, raw) in spec.allow_hidden.iter().flatten().enumerate() {
        let field = format!("allow_hidden[{i}]");
        if !raw.starts_with('.') || raw.contains('/') || raw == "." || raw == ".." {
            return Err(SandboxError::invalid(
                field,
                format!("`{raw}`: erwartet wird der Name eines Dotfiles, z. B. `.venv`"),
            ));
        }
    }
    if let Some(env) = &spec.env {
        for (i, name) in env.passthrough.iter().flatten().enumerate() {
            check_env_name(&format!("env.passthrough[{i}]"), name, true)?;
        }
        for (name, value) in &env.set {
            check_env_name(&format!("env.set.{name}"), name, false)?;
            if value.contains('\0') {
                return Err(SandboxError::invalid(
                    format!("env.set.{name}"),
                    "Wert enthält ein NUL-Byte",
                ));
            }
        }
    }
    for (i, rule) in spec.egress_rules.iter().flatten().enumerate() {
        if rule.trim().is_empty() {
            return Err(SandboxError::invalid(
                format!("egress_rules[{i}]"),
                "leere Regel",
            ));
        }
    }
    Ok(())
}

/// Verbotene Ziele auf kanonischen Pfaden.
#[derive(Debug, Clone)]
pub(crate) struct Forbidden {
    home: PathBuf,
    roots: Vec<PathBuf>,
    write_protected: Vec<PathBuf>,
}

/// `realpath`, falls der Pfad existiert.
pub(crate) fn canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

impl Forbidden {
    /// `home` ist bereits kanonisch.
    pub(crate) fn new(home: &Path) -> Self {
        let mut roots = Vec::new();
        for raw in FORBIDDEN_ROOTS {
            let p = expand(raw, home);
            if let Some(c) = canonical(&p) {
                roots.push(c);
            }
            roots.push(p);
        }
        let mut write_protected = Vec::new();
        for raw in WRITE_PROTECTED {
            let p = expand(raw, home);
            if let Some(c) = canonical(&p) {
                write_protected.push(c);
            }
            write_protected.push(p);
        }
        Self {
            home: home.to_path_buf(),
            roots,
            write_protected,
        }
    }

    /// Grund, warum `p` (kanonisch oder lexikalisch) kein Grant sein darf.
    pub(crate) fn reason(&self, p: &Path, write: bool) -> Option<String> {
        if p.parent().is_none() {
            return Some("`/` ist als Grant verboten".into());
        }
        if let Some(r) = self.roots.iter().find(|r| p.starts_with(r)) {
            return Some(format!(
                "liegt in {}, das als Grant verboten ist",
                r.display()
            ));
        }
        if p != self.home && self.home.starts_with(p) {
            return Some(format!(
                "ist ein Vorfahre von {}; nur `~` selbst ist als Lese-Grant erlaubt",
                self.home.display()
            ));
        }
        if write && let Some(w) = self.write_protected.iter().find(|w| w.starts_with(p)) {
            return Some(format!(
                "würde {} beschreibbar machen, das als Schreib-Wurzel verboten ist",
                w.display()
            ));
        }
        None
    }
}
