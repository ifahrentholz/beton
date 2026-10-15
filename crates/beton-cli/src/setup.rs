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
use beton_secrets::{SecretValue, mask};
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
    /// In diesem Release als Harness nutzbar.
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
        supported: true,
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
    #[expect(
        clippy::disallowed_methods,
        reason = "Stufe 0: Installation und Login einer Vendor-CLI im Vordergrund auf Wunsch des Benutzers"
    )]
    async fn run(&self, program: &str, args: &[String]) -> bool {
        tokio::process::Command::new(program)
            .args(args)
            .status()
            .await
            .is_ok_and(|s| s.success())
    }
}

/// Fragt eine CLI ab und liefert Erfolg und stdout (im Test ersetzbar, damit kein Test die
/// echten `gh`-/`glab`-Konfigurationen des Entwicklers liest).
#[async_trait]
pub trait CliOutput: Send + Sync {
    /// Liegt das Programm im `PATH`? Startet nichts.
    fn installed(&self, program: &str) -> bool;
    /// `None`: Programm nicht startbar.
    async fn output(&self, program: &str, args: &[&str]) -> Option<(bool, Vec<u8>)>;
}

#[async_trait]
impl CliOutput for RealSpawner {
    fn installed(&self, program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "Stufe 0: Token einer Git-CLI nach Zustimmung des Benutzers übernehmen (SEC-011)"
    )]
    async fn output(&self, program: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
        let run = tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(Duration::from_secs(10), run).await {
            Ok(Ok(out)) => Some((out.status.success(), out.stdout)),
            _ => None,
        }
    }
}

/// Eine Git-CLI, deren Login `beton setup` übernehmen kann (SEC-011).
#[derive(Debug, Clone, Copy)]
pub struct GitCli {
    pub command: &'static str,
    pub host: &'static str,
    /// Typ des Credential-Bindings (`sandbox.credentials[].type`).
    pub kind: &'static str,
    /// Variable der Stufe 2, die den Platzhalter bekommt.
    pub env: &'static str,
    /// Gibt den Token auf stdout aus (ohne Netzzugriff); läuft erst nach Zustimmung.
    pub token: &'static [&'static str],
}

pub const GIT_CLIS: [GitCli; 2] = [
    GitCli {
        command: "gh",
        host: "github.com",
        kind: "github",
        env: "GH_TOKEN",
        token: &["auth", "token", "--hostname", "github.com"],
    },
    GitCli {
        command: "glab",
        host: "gitlab.com",
        kind: "gitlab",
        env: "GITLAB_TOKEN",
        token: &["config", "get", "token", "--host", "gitlab.com"],
    },
];

