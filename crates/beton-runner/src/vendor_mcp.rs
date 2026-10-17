//! MCP-Server reservierter Namen in der Vendor-Konfiguration des Users (#252, POL-005).
//!
//! Bindet beton einen Server `beton` (bzw. `beton-*`, `builtin`) ein und definiert die
//! Konfiguration der CLI selbst einen gleichnamigen, lässt sich am Namen nicht unterscheiden,
//! welcher Server einen Aufruf bekommt; seine Tools hätten die Klasse `system`. Der Runner liest
//! deshalb vor dem Start nur die **Namen** der MCP-Server aus der Konfiguration (Claude:
//! `.claude.json` mit `mcpServers` und `projects.<pfad>.mcpServers`, `managed-mcp.json`; Codex:
//! `config.toml` mit `[mcp_servers.<name>]`) und verweigert den Start bei einer Kollision. Andere
//! Inhalte der Dateien werden weder gespeichert noch gemeldet; Tokens stehen dort nicht
//! (ADR-0005). Eine vorhandene, aber nicht lesbare oder ungültige Datei verweigert den Start
//! ebenfalls (fail closed).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Größte gelesene Datei (`.claude.json` wächst mit dem Projektverlauf).
const MAX_BYTES: u64 = 64 << 20;

/// Wo die Konfiguration der CLIs liegt.
#[derive(Debug, Clone, Default)]
pub struct VendorPaths {
    pub home: Option<PathBuf>,
    /// `$CLAUDE_CONFIG_DIR`.
    pub claude_config_dir: Option<PathBuf>,
    /// `$CODEX_HOME`.
    pub codex_home: Option<PathBuf>,
    /// Managed Settings von Claude (Dateien bzw. Verzeichnisse); `managed-mcp.json` liegt daneben.
    pub claude_managed: Vec<PathBuf>,
}

impl VendorPaths {
    /// Aus der Umgebung des Runners.
    pub fn from_process(home: Option<PathBuf>) -> Self {
        let abs = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            home,
            claude_config_dir: abs("CLAUDE_CONFIG_DIR"),
            codex_home: abs("CODEX_HOME"),
            claude_managed: beton_harness_claude::managed_settings_paths(),
        }
    }
}

/// Reservierte Namen, die die Vendor-Konfiguration für `harness` (`claude`, `codex`) definiert;
/// `Err` mit Grund, wenn eine Datei nicht sicher lesbar ist.
pub fn reserved_servers(
    harness: &str,
    paths: &VendorPaths,
    workdir: &Path,
) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    match harness {
        "claude" => {
            let state = match &paths.claude_config_dir {
                Some(dir) => Some(dir.join(".claude.json")),
                None => paths.home.as_ref().map(|h| h.join(".claude.json")),
            };
            if let Some(file) = state
                && let Some(v) = read_json(&file)?
            {
                names.extend(keys(&v["mcpServers"]));
                // Lokale Server je Projekt (`claude mcp add --scope local`).
                let dirs = ancestors(workdir);
                if let Some(projects) = v["projects"].as_object() {
                    for (path, project) in projects {
                        let key = Path::new(path);
                        let real = std::fs::canonicalize(key).ok();
                        if dirs
                            .iter()
                            .any(|d| key == d || real.as_deref() == Some(d.as_path()))
                        {
                            names.extend(keys(&project["mcpServers"]));
                        }
                    }
                }
            }
            // `managed-mcp.json` liegt im Verzeichnis der Managed Settings.
            let mut dirs: Vec<&Path> = paths
                .claude_managed
                .iter()
                .filter_map(|m| m.parent())
                .collect();
            dirs.dedup();
            for dir in dirs {
                if let Some(v) = read_json(&dir.join("managed-mcp.json"))? {
                    names.extend(keys(&v["mcpServers"]));
                }
            }
        }
        "codex" => {
            let home = paths
                .codex_home
                .clone()
                .or_else(|| paths.home.as_ref().map(|h| h.join(".codex")));
            if let Some(home) = home
                && let Some(text) = read_text(&home.join("config.toml"))?
            {
                let table: toml::Table = toml::from_str(&text)
                    .map_err(|_| "Codex-Konfiguration ist kein gültiges TOML".to_owned())?;
                if let Some(servers) = table.get("mcp_servers").and_then(toml::Value::as_table) {
                    names.extend(servers.keys().cloned());
                }
            }
        }
        _ => {}
    }
    names.retain(|n| beton_mcp::config::reserved_name(n));
    names.sort();
    names.dedup();
    Ok(names)
}

