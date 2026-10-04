//! Aufnahme von Golden-Transcripts mit der echten `claude`-CLI (HAR-025).
//!
//! Aufruf über `beton dev record-golden --harness claude [--scenario <name>]…`. Braucht eine
//! installierte, angemeldete CLI und verbraucht etwas Kontingent; nie in CI. Die CLI läuft
//! isoliert (`--safe-mode --strict-mcp-config`). Vor dem Schreiben entfernt der Recorder
//! Konto-, Pfad- und Benutzerdaten; danach greifen Scrubbing und Secret-Scan
//! (`golden::write_recording`). Die Erwartungen entstehen anschließend per Replay.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use beton_harness::golden::{self, Meta, RecordingLauncher, Script, ScriptDecision, ScriptInput};
use beton_harness::process::RealLauncher;
use beton_harness::{HarnessAdapter, HostEnv};
use serde_json::Value;

use crate::ClaudeAdapter;

/// Ein Aufnahme-Szenario.
pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    inputs: Vec<ScriptInput>,
    gate: Vec<ScriptDecision>,
    files: &'static [(&'static str, &'static str)],
    extra_args: &'static str,
}

fn send(text: &str) -> ScriptInput {
    ScriptInput {
        send: Some(text.into()),
        interrupt_after_events: None,
    }
}

/// Alle Szenarien in Aufnahme-Reihenfolge.
pub fn scenarios() -> Vec<Scenario> {
    use ScriptDecision::{Allow, Deny};
    vec![
        Scenario {
            name: "text",
            description: "Reiner Text-Turn",
            inputs: vec![send("Antworte nur mit dem Wort: ok")],
            gate: vec![],
            files: &[],
            extra_args: "",
        },
        Scenario {
            name: "bash-tool",
            description: "Turn mit Bash-Tool, Freigabe erteilt",
            inputs: vec![send(
                "Lege mit dem Bash-Tool genau `mkdir beton-ordner` an und antworte danach nur mit: erledigt.",
            )],
            gate: vec![Allow, Allow],
            files: &[],
            extra_args: "",
        },
        Scenario {
            name: "bash-deny",
            description: "Bash-Tool abgelehnt (HAR-005 AC1)",
            inputs: vec![send(
                "Lege mit dem Bash-Tool genau `mkdir beton-ordner` an. Wenn das abgelehnt wird, sag nur: abgelehnt.",
            )],
            gate: vec![Deny, Deny],
            files: &[],
            extra_args: "",
        },
        Scenario {
            name: "file-edit",
            description: "Turn mit Datei-Edit",
            inputs: vec![send(
                "Ersetze in der Datei notiz.txt das Wort alt durch neu. Nutze das Edit-Tool. Antworte danach nur mit: fertig.",
            )],
            gate: vec![Allow, Allow, Allow],
            files: &[("notiz.txt", "Das ist alt.\n")],
            extra_args: "",
        },
        Scenario {
            name: "reasoning",
            description: "Turn mit Reasoning",
            inputs: vec![send(
                "Denke kurz nach: Was ist 17 mal 23? Antworte nur mit der Zahl.",
            )],
            gate: vec![],
            files: &[],
            extra_args: "",
        },
        Scenario {
            name: "error-max-turns",
            description: "Fehler-Result error_max_turns",
            inputs: vec![send(
                "Führe mit dem Bash-Tool `echo eins` aus, danach `echo zwei`, danach `echo drei`.",
            )],
            gate: vec![Allow, Allow, Allow],
            files: &[],
            extra_args: "--max-turns 1",
        },
        Scenario {
            name: "interrupt",
            description: "Interrupt während der Antwort",
            inputs: vec![ScriptInput {
                send: Some(
                    "Schreibe die Zahlen von 1 bis 300 untereinander, eine pro Zeile.".into(),
                ),
                interrupt_after_events: Some(12),
            }],
            gate: vec![],
            files: &[],
            extra_args: "",
        },
    ]
}

