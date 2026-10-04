//! `beton agent list|show|validate|new|schema` (CLI-009; Semantik: AGT-002, AGT-003, AGT-013).
//!
//! Arbeitet nur auf Dateien und dem eingebetteten Built-in-Katalog, ohne Daemon.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use beton_agents::scaffold;
use beton_agents::spec::AgentName;
use beton_agents::{
    AgentRef, Builtins, Diagnostic, HarnessCatalog, ListEntry, Report, ResolvedAgent, SearchPath,
    Severity, Validator,
};
use serde_json::{Value, json};

use crate::cli::{AgentCommand, AgentNewArgs};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

/// `<projekt>/.beton`: dasselbe Projekt wie für `.beton/config.yaml` (CLI-008).
fn beton_dir(ctx: &Ctx) -> PathBuf {
    let config = ctx.paths().project;
    config
        .parent()
        .map_or_else(|| ctx.cwd.join(".beton"), Path::to_path_buf)
}

pub(crate) fn search_path(ctx: &Ctx) -> SearchPath {
    SearchPath {
        project: Some(beton_dir(ctx).join("agents")),
        user: Some(ctx.home.join("agents")),
        builtins: Builtins::embedded(),
    }
}

/// Harnesses dieses Binaries mit ihren Effort-Stufen. Den Fake-Harness gibt es wie bei
/// `serve` nur in Debug-Builds (HAR-026).
fn harness_catalog() -> HarnessCatalog {
    let registry = beton_runner::builtin_registry(
        &beton_harness::registry::HarnessLayers::default(),
        cfg!(debug_assertions),
    );
    let mut catalog = HarnessCatalog::new();
    for id in registry.ids() {
        let Some(adapter) = registry.get(id) else {
            continue;
        };
        let mut efforts: Vec<String> = Vec::new();
        for mode in adapter.modes() {
            for e in adapter
                .capabilities(*mode, &beton_harness::ProbeReport::default())
                .efforts
            {
                if !efforts.contains(&e) {
                    efforts.push(e);
                }
            }
        }
        catalog.insert(id, efforts);
    }
    catalog
}

/// Skill-Verzeichnisse außerhalb des Agents in Discovery-Reihenfolge (AGT-008).
fn skill_roots(ctx: &Ctx) -> Vec<PathBuf> {
    let beton = beton_dir(ctx);
    let mut roots = vec![beton.join("skills")];
    if let Some(project) = beton.parent() {
        roots.push(project.join(".claude/skills"));
        roots.push(project.join(".agents/skills"));
    }
    roots.push(ctx.home.join("skills"));
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".claude/skills"));
        roots.push(home.join(".agents/skills"));
    }
    roots
}

/// Pfad relativ zum Arbeitsverzeichnis bzw. mit `~`, sonst unverändert.
fn shown(ctx: &Ctx, path: &str) -> String {
    let p = Path::new(path);
    let cwd = ctx.cwd.canonicalize().unwrap_or_else(|_| ctx.cwd.clone());
    let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if let Ok(rel) = canon.strip_prefix(&cwd)
        && !rel.as_os_str().is_empty()
    {
        return rel.display().to_string();
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rel) = canon.strip_prefix(home.canonicalize().unwrap_or(home))
    {
        return format!("~/{}", rel.display());
    }
    path.to_owned()
}

pub fn run(ctx: &Ctx, cmd: AgentCommand) -> CliResult {
    match cmd {
        AgentCommand::List(args) => list(ctx, args.all),
        AgentCommand::Show(args) => show(ctx, &args.reference),
        AgentCommand::Validate(args) => validate(ctx, &args.path),
        AgentCommand::New(args) => new(ctx, &args),
        AgentCommand::Schema => {
            print!("{}", beton_agents::schema_json());
            Ok(())
        }
    }
}

fn table(rows: &[[String; 5]]) -> CliResult {
    let widths: Vec<usize> = (0..4)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = std::io::stdout().lock();
    for r in rows {
        writeln!(
            out,
            "{:w0$}  {:w1$}  {:w2$}  {:w3$}  {}",
            r[0],
            r[1],
            r[2],
            r[3],
            r[4],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3]
        )
        .context("stdout")?;
    }
    Ok(())
}

