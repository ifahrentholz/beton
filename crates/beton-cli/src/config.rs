//! Konfiguration (CLI-008): Ebenen `default` < `user` < `project` < `env`.
//!
//! - User: `~/.beton/config.yaml` (bzw. `$BETON_HOME/config.yaml` oder `--config FILE`).
//! - Projekt: `.beton/config.yaml` im aktuellen Verzeichnis oder darüber.
//! - Env: `BETON_CFG_<SCHLÜSSEL>` mit `__` als Trenner, z. B. `BETON_CFG_EVENTS__STORE_RAW=false`.
//!
//! Das Projekt (also das Repository) darf nur `harnesses.*` setzen und dort keine Auth-Herkunft:
//! daemonweite Einstellungen und die Wahl zwischen Subscription und API-Key bleiben beim
//! Benutzer (HAR-015). Geprüft wird gegen [`Settings`], aus dem auch das veröffentlichte
//! JSON-Schema `schemas/v1/config.schema.json` entsteht.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use beton_harness::registry::HarnessesConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Präfix der Env-Ebene.
pub const ENV_PREFIX: &str = "BETON_CFG_";

/// Die effektive Konfiguration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "beton-Konfiguration (config.yaml)")]
pub struct Settings {
    pub server: ServerSettings,
    pub auth: AuthSettings,
    pub events: EventsSettings,
    /// Harnesses: Binary, Argumente, Auth-Herkunft (HAR-003, HAR-015).
    pub harnesses: HarnessesConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ServerSettings {
    /// TCP-Adressen; im lokalen Modus nur Loopback (AUTH-001).
    pub listen: Vec<SocketAddr>,
    /// Zusätzlich erlaubte `Host`-Header (AUTH-002).
    pub allowed_hosts: Vec<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        let port = beton_server::config::DEFAULT_PORT;
        Self {
            listen: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
            ],
            allowed_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct AuthSettings {
    /// Zusätzlich erlaubte Origins für WebSocket und Cookie-Requests (AUTH-003).
    pub ws_allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct EventsSettings {
    /// Original-Payloads der Harnesses speichern (PROTO-001 AC3).
    pub store_raw: bool,
}

impl Default for EventsSettings {
    fn default() -> Self {
        Self { store_raw: true }
    }
}

/// Herkunft eines Werts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Default,
    User,
    Project,
    Env,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::User => "user",
            Source::Project => "project",
            Source::Env => "env",
        }
    }
}

/// Schreibbare Ebene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: kein gültiges YAML: {message}")]
    Parse { path: String, message: String },
    #[error("{origin}: {key}: {message}")]
    Invalid {
        origin: String,
        key: String,
        message: String,
    },
    #[error("{0}")]
    Key(String),
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Pfade der Konfigurationsdateien.
#[derive(Debug, Clone)]
pub struct Paths {
    pub user: PathBuf,
    /// Vorhandene oder (für `set --project`) anzulegende Projektdatei.
    pub project: PathBuf,
}

impl Paths {
    /// User-Datei unter `beton_home` (oder `override_user`), Projektdatei ab `cwd` aufwärts.
    pub fn discover(beton_home: &Path, override_user: Option<&Path>, cwd: &Path) -> Self {
        Self {
            user: override_user.map_or_else(|| beton_home.join("config.yaml"), Path::to_path_buf),
            project: project_file(cwd, beton_home),
        }
    }

    pub fn of(&self, scope: Scope) -> &Path {
        match scope {
            Scope::User => &self.user,
            Scope::Project => &self.project,
        }
    }
}

/// Nächstes `.beton/config.yaml` ab `cwd` aufwärts; das Datenverzeichnis selbst zählt nicht.
/// Ohne Treffer: `<Git-Wurzel oder cwd>/.beton/config.yaml`.
fn project_file(cwd: &Path, beton_home: &Path) -> PathBuf {
    let mut git_root = None;
    for dir in cwd.ancestors() {
        let candidate = dir.join(".beton");
        if candidate != beton_home && candidate.join("config.yaml").is_file() {
            return candidate.join("config.yaml");
        }
        if git_root.is_none() && dir.join(".git").exists() {
            git_root = Some(dir.to_path_buf());
        }
    }
    git_root
        .unwrap_or_else(|| cwd.to_path_buf())
        .join(".beton")
        .join("config.yaml")
}

