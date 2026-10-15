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
    if !name.ends_with('*') && crate::resolve::controlled_env(name) {
        return Err(SandboxError::invalid(
            field,
            format!("`{name}` setzt beton je Stufe selbst (Scratch, Proxy, CA, Session)"),
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
    if let Some(creds) = &spec.credentials {
        validate_credentials(spec, creds)?;
    }
    if let Some(limits) = &spec.limits {
        limits.validate()?;
    }
    for (i, rule) in spec.egress_rules.iter().flatten().enumerate() {
        if rule.trim().is_empty() {
            return Err(SandboxError::invalid(
                format!("egress_rules[{i}]"),
                "leere Regel",
            ));
        }
    }
    let harness = spec.harness.clone().unwrap_or_default();
    for (i, rule) in harness.egress_extra.iter().flatten().enumerate() {
        if rule.trim().is_empty() {
            return Err(SandboxError::invalid(
                format!("harness.egress_extra[{i}]"),
                "leere Regel",
            ));
        }
    }
    for (name, list) in [
        ("egress_passthrough", &spec.egress_passthrough),
        ("harness.egress_passthrough", &harness.egress_passthrough),
    ] {
        for (i, entry) in list.iter().flatten().enumerate() {
            check_passthrough(&format!("{name}[{i}]"), entry)?;
        }
    }
    for (i, port) in spec.dev_ports.iter().flatten().enumerate() {
        if *port == 0 {
            return Err(SandboxError::invalid(
                format!("dev_ports[{i}]"),
                "Port 0 ist kein gültiger Port",
            ));
        }
    }
    Ok(())
}

/// Hostname eines Credential-Bindings (PRX-006): exakt oder `*.domain`, ohne Port, Pfad oder
/// IP-Literal; die genaue Prüfung übernimmt der Proxy beim Start der Instanz.
fn check_credential_host(field: &str, raw: &str) -> Result<(), SandboxError> {
    let host = raw.strip_prefix("*.").unwrap_or(raw);
    let ip_like = host.chars().all(|c| c.is_ascii_digit() || c == '.');
    let bad = host.is_empty()
        || host.len() > 253
        || ip_like
        || host.starts_with('.')
        || host.ends_with('.')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if bad {
        return Err(SandboxError::invalid(
            field,
            format!("`{raw}`: erwartet wird ein Hostname oder `*.domain` (ohne Port, ohne IP)"),
        ));
    }
    Ok(())
}

/// `sandbox.credentials` (PRX-006): Namen, Hosts, genau eine Quelle, Variablen.
fn validate_credentials(
    spec: &SandboxSpec,
    creds: &[crate::spec::CredentialSpec],
) -> Result<(), SandboxError> {
    use crate::spec::CredentialType;
    let set: Vec<&String> = spec
        .env
        .as_ref()
        .map(|e| e.set.keys().collect())
        .unwrap_or_default();
    let mut names: Vec<&str> = Vec::new();
    let mut vars: Vec<&str> = Vec::new();
    for (i, c) in creds.iter().enumerate() {
        let f = |s: &str| format!("credentials[{i}].{s}");
        let name_ok = !c.name.is_empty()
            && c.name.len() <= 64
            && c.name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-');
        if !name_ok {
            return Err(SandboxError::invalid(
                f("name"),
                format!(
                    "`{}`: erlaubt sind a-z, 0-9, `_` und `-` (höchstens 64)",
                    c.name
                ),
            ));
        }
        if names.contains(&c.name.as_str()) {
            return Err(SandboxError::invalid(
                f("name"),
                format!("`{}` ist doppelt", c.name),
            ));
        }
        names.push(&c.name);
        let hosts = c.hosts.clone().unwrap_or_default();
        let needs_hosts = matches!(
            c.kind,
            CredentialType::Bearer | CredentialType::Basic | CredentialType::GitHttps
        );
        if needs_hosts && hosts.is_empty() {
            return Err(SandboxError::invalid(
                f("hosts"),
                format!("Typ `{}` braucht gebundene Hosts", c.kind.as_str()),
            ));
        }
        for (j, h) in hosts.iter().enumerate() {
            check_credential_host(&f(&format!("hosts[{j}]")), h)?;
        }
        match &c.username {
            None if c.kind == CredentialType::Basic => {
                return Err(SandboxError::invalid(
                    f("username"),
                    "Typ `basic` braucht `username`",
                ));
            }
            Some(u) if u.is_empty() || u.contains(':') || u.chars().any(char::is_control) => {
                return Err(SandboxError::invalid(
                    f("username"),
                    "leer, mit `:` oder Steuerzeichen",
                ));
            }
            _ => {}
        }
        let src = &c.source;
        let count = [
            src.secret.is_some(),
            src.env.is_some(),
            src.file.is_some(),
            src.command.is_some(),
        ]
        .into_iter()
        .filter(|b| *b)
        .count();
        if count != 1 {
            return Err(SandboxError::invalid(
                f("source"),
                "genau eine Quelle: `secret`, `env`, `file` oder `command`",
            ));
        }
        if let Some(r) = &src.secret
            && (!r.starts_with("secret://") || r.len() <= "secret://".len())
        {
            return Err(SandboxError::invalid(
                f("source.secret"),
                "erwartet wird eine SecretRef `secret://…`",
            ));
        }
        if let Some(v) = &src.env {
            let valid = !v.is_empty()
                && !v.starts_with(|ch: char| ch.is_ascii_digit())
                && v.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            if !valid || v.starts_with("BETON_") {
                return Err(SandboxError::invalid(
                    f("source.env"),
                    format!("`{v}`: kein gültiger Variablenname bzw. `BETON_*`"),
                ));
            }
        }
        if let Some(path) = &src.file {
            check_syntax(&f("source.file"), path)?;
        }
        if let Some(argv) = &src.command
            && (argv.first().is_none_or(String::is_empty) || argv.iter().any(|a| a.contains('\0')))
        {
            return Err(SandboxError::invalid(
                f("source.command"),
                "erwartet wird ein nicht leeres argv ohne NUL-Bytes",
            ));
        }
        if src.ttl.is_some() && src.command.is_none() {
            return Err(SandboxError::invalid(
                f("source.ttl"),
                "`ttl` gilt nur für `command`",
            ));
        }
        src.ttl_secs()
            .map_err(|e| SandboxError::invalid(f("source.ttl"), e))?;
        for (j, var) in c.env.iter().enumerate() {
            let field = f(&format!("env[{j}]"));
            check_env_name(&field, var, false)?;
            if vars.contains(&var.as_str()) {
                return Err(SandboxError::invalid(
                    field,
                    format!("`{var}` bekommt schon ein anderes Credential"),
                ));
            }
            if set.iter().any(|s| s.as_str() == var) {
                return Err(SandboxError::invalid(
                    field,
                    format!("`{var}` steht auch in `env.set`"),
                ));
            }
            vars.push(var);
        }
    }
    Ok(())
}

/// Form eines Passthrough-Eintrags (`host` oder `host:port`, PRX-005); die genaue Prüfung des
/// Hosts übernimmt der Proxy beim Start der Instanz.
fn check_passthrough(field: &str, raw: &str) -> Result<(), SandboxError> {
    let bad = raw.is_empty()
        || raw.len() > 260
        || raw.contains(['/', '*', '@', '[', ']', '?', '#'])
        || raw.chars().any(|c| c.is_whitespace() || c.is_control());
    if bad {
        return Err(SandboxError::invalid(
            field,
            format!(
                "`{raw}`: erwartet wird `host` oder `host:port` (ohne Schema, Pfad oder Wildcard)"
            ),
        ));
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
