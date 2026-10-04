//! `beton run`, `resume` und `attach` (CLI-002, CLI-003, API-006).

use std::io::Read as _;

use anyhow::Context as _;
use beton_sdk::Client;
use serde_json::{Value, json};

use crate::cli::{AttachArgs, ResumeArgs, RunArgs};
use crate::commands::Ctx;
use crate::drive::{Mode, Options, drive};
use crate::exit::{CliError, CliResult, Exit};
use crate::sessionref;

/// Harness für `run` ohne TARGET: `harnesses.default`, sonst `claude`.
fn default_target(ctx: &Ctx) -> CliResult<String> {
    let settings = ctx
        .layers()?
        .settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    Ok(settings
        .harnesses
        .default
        .unwrap_or_else(|| "claude".to_owned()))
}

/// Ist TARGET ein Harness (sonst ein Agent-Ref)? Harness-IDs haben Vorrang: `claude`,
/// `codex`, `fake`, `acp:<slug>`, `direct:<provider>`.
pub fn is_harness(target: &str) -> bool {
    matches!(target, "claude" | "codex" | "fake")
        || target.starts_with("acp:")
        || target.starts_with("direct:")
}

/// `--param k=v` als Objekt (Werte bleiben Text; der Server wandelt typgerecht um, AGT-010).
fn params(raw: &[String]) -> CliResult<serde_json::Map<String, Value>> {
    let mut out = serde_json::Map::new();
    for p in raw {
        let (k, v) = beton_agents::params::parse_assignment(p).map_err(CliError::usage)?;
        out.insert(k, v);
    }
    Ok(out)
}

/// Fehler eines Agent-Starts als Usage-Fehler (Exit-Code 2): Agent fehlt oder ist ungültig,
/// Parameter fehlen oder passen nicht (AGT-010 AC1, AC2).
fn agent_error(e: beton_sdk::Error) -> CliError {
    match &e {
        beton_sdk::Error::Problem { code, .. }
            if matches!(
                code.as_str(),
                "invalid_param"
                    | "params_required"
                    | "agent_not_found"
                    | "agent_invalid"
                    | "harness_incompatible"
            ) =>
        {
            CliError::new(Exit::Usage, e)
        }
        _ => e.into(),
    }
}

/// Startet eine gestoppte oder fehlgeschlagene Session neu (SES-003); sonst nichts.
async fn ensure_live(client: &Client, id: &str) -> CliResult<Value> {
    let session = client.session(id).await?;
    if matches!(session["status"].as_str(), Some("stopped" | "failed")) {
        return Ok(client.resume(id).await?);
    }
    Ok(session)
}