fn list(ctx: &Ctx, all: bool) -> CliResult {
    let entries = search_path(ctx).list(all);
    if ctx.global.json {
        return print_json(&serde_json::to_value(&entries).context("Liste")?);
    }
    if entries.is_empty() {
        ctx.note("Keine Agents. Neuer Agent: beton agent new <name>");
        return Ok(());
    }
    let mut rows = vec![[
        "NAME".to_owned(),
        "QUELLE".to_owned(),
        "VERSION".to_owned(),
        "HARNESS".to_owned(),
        "PFAD".to_owned(),
    ]];
    let dash = || "–".to_owned();
    for e in &entries {
        let ListEntry {
            name,
            source,
            version,
            harness,
            path,
            valid,
            problem,
            shadowed_by,
            shadows,
        } = e;
        let mut shown_path = if path.starts_with("builtin:") {
            path.clone()
        } else {
            shown(ctx, path)
        };
        if let Some(by) = shadowed_by {
            shown_path.push_str(&format!(" (verschattet von {})", by.as_str()));
        }
        if let Some(p) = problem.as_ref().filter(|_| !valid) {
            shown_path.push_str(&format!(" ({p})"));
        }
        rows.push([
            name.clone(),
            if *valid {
                source.as_str().to_owned()
            } else {
                "ungültig".to_owned()
            },
            version.clone().unwrap_or_else(dash),
            harness.clone().unwrap_or_else(dash),
            shown_path,
        ]);
        if let Some(b) = shadows {
            let warning = format!("! {name} ({}) verschattet {b}", source.as_str());
            if all {
                rows.push([
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    warning,
                ]);
            } else {
                ctx.note(warning);
            }
        }
    }
    table(&rows)
}

fn validator<'a>(
    ctx: &Ctx,
    search: &'a SearchPath,
    harnesses: &'a HarnessCatalog,
) -> Validator<'a> {
    Validator {
        search,
        harnesses,
        skill_roots: skill_roots(ctx),
    }
}

fn print_diagnostics(diags: &[Diagnostic], out: &mut dyn std::io::Write) -> CliResult {
    for d in diags {
        let mark = match d.severity {
            Severity::Error => "✗",
            Severity::Warning => "!",
        };
        writeln!(out, "{mark} {d}").context("Ausgabe")?;
    }
    Ok(())
}

fn summary(report: &Report) -> String {
    let errors = report.error_count();
    let warnings = report.warning_count();
    let w = if warnings == 1 {
        "Warnung"
    } else {
        "Warnungen"
    };
    format!("{errors} Fehler, {warnings} {w} · Schema: beton agent schema")
}

/// Ein Fehler vor der eigentlichen Prüfung (Agent nicht gefunden) als Befund.
fn not_found(arg: &str, code: &'static str, message: String) -> Diagnostic {
    Diagnostic {
        file: arg.to_owned(),
        path: String::new(),
        line: 1,
        column: 1,
        code,
        message,
        severity: Severity::Error,
    }
}

fn validate(ctx: &Ctx, arg: &str) -> CliResult {
    let search = search_path(ctx);
    let harnesses = harness_catalog();
    // `validate <pfad>`: ein vorhandener Pfad gilt immer als Pfad, sonst als Agent-Ref.
    let reference = if ctx.cwd.join(arg).exists() || Path::new(arg).is_absolute() {
        Ok(AgentRef::Path(PathBuf::from(arg)))
    } else {
        AgentRef::parse(arg)
    };
    let located = reference
        .map_err(|e| not_found(arg, beton_agents::diag::code::INVALID_VALUE, e))
        .and_then(|r| {
            search.resolve(&r, &ctx.cwd).map_err(|e| {
                let code = match e {
                    beton_agents::ResolveError::NoAgentYaml(_) => {
                        beton_agents::diag::code::MISSING_AGENT_YAML
                    }
                    _ => beton_agents::diag::code::AGENT_NOT_FOUND,
                };
                not_found(arg, code, e.to_string())
            })
        });
    let located = match located {
        Ok(l) => l,
        Err(d) => {
            if ctx.global.json {
                print_json(&json!([d]))?;
            } else {
                print_diagnostics(std::slice::from_ref(&d), &mut std::io::stdout().lock())?;
            }
            return Err(CliError::new(Exit::General, anyhow::anyhow!("1 Fehler")));
        }
    };
    if let Some(w) = located.shadow_warning() {
        ctx.note(format!("Warnung: {w}"));
    }
    let report = validator(ctx, &search, &harnesses).validate(&located);
    if ctx.global.json {
        print_json(&serde_json::to_value(&report.diagnostics).context("Befunde")?)?;
    } else {
        let mut out = std::io::stdout().lock();
        print_diagnostics(&report.diagnostics, &mut out)?;
        if report.is_valid() {
            writeln!(out, "✓ {} ist gültig", located.name).context("stdout")?;
        }
    }
    if report.is_valid() {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!(summary(&report)),
        ))
    }
}