/// Das Arbeitsverzeichnis und seine Vorfahren, wie angegeben und kanonisch.
fn ancestors(workdir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = workdir.ancestors().map(Path::to_path_buf).collect();
    if let Ok(real) = std::fs::canonicalize(workdir) {
        out.extend(real.ancestors().map(Path::to_path_buf));
    }
    out
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read as _;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if missing(&e) => return Ok(None),
        Err(_) => return Err(format!("{} ist nicht lesbar", shown(path))),
    };
    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| format!("{} ist nicht lesbar", shown(path)))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("{} ist zu groß", shown(path)));
    }
    Ok(Some(text))
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    // Die CLI schreibt `.claude.json` laufend; ein halb geschriebener Stand wird einmal erneut
    // gelesen.
    for attempt in 0..2 {
        let Some(text) = read_text(path)? else {
            return Ok(None);
        };
        match serde_json::from_str(&text) {
            Ok(v) => return Ok(Some(v)),
            Err(_) if attempt == 0 => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => {}
        }
    }
    Err(format!("{} ist kein gültiges JSON", shown(path)))
}

/// Nur der Dateiname, nie Inhalte.
fn shown(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Fehlt die Datei (auch, weil ein Teil des Pfads kein Verzeichnis ist)?
fn missing(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(home: &Path) -> VendorPaths {
        VendorPaths {
            home: Some(home.to_path_buf()),
            ..VendorPaths::default()
        }
    }

    #[test]
    fn pol_005_reserved_names_from_claude_and_codex_configs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let work = home.join("w/sub");
        std::fs::create_dir_all(&work).unwrap();
        assert!(
            reserved_servers("claude", &paths(home), &work)
                .unwrap()
                .is_empty()
        );
        std::fs::write(
            home.join(".claude.json"),
            serde_json::json!({
                "mcpServers": {"github": {}, "builtin": {}},
                "projects": {
                    home.join("w").display().to_string(): {"mcpServers": {"beton_y": {}}},
                    "/anderes": {"mcpServers": {"beton": {}}},
                },
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            reserved_servers("claude", &paths(home), &work).unwrap(),
            ["beton_y", "builtin"]
        );
        // `$CLAUDE_CONFIG_DIR` ersetzt das Home.
        let cfg = home.join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join(".claude.json"), r#"{"mcpServers": {"beton": {}}}"#).unwrap();
        let with_dir = VendorPaths {
            claude_config_dir: Some(cfg),
            ..paths(home)
        };
        assert_eq!(
            reserved_servers("claude", &with_dir, &work).unwrap(),
            ["beton"]
        );
        // Managed: `managed-mcp.json` neben den Managed Settings.
        let managed = home.join("managed");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(
            managed.join("managed-mcp.json"),
            r#"{"mcpServers": {"beton-m": {}}}"#,
        )
        .unwrap();
        let with_managed = VendorPaths {
            claude_managed: vec![managed.join("managed-settings.json")],
            home: None,
            ..VendorPaths::default()
        };
        assert_eq!(
            reserved_servers("claude", &with_managed, &work).unwrap(),
            ["beton-m"]
        );
        // Codex.
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.github]\ncommand = \"gh\"\n[mcp_servers.beton]\ncommand = \"x\"\n",
        )
        .unwrap();
        assert_eq!(
            reserved_servers("codex", &paths(home), &work).unwrap(),
            ["beton"]
        );
        // Kaputt: fail closed, ohne Inhalte in der Meldung.
        std::fs::write(home.join(".codex/config.toml"), "[mcp_servers\ngeheim = 1").unwrap();
        let err = reserved_servers("codex", &paths(home), &work).unwrap_err();
        assert!(!err.contains("geheim"), "{err}");
        std::fs::write(home.join(".claude.json"), "{ geheim").unwrap();
        let err = reserved_servers("claude", &paths(home), &work).unwrap_err();
        assert!(
            !err.contains("geheim") && err.contains(".claude.json"),
            "{err}"
        );
    }
}
