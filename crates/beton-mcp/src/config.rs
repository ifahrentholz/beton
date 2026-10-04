//! MCP-Server-Konfiguration (AGT-006).
//!
//! Tools gibt es ausschließlich als MCP-Server: stdio (`command`, `args`, `env`) oder HTTP
//! (`url`, `headers`), jeweils mit optionaler `allow`-Liste. Ebenen: User
//! (`~/.beton/mcp.yaml`), Projekt (`.beton/mcp.yaml`) und Agent (`tools.mcp`). Merge-Reihenfolge
//! User → Projekt → Agent; bei gleichem Namen ersetzt die spezifischere Ebene den Eintrag
//! vollständig. Agent-Läufe verwenden User- und Projekt-Server nur mit `tools.inherit: true`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use beton_agents::spec::Tools;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use beton_agents::spec::McpServer;

/// Name des eingebauten Servers mit den System-Tools (AGT-007); für eigene Server reserviert.
pub const SYSTEM_SERVER: &str = "beton";

/// Inhalt von `~/.beton/mcp.yaml` bzw. `.beton/mcp.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "beton MCP-Server (mcp.yaml)")]
pub struct McpFile {
    /// MCP-Server nach Name (`[a-z0-9_-]`, höchstens 64 Zeichen; `beton` ist reserviert).
    #[serde(default)]
    pub servers: BTreeMap<String, McpServer>,
}

/// Herkunft eines Servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    User,
    Project,
    Agent,
}

/// Ein wirksamer Server nach dem Merge.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfiguredServer {
    pub name: String,
    pub layer: Layer,
    pub server: McpServer,
}

/// Wie der Runner einen Server erreicht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint<'a> {
    Stdio {
        command: &'a str,
        args: &'a [String],
        env: &'a BTreeMap<String, String>,
    },
    Http {
        url: &'a str,
        headers: &'a BTreeMap<String, String>,
    },
}

/// Erlaubte Server-Namen: `[a-z0-9_-]`, 1–64 Zeichen (Codex verlangt `^[a-zA-Z0-9_-]+$`,
/// Claude bildet daraus `mcp__<name>__<tool>`).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl ConfiguredServer {
    /// Ist das Tool laut `allow` sichtbar? Ohne `allow` sind alle Tools sichtbar.
    pub fn allows(&self, tool: &str) -> bool {
        self.server
            .allow
            .as_ref()
            .is_none_or(|allow| allow.iter().any(|t| t == tool))
    }

    /// Prüft den Eintrag; ein ungültiger Server wird nicht gestartet (`mcp.server_failed`).
    pub fn endpoint(&self) -> Result<Endpoint<'_>, String> {
        if !valid_name(&self.name) {
            return Err(format!(
                "ungültiger Server-Name „{}“ (erlaubt: a-z, 0-9, -, _)",
                self.name
            ));
        }
        if self.name == SYSTEM_SERVER {
            return Err(format!(
                "der Name „{SYSTEM_SERVER}“ ist für die System-Tools reserviert"
            ));
        }
        let s = &self.server;
        // Secret-Referenzen löst erst SEC-001/PRX-006 (ab M2) in Platzhalter auf; bis dahin
        // wird der Server nicht mit dem Klartext-Ausdruck gestartet (fail closed).
        let secret = s
            .env
            .values()
            .chain(s.headers.values())
            .any(|v| v.contains("${secret:"));
        if secret {
            return Err(
                "Secret-Referenzen (${secret:…}) werden erst ab M2 aufgelöst (SEC-001)".into(),
            );
        }
        match (&s.command, &s.url) {
            (Some(command), None) if !command.trim().is_empty() => {
                if !s.headers.is_empty() {
                    return Err("`headers` gilt nur für HTTP-Server (`url`)".into());
                }
                Ok(Endpoint::Stdio {
                    command,
                    args: &s.args,
                    env: &s.env,
                })
            }
            (None, Some(url)) => {
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    // Die URL selbst nicht ausgeben: sie kann Zugangsdaten enthalten.
                    return Err("`url` muss mit http:// oder https:// beginnen".into());
                }
                if !s.args.is_empty() || !s.env.is_empty() {
                    return Err("`args` und `env` gelten nur für stdio-Server (`command`)".into());
                }
                Ok(Endpoint::Http {
                    url,
                    headers: &s.headers,
                })
            }
            (Some(_), Some(_)) => Err("`command` und `url` schließen sich aus".into()),
            _ => Err("`command` (stdio) oder `url` (HTTP) fehlt".into()),
        }
    }
}

