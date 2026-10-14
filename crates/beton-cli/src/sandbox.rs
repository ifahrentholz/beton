//! `beton sandbox probe|explain` (SBX-001 AC1, SBX-003 AC5). Beide ändern nichts und brauchen
//! keinen Daemon; `exec` und `test` folgen mit SBX-010/SBX-011.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use beton_sandbox::{
    BackendChoice, GitLayout, Mask, NetMode, ProbeReport, Registry, ResolveCtx, ResolvedPolicy,
    Stage,
};
use serde_json::{Value, json};

use crate::cli::{SandboxCommand, SandboxExplainArgs, SandboxStageArg};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

pub fn run(ctx: &Ctx, cmd: SandboxCommand) -> CliResult {
    match cmd {
        SandboxCommand::Probe => probe(ctx),
        SandboxCommand::Explain(args) => explain(ctx, args),
    }
}

fn probe(ctx: &Ctx) -> CliResult {
    let registry = Registry::platform();
    let reports = registry.probe_all();
    let auto = beton_sandbox::auto_backend(registry.os()).map(|p| p.as_str());
    if ctx.global.json {
        return print_json(&json!({ "auto": auto, "providers": reports }));
    }
    let mut out = String::new();
    for r in &reports {
        line(&mut out, r, auto == Some(r.id.as_str()));
    }
    print!("{out}");
    Ok(())
}

fn line(out: &mut String, r: &ProbeReport, auto: bool) {
    let state = if r.available {
        "verfügbar"
    } else {
        "nicht verfügbar"
    };
    let mark = if auto { " (auto)" } else { "" };
    let _ = writeln!(out, "{}{mark}: {state}", r.id);
    if let Some(reason) = &r.unavailable_reason {
        let _ = writeln!(out, "  Grund: {reason}");
        return;
    }
    if !r.version.is_empty() {
        let _ = writeln!(out, "  Version: {}", r.version);
    }
    let caps: Vec<&str> = r.caps.iter().map(|c| c.as_str()).collect();
    let _ = writeln!(
        out,
        "  erzwingt: {}",
        if caps.is_empty() {
            "nichts".to_owned()
        } else {
            caps.join(", ")
        }
    );
    for (cap, reason) in &r.degraded_reasons {
        let _ = writeln!(out, "  ohne {cap}: {reason}");
    }
}

fn home_dir() -> CliResult<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| CliError::new(Exit::General, anyhow::anyhow!("HOME ist nicht gesetzt")))
}

fn sandbox_error(e: beton_sandbox::SandboxError) -> CliError {
    CliError::new(Exit::General, anyhow::anyhow!("{e}"))
}

fn explain(ctx: &Ctx, args: SandboxExplainArgs) -> CliResult {
    let settings = ctx
        .layers()?
        .settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let spec = settings.sandbox;
    let stage = match args.stage {
        SandboxStageArg::Harness => Stage::Harness,
        SandboxStageArg::Tools => Stage::Tools,
    };
    let registry = Registry::platform();
    let provider = registry
        .select(spec.backend.unwrap_or(BackendChoice::Auto))
        .map_err(sandbox_error)?;
    let scratch = tempfile::tempdir().context("Scratch anlegen")?;
    let workspace =
        std::fs::canonicalize(&ctx.cwd).with_context(|| ctx.cwd.display().to_string())?;
    let mut rctx = ResolveCtx::new(stage, &workspace, &home_dir()?, scratch.path());
    rctx.git = GitLayout::discover(&workspace).map_err(sandbox_error)?;
    rctx.adapters = beton_sandbox::masks::builtin_harness_grants();
    if let Some(h) = &args.harness {
        rctx.harness = Some(beton_sandbox::masks::harness_grants(h).ok_or_else(|| {
            CliError::new(
                Exit::Usage,
                anyhow::anyhow!("unbekannter Harness `{h}` (bekannt: claude, codex)"),
            )
        })?);
    }
    rctx.host_env = std::env::vars().collect::<BTreeMap<_, _>>();
    // Auflösen auch, wenn das Backend hier nicht isolieren kann: `explain` zeigt dann, was
    // verlangt wäre, und warum der Start scheitern würde.
    let policy = beton_sandbox::resolve(provider.id(), &spec, &rctx).map_err(sandbox_error)?;
    let probe = provider.probe();
    let check = beton_sandbox::check_caps(&probe, &policy).err();
    let mut v = beton_sandbox::explain::explain(&policy);
    if let Value::Object(map) = &mut v {
        map.insert("available".into(), json!(check.is_none()));
        if let Some(e) = &check {
            map.insert("unavailable".into(), json!(e.to_string()));
        }
        map.insert("scratch".into(), json!("<Scratch der Session>"));
    }
    if ctx.global.json {
        return print_json(&v);
    }
    print!("{}", render(&policy, &probe, check.as_ref(), &rctx.home));
    Ok(())
}

