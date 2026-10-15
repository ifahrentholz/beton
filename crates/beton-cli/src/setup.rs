//! `beton setup` (CLI-005, HAR-016): nutzbare Harnesses ermitteln und fehlende Schritte
//! anbieten.
//!
//! - Installierte CLIs (Pfad, Version), Login-Status nur über das Statuskommando der CLI,
//!   API-Keys in der Umgebung (nur ob vorhanden, nie der Wert), lokale Modell-Server.
//! - Fehlende CLIs werden zur Installation **angeboten**: Befehl anzeigen, erst nach
//!   Bestätigung ausführen. `--non-interactive` und `--check` installieren nie etwas.

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

/// Legt die sicheren Policy-Defaults der User-Ebene an (POL-001):
/// `<beton_home>/policies/defaults.yaml` mit [`beton_policy::doc::USER_DEFAULTS`], wenn es die
/// Datei noch nicht gibt (eine vorhandene, auch geänderte, bleibt unangetastet), und das
/// Policy-Schema als lokale Datei `<beton_home>/schemas/v1/policy.schema.json`, auf die der
/// Kopfkommentar zeigt (ADR-0033). Liefert den Pfad, wenn die Datei neu ist.
pub fn write_policy_defaults(beton_home: &Path) -> std::io::Result<Option<PathBuf>> {
    let schema = beton_home.join(beton_policy::doc::SCHEMA_PATH);
    if let Some(dir) = schema.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&schema, beton_policy::doc::schema_json())?;
    let dir = beton_policy::store::policies_dir(beton_home);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("defaults.yaml");
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file);
    match created {
        Ok(mut f) => {
            f.write_all(beton_policy::doc::USER_DEFAULTS.as_bytes())?;
            Ok(Some(file))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
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

    #[test]
    fn pol_001_setup_writes_policy_defaults_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_policy_defaults(dir.path()).unwrap().unwrap();
        assert_eq!(file, dir.path().join("policies/defaults.yaml"));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            beton_policy::doc::USER_DEFAULTS
        );
        let schema = dir.path().join("schemas/v1/policy.schema.json");
        assert_eq!(
            std::fs::read_to_string(&schema).unwrap(),
            beton_policy::doc::schema_json()
        );
        // Lädt als Policy-Set der User-Ebene, wie der Runner es liest.
        beton_policy::load::load_file(beton_policy::load::Scope::User, &file).unwrap();
        // Eine geänderte Datei überschreibt setup nie.
        std::fs::write(&file, "spec_version: 1\nname: mine\nrules: []\n").unwrap();
        assert_eq!(write_policy_defaults(dir.path()).unwrap(), None);
        assert!(std::fs::read_to_string(&file).unwrap().contains("mine"));
    }

    /// Die Defaults verhindern bei keinem Katalog-Harness den Start (POL-024): Klassen, die ein
    /// Harness nicht vorab prüfen kann, ergeben nur eine Warnung (`enforcement: best_effort`).
    #[test]
    fn pol_001_user_defaults_do_not_refuse_any_catalog_harness() {
        let set = beton_policy::load::parse_set(
            beton_policy::load::Scope::User,
            "defaults.yaml",
            beton_policy::doc::USER_DEFAULTS,
        )
        .unwrap();
        let effective = beton_policy::engine::EffectiveSet::new(vec![std::sync::Arc::new(set)]);
        for harness in ["claude", "codex", "fake", "acp:gemini"] {
            let matrix = crate::policy::matrix_for(harness).unwrap();
            let check = beton_policy::enforcement::check_start(&effective, &matrix);
            assert!(check.refused.is_empty(), "{harness}: {check:?}");
        }
        // Direkt-Harness (Provider aus der Konfiguration): beton prüft jeden Call selbst.
        use beton_harness::HarnessAdapter as _;
        use beton_harness_direct::config::{KeyRef, Provider, WireKind};
        let direct = beton_harness_direct::DirectAdapter::new(
            Provider {
                name: "local".into(),
                wire: WireKind::Anthropic,
                base_url: "http://127.0.0.1:1".parse().unwrap(),
                key: KeyRef::None,
                prompt_caching: false,
                models: Vec::new(),
            },
            &beton_harness_direct::DirectOptions::default(),
        );
        let matrix = direct
            .capabilities(
                beton_harness::Mode::Native,
                &beton_harness::ProbeReport::default(),
            )
            .enforcement();
        let check = beton_policy::enforcement::check_start(&effective, &matrix);
        assert!(check.is_clean(), "direct: {check:?}");
    }

    /// Review #285 B2: Eine Allowlist (Default `deny` der Ebene) verweigert unter Codex den
    /// Start, weil MCP-Tools dort nur beobachtbar sind.
    #[test]
    fn pol_024_tool_allowlist_is_refused_under_codex() {
        let set = beton_policy::load::parse_set(
            beton_policy::load::Scope::Project,
            "p.yaml",
            "spec_version: 1\nname: p\nrules:\n  - id: list\n    type: tool_allowlist\n    params: { allow_kinds: [file_read] }\n",
        )
        .unwrap();
        let effective = beton_policy::engine::EffectiveSet::new(vec![std::sync::Arc::new(set)]);
        let codex = crate::policy::matrix_for("codex").unwrap();
        let check = beton_policy::enforcement::check_start(&effective, &codex);
        assert_eq!(check.refused_rules(), ["list"], "{check:?}");
        let claude = crate::policy::matrix_for("claude").unwrap();
        assert!(beton_policy::enforcement::check_start(&effective, &claude).is_clean());
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