/// Merge User → Projekt → Agent (AGT-006). Ein gleichnamiger Eintrag der spezifischeren Ebene
/// ersetzt den weniger spezifischen vollständig (keine Feld-Vermischung). Mit Agent gelten
/// User- und Projekt-Server nur bei `tools.inherit: true`; ohne Agent (interaktive Session)
/// gelten User und Projekt.
pub fn merge(user: &McpFile, project: &McpFile, agent: Option<&Tools>) -> Vec<ConfiguredServer> {
    let mut out: BTreeMap<String, ConfiguredServer> = BTreeMap::new();
    let inherit = agent.is_none_or(|t| t.inherit == Some(true));
    let mut put = |layer: Layer, servers: &BTreeMap<String, McpServer>| {
        for (name, server) in servers {
            out.insert(
                name.clone(),
                ConfiguredServer {
                    name: name.clone(),
                    layer,
                    server: server.clone(),
                },
            );
        }
    };
    if inherit {
        put(Layer::User, &user.servers);
        put(Layer::Project, &project.servers);
    }
    if let Some(tools) = agent {
        put(Layer::Agent, &tools.mcp);
    }
    out.into_values().collect()
}

/// Fehler beim Lesen einer `mcp.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    /// 1-basiert, falls bekannt.
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let (Some(l), Some(c)) = (self.line, self.column) {
            write!(f, ":{l}:{c}")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Liest eine `mcp.yaml`; fehlt die Datei, ist das Ergebnis leer.
pub fn load(path: &Path) -> Result<McpFile, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(McpFile::default()),
        Err(e) => {
            return Err(ConfigError {
                file: path.to_owned(),
                line: None,
                column: None,
                message: e.to_string(),
            });
        }
    };
    parse(&text, path)
}

/// Liest den Text einer `mcp.yaml`; `file` erscheint in Fehlern.
pub fn parse(text: &str, file: &Path) -> Result<McpFile, ConfigError> {
    if text.trim().is_empty() {
        return Ok(McpFile::default());
    }
    serde_yaml_ng::from_str(text).map_err(|e| ConfigError {
        file: file.to_owned(),
        line: e.location().map(|l| l.line()),
        column: e.location().map(|l| l.column()),
        message: redact_values(&e.to_string()),
    })
}

