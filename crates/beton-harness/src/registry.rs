//! Harness-Registry, Binary-Auflösung, Versionsprüfung und Katalog (HAR-002, HAR-003).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::adapter::{
    AdapterContext, HarnessAdapter, HarnessError, HarnessSession, HostEnv, Mode, ProbeReport,
    SessionSpec,
};
use crate::capabilities::Capabilities;
use crate::id::HarnessId;

/// Ein Eintrag unter `harnesses.<id>` in `.beton/config.yaml` bzw. `~/.beton/config.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessCommandConfig {
    /// Programm (Name in `PATH` oder Pfad), HAR-003.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_passthrough: Vec<String>,
    /// Auth-Herkunft (HAR-015); ohne Angabe `subscription`. Nur in der User-Konfiguration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<HarnessAuth>,
    /// Benutzer-Anpassungen der Vendor-CLI (Hooks, Skills, Plugins, MCP) abschalten (HAR-004).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolated: Option<bool>,
}

/// Auth-Herkunft in der Konfiguration (HAR-015): `subscription` = Login der offiziellen CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HarnessAuth {
    Subscription,
    ApiKey,
}

impl HarnessAuth {
    pub fn source(self) -> beton_core::event::AuthSource {
        match self {
            HarnessAuth::Subscription => beton_core::event::AuthSource::VendorCli,
            HarnessAuth::ApiKey => beton_core::event::AuthSource::ApiKey,
        }
    }
}

/// Abschnitt `harnesses:` der Konfiguration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HarnessesConfig {
    /// Standard-Harness für neue Sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, HarnessCommandConfig>,
}

impl HarnessesConfig {
    /// Liest `harnesses:` aus `<dir>/.beton/config.yaml`; andere Abschnitte bleiben unbeachtet.
    /// Fehlt die Datei, ist das Ergebnis leer.
    pub fn load_project(dir: &Path) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            harnesses: HarnessesConfig,
        }
        let path = dir.join(".beton").join("config.yaml");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_yaml_ng::from_str::<File>(&text)
            .map(|f| f.harnesses)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Auth-Herkunft eines Harness; Default `subscription` (ADR-0034).
    pub fn auth(&self, id: &str) -> HarnessAuth {
        self.entries
            .get(id)
            .and_then(|e| e.auth)
            .unwrap_or(HarnessAuth::Subscription)
    }
}

/// Konfigurationsebenen, die der Daemon an einen Runner weitergibt (JSON in
/// `BETON_RUNNER_HARNESSES`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessLayers {
    #[serde(default)]
    pub user: HarnessesConfig,
    #[serde(default)]
    pub project: HarnessesConfig,
}

/// Woher ein Binary-Pfad stammt (Präzedenz von oben nach unten, HAR-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum BinarySource {
    Env,
    ProjectConfig,
    UserConfig,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBinary {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub source: BinarySource,
}

/// Löst das Binary eines Harness auf: `BETON_<NAME>_PATH` → Projekt-Config → User-Config →
/// `PATH` (mit `default_command`). Ein Env-Wert, der keine existierende Datei ist, wird an
/// Leerzeichen in Programm und Argumente geteilt (z. B. `beton-fake-cli --protocol …`).
pub fn resolve_binary(
    id: &HarnessId,
    default_command: &str,
    env: &HostEnv,
) -> Option<ResolvedBinary> {
    if let Some(value) = env
        .vars
        .get(&id.path_env_var())
        .filter(|v| !v.trim().is_empty())
    {
        let (program, args) = split_command(value);
        return Some(ResolvedBinary {
            program: find_program(&program, env)?,
            args,
            source: BinarySource::Env,
        });
    }
    for (config, source) in [
        (&env.project, BinarySource::ProjectConfig),
        (&env.user, BinarySource::UserConfig),
    ] {
        if let Some(entry) = config.entries.get(id.as_str())
            && let Some(command) = &entry.command
        {
            return Some(ResolvedBinary {
                program: find_program(command, env)?,
                args: entry.extra_args.clone(),
                source,
            });
        }
    }
    Some(ResolvedBinary {
        program: find_program(default_command, env)?,
        args: Vec::new(),
        source: BinarySource::Path,
    })
}

