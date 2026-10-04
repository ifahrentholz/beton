//! Deterministische Fake-Vendor-CLI für Tests (QA-002).
//!
//! ```text
//! beton-fake-cli --protocol stream-json --scenario <datei.yaml> [Claude-Flags …]
//! beton-fake-cli --protocol app-server  --scenario <datei.yaml> [app-server …]
//! beton-fake-cli --protocol acp         --scenario <datei.yaml> [Agent-Flags …]
//! beton-fake-cli [--protocol …] --version
//! beton-fake-cli … --record <datei>          # Kontext je Eingabe als JSON-Zeilen (AGT-005)
//! beton-fake-cli --protocol mcp-server --tools a,b   # Test-MCP-Server (HAR-009)
//! ```
//!
//! Spielt Szenarien im Format des Fake-Harness (HAR-026, `beton_harness::scenario`) über das
//! Wire-Protokoll einer Vendor-CLI ab:
//!
//! - `stream-json`: Claude Code (JSON-Zeilen, `control_request`/`control_response`).
//! - `app-server`: Codex (`codex app-server`, JSON-RPC 2.0 über stdio ohne `jsonrpc`-Feld,
//!   Methoden laut `codex app-server generate-json-schema` von codex-cli 0.153.2).
//! - `acp`: Agent Client Protocol (JSON-RPC 2.0 über stdio, Protokollversion 1).
//!
//! Alle IDs sind aus dem Szenario abgeleitet, damit gleiche Eingaben byte-gleiche Ausgaben
//! liefern. Unbekannte Flags der echten CLIs werden ignoriert. Die Treue zu den echten
//! Protokollen sichern die Golden-Transcripts der Adapter (HAR-025).

mod acp;
mod app_server;
mod io;
mod mcp;
mod stream_json;

use std::path::PathBuf;
use std::process::ExitCode;

use beton_harness::scenario::{Faults, Scenario};

use crate::io::Stop;

const CLAUDE_VERSION: &str = "2.1.0";
/// Version, deren Protokoll-Schema `app-server` nachbildet.
pub const CODEX_VERSION: &str = "0.153.2";
const ACP_VERSION: &str = "0.1.0";

#[derive(Debug, Default)]
struct Args {
    positional: Vec<String>,
    protocol: Option<String>,
    scenario: Option<PathBuf>,
    version: bool,
    partial: bool,
    resume: Option<String>,
    /// `--fork-session` von Claude Code: mit `--resume` als neue Session abzweigen.
    fork_session: bool,
    /// Sessions wie die echte CLI unter `$CLAUDE_CONFIG_DIR/projects/<slug>/<id>.jsonl`
    /// lesen und schreiben (nur `stream-json`, für Fork- und Rebuild-Tests, HAR-019).
    persist: bool,
    faults: Faults,
    /// `--mcp-config` von Claude Code (JSON-Text oder Datei).
    mcp_config: Option<String>,
    /// Tools des Test-MCP-Servers (`--protocol mcp-server`).
    tools: Vec<String>,
    /// `--model`, `--effort`, `--permission-mode` und `--session-id` von Claude Code.
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    session_id: Option<String>,
    /// Fehlerinjektion: diesen Permission-Mode melden, egal was gesetzt ist (HAR-027).
    report_mode: Option<String>,
    /// `--append-system-prompt-file` von Claude Code (AGT-005).
    append_system_prompt_file: Option<PathBuf>,
    /// Kontext-Aufzeichnung (`--record <datei>`, sonst `BETON_FAKE_RECORD`).
    record: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.peekable();
    let number = |v: Option<String>, flag: &str| {
        v.and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{flag} braucht eine Zahl"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--protocol" => out.protocol = args.next(),
            "--scenario" => out.scenario = args.next().map(PathBuf::from),
            "--version" | "-v" | "-V" => out.version = true,
            "--include-partial-messages" => out.partial = true,
            "--resume" => out.resume = args.next(),
            "--fork-session" => out.fork_session = true,
            "--persist" => out.persist = true,
            "--mcp-config" => out.mcp_config = args.next(),
            "--model" => out.model = args.next(),
            "--effort" => out.effort = args.next(),
            "--permission-mode" => out.permission_mode = args.next(),
            "--session-id" => out.session_id = args.next(),
            "--report-mode" => out.report_mode = args.next(),
            "--record" => out.record = args.next().map(PathBuf::from),
            "--append-system-prompt-file" => {
                out.append_system_prompt_file = args.next().map(PathBuf::from);
            }
            "--tools" => {
                out.tools = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "--crash-after" => out.faults.crash_after = Some(number(args.next(), &arg)?),
            "--hang-after" => out.faults.hang_after = Some(number(args.next(), &arg)?),
            "--malformed-line" => out.faults.malformed_line = Some(number(args.next(), &arg)?),
            a if !a.starts_with('-') => out.positional.push(arg),
            _ => {} // Flags der echten CLIs (-p, --output-format, --model …) ignorieren.
        }
    }
    if out.scenario.is_none() {
        out.scenario = std::env::var_os("BETON_FAKE_SCENARIO").map(PathBuf::from);
    }
    Ok(out)
}

impl Args {
    fn command(&self) -> Vec<&str> {
        self.positional.iter().map(String::as_str).collect()
    }
}

