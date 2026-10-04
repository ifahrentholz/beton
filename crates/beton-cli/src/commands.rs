//! Ausführung der Kommandos (CLI-001). Konvention: stdout nur Nutzdaten (mit `--json`
//! maschinenlesbar), Hinweise und Fortschritt auf stderr.

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::CommandFactory as _;
use serde_json::{Value, json};

use crate::cli::{
    AdminCommand, AuthCommand, Cli, Command, CompletionArgs, ConfigCommand, ConfigScopeArgs,
    DevCommand, OpenArgs, ProjectionsCommand, RecordGoldenArgs, SessionCommand, Shell,
};
use crate::config::{Layers, Paths, Scope};
use crate::exit::{CliError, CliResult, Exit};

/// Umgebung eines Aufrufs.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub global: crate::cli::GlobalArgs,
    /// Datenverzeichnis (`$BETON_HOME` bzw. `~/.beton`).
    pub home: PathBuf,
    pub cwd: PathBuf,
}

impl Ctx {
    pub fn from_env(global: crate::cli::GlobalArgs) -> Self {
        Self {
            global,
            home: crate::logging::beton_home(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    pub fn paths(&self) -> Paths {
        Paths::discover(&self.home, self.global.config.as_deref(), &self.cwd)
    }

    pub fn layers(&self) -> CliResult<Layers> {
        Layers::load(self.paths(), std::env::vars()).map_err(|e| CliError::new(Exit::General, e))
    }

    /// Hinweis auf stderr (unterdrückt mit `-q`).
    pub fn note(&self, msg: impl std::fmt::Display) {
        if !self.global.quiet {
            eprintln!("{msg}");
        }
    }

    pub fn color(&self) -> bool {
        !self.global.no_color
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal()
    }

    /// Client für `--server` bzw. den lokalen Daemon.
    pub fn client(&self) -> CliResult<beton_sdk::Client> {
        match &self.global.server {
            Some(url) => {
                let token = match std::env::var("BETON_TOKEN") {
                    Ok(t) => t,
                    Err(_) => {
                        let path = beton_sdk::local_token_path(&self.home);
                        std::fs::read_to_string(&path)
                            .with_context(|| format!("Token aus {} lesen", path.display()))?
                    }
                };
                Ok(beton_sdk::Client::new(url.clone(), token.trim())?)
            }
            None => Ok(beton_sdk::Client::local(&self.home)?),
        }
    }
}

/// JSON auf stdout, eine Zeile Abschluss.
pub fn print_json(value: &Value) -> CliResult {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value).context("stdout")?;
    writeln!(out).context("stdout")?;
    Ok(())
}

/// Führt ein Kommando aus.
pub async fn run(cli: Cli) -> CliResult {
    let ctx = Ctx::from_env(cli.global);
    // `serve` und `__runner` initialisieren ihr Logging selbst (daemon.log, runner.log);
    // reine Hilfskommandos legen kein Log-Verzeichnis an.
    let _log = cli_logs(&cli.command).then(|| {
        crate::logging::init(crate::logging::LogConfig {
            dir: ctx.home.join("logs"),
            stderr_human: ctx.global.verbose > 0,
            ..crate::logging::LogConfig::for_component(crate::logging::Component::Cli)
        })
        .ok()
    });
    match cli.command {
        Command::Run(args) => crate::run::run(&ctx, args).await,
        Command::Resume(args) => crate::run::resume(&ctx, args).await,
        Command::Attach(args) => crate::run::attach(&ctx, args).await,
        Command::Open(args) => open(&ctx, args).await,
        Command::Session(SessionCommand::List(args)) => session_list(&ctx, args.archived).await,
        Command::Session(cmd) => session(&ctx, cmd).await,
        Command::Serve(args) => crate::serve::serve(&ctx, args).await,
        Command::Setup(args) => setup(&ctx, args).await,
        Command::Doctor => doctor(&ctx).await,
        Command::Config(cmd) => config(&ctx, cmd),
        Command::Auth(AuthCommand::RotateLocal) => rotate_local(&ctx),
        Command::Admin(AdminCommand::Projections(ProjectionsCommand::Rebuild(args))) => {
            rebuild_projections(&ctx, args.session.as_deref()).await
        }
        Command::Completion(args) => completion(&args),
        Command::Version => version(&ctx),
        Command::Dev(DevCommand::RecordGolden(args)) => record_golden(&ctx, args).await,
        Command::Runner => {
            let _log = crate::logging::init(crate::logging::LogConfig {
                stderr_human: false,
                ..crate::logging::LogConfig::for_component(crate::logging::Component::Runner)
            })
            .ok();
            runner_exit(beton_runner::main_from_env().await)
        }
    }
}

/// Kommandos, die mit dem Daemon sprechen und nach `cli.log` loggen (OBS-001).
fn cli_logs(command: &Command) -> bool {
    !matches!(
        command,
        Command::Serve(_)
            | Command::Runner
            | Command::Config(_)
            | Command::Completion(_)
            | Command::Version
            | Command::Auth(_)
    )
}

fn runner_exit(code: std::process::ExitCode) -> CliResult {
    match code {
        code if code == std::process::ExitCode::SUCCESS => Ok(()),
        _ => Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Runner beendet mit Fehler"),
        )),
    }
}

