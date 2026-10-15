//! `beton policy test|eval|explain|show` (POL-026, POL-027).
//!
//! - `test` arbeitet nur auf Dateien, ohne Daemon: Testdateien finden, ausführen, Ergebnis mit
//!   Explain-Trace je Fehlschlag, optional JUnit-XML. Exit 0 bei Erfolg, 1 bei Fehlschlag.
//!   `harness:` in einer Testdatei wählt die Enforcement-Matrix aus den Harness-Capabilities.
//! - `eval` wertet eine Anfrage ohne Seiteneffekte aus: mit `--session` über den Daemon im
//!   Kontext der Session, sonst lokal mit User- und Projekt-Policies und Fixtures.
//! - `explain` erklärt eine gespeicherte Entscheidung (Daemon).
//! - `show` listet die geltenden Regeln mit Ebene und Herkunft.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event};
use beton_policy::enforcement::Matrix;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::explain::{Explanation, context_values, explain, render};
use beton_policy::load::Scope;
use beton_policy::policytest::{self, Summary, TestOptions};
use beton_policy::store::{PolicyStore, policies_dir};
use serde_json::{Value, json};

use crate::cli::{PolicyCommand, PolicyEvalArgs, PolicyShowArgs, PolicyTestArgs};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

pub async fn run(ctx: &Ctx, cmd: PolicyCommand) -> CliResult {
    match cmd {
        PolicyCommand::Test(args) => test(ctx, args),
        PolicyCommand::Eval(args) => eval(ctx, args).await,
        PolicyCommand::Explain(args) => {
            let client = ctx.client()?;
            let v = client.explain_decision(&args.session, args.seq).await?;
            if ctx.global.json {
                return print_json(&v);
            }
            if let Some(e) = v["snapshot_error"].as_str() {
                ctx.note(format!("Hinweis: {e}"));
            }
            write_out(v["explain"].as_str().unwrap_or_default())
        }
        PolicyCommand::Show(args) => show(ctx, args).await,
    }
}

fn write_out(text: &str) -> CliResult {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes()).context("stdout")?;
    if !text.ends_with('\n') {
        writeln!(out).context("stdout")?;
    }
    Ok(())
}

/// `<projekt>/.beton`: dasselbe Projekt wie für `.beton/config.yaml` (CLI-008).
fn beton_dir(ctx: &Ctx) -> PathBuf {
    let config = ctx.paths().project;
    config
        .parent()
        .map_or_else(|| ctx.cwd.join(".beton"), Path::to_path_buf)
}

/// Enforcement-Matrix eines Harness aus seinen Capabilities (POL-024), ohne Vendor-CLI.
pub fn matrix_for(harness: &str) -> Result<Matrix, String> {
    let id: beton_harness::HarnessId = harness
        .parse()
        .map_err(|e| format!("ungültiger Harness: {e}"))?;
    if id.as_str() == beton_harness::HarnessId::FAKE {
        return Ok(beton_harness::fake::default_capabilities().enforcement());
    }
    let registry = beton_runner::builtin_registry(
        &beton_harness::registry::HarnessLayers::default(),
        cfg!(debug_assertions),
    );
    let adapter = registry
        .get(&id)
        .ok_or_else(|| format!("unbekannter Harness `{harness}`"))?;
    Ok(adapter
        .capabilities(
            beton_harness::Mode::Native,
            &beton_harness::ProbeReport::default(),
        )
        .enforcement())
}

fn test(ctx: &Ctx, args: PolicyTestArgs) -> CliResult {
    let paths = if args.paths.is_empty() {
        vec![beton_dir(ctx).join("policies/tests")]
    } else {
        args.paths
            .iter()
            .map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    ctx.cwd.join(p)
                }
            })
            .collect()
    };
    let matrix = |h: &str| matrix_for(h);
    let opts = TestOptions {
        home: std::env::var_os("HOME").map(PathBuf::from),
        matrix: Some(&matrix),
    };
    let mut reports = policytest::run_paths(&paths, &opts).map_err(CliError::usage)?;
    // Pfade relativ zum Arbeitsverzeichnis, wie in Editor und CI üblich.
    for r in &mut reports {
        if let Ok(rel) = Path::new(&r.file).strip_prefix(&ctx.cwd) {
            r.file = rel.display().to_string();
        }
    }
    if reports.is_empty() {
        return Err(CliError::usage(
            "keine Policy-Tests (*.policytest.yaml) gefunden",
        ));
    }
    if let Some(file) = &args.junit {
        let file = if file.is_absolute() {
            file.clone()
        } else {
            ctx.cwd.join(file)
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("{} anlegen", dir.display()))?;
        }
        std::fs::write(&file, policytest::junit(&reports))
            .with_context(|| format!("{} schreiben", file.display()))?;
    }
    if ctx.global.json {
        print_json(&serde_json::to_value(&reports).context("Ergebnis")?)?;
    } else {
        write_out(&policytest::render(&reports))?;
    }
    let summary = Summary::of(&reports);
    if summary.exit_code() == 0 {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!(
                "{} Fälle fehlgeschlagen, {} Dateien mit Ladefehler",
                summary.failed,
                summary.errors
            ),
        ))
    }
}

fn read_json(ctx: &Ctx, path: &Path, what: &str) -> CliResult<Value> {
    let text = if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("stdin lesen")?;
        s
    } else {
        let p = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };
        std::fs::read_to_string(&p).with_context(|| format!("{} lesen", p.display()))?
    };
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| CliError::usage(format!("{what}: kein gültiges JSON: {e}")))?;
    if !v.is_object() {
        return Err(CliError::usage(format!("{what} muss ein JSON-Objekt sein")));
    }
    Ok(v)
}

