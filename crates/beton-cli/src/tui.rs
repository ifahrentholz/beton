//! `beton tui` (TUI-001): startet bei Bedarf den lokalen Daemon und öffnet die
//! Terminal-Oberfläche aus `beton-tui`.

use std::io::IsTerminal as _;

use beton_tui::app::{AppOptions, WindowsHost};

use crate::cli::TuiArgs;
use crate::commands::Ctx;
use crate::exit::{CliError, CliResult, Exit};

pub async fn run(ctx: &Ctx, args: TuiArgs) -> CliResult {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(CliError::new(
            Exit::Usage,
            anyhow::anyhow!("beton tui braucht ein Terminal (stdin und stdout)"),
        ));
    }
    let client = crate::daemon::ensure_running(ctx).await?;
    let session = match &args.session {
        Some(r) => {
            let id = crate::sessionref::resolve(&client, r).await?;
            Some(id.parse().map_err(|_| {
                CliError::new(Exit::General, anyhow::anyhow!("Session-ID {id} ungültig"))
            })?)
        }
        None => None,
    };
    let server_label = match &ctx.global.server {
        Some(url) => url
            .split("://")
            .nth(1)
            .unwrap_or(url)
            .trim_end_matches('/')
            .to_owned(),
        None => "local".to_owned(),
    };
    let opts = beton_tui::Options {
        home: ctx.home.clone(),
        session,
        app: AppOptions {
            server_label,
            offset: crate::usage_report::local_offset(),
            windows: cfg!(windows)
                .then(|| WindowsHost::detect(std::env::var("WT_SESSION").ok().as_deref())),
        },
        on_events: None,
    };
    beton_tui::run(client, opts)
        .await
        .map_err(|e| CliError::new(Exit::General, anyhow::anyhow!("TUI: {e}")))
}
