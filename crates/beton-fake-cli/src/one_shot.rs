//! Einmal-Modus der Fake-CLI (SES-010): `claude -p --output-format json` bzw.
//! `codex exec --json`, im Ausgabeformat der echten CLIs (Claude Code 2.1.285,
//! codex-cli 0.153.2).

use std::io::{Read, Write};
use std::process::ExitCode;

use beton_harness::scenario::{ONE_SHOT_DEFAULT_REPLY, OneShotBehavior};
use serde_json::json;

/// Welche CLI nachgebildet wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Claude,
    Codex,
}

/// Variablen, die auf API-Keys hindeuten; die Fake-CLI meldet nur ihre Namen.
fn api_key_vars() -> Vec<String> {
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.ends_with("_API_KEY") || k == "ANTHROPIC_AUTH_TOKEN")
        .collect();
    names.sort();
    names
}

/// Hängt den Aufruf an `path` an (argv und Namen gesetzter API-Key-Variablen).
fn record(path: Option<&std::path::Path>, stdin_bytes: usize) {
    let Some(path) = path else {
        return;
    };
    let line = json!({
        "source": "one_shot",
        "argv": std::env::args().skip(1).collect::<Vec<_>>(),
        "api_key_vars": api_key_vars(),
        "stdin_bytes": stdin_bytes,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn run(
    flavor: Flavor,
    behavior: &OneShotBehavior,
    record_to: Option<&std::path::Path>,
) -> ExitCode {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    record(record_to, input.len());
    let tokens_in = input.len().div_ceil(4) as u64;
    let out = match (flavor, &behavior.fail) {
        (Flavor::Claude, Some(reason)) => vec![json!({
            "type": "result",
            "subtype": "error_during_execution",
            "is_error": true,
            "result": reason,
        })],
        (Flavor::Claude, None) => {
            let reply = behavior.reply.as_deref().unwrap_or(ONE_SHOT_DEFAULT_REPLY);
            vec![json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "num_turns": 1,
                "result": reply,
                "session_id": "00000000-0000-4000-8000-000000000000",
                "total_cost_usd": 0.0001,
                "usage": {
                    "input_tokens": tokens_in,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": reply.len().div_ceil(4),
                },
                "modelUsage": {"claude-haiku-fake": {"contextWindow": 200000}},
            })]
        }
        (Flavor::Codex, Some(reason)) => vec![
            json!({"type": "thread.started", "thread_id": "fake-thread"}),
            json!({"type": "turn.started"}),
            json!({"type": "error", "message": reason}),
            json!({"type": "turn.failed", "error": {"message": reason}}),
        ],
        (Flavor::Codex, None) => {
            let reply = behavior.reply.as_deref().unwrap_or(ONE_SHOT_DEFAULT_REPLY);
            vec![
                json!({"type": "thread.started", "thread_id": "fake-thread"}),
                json!({"type": "turn.started"}),
                json!({"type": "item.completed", "item": {"id": "item_0", "type": "agent_message", "text": reply}}),
                json!({"type": "turn.completed", "usage": {"input_tokens": tokens_in, "cached_input_tokens": 0, "output_tokens": reply.len().div_ceil(4)}}),
            ]
        }
    };
    let mut stdout = std::io::stdout().lock();
    match flavor {
        // `--output-format json`: ein JSON-Objekt.
        Flavor::Claude => {
            let _ = writeln!(stdout, "{}", out[0]);
        }
        // `--json`: JSONL.
        Flavor::Codex => {
            for line in &out {
                let _ = writeln!(stdout, "{line}");
            }
        }
    }
    if behavior.fail.is_some() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