/// Entfernt zitierte Werte aus Typ- und Wertfehlern („invalid type: string "…"“), damit
/// Env- oder Header-Werte nicht in Hinweise, Events oder Logs gelangen. Feldnamen in
/// Fehlern zu unbekannten Feldern bleiben erhalten.
fn redact_values(message: &str) -> String {
    if !message.contains("invalid") {
        return message.to_owned();
    }
    let mut out = String::with_capacity(message.len());
    let mut quote: Option<char> = None;
    for c in message.chars() {
        match quote {
            Some(q) if c == q => {
                out.push('…');
                out.push(c);
                quote = None;
            }
            Some(_) => {}
            None if c == '"' || c == '`' => {
                out.push(c);
                quote = Some(c);
            }
            None => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn file(yaml: &str) -> McpFile {
        parse(yaml, Path::new("mcp.yaml")).unwrap()
    }

    #[test]
    fn agt_006_ac2_project_server_replaces_user_server_completely() {
        let user = file(
            "servers:\n  github:\n    command: gh-mcp\n    args: [stdio]\n    env: { A: \"1\" }\n    allow: [a, b]\n  docs:\n    url: https://docs.example.invalid/mcp\n",
        );
        let project = file("servers:\n  github:\n    command: ./local-github\n");
        let merged = merge(&user, &project, None);
        assert_eq!(merged.len(), 2);
        let gh = merged.iter().find(|s| s.name == "github").unwrap();
        assert_eq!(gh.layer, Layer::Project);
        assert_eq!(gh.server.command.as_deref(), Some("./local-github"));
        // Vollständig ersetzt: nichts aus dem User-Eintrag bleibt übrig.
        assert!(gh.server.args.is_empty());
        assert!(gh.server.env.is_empty());
        assert_eq!(gh.server.allow, None);
        let docs = merged.iter().find(|s| s.name == "docs").unwrap();
        assert_eq!(docs.layer, Layer::User);
    }

    #[test]
    fn agt_006_agent_runs_ignore_user_and_project_unless_inherit() {
        let user = file("servers:\n  u:\n    command: u\n");
        let project = file("servers:\n  p:\n    command: p\n  a:\n    command: from-project\n");
        let mut tools = Tools {
            mcp: [(
                "a".to_owned(),
                McpServer {
                    command: Some("from-agent".into()),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    url: None,
                    headers: BTreeMap::new(),
                    allow: None,
                },
            )]
            .into(),
            system: Vec::new(),
            inherit: None,
        };
        let names = |v: Vec<ConfiguredServer>| v.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(merge(&user, &project, Some(&tools))), ["a"]);
        tools.inherit = Some(true);
        let merged = merge(&user, &project, Some(&tools));
        assert_eq!(names(merged.clone()), ["a", "p", "u"]);
        // Die Agent-Ebene ist die spezifischste.
        let a = merged.iter().find(|s| s.name == "a").unwrap();
        assert_eq!(a.layer, Layer::Agent);
        assert_eq!(a.server.command.as_deref(), Some("from-agent"));
    }

    #[test]
    fn agt_006_allow_list_filters_tools() {
        let f = file("servers:\n  s:\n    command: x\n    allow: [get]\n  t:\n    command: y\n");
        let merged = merge(&McpFile::default(), &f, None);
        assert!(merged[0].allows("get"));
        assert!(!merged[0].allows("delete"));
        assert!(merged[1].allows("anything"));
    }

    #[test]
    fn invalid_entries_are_rejected_with_reason() {
        let f = file(
            "servers:\n  beton:\n    command: x\n  both:\n    command: x\n    url: http://a\n  none: {}\n  ftp:\n    url: ftp://x\n  Bad.Name:\n    command: x\n  secret:\n    command: x\n    env: { T: \"${secret:gh}\" }\n  ok:\n    url: http://127.0.0.1:1/mcp\n",
        );
        let merged = merge(&McpFile::default(), &f, None);
        let err = |n: &str| {
            merged
                .iter()
                .find(|s| s.name == n)
                .unwrap()
                .endpoint()
                .err()
                .unwrap_or_default()
        };
        assert!(err("beton").contains("reserviert"));
        assert!(err("both").contains("schließen sich aus"));
        assert!(err("none").contains("fehlt"));
        assert!(err("ftp").contains("http://"));
        assert!(err("Bad.Name").contains("Server-Name"));
        assert!(err("secret").contains("M2"));
        assert!(
            merged
                .iter()
                .find(|s| s.name == "ok")
                .unwrap()
                .endpoint()
                .is_ok()
        );
    }

    #[test]
    fn unknown_fields_are_errors_with_position() {
        let e = parse("servers:\n  s:\n    comand: x\n", Path::new("/p/mcp.yaml")).unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.to_string().starts_with("/p/mcp.yaml:3:"), "{e}");
        assert!(e.message.contains("comand"), "{e}");
    }

    #[test]
    fn config_errors_do_not_echo_values() {
        // Falsch eingerückt: der Wert landet sonst in Fehlermeldung, Notice und Log.
        let e = parse(
            "servers:\n  gh:\n    command: x\n    env: ghp_geheimerwert123\n",
            Path::new("mcp.yaml"),
        )
        .unwrap_err();
        assert!(!e.to_string().contains("ghp_geheimerwert123"), "{e}");
        assert_eq!(e.line, Some(4));
        let f = parse(
            "servers:\n  web:\n    url: ftp://user:pw@host/x\n",
            Path::new("m"),
        )
        .unwrap();
        let s = merge(&McpFile::default(), &f, None);
        assert!(!s[0].endpoint().unwrap_err().contains("pw@host"));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&dir.path().join("mcp.yaml")).unwrap(),
            McpFile::default()
        );
    }
}