fn split_command(value: &str) -> (String, Vec<String>) {
    if Path::new(value).is_file() {
        return (value.to_owned(), Vec::new());
    }
    let mut parts = value.split_whitespace().map(str::to_owned);
    let program = parts.next().unwrap_or_default();
    (program, parts.collect())
}

/// Pfad mit Trennzeichen direkt, sonst Suche in `PATH` (unter Windows mit `.exe`).
fn find_program(command: &str, env: &HostEnv) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let dirs = env.path.as_ref()?;
    for dir in std::env::split_paths(dirs) {
        for candidate in [dir.join(command), dir.join(format!("{command}.exe"))] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Ausgabe eines Status-Kommandos.
#[derive(Debug, Clone, Default)]
pub struct StatusOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Führt ein Status-Kommando einer Vendor-CLI aus (z. B. `claude auth status --json`).
/// Ohne stdin, mit Timeout; die Ausgabe wird nicht geloggt (sie kann Kontodaten enthalten).
pub async fn run_status(
    program: &Path,
    args: &[&str],
    remove_env: &[&str],
    timeout: Duration,
) -> Result<StatusOutput, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for k in remove_env {
        cmd.env_remove(k);
    }
    match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => Err(format!(
            "`{}` antwortet nicht innerhalb von {timeout:?}",
            program.display()
        )),
        Ok(Err(e)) => Err(format!("`{}`: {e}", program.display())),
        Ok(Ok(out)) => Ok(StatusOutput {
            success: out.status.success(),
            stdout: out.stdout,
            stderr: out.stderr,
        }),
    }
}

/// Cache-Schlüssel: Pfad und Änderungszeit des Binaries.
type ProbeKey = (PathBuf, Option<SystemTime>);

/// Ermittelt CLI-Versionen über `<bin> --version` mit Timeout und Cache pro (Pfad, mtime).
#[derive(Debug, Clone)]
pub struct VersionProbe {
    timeout: Duration,
    cache: Arc<Mutex<HashMap<ProbeKey, Result<String, String>>>>,
}

impl Default for VersionProbe {
    fn default() -> Self {
        Self::with_timeout(Duration::from_secs(5))
    }
}

impl VersionProbe {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cache: Arc::default(),
        }
    }

    /// Version als SemVer-String oder Begründung, warum sie nicht ermittelbar war.
    pub async fn version(&self, program: &Path) -> Result<String, String> {
        let mtime = std::fs::metadata(program).and_then(|m| m.modified()).ok();
        let key = (program.to_path_buf(), mtime);
        if let Some(hit) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return hit;
        }
        let result = self.run(program).await;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, result.clone());
        }
        result
    }

    async fn run(&self, program: &Path) -> Result<String, String> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = match tokio::time::timeout(self.timeout, cmd.output()).await {
            Err(_) => {
                return Err(format!(
                    "`{} --version` antwortet nicht innerhalb von {:?}",
                    program.display(),
                    self.timeout
                ));
            }
            Ok(Err(e)) => return Err(format!("`{} --version`: {e}", program.display())),
            Ok(Ok(o)) => o,
        };
        let text = String::from_utf8_lossy(&output.stdout);
        extract_version(&text)
            .ok_or_else(|| format!("keine Version in der Ausgabe von --version: {text:?}"))
    }
}

/// Erste SemVer-ähnliche Angabe, z. B. `2.1.4` aus `2.1.4 (Claude Code)`.
pub fn extract_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|w| w.trim_start_matches('v'))
        .find(|w| semver::Version::parse(w).is_ok())
        .map(str::to_owned)
}

/// Grund, warum eine Version nicht zum unterstützten Bereich passt.
pub fn incompatibility(caps: &Capabilities, probe: &ProbeReport) -> Option<(String, String)> {
    let range = caps.version_range.as_ref()?;
    let version = probe.version.as_ref()?;
    let req = semver::VersionReq::parse(range).ok()?;
    let v = semver::Version::parse(version).ok()?;
    (!req.matches(&v)).then(|| (version.clone(), range.clone()))
}

/// Ein Katalogeintrag für `GET /v1/harnesses?host=<id>` (HAR-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct HarnessInfo {
    pub id: HarnessId,
    pub modes: Vec<Mode>,
    /// Je Modus, nach dem Probe verfeinert.
    pub capabilities: Vec<Capabilities>,
    pub probe: ProbeReport,
    pub incompatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub incompatible_reason: Option<String>,
}

