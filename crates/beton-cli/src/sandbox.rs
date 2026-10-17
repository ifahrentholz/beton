//! `beton sandbox probe|explain|exec|test` (SBX-001 AC1, SBX-003 AC5, SBX-010). Keins braucht
//! einen Daemon; `probe`, `explain` und `test` ändern nichts, `exec` startet ein Kommando in
//! der Sandbox einer Stufe (ohne Netz). `test` führt die Selbsttests der Probe aus (ADR-0042).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use beton_sandbox::registry::Os;
use beton_sandbox::{
    BackendChoice, CommandSpec, GitLayout, Mask, NetMode, ProbeReport, ProviderId, Registry,
    ResolveCtx, ResolvedPolicy, SandboxProvider, Stage, Stdio,
};
use serde_json::{Value, json};

use crate::cli::{SandboxCommand, SandboxExecArgs, SandboxExplainArgs, SandboxStageArg};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

pub async fn run(ctx: &Ctx, cmd: SandboxCommand) -> CliResult {
    match cmd {
        SandboxCommand::Probe => probe(ctx),
        SandboxCommand::Explain(args) => explain(ctx, args),
        SandboxCommand::Exec(args) => exec(ctx, args).await,
        SandboxCommand::Test => self_test(ctx),
    }
}

/// Backends mit Selbsttest auf diesem Betriebssystem (die nativen Backends der Probe).
fn native_backends(os: Os) -> &'static [ProviderId] {
    match os {
        Os::MacOs => &[ProviderId::Seatbelt],
        Os::Linux => &[ProviderId::Landlock, ProviderId::Bwrap],
        Os::Windows => &[ProviderId::Windows],
        Os::Other => &[],
    }
}

/// Ein Fall von `beton sandbox test`: der Selbsttest eines Backends.
fn case(report: Option<&ProbeReport>, id: ProviderId, required: bool) -> Value {
    let (passed, reason) = match report {
        // `none` isoliert nichts; als Pflicht besteht es nie (SBX-006).
        _ if id == ProviderId::None => (false, Some("`none` isoliert nicht".to_owned())),
        Some(r) if r.available => (true, None),
        Some(r) => (
            false,
            Some(
                r.unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "nicht verfügbar".to_owned()),
            ),
        ),
        None => (false, Some("Backend ist nicht registriert".to_owned())),
    };
    json!({
        "id": format!("selftest:{}", id.as_str()),
        "backend": id.as_str(),
        "required": required,
        "passed": passed,
        "reason": reason,
    })
}

/// SBX-010 AC3: die Selbsttests der Probe als Fälle; Pflicht ist das Backend einer Session.
fn self_test(ctx: &Ctx) -> CliResult {
    let settings = ctx
        .layers()?
        .settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let (spec, _) = beton_sandbox::SandboxSpec::merged_with_origins(&[("user", &settings.sandbox)]);
    let registry = Registry::platform();
    let required = match spec.backend.unwrap_or(BackendChoice::Auto) {
        BackendChoice::Auto => registry.auto_choice(),
        BackendChoice::Provider(id) => Some(id),
    };
    let reports = registry.probe_all();
    let report = |id: ProviderId| reports.iter().find(|r| r.id == id);
    let mut cases: Vec<Value> = native_backends(registry.os())
        .iter()
        .map(|&id| case(report(id), id, Some(id) == required))
        .collect();
    if let Some(id) = required
        && !native_backends(registry.os()).contains(&id)
    {
        cases.push(case(report(id), id, true));
    }
    let failed: Vec<String> = cases
        .iter()
        .filter(|c| c["required"] == true && c["passed"] != true)
        .map(|c| {
            format!(
                "{} ({})",
                c["id"].as_str().unwrap_or_default(),
                c["reason"].as_str().unwrap_or_default()
            )
        })
        .collect();
    let passed = required.is_some() && failed.is_empty();
    if ctx.global.json {
        print_json(&json!({
            "passed": passed,
            "backend": required.map(|id| id.as_str()),
            "cases": cases,
        }))?;
    } else {
        let mut out = String::from("Selbsttests der Sandbox-Backends (Probe):\n");
        for c in &cases {
            let mark = if c["required"] == true {
                " (Pflicht)"
            } else {
                ""
            };
            let state = if c["passed"] == true {
                "bestanden".to_owned()
            } else {
                format!(
                    "nicht bestanden: {}",
                    c["reason"].as_str().unwrap_or_default()
                )
            };
            let _ = writeln!(
                out,
                "  {}{mark}: {state}",
                c["id"].as_str().unwrap_or_default()
            );
        }
        let _ = writeln!(
            out,
            "Ergebnis: {}",
            if passed {
                "bestanden"
            } else {
                "nicht bestanden"
            }
        );
        print!("{out}");
    }
    if passed {
        return Ok(());
    }
    let why = if required.is_none() {
        "für dieses Betriebssystem gibt es kein Sandbox-Backend".to_owned()
    } else {
        failed.join(", ")
    };
    Err(CliError::new(
        Exit::General,
        anyhow::anyhow!("Pflichtfall nicht bestanden: {why}"),
    ))
}