async fn setup(ctx: &Ctx, args: crate::cli::SetupArgs) -> CliResult {
    let settings = ctx
        .layers()?
        .settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let layers = beton_harness::registry::HarnessLayers {
        user: settings.harnesses.clone(),
        project: Default::default(),
    };
    let env = beton_harness::HostEnv {
        user: settings.harnesses,
        ..beton_harness::HostEnv::from_process()
    };
    let registry = beton_runner::builtin_registry(&layers, false);
    let report = crate::setup::check(&env, &registry, &crate::setup::default_local_servers()).await;
    if args.check && ctx.global.json {
        print_json(&serde_json::to_value(&report).context("Bericht")?)?;
        let missing = report.harnesses.iter().any(|h| h.supported && !h.installed);
        return if missing {
            Err(CliError::new(
                Exit::General,
                anyhow::anyhow!("Harness-CLI fehlt"),
            ))
        } else {
            Ok(())
        };
    }
    // Ohne Terminal keine Rückfragen (wie --non-interactive).
    let interactive = !args.non_interactive && !args.check && std::io::stdin().is_terminal();
    let mut out = std::io::stdout().lock();
    let code = crate::setup::run(
        &report,
        interactive,
        &mut crate::setup::TerminalPrompt,
        &crate::setup::RealSpawner,
        &mut out,
    )
    .await
    .context("stdout")?;
    if code == 0 {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Mindestens eine nutzbare Harness-CLI fehlt"),
        ))
    }
}

async fn doctor(ctx: &Ctx) -> CliResult {
    let report = crate::doctor::run(ctx).await;
    if ctx.global.json {
        print_json(&serde_json::to_value(&report).context("Bericht")?)?;
    } else {
        crate::doctor::render(&report, &mut std::io::stdout().lock()).context("stdout")?;
    }
    match report.exit_code() {
        0 => Ok(()),
        1 => Err(CliError::new(Exit::General, anyhow::anyhow!("Warnungen"))),
        _ => Err(CliError::new(
            Exit::Usage,
            anyhow::anyhow!("Fehler gefunden"),
        )),
    }
}

fn version(ctx: &Ctx) -> CliResult {
    if ctx.global.json {
        print_json(&json!({ "version": crate::VERSION }))
    } else {
        println!("beton {}", crate::VERSION);
        Ok(())
    }
}

fn completion(args: &CompletionArgs) -> CliResult {
    let mut cmd = Cli::command();
    let shell = match args.shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
        Shell::Powershell => clap_complete::Shell::PowerShell,
    };
    clap_complete::generate(shell, &mut cmd, "beton", &mut std::io::stdout());
    Ok(())
}