/// Alle verfügbaren Harnesses eines Hosts.
#[derive(Default, Clone)]
pub struct Registry {
    adapters: BTreeMap<HarnessId, Arc<dyn HarnessAdapter>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.adapters.keys()).finish()
    }
}

/// Einstellungen der Registry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegistryOptions {
    /// Entwicklermodus (`--dev` bzw. Debug-Build): nur dann gibt es den Fake-Harness
    /// (HAR-026 AC3).
    pub dev: bool,
}

impl Registry {
    /// Registry mit den eingebauten Adaptern dieses Crates. Vendor-Adapter registriert der
    /// Aufrufer selbst (sie liegen in eigenen Crates).
    pub fn new(options: RegistryOptions) -> Self {
        let mut registry = Self::default();
        if options.dev {
            registry.register(Arc::new(crate::fake::FakeAdapter));
        }
        registry
    }

    pub fn register(&mut self, adapter: Arc<dyn HarnessAdapter>) {
        self.adapters.insert(adapter.id(), adapter);
    }

    pub fn get(&self, id: &HarnessId) -> Option<&Arc<dyn HarnessAdapter>> {
        self.adapters.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &HarnessId> {
        self.adapters.keys()
    }

    /// Katalog aller Harnesses mit Probe-Ergebnis. Probes laufen parallel und blockieren
    /// den Aufrufer höchstens bis zum Probe-Timeout.
    pub async fn catalog(&self, env: &HostEnv) -> Vec<HarnessInfo> {
        let probes = self.adapters.values().map(|adapter| {
            let adapter = adapter.clone();
            let env = env.clone();
            tokio::spawn(async move {
                let probe = adapter.probe(&env).await;
                info(adapter.as_ref(), probe)
            })
        });
        let mut out = Vec::new();
        for handle in probes.collect::<Vec<_>>() {
            if let Ok(info) = handle.await {
                out.push(info);
            }
        }
        out
    }

    /// Startet eine Session; inkompatible Versionen werden abgelehnt (HAR-002 AC2).
    pub async fn start(
        &self,
        id: &HarnessId,
        spec: SessionSpec,
        ctx: AdapterContext,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let adapter = self
            .adapters
            .get(id)
            .ok_or_else(|| HarnessError::NotFound(id.to_string()))?;
        let probe = adapter.probe(&ctx.env).await;
        if !probe.installed {
            return Err(HarnessError::NotFound(format!(
                "{id} ist nicht installiert"
            )));
        }
        let mode = spec.mode.unwrap_or(Mode::Native);
        if let Some((detected, expected)) =
            incompatibility(&adapter.capabilities(mode, &probe), &probe)
        {
            return Err(HarnessError::Incompatible { detected, expected });
        }
        adapter.start(spec, ctx).await
    }
}

fn info(adapter: &dyn HarnessAdapter, probe: ProbeReport) -> HarnessInfo {
    let capabilities: Vec<Capabilities> = adapter
        .modes()
        .iter()
        .map(|m| adapter.capabilities(*m, &probe))
        .collect();
    let reason = capabilities
        .iter()
        .find_map(|c| incompatibility(c, &probe))
        .map(|(v, r)| format!("Version {v} liegt nicht in {r}"));
    HarnessInfo {
        id: adapter.id(),
        modes: adapter.modes().to_vec(),
        capabilities,
        incompatible: reason.is_some(),
        incompatible_reason: reason,
        probe,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn har_003_project_config_is_read_from_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            HarnessesConfig::load_project(dir.path()).unwrap(),
            HarnessesConfig::default(),
            "ohne Datei leer"
        );
        std::fs::create_dir_all(dir.path().join(".beton")).unwrap();
        std::fs::write(
            dir.path().join(".beton/config.yaml"),
            "events:\n  store_raw: false\nharnesses:\n  claude:\n    command: ./bin/claude\n    isolated: true\n",
        )
        .unwrap();
        let cfg = HarnessesConfig::load_project(dir.path()).unwrap();
        assert_eq!(
            cfg.entries["claude"].command.as_deref(),
            Some("./bin/claude")
        );
        assert_eq!(cfg.entries["claude"].isolated, Some(true));
        assert_eq!(cfg.auth("claude"), HarnessAuth::Subscription);
        std::fs::write(
            dir.path().join(".beton/config.yaml"),
            "harnesses:\n  claude:\n    unbekannt: 1\n",
        )
        .unwrap();
        let err = HarnessesConfig::load_project(dir.path()).unwrap_err();
        assert!(err.contains("config.yaml"), "{err}");
    }

