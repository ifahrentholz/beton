//! Lokalen Daemon finden oder im Hintergrund starten (CLI-002 AC1, CLI-004).
//!
//! Ein Daemon läuft, wenn `GET /v1/info` mit dem lokalen Token antwortet. Sonst startet
//! `beton serve --foreground` als abgelöster Prozess (eigene Prozessgruppe, ohne Terminal);
//! dessen Ausgaben landen in `<data_dir>/logs/serve.out`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use beton_sdk::{Client, DaemonInfo};

use crate::commands::Ctx;
use crate::exit::{CliError, CliResult, Exit};

/// So lange wartet der Start auf einen antwortenden Daemon.
pub const START_TIMEOUT: Duration = Duration::from_secs(30);

/// Läuft für dieses Datenverzeichnis ein erreichbarer Daemon?
pub async fn running(home: &Path) -> Option<Client> {
    let client = Client::local(home).ok()?;
    client.info().await.ok()?;
    Some(client)
}

/// Client für den lokalen Daemon; startet ihn bei Bedarf im Hintergrund.
pub async fn ensure_running(ctx: &Ctx) -> CliResult<Client> {
    if ctx.global.server.is_some() {
        return ctx.client();
    }
    if let Some(client) = running(&ctx.home).await {
        return Ok(client);
    }
    ctx.note("Starte lokalen Daemon …");
    start_background(ctx, &[]).await
}

fn serve_log(home: &Path) -> PathBuf {
    home.join("logs").join("serve.out")
}

/// Startet `beton serve --foreground <extra>` abgelöst und wartet, bis er antwortet.
pub async fn start_background(ctx: &Ctx, extra: &[String]) -> CliResult<Client> {
    let exe = std::env::current_exe().context("eigenes Binary")?;
    let log_path = serve_log(&ctx.home);
    if let Some(dir) = log_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    let log = std::fs::File::create(&log_path).with_context(|| log_path.display().to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("serve").arg("--foreground").arg("-q").args(extra);
    if let Some(config) = &ctx.global.config {
        cmd.arg("--config").arg(config);
    }
    cmd.env("BETON_HOME", &ctx.home)
        .current_dir(&ctx.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    detach(&mut cmd);
    let mut child = cmd.spawn().context("Daemon starten")?;
    let pid = child.id();
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(info) = DaemonInfo::read(&ctx.home)
            && info.pid == pid
            && let Some(client) = running(&ctx.home).await
        {
            return Ok(client);
        }
        if let Some(status) = child.try_wait().context("Daemon-Status")? {
            let tail = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail: String = tail.lines().rev().take(5).collect::<Vec<_>>().join("\n");
            return Err(CliError::new(
                Exit::Unreachable,
                anyhow::anyhow!(
                    "Daemon endete beim Start mit {status} (Log: {}){}",
                    log_path.display(),
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(":\n{tail}")
                    }
                ),
            ));
        }
        if Instant::now() > deadline {
            return Err(CliError::new(
                Exit::Unreachable,
                anyhow::anyhow!(
                    "Daemon antwortet nicht binnen {} s (Log: {})",
                    START_TIMEOUT.as_secs(),
                    log_path.display()
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Eigene Prozessgruppe bzw. ohne Konsole, damit `Strg+C` im Terminal den Daemon nicht trifft.
fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}
