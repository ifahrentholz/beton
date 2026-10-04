//! Runner-Prozess. Startet über den Host (RUN-002); das Token kommt über stdin.
//! Mit WP-10 wird daraus `beton runner` im einen Binary.

use std::process::ExitCode;
use std::sync::Arc;

use beton_harness::registry::{Registry, RegistryOptions};
use beton_runner::{Exit, RunnerBoot, run};

#[tokio::main]
async fn main() -> ExitCode {
    let boot = match RunnerBoot::from_env_and_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("beton-runner: {e}");
            return ExitCode::from(2);
        }
    };
    let mut registry = Registry::new(RegistryOptions { dev: boot.dev });
    registry.register(Arc::new(beton_harness_claude::ClaudeAdapter::default()));
    match run(boot, registry).await {
        Ok(Exit::Stopped | Exit::ParentGone) => ExitCode::SUCCESS,
        Ok(Exit::HarnessExited { code }) => {
            ExitCode::from(u8::try_from(code.unwrap_or(1)).unwrap_or(1))
        }
        Err(e) => {
            eprintln!("beton-runner: {e}");
            ExitCode::FAILURE
        }
    }
}