    use super::*;

    fn exe(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn har_003_ac1_binary_precedence_env_project_user_path() {
        let dir = tempfile::tempdir().unwrap();
        let (env_bin, project_bin, user_bin) = (
            exe(dir.path(), "claude-env", ""),
            exe(dir.path(), "claude-project", ""),
            exe(dir.path(), "claude-user", ""),
        );
        let path_dir = dir.path().join("bin");
        std::fs::create_dir(&path_dir).unwrap();
        let path_bin = exe(&path_dir, "claude", "");
        let id: HarnessId = "claude".parse().unwrap();
        let entry = |p: &Path| HarnessCommandConfig {
            command: Some(p.display().to_string()),
            ..HarnessCommandConfig::default()
        };
        let mut env = HostEnv {
            path: Some(path_dir.into_os_string()),
            ..HostEnv::default()
        };
        env.vars
            .insert("BETON_CLAUDE_PATH".into(), env_bin.display().to_string());
        env.project
            .entries
            .insert("claude".into(), entry(&project_bin));
        env.user.entries.insert("claude".into(), entry(&user_bin));

        let resolve = |env: &HostEnv| resolve_binary(&id, "claude", env).unwrap();
        assert_eq!(resolve(&env).program, env_bin);
        assert_eq!(resolve(&env).source, BinarySource::Env);
        env.vars.clear();
        assert_eq!(resolve(&env).program, project_bin);
        assert_eq!(resolve(&env).source, BinarySource::ProjectConfig);
        env.project.entries.clear();
        assert_eq!(resolve(&env).program, user_bin);
        assert_eq!(resolve(&env).source, BinarySource::UserConfig);
        env.user.entries.clear();
        assert_eq!(resolve(&env).program, path_bin);
        assert_eq!(resolve(&env).source, BinarySource::Path);
    }

    #[test]
    fn env_value_with_arguments_is_split() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "beton-fake-cli", "");
        let mut env = HostEnv {
            path: Some(dir.path().as_os_str().to_owned()),
            ..HostEnv::default()
        };
        env.vars.insert(
            "BETON_CLAUDE_PATH".into(),
            "beton-fake-cli --protocol stream-json --scenario s.yaml".into(),
        );
        let r = resolve_binary(&"claude".parse().unwrap(), "claude", &env).unwrap();
        assert!(r.program.ends_with("beton-fake-cli"));
        assert_eq!(
            r.args,
            ["--protocol", "stream-json", "--scenario", "s.yaml"]
        );
    }

    #[test]
    fn harnesses_config_parses_spec_example() {
        let yaml = "claude: { command: /opt/homebrew/bin/claude, extra_args: [], env_passthrough: [] }\ncodex:  { command: codex }\ndefault: claude\n";
        let cfg: HarnessesConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(cfg.default.as_deref(), Some("claude"));
        assert_eq!(cfg.entries["codex"].command.as_deref(), Some("codex"));
    }

    #[test]
    fn versions_are_extracted() {
        assert_eq!(
            extract_version("2.1.4 (Claude Code)\n").as_deref(),
            Some("2.1.4")
        );
        assert_eq!(
            extract_version("codex-cli 0.46.0").as_deref(),
            Some("0.46.0")
        );
        assert_eq!(extract_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(extract_version("unbekannt"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn har_003_ac2_hanging_version_probe_fails_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let slow = exe(dir.path(), "langsam", "sleep 30");
        let probe = VersionProbe::with_timeout(Duration::from_millis(200));
        let started = std::time::Instant::now();
        let err = probe.version(&slow).await.unwrap_err();
        assert!(err.contains("antwortet nicht"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn har_003_version_is_cached_per_path_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("aufrufe");
        let bin = exe(
            dir.path(),
            "cli",
            &format!("echo x >> '{}'; echo '2.0.1 (Test)'", counter.display()),
        );
        let probe = VersionProbe::default();
        assert_eq!(probe.version(&bin).await.unwrap(), "2.0.1");
        assert_eq!(probe.version(&bin).await.unwrap(), "2.0.1");
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            1
        );
    }
}
