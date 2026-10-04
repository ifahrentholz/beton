//! Eine Session im Terminal begleiten: `run`, `resume`, `attach` (CLI-002, CLI-003, API-006).
//!
//! Der Treiber hängt sich per WebSocket an (Replay, dann live), stellt Events dar, schickt
//! Eingaben, beantwortet Freigaben und reagiert auf `Strg+C`: während eines Turns unterbricht
//! es ihn, ein zweites `Strg+C` binnen 1 s (oder eines ohne laufenden Turn) koppelt ab, ohne
//! die Session zu stoppen.

use std::collections::VecDeque;
use std::io::IsTerminal as _;
use std::time::{Duration, Instant};

use beton_core::id::SessionId;
use beton_sdk::Client;
use beton_sdk::ws::ServerMsg;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt as _;
use tokio::sync::mpsc;

use crate::cli::{OnAsk, OutputFormat};
use crate::exit::{CliError, CliResult, Exit};
use crate::render::{Renderer, Style, approval_summary};

/// Abstand, in dem ein zweites `Strg+C` abkoppelt.
pub const DOUBLE_CTRL_C: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub enum Mode {
    /// Eingaben von stdin, bis stdin endet oder abgekoppelt wird.
    Interactive { read_only: bool },
    /// Genau ein Prompt; endet mit dem Turn.
    Script {
        prompt: String,
        format: OutputFormat,
        timeout: Option<Duration>,
    },
}

#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub on_ask: OnAsk,
    /// Replay ab dieser `seq` (exklusiv).
    pub from_seq: u64,
    pub quiet: bool,
}

/// Wie der Lauf endete (für Exit-Code und JSON-Ergebnis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Completed,
    Failed,
    Interrupted,
    Denied,
    Detached,
    TimedOut,
}

struct State {
    id: String,
    status: String,
    live: bool,
    /// Eingabe gesendet bzw. Turn läuft.
    busy: bool,
    /// Turn dieses Skript-Laufs.
    our_turn: Option<String>,
    queue: VecDeque<String>,
    pending: VecDeque<(String, String)>,
    asked: bool,
    denied_by_on_ask: bool,
    stdin_open: bool,
    last_ctrl_c: Option<Instant>,
    cost_micro: i64,
    usage: Value,
    outcome: Option<Outcome>,
}

/// `Strg+C` als Strom mit Ankunftszeit. Ein dauerhafter Listener verliert keine Signale,
/// die eintreffen, während der Treiber gerade etwas anderes erledigt.
fn interrupts() -> mpsc::Receiver<Instant> {
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let Ok(mut sigint) = signal(SignalKind::interrupt()) else {
                return;
            };
            while sigint.recv().await.is_some() {
                if tx.send(Instant::now()).await.is_err() {
                    return;
                }
            }
        }
        #[cfg(not(unix))]
        {
            while tokio::signal::ctrl_c().await.is_ok() {
                if tx.send(Instant::now()).await.is_err() {
                    return;
                }
            }
        }
    });
    rx
}

fn stdin_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).await.is_err() {
                break;
            }
        }
    });
    rx
}