fn tilde(p: &Path, home: &Path) -> String {
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    match p.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn list(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        let _ = writeln!(out, "{title:<12} –");
        return;
    }
    for (i, item) in items.iter().enumerate() {
        let label = if i == 0 { title } else { "" };
        let _ = writeln!(out, "{label:<12} {item}");
    }
}

fn render(
    p: &ResolvedPolicy,
    probe: &ProbeReport,
    check: Option<&beton_sandbox::SandboxError>,
    home: &Path,
) -> String {
    let mut out = String::new();
    let state = match check {
        None => "verfügbar".to_owned(),
        Some(e) => format!("Start würde scheitern: {e}"),
    };
    let _ = writeln!(out, "{:<12} {} ({state})", "Backend", p.backend);
    if !probe.version.is_empty() {
        let _ = writeln!(out, "{:<12} {}", "Version", probe.version);
    }
    let _ = writeln!(out, "{:<12} {}", "Stufe", p.stage.as_str());
    let _ = writeln!(out, "{:<12} {}", "Workspace", tilde(&p.workspace, home));
    let t = |v: &[PathBuf]| v.iter().map(|r| tilde(r, home)).collect::<Vec<_>>();
    list(&mut out, "Lesen", &t(&p.read_roots));
    let mut write = t(&p.write_roots);
    for w in &mut write {
        if Path::new(w) == p.scratch {
            *w = "<Scratch der Session>".into();
        }
    }
    list(&mut out, "Schreiben", &write);
    let masks: Vec<String> = p
        .masks
        .iter()
        .map(|m| match m {
            Mask::Path { path, source } => format!("{} ({source})", tilde(path, home)),
            Mask::Pattern {
                root,
                glob,
                any_depth,
                source,
            } => {
                let depth = if *any_depth { "**/" } else { "" };
                format!("{}/{depth}{glob} ({source})", tilde(root, home))
            }
        })
        .collect();
    list(&mut out, "Masken", &masks);
    let hidden: Vec<String> = p
        .hidden
        .iter()
        .map(|h| {
            let allow = if h.allow.is_empty() {
                String::new()
            } else {
                format!(", außer {}", h.allow.join(", "))
            };
            format!("Dotfiles direkt in {}{allow}", tilde(&h.root, home))
        })
        .collect();
    list(&mut out, "Dotfiles", &hidden);
    if let Some(g) = &p.git {
        let _ = writeln!(
            out,
            "{:<12} Hooks und Konfiguration read-only in {}",
            "Git",
            tilde(&g.common_dir, home)
        );
    }
    let net = match p.net {
        NetMode::None => "kein Netz".to_owned(),
        NetMode::ProxyOnly { port } => format!("nur Proxy (localhost:{port})"),
    };
    let _ = writeln!(out, "{:<12} {net}", "Netz");
    list(&mut out, "Regeln", &p.egress_rules);
    let names: Vec<String> = p.env.keys().cloned().collect();
    let _ = writeln!(
        out,
        "{:<12} {} (nur Namen)",
        "Environment",
        names.join(", ")
    );
    if p.keychain {
        let _ = writeln!(out, "{:<12} ja (Stufe 1)", "Keychain");
    }
    list(&mut out, "Hinweise", &p.notes);
    out
}