fn logged_in() -> bool {
    std::env::var("BETON_FAKE_AUTH").as_deref() != Ok("logged_out")
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("beton-fake-cli: {e}");
            return ExitCode::from(2);
        }
    };
    if args.protocol.as_deref() == Some("mcp-server") {
        return match mcp::serve(&args.tools) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("beton-fake-cli: {e}");
                ExitCode::from(1)
            }
        };
    }
    let scenario = match &args.scenario {
        Some(path) => match Scenario::load(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("beton-fake-cli: {e}");
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    match args.command().as_slice() {
        ["auth", "status", ..] => {
            // Wie `claude auth status --json`, samt Kontodaten, die beton verwerfen muss.
            println!(
                "{}",
                serde_json::json!({
                    "loggedIn": logged_in(),
                    "email": "fake-user@example.invalid",
                    "orgName": "Fake Org",
                })
            );
            return ExitCode::SUCCESS;
        }
        ["login", "status", ..] => {
            // Wie `codex login status`: Text auf stderr, Exit-Code 1 ohne Login.
            return if logged_in() {
                eprintln!("Logged in using ChatGPT");
                ExitCode::SUCCESS
            } else {
                eprintln!("Not logged in");
                ExitCode::from(1)
            };
        }
        _ => {}
    }
    let configured = scenario
        .as_ref()
        .and_then(|s| s.version.clone())
        .or_else(|| std::env::var("BETON_FAKE_VERSION").ok());
    if args.version {
        // Der Versions-Probe ruft nur `<programm> --version` auf (ohne die konfigurierten
        // Argumente); ein Link namens `codex` meldet daher die Codex-Version.
        let invoked_as_codex = std::env::args()
            .next()
            .map(std::path::PathBuf::from)
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .is_some_and(|name| name == "codex");
        let protocol = args
            .protocol
            .clone()
            .or_else(|| invoked_as_codex.then(|| "app-server".to_owned()));
        match protocol.as_deref() {
            Some("app-server") => println!(
                "codex-cli {}",
                configured.as_deref().unwrap_or(CODEX_VERSION)
            ),
            Some("acp") => println!("{}", configured.as_deref().unwrap_or(ACP_VERSION)),
            _ => println!(
                "{} (Claude Code)",
                configured.as_deref().unwrap_or(CLAUDE_VERSION)
            ),
        }
        return ExitCode::SUCCESS;
    }
    let Some(protocol) = args.protocol.clone() else {
        eprintln!("beton-fake-cli: --protocol stream-json|app-server|acp fehlt");
        return ExitCode::from(2);
    };
    let Some(mut scenario) = scenario else {
        eprintln!("beton-fake-cli: --scenario <datei> fehlt");
        return ExitCode::from(2);
    };
    // Flags überschreiben die Fehlerinjektion des Szenarios.
    let f = args.faults;
    scenario.faults.crash_after = f.crash_after.or(scenario.faults.crash_after);
    scenario.faults.hang_after = f.hang_after.or(scenario.faults.hang_after);
    scenario.faults.malformed_line = f.malformed_line.or(scenario.faults.malformed_line);

    io::set_record(args.record.clone());
    // Wie Claude Code: die Datei beim Start lesen und an den System-Prompt hängen.
    if let Some(file) = &args.append_system_prompt_file {
        match std::fs::read_to_string(file) {
            Ok(text) => io::record_context("append_system_prompt", &text),
            Err(e) => {
                eprintln!("beton-fake-cli: {}: {e}", file.display());
                return ExitCode::from(1);
            }
        }
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let persist = if args.persist {
        // Nie das echte Konfigurationsverzeichnis: ohne CLAUDE_CONFIG_DIR kein Verlauf.
        let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") else {
            eprintln!("beton-fake-cli: --persist braucht CLAUDE_CONFIG_DIR");
            return ExitCode::from(2);
        };
        let cwd = std::env::current_dir().unwrap_or_default();
        Some(stream_json::project_dir(std::path::Path::new(&dir), &cwd))
    } else {
        None
    };
    let result = match protocol.as_str() {
        "stream-json" => stream_json::Sim::new(
            &scenario,
            args.partial,
            stream_json::Resume {
                session: args.resume.clone(),
                fork: args.fork_session,
                persist,
                session_id: args.session_id.clone(),
            },
            stream_json::Settings::from_flags(
                args.model.clone(),
                args.effort.clone(),
                args.permission_mode.clone(),
                args.report_mode.clone(),
            ),
            args.mcp_config
                .as_deref()
                .map(mcp::from_claude_config)
                .unwrap_or_default(),
            stdin.lock(),
            stdout.lock(),
        )
        .run(&scenario),
        "app-server" => app_server::AppServer::new(
            &scenario,
            configured.unwrap_or_else(|| CODEX_VERSION.into()),
            stdin.lock(),
            stdout.lock(),
        )
        .run(),
        "acp" => acp::Agent::new(
            &scenario,
            configured.unwrap_or_else(|| ACP_VERSION.into()),
            stdin.lock(),
            stdout.lock(),
        )
        .run(),
        other => {
            eprintln!("beton-fake-cli: unbekanntes Protokoll {other:?}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) | Err(Stop::Eof) => ExitCode::SUCCESS,
        Err(Stop::Crash(code)) => {
            eprintln!("beton-fake-cli: Absturz (Fehlerinjektion)");
            ExitCode::from(code)
        }
        Err(Stop::Io(e)) => {
            eprintln!("beton-fake-cli: {e}");
            ExitCode::from(1)
        }
    }
}