/// Bietet an, den Login installierter `gh`-/`glab`-CLIs in den Secret-Store zu übernehmen
/// (SEC-011 AC2): nie still und nie im nicht-interaktiven Modus. Die CLI wird erst nach der
/// Zustimmung gefragt (ohne Netzzugriff, ADR-0033); der Token wird nie ausgegeben.
pub async fn offer_git_token_import(
    clis: &[GitCli],
    probe: &dyn CliOutput,
    interactive: bool,
    prompt: &mut dyn Prompt,
    importer: &dyn KeyImporter,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    for cli in clis {
        if !probe.installed(cli.command) {
            continue;
        }
        let secret_ref = format!("secret://user/{}/default", cli.host);
        if !interactive {
            writeln!(
                out,
                "{} gefunden; Login übernehmen: beton setup (interaktiv) oder `{} {} | beton secrets add {} --stdin`",
                cli.command,
                cli.command,
                cli.token.join(" "),
                cli.host
            )?;
            continue;
        }
        if !prompt.confirm(&format!(
            "{} ist installiert: dessen Login ({}) als {secret_ref} in den Secret-Store übernehmen?",
            cli.command, cli.host
        )) {
            writeln!(out, "  {}: nicht übernommen", cli.command)?;
            continue;
        }
        let token = match probe.output(cli.command, cli.token).await {
            Some((true, stdout)) => SecretValue::from_bytes(stdout.trim_ascii().to_vec()),
            _ => SecretValue::from_bytes(Vec::new()),
        };
        if token.is_empty() {
            writeln!(
                out,
                "  {}: nicht übernommen (kein Login für {})",
                cli.command, cli.host
            )?;
            continue;
        }
        match importer.import(&secret_ref, &token).await {
            Ok(()) => {
                writeln!(out, "  {}: übernommen als {secret_ref}", cli.command)?;
                writeln!(
                    out,
                    "  Für Tools in der Sandbox (nur als Platzhalter) in ~/.beton/config.yaml unter \
                     sandbox.credentials eintragen:\n    - {{ name: {}, type: {}, source: {{ secret: \"{secret_ref}\" }}, env: [{}] }}",
                    cli.kind, cli.kind, cli.env
                )?;
            }
            Err(e) => writeln!(out, "  {}: nicht übernommen: {e}", cli.command)?,
        }
    }
    Ok(())
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

/// API-Keys in der Umgebung, die `beton setup` in den Secret-Store übernehmen kann
/// (HAR-016 AC3): Env-Variable und Provider der SecretRef `secret://user/<provider>/default`.
pub const ENV_KEYS: [(&str, &str); 4] = [
    ("ANTHROPIC_API_KEY", "anthropic"),
    ("OPENAI_API_KEY", "openai"),
    ("GEMINI_API_KEY", "gemini"),
    ("OPENROUTER_API_KEY", "openrouter"),
];

/// Ein in der Umgebung gefundener API-Key.
#[derive(Debug)]
pub struct FoundKey {
    pub env: &'static str,
    pub provider: &'static str,
    pub value: SecretValue,
}

/// Gesetzte API-Keys aus der Prozessumgebung.
pub fn found_env_keys() -> Vec<FoundKey> {
    ENV_KEYS
        .iter()
        .filter_map(|(env, provider)| {
            let value = std::env::var(env).ok().filter(|v| !v.is_empty())?;
            Some(FoundKey {
                env,
                provider,
                value: SecretValue::from(value),
            })
        })
        .collect()
}

/// Speichert einen Key im Secret-Store (im Test ersetzbar).
#[async_trait]
pub trait KeyImporter: Send + Sync {
    /// Fehlermeldungen enthalten nie den Wert.
    async fn import(&self, secret_ref: &str, value: &SecretValue) -> Result<(), String>;
}

/// Bietet an, gefundene API-Keys in den Secret-Store (Keychain) zu übernehmen – je Key nur
/// nach ausdrücklicher Zustimmung, nie im nicht-interaktiven Modus. Ausgaben zeigen den Wert
/// nur maskiert (`sk-…abcd`).
pub async fn offer_key_import(
    found: &[FoundKey],
    interactive: bool,
    prompt: &mut dyn Prompt,
    importer: &dyn KeyImporter,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    for key in found {
        let secret_ref = format!("secret://user/{}/default", key.provider);
        let masked = mask(&key.value);
        writeln!(out, "{} gefunden ({masked})", key.env)?;
        if !interactive {
            writeln!(
                out,
                "  Übernehmen: beton setup (interaktiv) oder beton secrets add {} --stdin",
                key.provider
            )?;
            continue;
        }
        if !prompt.confirm(&format!(
            "{} ({masked}) als {secret_ref} in den Secret-Store übernehmen?",
            key.env
        )) {
            writeln!(out, "  nicht übernommen")?;
            continue;
        }
        match importer.import(&secret_ref, &key.value).await {
            Ok(()) => writeln!(out, "  übernommen als {secret_ref}")?,
            Err(e) => writeln!(out, "  nicht übernommen: {e}")?,
        }
    }
    Ok(())
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
            " (Harness folgt später)"
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

    #[derive(Default)]
    struct Imports(Mutex<Vec<(String, String)>>);

    #[async_trait]
    impl KeyImporter for Imports {
        async fn import(&self, secret_ref: &str, value: &SecretValue) -> Result<(), String> {
            self.0.lock().unwrap().push((
                secret_ref.to_owned(),
                String::from_utf8(value.expose().to_vec()).unwrap(),
            ));
            Ok(())
        }
    }

    const KEY: &str = "sk-proj-0123456789ABCDEFGHIJabcd";

    fn found() -> Vec<FoundKey> {
        vec![FoundKey {
            env: "OPENAI_API_KEY",
            provider: "openai",
            value: SecretValue::from(KEY),
        }]
    }

    #[tokio::test]
    async fn har_016_ac3_env_key_is_imported_only_with_consent_and_shown_masked() {
        // Ohne Zustimmung: nichts übernommen.
        let imports = Imports::default();
        let mut prompt = Answer(false, Vec::new());
        let mut out = Vec::new();
        offer_key_import(&found(), true, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            imports.0.lock().unwrap().is_empty(),
            "ohne Zustimmung nichts gespeichert"
        );
        assert_eq!(prompt.1.len(), 1);
        assert!(prompt.1[0].contains("sk-…abcd"), "{:?}", prompt.1);
        assert!(text.contains("sk-…abcd"), "{text}");
        for leak in [KEY, "0123456789", "sk-proj"] {
            assert!(!text.contains(leak), "{text}");
            assert!(!prompt.1[0].contains(leak), "{:?}", prompt.1);
        }

        // Mit Zustimmung: genau dieser Wert unter secret://user/openai/default.
        let mut prompt = Answer(true, Vec::new());
        let mut out = Vec::new();
        offer_key_import(&found(), true, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert_eq!(
            *imports.0.lock().unwrap(),
            vec![("secret://user/openai/default".to_owned(), KEY.to_owned())]
        );
        assert!(!String::from_utf8(out).unwrap().contains(KEY));

        // Nicht interaktiv: keine Rückfrage, nichts übernommen.
        let imports = Imports::default();
        let mut prompt = Answer(true, Vec::new());
        let mut out = Vec::new();
        offer_key_import(&found(), false, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert!(prompt.1.is_empty());
        assert!(imports.0.lock().unwrap().is_empty());
    }

    /// `gh`/`glab`-Attrappe: zeichnet Aufrufe auf; liest nie echte Konfigurationen.
    struct FakeGit {
        logged_in: bool,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CliOutput for FakeGit {
        fn installed(&self, program: &str) -> bool {
            program == "gh"
        }

        async fn output(&self, program: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            match (program, self.logged_in) {
                ("gh", true) => Some((true, b"gho_FIXTUREgh0123456789\n".to_vec())),
                ("gh", false) => Some((false, Vec::new())),
                _ => None,
            }
        }
    }

    #[tokio::test]
    async fn sec_011_ac2_gh_token_is_taken_over_only_after_explicit_confirmation() {
        let git = FakeGit {
            logged_in: true,
            calls: Mutex::default(),
        };
        // Ohne Zustimmung: nichts gespeichert, gh nicht einmal gefragt.
        let imports = Imports::default();
        let mut prompt = Answer(false, Vec::new());
        let mut out = Vec::new();
        offer_git_token_import(&GIT_CLIS, &git, true, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert!(imports.0.lock().unwrap().is_empty());
        assert_eq!(prompt.1.len(), 1, "genau eine Rückfrage für gh");
        assert!(git.calls.lock().unwrap().is_empty());
        // Nicht interaktiv: keine Rückfrage, nichts gelesen, nichts gespeichert.
        let mut prompt = Answer(true, Vec::new());
        let mut out = Vec::new();
        offer_git_token_import(&GIT_CLIS, &git, false, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert!(prompt.1.is_empty());
        assert!(imports.0.lock().unwrap().is_empty());
        assert!(git.calls.lock().unwrap().is_empty());
        assert!(String::from_utf8(out).unwrap().contains("gh auth token"));
        // Mit Zustimmung: genau dieser Token unter secret://user/github.com/default.
        let mut prompt = Answer(true, Vec::new());
        let mut out = Vec::new();
        offer_git_token_import(&GIT_CLIS, &git, true, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert_eq!(
            *imports.0.lock().unwrap(),
            vec![(
                "secret://user/github.com/default".to_owned(),
                "gho_FIXTUREgh0123456789".to_owned()
            )]
        );
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("FIXTURE"), "{text}");
        assert!(text.contains("sandbox.credentials"), "{text}");
        // Nicht angemeldet: nichts gespeichert.
        let git = FakeGit {
            logged_in: false,
            calls: Mutex::default(),
        };
        let mut prompt = Answer(true, Vec::new());
        let imports = Imports::default();
        let mut out = Vec::new();
        offer_git_token_import(&GIT_CLIS, &git, true, &mut prompt, &imports, &mut out)
            .await
            .unwrap();
        assert!(imports.0.lock().unwrap().is_empty());
        assert!(String::from_utf8(out).unwrap().contains("kein Login"));
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