/// Alle Ebenen einer Konfiguration.
#[derive(Debug, Clone)]
pub struct Layers {
    pub paths: Paths,
    default: Value,
    user: Value,
    project: Value,
    env: Value,
}

impl Layers {
    /// Liest beide Dateien und die Env-Ebene; prüft jede Ebene für sich.
    pub fn load(
        paths: Paths,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ConfigError> {
        let layers = Self {
            default: serde_json::to_value(Settings::default()).unwrap_or(Value::Null),
            user: read_file(&paths.user)?,
            project: read_file(&paths.project)?,
            env: env_layer(env),
            paths,
        };
        layers.validate()?;
        Ok(layers)
    }

    fn layer(&self, source: Source) -> &Value {
        match source {
            Source::Default => &self.default,
            Source::User => &self.user,
            Source::Project => &self.project,
            Source::Env => &self.env,
        }
    }

    fn origin(&self, source: Source) -> String {
        match source {
            Source::User => self.paths.user.display().to_string(),
            Source::Project => self.paths.project.display().to_string(),
            Source::Env => format!("Umgebung ({ENV_PREFIX}*)"),
            Source::Default => "Default".into(),
        }
    }

    fn merged_with(&self, sources: &[Source]) -> Value {
        let mut out = Value::Object(Map::new());
        for s in sources {
            merge(&mut out, self.layer(*s));
        }
        out
    }

    /// Effektive Konfiguration aller Ebenen.
    pub fn settings(&self) -> Result<Settings, ConfigError> {
        parse_settings(
            &self.merged_with(&[Source::Default, Source::User, Source::Project, Source::Env]),
            "effektive Konfiguration",
        )
    }

    /// Konfiguration des Daemons: ohne Projektebene (der Daemon gehört dem Benutzer).
    pub fn daemon_settings(&self) -> Result<Settings, ConfigError> {
        parse_settings(
            &self.merged_with(&[Source::Default, Source::User, Source::Env]),
            "Daemon-Konfiguration",
        )
    }

    /// `harnesses:` nur aus User-Datei und Env (für den Daemon).
    pub fn user_harnesses(&self) -> Result<HarnessesConfig, ConfigError> {
        Ok(self.daemon_settings()?.harnesses)
    }

    /// Jede Ebene für sich und alle zusammen.
    fn validate(&self) -> Result<(), ConfigError> {
        for source in [Source::User, Source::Project, Source::Env] {
            let layer = self.layer(source);
            if source == Source::Project {
                check_project_layer(layer).map_err(|(key, message)| ConfigError::Invalid {
                    origin: self.origin(source),
                    key,
                    message,
                })?;
            }
            let mut probe = self.default.clone();
            merge(&mut probe, layer);
            parse_settings(&probe, &self.origin(source))?;
        }
        self.settings().map(|_| ())
    }

    /// Effektive Werte als `(schlüssel, wert, herkunft)`, sortiert.
    pub fn list(&self) -> Vec<(String, Value, Source)> {
        let merged =
            self.merged_with(&[Source::Default, Source::User, Source::Project, Source::Env]);
        let mut leaves = Vec::new();
        flatten("", &merged, &mut leaves);
        leaves
            .into_iter()
            .map(|(key, value)| {
                let source = [Source::Env, Source::Project, Source::User]
                    .into_iter()
                    .find(|s| lookup(self.layer(*s), &key).is_some())
                    .unwrap_or(Source::Default);
                (key, value, source)
            })
            .collect()
    }

    /// Effektiver Wert (bzw. Wert einer Ebene) eines Schlüssels.
    pub fn get(&self, key: &str, scope: Option<Scope>) -> Option<Value> {
        let value = match scope {
            Some(Scope::User) => self.user.clone(),
            Some(Scope::Project) => self.project.clone(),
            None => {
                self.merged_with(&[Source::Default, Source::User, Source::Project, Source::Env])
            }
        };
        lookup(&value, key).cloned()
    }

    /// Setzt einen Wert in einer Ebene. Ungültige Werte lehnt es ab, ohne die Datei zu ändern.
    pub fn set(&mut self, scope: Scope, key: &str, raw: &str) -> Result<(), ConfigError> {
        let path = split_key(key)?;
        let parsed = parse_scalar(raw);
        let attempt = |value: Value| -> Result<Self, ConfigError> {
            let mut next = self.clone();
            insert(next.layer_mut(scope), &path, value)?;
            next.validate()?;
            Ok(next)
        };
        let next = match attempt(parsed.clone()) {
            Ok(n) => n,
            // `127.0.0.1:7420` o. Ä. ist als YAML kein String; dann als Text versuchen.
            Err(first) if !parsed.is_string() => {
                attempt(Value::String(raw.to_owned())).map_err(|_| first)?
            }
            Err(e) => return Err(e),
        };
        write_file(next.paths.of(scope), next.layer(source_of(scope)))?;
        *self = next;
        Ok(())
    }

    /// Entfernt einen Wert aus einer Ebene; `false`, wenn er dort nicht gesetzt war.
    pub fn unset(&mut self, scope: Scope, key: &str) -> Result<bool, ConfigError> {
        let path = split_key(key)?;
        let mut next = self.clone();
        if !remove(next.layer_mut(scope), &path) {
            return Ok(false);
        }
        next.validate()?;
        write_file(next.paths.of(scope), next.layer(source_of(scope)))?;
        *self = next;
        Ok(true)
    }

    fn layer_mut(&mut self, scope: Scope) -> &mut Value {
        match scope {
            Scope::User => &mut self.user,
            Scope::Project => &mut self.project,
        }
    }
}

fn source_of(scope: Scope) -> Source {
    match scope {
        Scope::User => Source::User,
        Scope::Project => Source::Project,
    }
}

fn check_project_layer(layer: &Value) -> Result<(), (String, String)> {
    let Some(map) = layer.as_object() else {
        return Ok(());
    };
    for key in map.keys() {
        if key != "harnesses" {
            return Err((
                key.clone(),
                "nur in der User-Konfiguration erlaubt; das Projekt darf nur `harnesses` setzen"
                    .into(),
            ));
        }
    }
    if let Some(harnesses) = map.get("harnesses").and_then(Value::as_object) {
        for (id, entry) in harnesses {
            if entry.get("auth").is_some() {
                return Err((
                    format!("harnesses.{id}.auth"),
                    "die Auth-Herkunft wählt nur der Benutzer (User-Konfiguration, HAR-015)".into(),
                ));
            }
        }
    }
    Ok(())
}

fn parse_settings(value: &Value, origin: &str) -> Result<Settings, ConfigError> {
    let de = value.clone();
    serde_path_to_error::deserialize::<_, Settings>(de).map_err(|e| ConfigError::Invalid {
        origin: origin.to_owned(),
        key: e.path().to_string(),
        message: e.inner().to_string(),
    })
}

fn read_file(path: &Path) -> Result<Value, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Object(Map::new())),
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    };
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_yaml_ng::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    match value {
        Value::Object(_) => Ok(value),
        Value::Null => Ok(Value::Object(Map::new())),
        _ => Err(ConfigError::Parse {
            path: path.display().to_string(),
            message: "erwartet eine Zuordnung (Schlüssel: Wert)".into(),
        }),
    }
}