async fn session_list(ctx: &Ctx, include_archived: bool) -> CliResult {
    let client = ctx.client()?;
    let sessions = client.all_sessions(include_archived).await?;
    if ctx.global.json {
        return print_json(&Value::Array(sessions));
    }
    if sessions.is_empty() {
        ctx.note("Keine Sessions. Neue Session: beton run");
        return Ok(());
    }
    let text = |s: &Value, k: &str| s[k].as_str().unwrap_or_default().to_owned();
    let rows: Vec<[String; 4]> = sessions
        .iter()
        .map(|s| {
            let mut status = text(s, "status");
            if s["archived"].as_bool() == Some(true) {
                status.push_str(" (archiviert)");
            }
            [text(s, "id"), status, text(s, "harness"), text(s, "title")]
        })
        .collect();
    let header = ["ID", "STATUS", "HARNESS", "TITEL"];
    let widths: Vec<usize> = (0..3)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([header[i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = std::io::stdout().lock();
    let line = |out: &mut std::io::StdoutLock<'_>, r: [&str; 4]| {
        writeln!(
            out,
            "{:w0$}  {:w1$}  {:w2$}  {}",
            r[0],
            r[1],
            r[2],
            r[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2]
        )
    };
    line(&mut out, header).context("stdout")?;
    for r in &rows {
        line(&mut out, [&r[0], &r[1], &r[2], &r[3]]).context("stdout")?;
    }
    Ok(())
}

async fn session(ctx: &Ctx, cmd: SessionCommand) -> CliResult {
    let client = ctx.client()?;
    let reference = match &cmd {
        SessionCommand::List(_) => return Ok(()),
        SessionCommand::Show(a)
        | SessionCommand::Archive(a)
        | SessionCommand::Unarchive(a)
        | SessionCommand::Delete(a)
        | SessionCommand::Interrupt(a) => a.session.clone(),
        SessionCommand::Rename(a) => a.session.clone(),
    };
    let id = crate::sessionref::resolve(&client, &reference).await?;
    let result = match cmd {
        SessionCommand::List(_) => return Ok(()),
        SessionCommand::Show(_) => {
            let mut session = client.session(&id).await?;
            if let Some(cwd) = crate::sessionref::cwd_of(&client, &id).await? {
                session["cwd"] = Value::String(cwd);
            }
            session["url"] = Value::String(client.session_url(&id));
            if ctx.global.json {
                return print_json(&session);
            }
            let mut out = std::io::stdout().lock();
            for key in [
                "id",
                "title",
                "status",
                "harness",
                "cwd",
                "head_seq",
                "archived",
                "created_at",
                "last_activity_at",
                "url",
            ] {
                let v = &session[key];
                if !v.is_null() {
                    writeln!(out, "{key:<17} {}", render(v)).context("stdout")?;
                }
            }
            return Ok(());
        }
        SessionCommand::Rename(a) => {
            client
                .patch_session(&id, &json!({ "title": a.title }))
                .await?
        }
        SessionCommand::Archive(_) => client.set_archived(&id, true).await?,
        SessionCommand::Unarchive(_) => client.set_archived(&id, false).await?,
        SessionCommand::Delete(_) => {
            client.delete_session(&id).await?;
            json!({ "id": id, "deleted": true })
        }
        SessionCommand::Interrupt(_) => {
            client.interrupt(&id).await?;
            json!({ "id": id, "interrupted": true })
        }
    };
    if ctx.global.json {
        print_json(&result)
    } else {
        ctx.note(format!("{id}: erledigt"));
        Ok(())
    }
}

/// URL für `beton open`: Einmal-Link, optional mit Weiterleitung zur Session.
pub fn open_url(redeem_url: &str, session: Option<&str>) -> CliResult<String> {
    match session {
        None => Ok(redeem_url.to_owned()),
        Some(id) => {
            let id: beton_core::id::SessionId = id
                .parse()
                .map_err(|e| CliError::usage(format!("ungültige Session-ID `{id}`: {e}")))?;
            Ok(format!("{redeem_url}&next=/s/{id}"))
        }
    }
}

async fn open(ctx: &Ctx, args: OpenArgs) -> CliResult {
    let client = ctx.client()?;
    let code = client.create_login_code().await?;
    let url = open_url(&code.redeem_url, args.session.as_deref())?;
    if ctx.global.json {
        print_json(&json!({ "url": url, "expires_in_s": code.expires_in_s }))?;
    }
    match open_browser(&url) {
        Ok(()) => ctx.note("Web-UI im Browser geöffnet (Link gilt 60 s, einmalig)."),
        Err(e) => {
            ctx.note(format!(
                "Browser nicht startbar ({e:#}). Link (60 s, einmalig):"
            ));
            if !ctx.global.json {
                println!("{url}");
            }
        }
    }
    Ok(())
}

/// Öffnet die URL mit `$BROWSER` oder dem Standard der Plattform.
fn open_browser(url: &str) -> anyhow::Result<()> {
    let mut cmd = if let Some(browser) = std::env::var_os("BROWSER").filter(|b| !b.is_empty()) {
        let mut c = std::process::Command::new(browser);
        c.arg(url);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let status = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("Browser starten")?;
    anyhow::ensure!(status.success(), "Browser-Kommando endete mit {status}");
    Ok(())
}

fn scope_of(args: ConfigScopeArgs) -> Option<Scope> {
    if args.project {
        Some(Scope::Project)
    } else if args.global {
        Some(Scope::User)
    } else {
        None
    }
}

fn config(ctx: &Ctx, cmd: ConfigCommand) -> CliResult {
    match cmd {
        ConfigCommand::List => {
            let layers = ctx.layers()?;
            let entries = layers.list();
            if ctx.global.json {
                let items: Vec<Value> = entries
                    .into_iter()
                    .map(|(key, value, source)| {
                        json!({ "key": key, "value": value, "source": source.as_str() })
                    })
                    .collect();
                return print_json(&Value::Array(items));
            }
            let mut out = std::io::stdout().lock();
            for (key, value, source) in entries {
                writeln!(out, "{key} = {} ({})", render(&value), source.as_str())
                    .context("stdout")?;
            }
            Ok(())
        }
        ConfigCommand::Get(args) => {
            let layers = ctx.layers()?;
            let value = layers.get(&args.key, scope_of(args.scope)).ok_or_else(|| {
                CliError::new(
                    Exit::General,
                    anyhow::anyhow!("`{}` ist nicht gesetzt", args.key),
                )
            })?;
            if ctx.global.json {
                print_json(&value)
            } else {
                println!("{}", render(&value));
                Ok(())
            }
        }
        ConfigCommand::Set(args) => {
            let mut layers = ctx.layers()?;
            let scope = scope_of(args.scope).unwrap_or(Scope::User);
            layers
                .set(scope, &args.key, &args.value)
                .map_err(|e| CliError::new(Exit::General, e))?;
            ctx.note(format!(
                "{} gesetzt in {}",
                args.key,
                layers.paths.of(scope).display()
            ));
            Ok(())
        }
        ConfigCommand::Unset(args) => {
            let mut layers = ctx.layers()?;
            let scope = scope_of(args.scope).unwrap_or(Scope::User);
            let removed = layers
                .unset(scope, &args.key)
                .map_err(|e| CliError::new(Exit::General, e))?;
            if removed {
                ctx.note(format!("{} entfernt", args.key));
            } else {
                ctx.note(format!(
                    "{} war in {} nicht gesetzt",
                    args.key,
                    layers.paths.of(scope).display()
                ));
            }
            Ok(())
        }
        ConfigCommand::Edit(args) => {
            let scope = scope_of(args).unwrap_or(Scope::User);
            config_edit(ctx, scope)
        }
    }
}

fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Öffnet die Datei in `$VISUAL`/`$EDITOR`; ist das Ergebnis ungültig, wird der vorige Stand
/// wiederhergestellt.
fn config_edit(ctx: &Ctx, scope: Scope) -> CliResult {
    let paths = ctx.paths();
    let path = paths.of(scope).to_path_buf();
    let before = std::fs::read(&path).ok();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    if before.is_none() {
        std::fs::write(&path, "").with_context(|| path.display().to_string())?;
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .with_context(|| format!("Editor `{editor}` starten"))?;
    if !status.success() {
        restore(&path, before.as_deref());
        return Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Editor endete mit {status}; Datei unverändert"),
        ));
    }
    if let Err(e) = Layers::load(paths, std::env::vars()) {
        restore(&path, before.as_deref());
        return Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("{e}; Änderung verworfen"),
        ));
    }
    ctx.note(format!("{} gespeichert", path.display()));
    Ok(())
}