/// Lokales effektives Set: User (`$BETON_HOME/policies`) und Projekt (`.beton/policies`).
fn local_set(ctx: &Ctx) -> std::sync::Arc<EffectiveSet> {
    let mut store = PolicyStore::new();
    store.add_dir(Scope::User, policies_dir(&ctx.home));
    store.add_dir(Scope::Project, policies_dir(&beton_dir(ctx)));
    for e in store.refresh() {
        if let beton_policy::store::StoreEvent::Invalid(i) = e {
            ctx.note(format!("Warnung: {}:{}: {}", i.file, i.line, i.error));
        }
    }
    store.effective()
}

async fn eval(ctx: &Ctx, args: PolicyEvalArgs) -> CliResult {
    let input = read_json(ctx, &args.input, "--input")?;
    let fixtures = match &args.fixtures {
        Some(f) => Some(read_json(ctx, f, "--fixtures")?),
        None => None,
    };
    let v = match &args.session {
        Some(session) => {
            let mut req = json!({ "phase": args.phase, "input": input, "session_id": session });
            if let Some(h) = &args.harness {
                req["harness"] = json!(h);
            }
            if let Some(f) = fixtures {
                req["fixtures"] = f;
            }
            ctx.client()?.evaluate_policy(&req).await?
        }
        None => {
            let phase: Phase = serde_json::from_value(json!(args.phase))
                .map_err(|_| CliError::usage(format!("Phase `{}` ist unbekannt", args.phase)))?;
            let harness = args
                .harness
                .clone()
                .unwrap_or_else(|| policytest::DEFAULT_HARNESS.to_owned());
            let matrix = matrix_for(&harness).map_err(CliError::usage)?;
            let set = local_set(ctx);
            let input = input.as_object().cloned().unwrap_or_default();
            let mut base = json!({ "session": { "worktree": ctx.cwd.display().to_string() } });
            if let Some(f) = &fixtures {
                merge(&mut base, f);
            }
            let act = policytest::activation(
                phase,
                &base,
                &input,
                Some(&harness),
                std::env::var("HOME").ok().as_deref(),
            )
            .map_err(CliError::usage)?;
            let decision = set.evaluate(
                &act,
                &EvalOptions {
                    capabilities: matrix.capabilities(phase),
                    ..EvalOptions::default()
                },
            );
            let kind = match act.field("tool", "kind") {
                Some(beton_policy::value::Value::String(k)) => Some(k.to_string()),
                _ => None,
            };
            let level = matrix.level(phase, kind.as_deref());
            let mut event = decision_event(&decision, DecisionMeta::new(level));
            event.decision_id.clear();
            let mut ex: Explanation = explain(&event, Some(&set));
            ex.context = context_values(&set, phase, &act);
            json!({
                "outcome": ex.outcome,
                "enforcement": level.as_str(),
                "decision": event,
                "explanation": ex,
                "explain": render(&ex),
            })
        }
    };
    if ctx.global.json {
        print_json(&v)
    } else {
        write_out(v["explain"].as_str().unwrap_or_default())
    }
}

/// Objekte rekursiv zusammenführen; `over` gewinnt.
fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn render_sets(v: &Value) -> String {
    let mut out = format!(
        "Policy-Set {}\n",
        v["policy_set_hash"].as_str().unwrap_or_default()
    );
    let sets = v["sets"].as_array().cloned().unwrap_or_default();
    if sets.is_empty() {
        out.push_str("Keine Policies geladen.\n");
    }
    for s in sets {
        let mode = if s["mode"] == "dry_run" {
            " [dry_run]"
        } else {
            ""
        };
        out.push_str(&format!(
            "\n{:<8} {}{mode}  {}\n",
            s["scope"].as_str().unwrap_or_default(),
            s["name"].as_str().unwrap_or_default(),
            s["origin"].as_str().unwrap_or_default()
        ));
        for r in s["rules"].as_array().into_iter().flatten() {
            let phases: Vec<&str> = r["phases"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let dry = if r["dry_run"] == true {
                " [dry_run]"
            } else {
                ""
            };
            let when = r["when"]
                .as_str()
                .map(|w| format!("  when: {w}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  - {} ({}) {}{dry}{when}\n",
                r["id"].as_str().unwrap_or_default(),
                phases.join(", "),
                r["action"].as_str().unwrap_or_default()
            ));
        }
    }
    for f in v["invalid"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\nUngültig (letzte gültige Fassung aktiv): {}\n",
            f.as_str().unwrap_or_default()
        ));
    }
    out
}

async fn show(ctx: &Ctx, args: PolicyShowArgs) -> CliResult {
    let _ = args.effective;
    let v = match &args.session {
        Some(id) => ctx.client()?.policies(Some(id)).await?,
        None => {
            let set = local_set(ctx);
            let event = beton_core::event::PolicyDecision {
                policy_set_hash: set.hash().to_owned(),
                ..beton_core::event::PolicyDecision::default()
            };
            json!({
                "policy_set_hash": set.hash(),
                "sets": explain(&event, Some(&set)).sets,
                "invalid": set.invalid(),
            })
        }
    };
    if ctx.global.json {
        print_json(&v)
    } else {
        write_out(&render_sets(&v))
    }
}
