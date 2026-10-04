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
            // Einstellungen einer fortgesetzten Session per PATCH (HAR-017, HAR-027); beim
            // Fork gehen sie in den Fork-Request.
            let settings = settings_body(&args);
            if args.fork.is_none() && settings.as_object().is_some_and(|o| !o.is_empty()) {
                client.patch_session(&id, &settings).await?;
            }
            let head = session["head_seq"].as_u64().unwrap_or(0);
            // Im Skript-Modus nur Neues; interaktiv mit Verlauf.
            (id, if prompt.is_some() { head } else { 0 })
        }
        None => {
            let target = match &args.target {
                Some(t) => t.clone(),
                None => default_target(ctx)?,
            };
            if args.scenario.is_some() && target != "fake" {
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
            let mut body = json!({ "target": target, "cwd": cwd_text, "harness_opts": opts });
            if let Some(t) = &args.title {
                body["title"] = Value::String(t.clone());
            }
            merge(&mut body, settings_body(&args));
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
            let created = client.create_session(&body).await?;
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

/// `--model`, `--effort` und `--permission-mode` als Felder für Anlegen, Fork bzw. PATCH.
fn settings_body(args: &RunArgs) -> Value {
    let mut body = json!({});
    if let Some(m) = &args.model {
        body["model"] = Value::String(m.clone());
    }
    if let Some(e) = args.effort {
        body["effort"] = Value::String(e.as_str().into());
    }
    if let Some(p) = args.permission_mode {
        body["permission_mode"] = Value::String(p.as_str().into());
    }
    body
}

fn merge(body: &mut Value, extra: Value) {
    if let (Some(b), Value::Object(e)) = (body.as_object_mut(), extra) {
        b.extend(e);
    }
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
    merge(&mut body, settings_body(args));
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