pub async fn drive(
    ctx: &crate::commands::Ctx,
    client: &Client,
    id: &str,
    opts: Options,
) -> CliResult {
    let sid: SessionId = id
        .parse()
        .map_err(|e| CliError::usage(format!("ungültige Session-ID `{id}`: {e}")))?;
    let script = matches!(opts.mode, Mode::Script { .. });
    let read_only = matches!(opts.mode, Mode::Interactive { read_only: true });
    let style = match &opts.mode {
        Mode::Interactive { .. } => Style::Interactive,
        Mode::Script { format, .. } => Style::Script(*format),
    };
    let mut renderer = Renderer::new(style, !opts.quiet);
    let tty = std::io::stdin().is_terminal();
    let started = Instant::now();
    let deadline = match &opts.mode {
        Mode::Script {
            timeout: Some(t), ..
        } => Some(started + *t),
        _ => None,
    };

    let mut conn = client.connect_ws().await?;
    conn.attach(sid, opts.from_seq).await?;
    let mut st = State {
        id: id.to_owned(),
        status: client.session(id).await?["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        live: false,
        busy: false,
        our_turn: None,
        queue: VecDeque::new(),
        pending: VecDeque::new(),
        asked: false,
        denied_by_on_ask: false,
        stdin_open: !script && !read_only,
        last_ctrl_c: None,
        cost_micro: 0,
        usage: Value::Null,
        outcome: None,
    };
    if let Mode::Script { prompt, .. } = &opts.mode {
        st.queue.push_back(prompt.clone());
    }
    let mut lines = st.stdin_open.then(stdin_lines);
    let mut ctrl_c = interrupts();

    loop {
        let timeout = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(d.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            msg = conn.next() => {
                let Some(msg) = msg else {
                    renderer.finish();
                    return Err(CliError::new(Exit::Unreachable, anyhow::anyhow!("Verbindung zum Daemon getrennt")));
                };
                match msg? {
                    ServerMsg::Events { events, .. } => {
                        for e in events {
                            let e = serde_json::to_value(&e).unwrap_or(Value::Null);
                            renderer.event(&e);
                            on_event(&mut st, &e, script);
                        }
                    }
                    ServerMsg::Live { .. } => {
                        st.live = true;
                        if !script && !read_only && tty && st.status == "idle" {
                            eprintln!("Eingabe senden mit Enter; Strg+C koppelt ab.");
                        }
                    }
                    ServerMsg::Overflow { resume_from, .. } => {
                        conn.attach(sid, resume_from).await?;
                    }
                    ServerMsg::Nack { problem, .. } => {
                        renderer.finish();
                        eprintln!("Fehler: {}", problem["detail"].as_str().or_else(|| problem["title"].as_str()).unwrap_or("unbekannt"));
                    }
                    _ => {}
                }
            }
            line = async { match lines.as_mut() { Some(l) => l.recv().await, None => std::future::pending().await } }, if lines.is_some() => {
                match line {
                    Some(line) => {
                        if let Some((approval, _)) = st.pending.front().cloned() && st.asked {
                            let allow = matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "j" | "ja");
                            st.pending.pop_front();
                            st.asked = false;
                            client.resolve_approval(&st.id, &approval, allow, (!allow).then_some("im Terminal abgelehnt")).await?;
                        } else if !line.trim().is_empty() && !read_only {
                            st.queue.push_back(line);
                        }
                    }
                    None => {
                        lines = None;
                        st.stdin_open = false;
                    }
                }
            }
            Some(now) = ctrl_c.recv() => {
                let double = st.last_ctrl_c.is_some_and(|t| now.duration_since(t) < DOUBLE_CTRL_C);
                st.last_ctrl_c = Some(now);
                if st.busy && !double {
                    renderer.finish();
                    eprintln!("Unterbreche … (nochmal Strg+C koppelt ab)");
                    client.interrupt(&st.id).await?;
                } else {
                    renderer.finish();
                    eprintln!("Abgekoppelt; die Session läuft weiter: beton attach {}", st.id);
                    st.outcome = Some(Outcome::Detached);
                }
            }
            () = timeout => {
                renderer.finish();
                eprintln!("Zeitlimit erreicht; Turn wird abgebrochen.");
                let _ = client.interrupt(&st.id).await;
                st.outcome = Some(Outcome::TimedOut);
            }
        }

        if st.outcome.is_some() {
            break;
        }
        if !st.live {
            continue;
        }
        // Freigaben beantworten.
        if !st.asked
            && let Some((approval, summary)) = st.pending.front().cloned()
        {
            if read_only {
                // Zuschauer entscheiden nicht.
            } else if opts.on_ask == OnAsk::Deny && (script || !tty) {
                st.pending.pop_front();
                st.denied_by_on_ask = true;
                renderer.finish();
                if !opts.quiet {
                    eprintln!("Freigabe abgelehnt (--on-ask deny): {summary}");
                }
                client
                    .resolve_approval(&st.id, &approval, false, Some("--on-ask deny"))
                    .await?;
            } else if tty && !script {
                renderer.ask(&format!("Freigabe für {summary}? [y/N]"));
                st.asked = true;
            } else if !st.asked {
                renderer.finish();
                if !opts.quiet {
                    eprintln!("Wartet auf Freigabe (Web-UI): {summary}");
                }
                st.asked = true;
            }
        }
        // Nächste Eingabe senden, sobald die Session bereit ist.
        if !st.busy
            && st.status == "idle"
            && let Some(text) = st.queue.pop_front()
        {
            let accepted = client.input(&st.id, &text).await?;
            st.busy = true;
            st.our_turn = accepted["turn_id"].as_str().map(str::to_owned);
        }
        if matches!(st.status.as_str(), "stopped" | "failed") && !st.busy && script {
            st.outcome = Some(Outcome::Failed);
        }
        // Zuschauer (`--read-only`) bleiben, bis Strg+C oder die Verbindung endet.
        if !script
            && !read_only
            && !st.stdin_open
            && !st.busy
            && st.queue.is_empty()
            && st.pending.is_empty()
        {
            st.outcome = Some(Outcome::Detached);
        }
        if st.outcome.is_some() {
            break;
        }
    }
    renderer.finish();
    conn.close().await;

    let outcome = st.outcome.unwrap_or(Outcome::Detached);
    if let Mode::Script { format, .. } = &opts.mode {
        let status = match outcome {
            Outcome::Completed if st.denied_by_on_ask => "denied",
            Outcome::Completed => "completed",
            Outcome::Failed => "failed",
            Outcome::Interrupted => "interrupted",
            Outcome::Denied => "denied",
            Outcome::Detached => "detached",
            Outcome::TimedOut => "timeout",
        };
        match format {
            OutputFormat::Text => {
                if !renderer.last_answer.is_empty() {
                    println!("{}", renderer.last_answer);
                }
            }
            OutputFormat::Json => crate::commands::print_json(&json!({
                "session_id": st.id,
                "status": status,
                "result": renderer.last_answer,
                "cost_usd": st.cost_micro as f64 / 1_000_000.0,
                "usage": st.usage,
                "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }))?,
            OutputFormat::StreamJson => {}
        }
        let _ = ctx;
        return match outcome {
            Outcome::Completed if st.denied_by_on_ask => Err(CliError::new(
                Exit::ApprovalDenied,
                anyhow::anyhow!("Freigabe abgelehnt (--on-ask deny)"),
            )),
            Outcome::Completed | Outcome::Detached => Ok(()),
            Outcome::Denied => Err(CliError::new(
                Exit::ApprovalDenied,
                anyhow::anyhow!("Freigabe abgelehnt"),
            )),
            Outcome::Failed => Err(CliError::new(
                Exit::Harness,
                anyhow::anyhow!("Turn fehlgeschlagen"),
            )),
            Outcome::Interrupted => Err(CliError::new(
                Exit::Interrupted,
                anyhow::anyhow!("Turn unterbrochen"),
            )),
            Outcome::TimedOut => Err(CliError::new(
                Exit::General,
                anyhow::anyhow!("Zeitlimit erreicht"),
            )),
        };
    }
    Ok(())
}

fn on_event(st: &mut State, e: &Value, script: bool) {
    let p = &e["payload"];
    let ours = |st: &State| {
        st.our_turn
            .as_deref()
            .is_none_or(|t| p["turn_id"].as_str() == Some(t))
    };
    match e["type"].as_str().unwrap_or_default() {
        "session.status" => {
            if let Some(s) = p["status"].as_str() {
                st.status = s.to_owned();
                if matches!(s, "running" | "waiting_approval") {
                    st.busy = true;
                }
            }
        }
        "turn.started" => st.busy = true,
        "turn.completed" => {
            if let Some(usage) = p.get("usage_summary") {
                st.usage = usage.clone();
            }
            if ours(st) {
                st.busy = false;
                if script && st.live && st.our_turn.is_some() {
                    st.outcome = Some(Outcome::Completed);
                }
            }
        }
        "turn.failed" => {
            if ours(st) {
                st.busy = false;
                if script && st.live && st.our_turn.is_some() {
                    st.outcome = Some(Outcome::Failed);
                }
            }
        }
        "turn.interrupted" => {
            if ours(st) {
                st.busy = false;
                if script && st.live && st.our_turn.is_some() {
                    st.outcome = Some(if st.denied_by_on_ask {
                        Outcome::Denied
                    } else {
                        Outcome::Interrupted
                    });
                }
            }
        }
        "approval.requested" => {
            if let Some(a) = p["approval_id"].as_str() {
                st.pending.push_back((a.to_owned(), approval_summary(p)));
            }
        }
        "approval.resolved" => {
            if let Some(a) = p["approval_id"].as_str() {
                let was_front = st.pending.front().is_some_and(|(id, _)| id == a);
                st.pending.retain(|(id, _)| id != a);
                if was_front {
                    st.asked = false;
                }
            }
        }
        "cost.delta" => {
            st.cost_micro += p["cost_micro"].as_i64().unwrap_or(0);
        }
        "harness.exited" | "harness.incompatible" | "harness.auth_required"
            if script && st.live =>
        {
            st.busy = false;
            st.outcome = Some(Outcome::Failed);
        }
        _ => {}
    }
}