/// Einzeilig und höchstens 40 Zeichen.
fn render(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.lines().next().unwrap_or_default().to_owned(),
        other => other.to_string(),
    };
    if text.chars().count() > 40 {
        let mut short: String = text.chars().take(39).collect();
        short.push('…');
        short
    } else {
        text
    }
}

fn show(ctx: &Ctx, arg: &str) -> CliResult {
    let reference = AgentRef::parse(arg).map_err(CliError::usage)?;
    let search = search_path(ctx);
    let harnesses = harness_catalog();
    let located = search
        .resolve(&reference, &ctx.cwd)
        .map_err(|e| CliError::new(Exit::General, e))?;
    if let Some(w) = located.shadow_warning() {
        ctx.note(format!("Warnung: {w}"));
    }
    let report = validator(ctx, &search, &harnesses).validate(&located);
    let resolved = match ResolvedAgent::new(&located, &report) {
        Some(r) if report.is_valid() => r,
        _ => {
            print_diagnostics(&report.diagnostics, &mut std::io::stderr().lock())?;
            return Err(CliError::new(
                Exit::General,
                anyhow::anyhow!(
                    "{} ist ungültig ({}); Details: beton agent validate {arg}",
                    located.name,
                    summary(&report)
                ),
            ));
        }
    };
    if ctx.global.json {
        return print_json(&serde_json::to_value(&resolved).context("Agent")?);
    }
    print_diagnostics(&resolved.warnings, &mut std::io::stderr().lock())?;
    let mut out = std::io::stdout().lock();
    let path = if resolved.path.starts_with("builtin:") {
        resolved.path.clone()
    } else {
        shown(ctx, &resolved.path)
    };
    writeln!(
        out,
        "{} {} · {} · {} · {}",
        resolved.name,
        resolved.version.as_deref().unwrap_or("–"),
        resolved.source.as_str(),
        path,
        resolved.hash
    )
    .context("stdout")?;
    writeln!(out).context("stdout")?;
    let fields = resolved.fields();
    let width = fields
        .iter()
        .map(|(p, _, _)| p.chars().count())
        .max()
        .unwrap_or(0);
    let values: Vec<String> = fields.iter().map(|(_, v, _)| render(v)).collect();
    let vwidth = values.iter().map(|v| v.chars().count()).max().unwrap_or(0);
    for ((p, _, origin), value) in fields.iter().zip(&values) {
        writeln!(out, "{p:width$}  {value:vwidth$}  {origin}").context("stdout")?;
    }
    Ok(())
}

fn new(ctx: &Ctx, args: &AgentNewArgs) -> CliResult {
    let name: AgentName = args.name.parse().map_err(CliError::usage)?;
    let search = search_path(ctx);
    let from = match &args.from {
        Some(raw) => {
            let r = AgentRef::parse(raw).map_err(CliError::usage)?;
            let located = search
                .resolve(&r, &ctx.cwd)
                .map_err(|e| CliError::new(Exit::General, e))?;
            if let Some(w) = located.shadow_warning() {
                ctx.note(format!("Warnung: {w}"));
            }
            Some(located.dir)
        }
        None => None,
    };
    let created = scaffold::new_agent(&beton_dir(ctx), &name, from.as_ref())
        .map_err(|e| CliError::new(Exit::General, e))?;
    let located = search
        .resolve(&AgentRef::Path(created.dir.clone()), &ctx.cwd)
        .map_err(|e| CliError::new(Exit::General, e))?;
    let harnesses = harness_catalog();
    let report = validator(ctx, &search, &harnesses).validate(&located);
    let dir = shown(ctx, &created.dir.display().to_string());
    if ctx.global.json {
        print_json(&json!({
            "name": name.as_str(),
            "dir": created.dir.display().to_string(),
            "files": created.files,
            "schema": created.schema.display().to_string(),
            "valid": report.is_valid(),
            "diagnostics": report.diagnostics,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Angelegt in {dir}/").context("stdout")?;
        for f in &created.files {
            if f == beton_agents::dir::AGENT_YAML {
                writeln!(
                    out,
                    "  {f:<24}mit $schema-Kommentar für Editor-Vervollständigung"
                )
                .context("stdout")?;
            } else {
                writeln!(out, "  {f}").context("stdout")?;
            }
        }
        writeln!(
            out,
            "Schema: {}",
            shown(ctx, &created.schema.display().to_string())
        )
        .context("stdout")?;
        print_diagnostics(&report.diagnostics, &mut out)?;
        if report.is_valid() {
            writeln!(out, "✓ beton agent validate {dir}: gültig").context("stdout")?;
        }
    }
    if report.is_valid() {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!(summary(&report)),
        ))
    }
}