pub async fn run(ctx: &Ctx, args: RunArgs) -> CliResult {
    let prompt = match args.prompt.as_deref() {
        Some("-") => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("Prompt von stdin lesen")?;
            Some(text.trim_end().to_owned())
        }
        Some(p) => Some(p.to_owned()),
        None => None,
    };
    if prompt.as_deref().is_some_and(str::is_empty) {
        return Err(CliError::usage("Leerer Prompt"));
    }
    let cwd = match &args.cwd {
        Some(dir) => dir
            .canonicalize()
            .with_context(|| format!("Arbeitsverzeichnis {}", dir.display()))?,
        None => ctx.cwd.canonicalize().unwrap_or_else(|_| ctx.cwd.clone()),
    };
    let cwd_text = cwd.display().to_string();
    let client = crate::daemon::ensure_running(ctx).await?;

    let continued = if args.continue_last {
        Some(
            sessionref::last_in_dir(&client, &cwd_text)
                .await?
                .ok_or_else(|| {
                    CliError::new(
                        Exit::General,
                        anyhow::anyhow!("Keine Session in {cwd_text} zum Fortsetzen"),
                    )
                })?,
        )
    } else if let Some(r) = &args.resume {
        Some(sessionref::resolve(&client, r).await?)
    } else if let Some(f) = &args.fork {
        Some(fork(ctx, &client, &args, f).await?)
    } else {
        None
    };

    let (id, from_seq) = match continued {
        Some(id) => {
            let session = ensure_live(&client, &id).await?;
            let head = session["head_seq"].as_u64().unwrap_or(0);
            // Im Skript-Modus nur Neues; interaktiv mit Verlauf.
            (id, if prompt.is_some() { head } else { 0 })
        }
        None => {
            let target = match &args.target {
                Some(t) => t.clone(),
                None => default_target(ctx)?,
            };
            // Agent als TARGET (CLI-002, AGT-004): Harness und Modell aus `executor`.
            let agent = (!is_harness(&target)).then(|| target.clone());
            if agent.is_none() && !args.params.is_empty() {
                return Err(CliError::usage("--param gilt nur für Agents"));
            }
            if agent.is_none() && args.harness.as_ref().is_some_and(|h| *h != target) {
                return Err(CliError::usage(format!(
                    "--harness {} widerspricht TARGET {target}",
                    args.harness.as_deref().unwrap_or_default()
                )));
            }
            if let Some(reference) = &agent {
                // Verschattung eines Built-ins melden (AGT-003 AC1); der Server löst denselben
                // Suchpfad auf.
                if let Ok(r) = beton_agents::AgentRef::parse(reference)
                    && let Ok(found) = crate::agent::search_path(ctx).resolve(&r, &cwd)
                    && let Some(warning) = found.shadow_warning()
                {
                    eprintln!("Warnung: {warning}");
                }
            }
            if args.scenario.is_some() && agent.is_none() && target != "fake" {
                return Err(CliError::usage(
                    "--scenario gilt nur für den Harness `fake`",
                ));
            }
            let mut opts = json!({});
            if let Some(scenario) = &args.scenario {
                let path = scenario
                    .canonicalize()
                    .with_context(|| format!("Szenario {}", scenario.display()))?;
                opts["scenario"] = Value::String(path.display().to_string());
            }
            let mut body = match &agent {
                Some(reference) => {
                    let mut b =
                        json!({ "agent": reference, "cwd": cwd_text, "harness_opts": opts });
                    if let Some(h) = &args.harness {
                        b["target"] = Value::String(h.clone());
                    }
                    let params = params(&args.params)?;
                    if !params.is_empty() {
                        b["params"] = Value::Object(params);
                    }
                    b
                }
                None => json!({ "target": target, "cwd": cwd_text, "harness_opts": opts }),
            };
            if let Some(t) = &args.title {
                body["title"] = Value::String(t.clone());
            }
            if let Some(m) = &args.model {
                body["model"] = Value::String(m.clone());
            }
            // SES-015: eigener Worktree, optional mit Branch-Name und Base.
            if let Some(branch) = &args.worktree {
                let mut wt = json!({});
                if !branch.is_empty() {
                    wt["branch"] = Value::String(branch.clone());
                }
                if let Some(base) = &args.base {
                    wt["base"] = Value::String(base.clone());
                }
                body["worktree"] = wt;
            }
            let created = client.create_session(&body).await.map_err(|e| {
                if agent.is_some() {
                    agent_error(e)
                } else {
                    e.into()
                }
            })?;
            if let Some(wt) = created.get("worktree").filter(|w| w.is_object()) {
                ctx.note(format!(
                    "Worktree {} (Branch {}, Base {})",
                    wt["path"].as_str().unwrap_or_default(),
                    wt["branch"].as_str().unwrap_or_default(),
                    wt["base"].as_str().unwrap_or_default()
                ));
            }
            let id = created["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Antwort ohne Session-ID"))?
                .to_owned();
            (id, 0)
        }
    };

    if args.detach {
        if ctx.global.json {
            crate::commands::print_json(
                &json!({ "session_id": id, "url": client.session_url(&id) }),
            )?;
        } else {
            println!("{id}");
        }
        return Ok(());
    }
    ctx.note(format!("Session {id} · {}", client.session_url(&id)));
    let mode = match prompt {
        Some(prompt) => Mode::Script {
            prompt,
            format: args.output_format,
            timeout: args.timeout,
        },
        None => Mode::Interactive { read_only: false },
    };
    drive(
        ctx,
        &client,
        &id,
        Options {
            mode,
            on_ask: args.on_ask,
            from_seq,
            quiet: ctx.global.quiet,
        },
    )
    .await
}

