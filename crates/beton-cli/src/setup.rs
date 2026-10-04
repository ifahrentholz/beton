//! `beton setup` (CLI-005, HAR-016): nutzbare Harnesses ermitteln und fehlende Schritte
//! anbieten.
//!
//! - Installierte CLIs (Pfad, Version), Login-Status nur über das Statuskommando der CLI,
//!   API-Keys in der Umgebung (nur ob vorhanden, nie der Wert), lokale Modell-Server.
//! - Fehlende CLIs werden zur Installation **angeboten**: Befehl anzeigen, erst nach
//!   Bestätigung ausführen. `--non-interactive` und `--check` installieren nie etwas.

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use beton_harness::registry::{BinarySource, Registry, VersionProbe, resolve_binary};
use beton_harness::{AuthStatus, HarnessId, HostEnv};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Eine bekannte Vendor-CLI.
#[derive(Debug, Clone, Copy)]
pub struct VendorCli {
    pub id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    /// Offizieller Installationsbefehl des Vendors.
    pub install: &'static str,
    pub login: &'static [&'static str],
    pub api_key_env: &'static str,
    /// In diesem Release als Harness nutzbar (Codex folgt mit M1).
    pub supported: bool,
}

pub const VENDOR_CLIS: [VendorCli; 2] = [
    VendorCli {
        id: HarnessId::CLAUDE,
        name: "Claude Code",
        command: "claude",
        install: "npm install -g @anthropic-ai/claude-code",
        login: &["auth", "login"],
        api_key_env: "ANTHROPIC_API_KEY",
        supported: true,
    },
    VendorCli {
        id: HarnessId::CODEX,
        name: "Codex",
        command: "codex",
        install: "npm install -g @openai/codex",
        login: &["login"],
        api_key_env: "OPENAI_API_KEY",
        supported: false,
    },
];

/// Ergebnis von `beton setup --check --json` (Schema: `schemas/v1/setup-check.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupReport {
    pub harnesses: Vec<HarnessSetup>,
    /// Erreichbare lokale Modell-Server (Vorschläge für `providers.*`).
    pub local_servers: Vec<LocalServer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessSetup {
    pub id: String,
    pub name: String,
    /// In diesem Release als Harness nutzbar.
    pub supported: bool,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Woher der Pfad stammt (Env, Projekt-, User-Konfiguration, `PATH`).
    pub source: Option<BinarySource>,
    pub auth_status: AuthStatus,
    /// Ob der API-Key des Vendors in der Umgebung gesetzt ist; der Wert erscheint nie.
    pub api_key_env_found: bool,
    /// Installationsbefehl, falls nicht installiert.
    pub install_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalServer {
    pub id: String,
    pub url: String,
    /// Konfigurationsschlüssel des Vorschlags, z. B. `providers.ollama`.
    pub suggestion: String,
}

/// Bekannte lokale Modell-Server: Ollama und LM Studio (HAR-016).
pub fn default_local_servers() -> Vec<(&'static str, SocketAddr)> {
    vec![
        ("ollama", SocketAddr::from(([127, 0, 0, 1], 11434))),
        ("lmstudio", SocketAddr::from(([127, 0, 0, 1], 1234))),
    ]
}

/// Startet Prozesse für Installation und Login (im Test ersetzbar).
#[async_trait]
pub trait Spawner: Send + Sync {
    /// Führt das Programm im Vordergrund mit geerbtem Terminal aus; `true` bei Erfolg.
    async fn run(&self, program: &str, args: &[String]) -> bool;
}

/// Fragt den Nutzer (im Test ersetzbar).
pub trait Prompt {
    fn confirm(&mut self, question: &str) -> bool;
}

/// Echte Prozesse mit geerbtem Terminal.
#[derive(Debug, Default)]
pub struct RealSpawner;

#[async_trait]
impl Spawner for RealSpawner {
    async fn run(&self, program: &str, args: &[String]) -> bool {
        tokio::process::Command::new(program)
            .args(args)
            .status()
            .await
            .is_ok_and(|s| s.success())
    }
}

/// `[y/N]` auf stderr, Antwort von stdin.
#[derive(Debug, Default)]
pub struct TerminalPrompt;

impl Prompt for TerminalPrompt {
    fn confirm(&mut self, question: &str) -> bool {
        eprint!("{question} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(
            line.trim().to_lowercase().as_str(),
            "y" | "yes" | "j" | "ja"
        )
    }
}

/// Status eines Logins über `codex login status` (Textausgabe).
fn parse_codex_login(stdout: &[u8]) -> AuthStatus {
    let text = String::from_utf8_lossy(stdout).to_lowercase();
    if text.contains("not logged in") {
        AuthStatus::LoggedOut
    } else if text.contains("logged in") {
        AuthStatus::LoggedIn
    } else {
        AuthStatus::Unknown
    }
}

/// Ermittelt den Stand aller bekannten CLIs und lokalen Server; ändert nichts.
pub async fn check(
    env: &HostEnv,
    registry: &Registry,
    servers: &[(&str, SocketAddr)],
) -> SetupReport {
    let probe = VersionProbe::default();
    let mut harnesses = Vec::new();
    for cli in VENDOR_CLIS {
        let id: HarnessId = match cli.id.parse() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let resolved = resolve_binary(&id, cli.command, env);
        let (version, auth_status) = match (&resolved, registry.get(&id)) {
            (Some(_), Some(adapter)) => {
                let report = adapter.probe(env).await;
                (report.version, adapter.auth_status(env).await)
            }
            (Some(bin), None) => {
                let version = probe.version(&bin.program).await.ok();
                let auth = beton_harness::registry::run_status(
                    &bin.program,
                    &["login", "status"],
                    &[],
                    Duration::from_secs(5),
                )
                .await
                .map_or(AuthStatus::Unknown, |out| {
                    // Codex schreibt den Status auf stderr.
                    let mut text = out.stdout;
                    text.extend_from_slice(&out.stderr);
                    parse_codex_login(&text)
                });
                (version, auth)
            }
            (None, _) => (None, AuthStatus::Unknown),
        };
        harnesses.push(HarnessSetup {
            id: cli.id.to_owned(),
            name: cli.name.to_owned(),
            supported: cli.supported,
            installed: resolved.is_some(),
            path: resolved.as_ref().map(|b| b.program.display().to_string()),
            version,
            source: resolved.as_ref().map(|b| b.source),
            auth_status,
            api_key_env_found: std::env::var_os(cli.api_key_env).is_some_and(|v| !v.is_empty()),
            install_command: resolved.is_none().then(|| cli.install.to_owned()),
        });
    }
    let mut local_servers = Vec::new();
    for (id, addr) in servers {
        if probe_local_server(id, *addr).await {
            local_servers.push(LocalServer {
                id: (*id).to_owned(),
                url: format!("http://{addr}"),
                suggestion: format!("providers.{id}"),
            });
        }
    }
    SetupReport {
        harnesses,
        local_servers,
    }
}

/// Fragt einen lokalen Server ab (über das SDK: CLI-Code macht selbst kein HTTP, API-005).
pub async fn probe_local_server(id: &str, addr: SocketAddr) -> bool {
    beton_sdk::probe::local_server(id, addr).await
}

fn auth_text(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::LoggedIn => "angemeldet",
        AuthStatus::LoggedOut => "nicht angemeldet",
        AuthStatus::Unknown | AuthStatus::NotApplicable => "Anmeldung unbekannt",
    }
}