fn restore(path: &Path, before: Option<&[u8]>) {
    match before {
        Some(bytes) => {
            let _ = std::fs::write(path, bytes);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn rotate_local(ctx: &Ctx) -> CliResult {
    let token = beton_server::local_auth::LocalToken::load_or_create(&ctx.home)
        .map_err(|e| CliError::new(Exit::General, e))?;
    token
        .rotate()
        .map_err(|e| CliError::new(Exit::General, e))?;
    ctx.note(format!(
        "Lokales Token erneuert ({}). Verbundene Clients melden sich mit dem neuen Token an.",
        token.path().display()
    ));
    Ok(())
}

async fn rebuild_projections(ctx: &Ctx, session: Option<&str>) -> CliResult {
    let session = session
        .map(|s| {
            s.parse::<beton_core::id::SessionId>()
                .map_err(|e| CliError::usage(format!("ungültige Session-ID `{s}`: {e}")))
        })
        .transpose()?;
    let layers = ctx.layers()?;
    let settings = layers
        .daemon_settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let store = beton_store::Store::open(&ctx.home, crate::serve::store_options(&settings))
        .await
        .context("Datenbank öffnen")?;
    let local = store.ensure_local().await.context("lokale Identität")?;
    let count = store
        .rebuild_projections(local.org, session)
        .await
        .context("Projektionen neu aufbauen")?;
    store.close().await;
    if ctx.global.json {
        print_json(&json!({ "sessions": count }))
    } else {
        ctx.note(format!("Projektionen für {count} Session(s) neu aufgebaut"));
        Ok(())
    }
}

async fn record_golden(ctx: &Ctx, args: RecordGoldenArgs) -> CliResult {
    if args.harness != "claude" {
        return Err(CliError::usage(format!(
            "Für `{}` gibt es noch keine Aufnahme-Szenarien (verfügbar: claude)",
            args.harness
        )));
    }
    use beton_harness_claude::record;
    let all = record::scenarios();
    for wanted in &args.scenario {
        if !all.iter().any(|s| s.name == wanted) {
            let names: Vec<&str> = all.iter().map(|s| s.name).collect();
            return Err(CliError::usage(format!(
                "unbekanntes Szenario `{wanted}` (verfügbar: {})",
                names.join(", ")
            )));
        }
    }
    let out = args.out.unwrap_or_else(|| {
        ctx.cwd.join(format!(
            "crates/beton-harness-{}/tests/golden",
            args.harness
        ))
    });
    let version = record::cli_version()
        .await
        .map_err(|e| CliError::new(Exit::Harness, anyhow::anyhow!(e)))?;
    ctx.note(format!(
        "Nehme mit claude {version} nach {} auf …",
        out.display()
    ));
    for s in all
        .iter()
        .filter(|s| args.scenario.is_empty() || args.scenario.iter().any(|w| w == s.name))
    {
        record::record(s, &out, &version)
            .await
            .map_err(|e| CliError::new(Exit::Harness, anyhow::anyhow!(e)))?;
        ctx.note(format!("  {} aufgenommen", s.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_004_open_url_appends_session_route() {
        let base = "http://127.0.0.1:7420/auth/local/redeem?code=abc";
        assert_eq!(open_url(base, None).unwrap(), base);
        assert_eq!(
            open_url(base, Some("ses_01JB8Y2D0M3K4J5H6G7F8E9D0C")).unwrap(),
            format!("{base}&next=/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C")
        );
        // Keine beliebigen Pfade über die Session-ID.
        let err = open_url(base, Some("../../evil")).unwrap_err();
        assert_eq!(err.exit, Exit::Usage);
    }
}