fn probe(ctx: &Ctx) -> CliResult {
    let registry = Registry::platform();
    let reports = registry.probe_all();
    let auto = registry.auto_choice().map(|p| p.as_str());
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
    kernel_line(out, r);
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

/// Kernel-Angaben der Linux-Backends (SBX-010 AC1), auch für nicht verfügbare.
fn kernel_line(out: &mut String, r: &ProbeReport) {
    if let Some(k) = &r.kernel {
        let abi = k
            .landlock_abi
            .map_or_else(|| "fehlt".to_owned(), |a| a.to_string());
        let _ = writeln!(out, "  Kernel: {}, Landlock-ABI: {abi}", k.kernel);
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

/// Spezifikation, Provider und Auflösungskontext einer Stufe für das aktuelle Verzeichnis.
struct Resolved {
    spec: beton_sandbox::SandboxSpec,
    origins: BTreeMap<String, String>,
    provider: Arc<dyn SandboxProvider>,
    rctx: ResolveCtx,
    /// Scratch der Stufe; lebt, solange die Auflösung gebraucht wird.
    _scratch: tempfile::TempDir,
}

fn resolve_ctx(ctx: &Ctx, stage: SandboxStageArg, harness: Option<&str>) -> CliResult<Resolved> {
    let settings = ctx
        .layers()?
        .settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    // Presets und Herkunft je Feld (SBX-012 AC3).
    let (spec, origins) =
        beton_sandbox::SandboxSpec::merged_with_origins(&[("user", &settings.sandbox)]);
    let stage = match stage {
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
    if let Some(h) = harness {
        rctx.harness = Some(beton_sandbox::masks::harness_grants(h).ok_or_else(|| {
            CliError::new(
                Exit::Usage,
                anyhow::anyhow!("unbekannter Harness `{h}` (bekannt: claude, codex)"),
            )
        })?);
    }
    rctx.host_env = std::env::vars().collect::<BTreeMap<_, _>>();
    Ok(Resolved {
        spec,
        origins,
        provider,
        rctx,
        _scratch: scratch,
    })
}

fn explain(ctx: &Ctx, args: SandboxExplainArgs) -> CliResult {
    let Resolved {
        spec,
        origins,
        provider,
        rctx,
        _scratch,
    } = resolve_ctx(ctx, args.stage, args.harness.as_deref())?;
    // Auflösen auch, wenn das Backend hier nicht isolieren kann: `explain` zeigt dann, was
    // verlangt wäre, und warum der Start scheitern würde.
    let policy = beton_sandbox::resolve(provider.id(), &spec, &rctx).map_err(sandbox_error)?;
    let probe = provider.probe();
    let check = beton_sandbox::check_caps(&probe, &policy).err();
    let mut v = beton_sandbox::explain::with_origins(&policy, &origins);
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
    let mut text = render(&policy, &probe, check.as_ref(), &rctx.home);
    let from: Vec<String> = origins
        .iter()
        .map(|(field, origin)| format!("{field}: {origin}"))
        .collect();
    list(&mut text, "Herkunft", &from);
    print!("{text}");
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

/// Programm wie die Shell finden: mit `/` relativ zum Arbeitsverzeichnis, sonst über `PATH`
/// der Sandbox (nicht des Hosts).
fn program(name: &str, cwd: &Path, policy: &ResolvedPolicy) -> CliResult<PathBuf> {
    if name.contains('/') {
        return Ok(cwd.join(name));
    }
    let path = policy
        .env
        .get("PATH")
        .map(|v| v.as_str())
        .unwrap_or_default();
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            CliError::new(
                Exit::Usage,
                anyhow::anyhow!("`{name}` nicht im PATH der Sandbox gefunden ({path})"),
            )
        })
}

/// SBX-010 AC2: ein Kommando von Hand in der Sandbox einer Stufe, ohne Netz. Kann das Backend
/// die Sandbox nicht vollständig erzwingen (auch `backend: none` oder *degraded*), startet
/// nichts (fail closed).
async fn exec(ctx: &Ctx, args: SandboxExecArgs) -> CliResult {
    let Resolved {
        spec,
        provider,
        rctx,
        _scratch,
        ..
    } = resolve_ctx(ctx, args.stage, args.harness.as_deref())?;
    let stage = rctx.stage.as_str();
    let policy = provider.resolve(&spec, &rctx).map_err(|e| {
        CliError::new(
            Exit::General,
            anyhow::anyhow!(
                "Sandbox {} (Stufe {stage}) ist nicht nutzbar, nichts gestartet: {e}; Details: \
                 `beton sandbox explain --stage {stage}`",
                provider.id()
            ),
        )
    })?;
    // Fail closed: `exec` ist zum Prüfen der Sandbox da; ohne volle Isolation (`backend:
    // none` oder ausdrücklich nicht erzwungene Capabilities) startet nichts.
    if policy.backend == beton_sandbox::ProviderId::None || !policy.degraded.is_empty() {
        let degraded: Vec<&str> = policy.degraded.iter().map(|c| c.as_str()).collect();
        return Err(CliError::new(
            Exit::General,
            anyhow::anyhow!(
                "Sandbox {} (Stufe {stage}) isoliert nicht vollständig (nicht erzwungen: {}), \
                 nichts gestartet; Details: `beton sandbox explain --stage {stage}`",
                policy.backend,
                degraded.join(", ")
            ),
        ));
    }
    let (name, rest) = args
        .command
        .split_first()
        .ok_or_else(|| CliError::usage("Kommando fehlt (nach `--`)"))?;
    let cmd = CommandSpec::new(program(name, &policy.workspace, &policy)?)
        .args(rest.iter().cloned())
        .cwd(&policy.workspace)
        .stdio(Stdio::Inherit, Stdio::Inherit, Stdio::Inherit);
    let mut child = provider.spawn(&policy, cmd).map_err(sandbox_error)?;
    let status = child.wait().await.context("auf das Kommando warten")?;
    if status.success() {
        return Ok(());
    }
    Err(CliError::new(
        Exit::General,
        anyhow::anyhow!(
            "`{name}` scheiterte in der Sandbox {} (Stufe {stage}, {status}); was die Sandbox \
             erlaubt, zeigt `beton sandbox explain --stage {stage}`",
            provider.id()
        ),
    ))
}