/// `--fork ID[@SEQ]` (SES-006, SES-007): Fork über die API anlegen; liefert die neue ID.
async fn fork(ctx: &Ctx, client: &Client, args: &RunArgs, reference: &str) -> CliResult<String> {
    let (source, at_seq) = match reference.rsplit_once('@') {
        Some((id, seq)) => (
            id,
            Some(seq.parse::<u64>().map_err(|_| {
                CliError::usage(format!("`{seq}` ist keine Ereignis-Nummer (ID@SEQ)"))
            })?),
        ),
        None => (reference, None),
    };
    let source = sessionref::resolve(client, source).await?;
    let harness = match (&args.harness, &args.target) {
        (Some(h), Some(t)) if h != t => {
            return Err(CliError::usage(format!(
                "--harness {h} widerspricht TARGET {t}"
            )));
        }
        (Some(h), _) | (None, Some(h)) => Some(h.clone()),
        (None, None) => None,
    };
    let mut body = json!({});
    if let Some(seq) = at_seq {
        body["at_seq"] = json!(seq);
    }
    if let Some(h) = &harness {
        body["harness"] = Value::String(h.clone());
    }
    if let Some(m) = &args.model {
        body["model"] = Value::String(m.clone());
    }
    if let Some(t) = &args.title {
        body["title"] = Value::String(t.clone());
    }
    if let Some(w) = args.workspace {
        body["workspace"] = Value::String(w.as_str().into());
    }
    if let Some(scenario) = &args.scenario {
        let path = scenario
            .canonicalize()
            .with_context(|| format!("Szenario {}", scenario.display()))?;
        body["harness_opts"] = json!({ "scenario": path.display().to_string() });
    }
    let forked = client.fork_session(&source, &body).await?;
    let effective = forked["effective_seq"].as_u64().unwrap_or_default();
    if let Some(at) = at_seq.filter(|at| *at != effective) {
        ctx.note(format!(
            "Ereignis {at} liegt mitten in einem Turn, der Fork beginnt am Ende des \
             vorherigen Turns ({effective})"
        ));
    }
    let session = &forked["session"];
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Antwort ohne Session-ID"))?
        .to_owned();
    let mut line = format!(
        "Fork {id} von {source}@{effective} · {}",
        beton_harness::handover::harness_label(session["harness"].as_str().unwrap_or_default())
    );
    if let Some(branch) = session["worktree"]["branch"].as_str() {
        line.push_str(&format!(" · Worktree {branch}"));
    }
    ctx.note(line);
    Ok(id)
}

pub async fn resume(ctx: &Ctx, args: ResumeArgs) -> CliResult {
    let client = crate::daemon::ensure_running(ctx).await?;
    let id = sessionref::resolve(&client, &args.session).await?;
    ensure_live(&client, &id).await?;
    ctx.note(format!("Session {id} · {}", client.session_url(&id)));
    drive(
        ctx,
        &client,
        &id,
        Options {
            mode: Mode::Interactive { read_only: false },
            on_ask: crate::cli::OnAsk::Wait,
            from_seq: 0,
            quiet: ctx.global.quiet,
        },
    )
    .await
}

pub async fn attach(ctx: &Ctx, args: AttachArgs) -> CliResult {
    let client = ctx.client()?;
    let id = sessionref::resolve(&client, &args.session).await?;
    ctx.note(format!("Angehängt an {id} · {}", client.session_url(&id)));
    drive(
        ctx,
        &client,
        &id,
        Options {
            mode: Mode::Interactive {
                read_only: args.read_only,
            },
            on_ask: crate::cli::OnAsk::Wait,
            from_seq: 0,
            quiet: ctx.global.quiet,
        },
    )
    .await
}
