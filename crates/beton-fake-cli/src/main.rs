//! Deterministische Fake-Vendor-CLI für Tests (QA-002).
//!
//! ```text
//! beton-fake-cli --protocol stream-json --scenario <datei.yaml> [Claude-Flags …]
//! beton-fake-cli --protocol app-server  --scenario <datei.yaml> [app-server …]
//! beton-fake-cli --protocol acp         --scenario <datei.yaml> [Agent-Flags …]
//! beton-fake-cli [--protocol …] --version
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
    faults: Faults,
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
        match args.protocol.as_deref() {
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

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let result = match protocol.as_str() {
        "stream-json" => stream_json::Sim::new(
            &scenario,
            args.partial,
            args.resume.clone(),
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