/// Darstellung, Angebote und Exit-Code. `interactive = false` installiert nie etwas.
/// Exit 0, wenn jede nutzbare CLI installiert ist, sonst 1.
pub async fn run(
    report: &SetupReport,
    interactive: bool,
    prompt: &mut dyn Prompt,
    spawner: &dyn Spawner,
    out: &mut dyn Write,
) -> std::io::Result<u8> {
    let mut missing = false;
    for h in &report.harnesses {
        let cli = VENDOR_CLIS.iter().find(|c| c.id == h.id);
        let later = if h.supported {
            ""
        } else {
            " (Harness folgt in M1)"
        };
        if h.installed {
            writeln!(
                out,
                "{}: installiert ({}, {}), {}{later}",
                h.name,
                h.version.as_deref().unwrap_or("Version unbekannt"),
                h.path.as_deref().unwrap_or("?"),
                auth_text(h.auth_status),
            )?;
            if h.auth_status == AuthStatus::LoggedOut
                && let Some(cli) = cli
            {
                let login = format!("{} {}", cli.command, cli.login.join(" "));
                writeln!(out, "  Anmelden mit: {login}")?;
                if interactive && prompt.confirm(&format!("{} jetzt anmelden?", h.name)) {
                    let args: Vec<String> = cli.login.iter().map(|a| (*a).to_owned()).collect();
                    let program = h.path.clone().unwrap_or_else(|| cli.command.to_owned());
                    if !spawner.run(&program, &args).await {
                        writeln!(out, "  Anmeldung nicht abgeschlossen")?;
                    }
                }
            }
        } else {
            let install = h.install_command.as_deref().unwrap_or_default();
            writeln!(
                out,
                "{}: nicht installiert{later}. Installieren mit: {install}",
                h.name
            )?;
            if h.supported {
                missing = true;
            }
            if interactive && prompt.confirm(&format!("{} jetzt installieren?", h.name)) {
                let (shell, flag) = if cfg!(windows) {
                    ("cmd", "/C")
                } else {
                    ("sh", "-c")
                };
                if spawner
                    .run(shell, &[flag.to_owned(), install.to_owned()])
                    .await
                {
                    writeln!(out, "  {} installiert", h.name)?;
                    if h.supported {
                        missing = false;
                    }
                } else {
                    writeln!(out, "  Installation fehlgeschlagen")?;
                }
            }
        }
        if h.api_key_env_found
            && let Some(cli) = cli
        {
            writeln!(
                out,
                "  {} ist gesetzt; beton nutzt standardmäßig die Anmeldung der CLI (Subscription)",
                cli.api_key_env
            )?;
        }
    }
    for s in &report.local_servers {
        writeln!(
            out,
            "Lokaler Modell-Server {} auf {} – Vorschlag: {}",
            s.id, s.url, s.suggestion
        )?;
    }
    Ok(u8::from(missing))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    #[async_trait]
    impl Spawner for Recorder {
        async fn run(&self, program: &str, args: &[String]) -> bool {
            self.0
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            true
        }
    }

    struct Answer(bool, Vec<String>);

    impl Prompt for Answer {
        fn confirm(&mut self, question: &str) -> bool {
            self.1.push(question.to_owned());
            self.0
        }
    }

    fn missing_codex() -> SetupReport {
        SetupReport {
            harnesses: vec![
                HarnessSetup {
                    id: "claude".into(),
                    name: "Claude Code".into(),
                    supported: true,
                    installed: true,
                    path: Some("/usr/bin/claude".into()),
                    version: Some("2.1.285".into()),
                    source: Some(BinarySource::Path),
                    auth_status: AuthStatus::LoggedIn,
                    api_key_env_found: false,
                    install_command: None,
                },
                HarnessSetup {
                    id: "codex".into(),
                    name: "Codex".into(),
                    supported: false,
                    installed: false,
                    path: None,
                    version: None,
                    source: None,
                    auth_status: AuthStatus::Unknown,
                    api_key_env_found: false,
                    install_command: Some("npm install -g @openai/codex".into()),
                },
            ],
            local_servers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn har_016_ac1_missing_codex_is_offered_but_not_installed_without_consent() {
        let spawner = Recorder::default();
        let mut prompt = Answer(false, Vec::new());
        let mut out = Vec::new();
        run(&missing_codex(), true, &mut prompt, &spawner, &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Codex: nicht installiert"), "{text}");
        assert!(text.contains("npm install -g @openai/codex"), "{text}");
        assert_eq!(prompt.1, vec!["Codex jetzt installieren?".to_owned()]);
        assert!(
            spawner.0.lock().unwrap().is_empty(),
            "ohne Zustimmung nichts ausgeführt"
        );

        // Mit Zustimmung läuft genau der angezeigte Befehl.
        let mut prompt = Answer(true, Vec::new());
        run(
            &missing_codex(),
            true,
            &mut prompt,
            &spawner,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        let calls = spawner.0.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].ends_with("npm install -g @openai/codex"),
            "{calls:?}"
        );
    }

    #[tokio::test]
    async fn cli_005_ac1_non_interactive_never_installs_and_fails_on_missing_cli() {
        let mut report = missing_codex();
        report.harnesses[0].installed = false;
        report.harnesses[0].install_command =
            Some("npm install -g @anthropic-ai/claude-code".into());
        let spawner = Recorder::default();
        let mut prompt = Answer(true, Vec::new());
        let code = run(&report, false, &mut prompt, &spawner, &mut Vec::new())
            .await
            .unwrap();
        assert_ne!(code, 0);
        assert!(prompt.1.is_empty(), "keine Rückfrage");
        assert!(spawner.0.lock().unwrap().is_empty(), "nichts installiert");
        // Nur Codex (noch nicht nutzbar) fehlt: kein Fehler.
        let code = run(
            &missing_codex(),
            false,
            &mut prompt,
            &spawner,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn har_016_ac4_running_ollama_is_suggested() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\n\r\nOllama is running")
                    .await;
            }
        });
        let report = check(
            &HostEnv::default(),
            &Registry::new(beton_harness::registry::RegistryOptions { dev: false }),
            &[("ollama", addr)],
        )
        .await;
        assert_eq!(
            report.local_servers,
            vec![LocalServer {
                id: "ollama".into(),
                url: format!("http://{addr}"),
                suggestion: "providers.ollama".into(),
            }]
        );
        // Ein anderer Dienst auf dem Port ist kein Ollama; ein freier Port ist nichts.
        let free = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(!probe_local_server("ollama", free).await);
    }

    #[test]
    fn codex_login_text_is_parsed() {
        assert_eq!(
            parse_codex_login(b"Logged in using ChatGPT"),
            AuthStatus::LoggedIn
        );
        assert_eq!(parse_codex_login(b"Not logged in"), AuthStatus::LoggedOut);
        assert_eq!(parse_codex_login(b"?"), AuthStatus::Unknown);
    }
}