/// Entfernt Konto-, Benutzer- und Pfaddaten aus einer stdout-Zeile.
fn sanitize(line: &str, replacements: &[(String, String)]) -> String {
    let mut text = line.to_owned();
    for (from, to) in replacements {
        text = text.replace(from.as_str(), to.as_str());
    }
    let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
        return text;
    };
    if v["type"] == "control_response" && v["response"]["request_id"] == "beton_init" {
        // Nur der Permission-Mode bleibt (HAR-027: der Adapter prüft ihn); Konto- und
        // Benutzerdaten fliegen raus.
        let mode = v["response"]["response"]["current_permission_mode"].clone();
        let mut kept = serde_json::json!({"sanitized": "Initialize-Antwort entfernt (Konto- und Benutzerdaten)"});
        if mode.is_string() {
            kept["current_permission_mode"] = mode;
        }
        v["response"]["response"] = kept;
    }
    clean(&mut v);
    v.to_string()
}

fn clean(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for key in ["account", "email", "organization", "subscriptionType"] {
                map.remove(key);
            }
            for key in [
                "slash_commands",
                "terminal_slash_commands",
                "skills",
                "agents",
                "plugins",
            ] {
                if map.contains_key(key) {
                    map.insert(key.into(), Value::Array(Vec::new()));
                }
            }
            for key in [
                "scratchpad_path",
                "messaging_socket_path",
                "user_output_styles_dir",
            ] {
                if map.contains_key(key) {
                    map.insert(key.into(), Value::String("<lokal>".into()));
                }
            }
            map.values_mut().for_each(clean);
        }
        Value::Array(items) => items.iter_mut().for_each(clean),
        Value::String(s) if s.contains('@') && s.contains('.') && !s.contains(' ') => {
            *s = "<email>".into();
        }
        _ => {}
    }
}

/// Nimmt ein Szenario nach `<root>/<name>` auf und prüft es per Replay.
pub async fn record(s: &Scenario, root: &Path, version: &str) -> Result<(), String> {
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (name, content) in s.files {
        std::fs::write(work.path().join(name), content).map_err(|e| e.to_string())?;
    }
    let workdir = work.path().to_path_buf();
    let canonical = workdir.canonicalize().map_err(|e| e.to_string())?;
    let home = std::env::var("HOME").unwrap_or_default();
    let mut replacements = vec![
        (
            canonical.display().to_string(),
            golden::GOLDEN_WORKDIR.to_owned(),
        ),
        (
            workdir.display().to_string(),
            golden::GOLDEN_WORKDIR.to_owned(),
        ),
    ];
    if !home.is_empty() {
        replacements.push((home, "/home/beton".into()));
    }

    let script = Script {
        model: Some("haiku".into()),
        inputs: s.inputs.clone(),
        gate: s.gate.clone(),
    };
    let mut env = HostEnv::from_process();
    if !s.extra_args.is_empty() {
        let claude = which("claude").ok_or("claude nicht in PATH")?;
        env.vars.insert(
            "BETON_CLAUDE_PATH".into(),
            format!("{} {}", claude.display(), s.extra_args),
        );
    }
    let launcher = RecordingLauncher::new(Arc::new(RealLauncher));
    let adapter = ClaudeAdapter {
        isolated: true,
        ..ClaudeAdapter::default()
    };
    let events = golden::drive(
        &adapter,
        &script,
        Arc::new(launcher.clone()),
        env,
        workdir,
        Duration::from_secs(180),
    )
    .await
    .map_err(|e| format!("{}: {e}", s.name))?;
    tracing::info!(
        scenario = s.name,
        events = events.len(),
        lines = launcher.raw_lines().len(),
        "Szenario aufgenommen"
    );

    let raw: Vec<golden::RawLine> = launcher
        .raw_lines()
        .into_iter()
        .map(|mut l| {
            l.out = sanitize(&l.out, &replacements);
            l
        })
        .collect();
    let meta = Meta {
        harness: "claude".into(),
        cli_version: version.into(),
        recorded_at: beton_core::time::Timestamp::now().to_string(),
        scenario: s.description.into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        exit_code: 0,
        recorded_with: None,
        migrated: None,
    };
    let dir = root.join(s.name);
    let _ = std::fs::remove_dir_all(&dir);
    golden::write_recording(&dir, &meta, &script, &raw, &[]).map_err(|e| e.to_string())?;
    golden::check_case(&dir, &ClaudeAdapter::default(), true)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Version der installierten CLI.
pub async fn cli_version() -> Result<String, String> {
    ClaudeAdapter::default()
        .probe(&HostEnv::from_process())
        .await
        .version
        .ok_or_else(|| "claude --version liefert keine Version; ist die CLI installiert?".into())
}

fn which(cmd: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(cmd))
        .find(|p| p.is_file())
}