/// Schreibt atomar (temporäre Datei, dann Umbenennen).
fn write_file(path: &Path, value: &Value) -> Result<(), ConfigError> {
    let io = |source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let text = serde_yaml_ng::to_string(value).map_err(|e| ConfigError::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

fn env_layer(env: impl IntoIterator<Item = (String, String)>) -> Value {
    let mut out = Value::Object(Map::new());
    let mut vars: Vec<(String, String)> = env
        .into_iter()
        .filter(|(k, _)| k.starts_with(ENV_PREFIX))
        .collect();
    vars.sort();
    for (key, raw) in vars {
        let path: Vec<String> = key[ENV_PREFIX.len()..]
            .split("__")
            .map(str::to_lowercase)
            .collect();
        if path.iter().any(String::is_empty) {
            continue;
        }
        let _ = insert(&mut out, &path, parse_scalar(&raw));
    }
    out
}

/// Wert als YAML; was kein YAML ist, bleibt Text.
fn parse_scalar(raw: &str) -> Value {
    serde_yaml_ng::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

fn split_key(key: &str) -> Result<Vec<String>, ConfigError> {
    let parts: Vec<String> = key.split('.').map(str::to_owned).collect();
    if parts.iter().any(String::is_empty) {
        return Err(ConfigError::Key(format!("ungültiger Schlüssel `{key}`")));
    }
    Ok(parts)
}

fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge(existing, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn lookup<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.')
        .try_fold(value, |v, part| v.as_object()?.get(part))
}

fn insert(root: &mut Value, path: &[String], value: Value) -> Result<(), ConfigError> {
    let Some((last, parents)) = path.split_last() else {
        return Ok(());
    };
    let mut cur = root;
    for (i, part) in parents.iter().enumerate() {
        let map = cur.as_object_mut().ok_or_else(|| {
            ConfigError::Key(format!("`{}` ist kein Abschnitt", path[..i].join(".")))
        })?;
        cur = map
            .entry(part.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let map = cur
        .as_object_mut()
        .ok_or_else(|| ConfigError::Key(format!("`{}` ist kein Abschnitt", parents.join("."))))?;
    map.insert(last.clone(), value);
    Ok(())
}

/// Entfernt den Schlüssel und leere Elternabschnitte.
fn remove(root: &mut Value, path: &[String]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    if rest.is_empty() {
        return map.remove(first).is_some();
    }
    let Some(child) = map.get_mut(first) else {
        return false;
    };
    let removed = remove(child, rest);
    if removed && child.as_object().is_some_and(Map::is_empty) {
        map.remove(first);
    }
    removed
}

fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        home: PathBuf,
        project: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home/.beton");
        let project = dir.path().join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        Fixture {
            _dir: dir,
            home,
            project,
        }
    }

    impl Fixture {
        fn paths(&self) -> Paths {
            Paths::discover(&self.home, None, &self.project.join("src"))
        }
        fn load(&self, env: &[(&str, &str)]) -> Result<Layers, ConfigError> {
            Layers::load(
                self.paths(),
                env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            )
        }
        fn write_user(&self, yaml: &str) {
            std::fs::write(self.home.join("config.yaml"), yaml).unwrap();
        }
        fn write_project(&self, yaml: &str) {
            std::fs::create_dir_all(self.project.join(".beton")).unwrap();
            std::fs::write(self.project.join(".beton/config.yaml"), yaml).unwrap();
        }
    }

    fn source_of_key(layers: &Layers, key: &str) -> Source {
        layers
            .list()
            .into_iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, s)| s)
            .unwrap_or_else(|| panic!("{key} fehlt"))
    }

    #[test]
    fn cli_008_ac1_list_shows_the_source_per_key() {
        let f = fixture();
        f.write_user("server:\n  allowed_hosts: [beton.local]\nharnesses:\n  claude:\n    command: /opt/claude\n");
        f.write_project("harnesses:\n  claude:\n    isolated: true\n");
        let layers = f
            .load(&[("BETON_CFG_EVENTS__STORE_RAW", "false"), ("PATH", "/bin")])
            .unwrap();
        assert_eq!(source_of_key(&layers, "server.listen"), Source::Default);
        assert_eq!(source_of_key(&layers, "server.allowed_hosts"), Source::User);
        assert_eq!(
            source_of_key(&layers, "harnesses.claude.command"),
            Source::User
        );
        assert_eq!(
            source_of_key(&layers, "harnesses.claude.isolated"),
            Source::Project
        );
        assert_eq!(source_of_key(&layers, "events.store_raw"), Source::Env);
        let s = layers.settings().unwrap();
        assert!(!s.events.store_raw);
        assert_eq!(s.harnesses.entries["claude"].isolated, Some(true));
        // Der Daemon sieht die Projektebene nicht.
        assert_eq!(
            layers.daemon_settings().unwrap().harnesses.entries["claude"].isolated,
            None
        );
    }

    #[test]
    fn cli_008_ac2_invalid_value_is_rejected_and_file_unchanged() {
        let f = fixture();
        f.write_user("events:\n  store_raw: true\n");
        let before = std::fs::read_to_string(f.home.join("config.yaml")).unwrap();
        let mut layers = f.load(&[]).unwrap();
        let err = layers
            .set(Scope::User, "events.store_raw", "vielleicht")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("events.store_raw"), "{msg}");
        assert!(msg.contains("boolean"), "{msg}");
        let err = layers
            .set(Scope::User, "server.listen", "[\"0.0.0.0\"]")
            .unwrap_err();
        assert!(err.to_string().contains("server.listen"), "{err}");
        let err = layers.set(Scope::User, "gibt.es.nicht", "1").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
        assert_eq!(
            std::fs::read_to_string(f.home.join("config.yaml")).unwrap(),
            before
        );
    }

    #[test]
    fn set_get_unset_round_trip() {
        let f = fixture();
        let mut layers = f.load(&[]).unwrap();
        layers
            .set(Scope::User, "events.store_raw", "false")
            .unwrap();
        layers
            .set(Scope::User, "server.listen", "[\"127.0.0.1:7421\"]")
            .unwrap();
        layers
            .set(Scope::Project, "harnesses.claude.command", "./claude")
            .unwrap();
        let again = f.load(&[]).unwrap();
        assert_eq!(
            again.get("events.store_raw", None),
            Some(Value::Bool(false))
        );
        assert_eq!(
            again.settings().unwrap().server.listen,
            vec!["127.0.0.1:7421".parse::<SocketAddr>().unwrap()]
        );
        assert!(f.project.join(".beton/config.yaml").is_file());
        assert_eq!(
            again.get("harnesses.claude.command", Some(Scope::Project)),
            Some(Value::String("./claude".into()))
        );
        let mut layers = again;
        assert!(layers.unset(Scope::User, "events.store_raw").unwrap());
        assert!(!layers.unset(Scope::User, "events.store_raw").unwrap());
        assert_eq!(
            f.load(&[]).unwrap().get("events.store_raw", None),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn project_may_not_set_daemon_keys_or_auth() {
        let f = fixture();
        let mut layers = f.load(&[]).unwrap();
        let err = layers
            .set(Scope::Project, "events.store_raw", "false")
            .unwrap_err();
        assert!(err.to_string().contains("User-Konfiguration"), "{err}");
        let err = layers
            .set(Scope::Project, "harnesses.claude.auth", "api_key")
            .unwrap_err();
        assert!(err.to_string().contains("HAR-015"), "{err}");
        assert!(!f.project.join(".beton/config.yaml").exists());
        // Eine von Hand geschriebene Projektdatei mit Auth wird beim Laden abgelehnt.
        f.write_project("harnesses:\n  claude:\n    auth: api_key\n");
        assert!(f.load(&[]).is_err());
        // In der User-Datei ist sie erlaubt.
        layers
            .set(Scope::User, "harnesses.claude.auth", "api_key")
            .unwrap();
    }

    #[test]
    fn data_dir_is_not_mistaken_for_a_project() {
        let f = fixture();
        f.write_user("events:\n  store_raw: false\n");
        let inside_home = Paths::discover(&f.home, None, &f.home);
        assert_ne!(inside_home.project, f.home.join("config.yaml"));
    }

    #[test]
    fn broken_yaml_names_the_file() {
        let f = fixture();
        f.write_user("events: [\n");
        let err = f.load(&[]).unwrap_err();
        assert!(err.to_string().contains("config.yaml"), "{err}");
    }
}
