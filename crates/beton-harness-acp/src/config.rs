//! ACP-Agent-Registrierung und Presets (HAR-008).
//!
//! Agents stehen unter `harnesses.acp.agents.<slug>` in User- oder Projekt-Konfiguration;
//! die Projektebene überschreibt die User-Ebene, beide überschreiben ein Preset mit gleichem
//! Slug. Presets erscheinen immer im Katalog, sind aber nur startbar, wenn ihr Binary
//! gefunden wird (ADR-0026, ADR-0033: nichts wird still installiert). Ungültige Einträge
//! werden mit Datei und Zeile gemeldet und übersprungen, ohne andere Harnesses zu
//! beeinträchtigen.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use beton_harness::registry::{AcpAgentConfig, HarnessLayers, HarnessesConfig};

/// Ein mitgeliefertes Preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub slug: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// Presets für verbreitete ACP-Agents *(Annahme: Flags je Vendor noch gegen die echten CLIs
/// zu verifizieren)*. Kein Preset setzt einen API-Key voraus; der Agent nutzt seinen eigenen
/// Login (ADR-0034).
pub const PRESETS: [Preset; 3] = [
    Preset {
        slug: "gemini",
        name: "Gemini CLI",
        command: "gemini",
        args: &["--experimental-acp"],
    },
    Preset {
        slug: "goose",
        name: "Goose",
        command: "goose",
        args: &["acp"],
    },
    Preset {
        slug: "qwen",
        name: "Qwen Code",
        command: "qwen",
        args: &["--acp"],
    },
];

/// Woher ein Agent stammt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Preset,
    User,
    Project,
}

/// Ein registrierter ACP-Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAgent {
    pub slug: String,
    pub config: AcpAgentConfig,
    pub origin: Origin,
}

/// Ein übersprungener Konfigurationseintrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub file: Option<PathBuf>,
    /// 1-basiert.
    pub line: Option<usize>,
    pub key: String,
    pub message: String,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.file, self.line) {
            (Some(file), Some(line)) => write!(f, "{}:{line}: ", file.display())?,
            (Some(file), None) => write!(f, "{}: ", file.display())?,
            (None, _) => {}
        }
        write!(f, "{}: {}", self.key, self.message)
    }
}

/// Erlaubte Slugs: `[a-z0-9-]`, 1 bis 64 Zeichen.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Prüft einen Eintrag; `Err` mit der Meldung für den Nutzer.
pub fn parse_entry(slug: &str, raw: &serde_json::Value) -> Result<AcpAgentConfig, String> {
    if !valid_slug(slug) {
        return Err(format!(
            "ungültiger Slug `{slug}` (erlaubt: Kleinbuchstaben, Ziffern und `-`)"
        ));
    }
    let config: AcpAgentConfig = serde_json::from_value(raw.clone()).map_err(|e| {
        let text = e.to_string();
        if text.contains("missing field `command`") {
            "`command` fehlt".to_owned()
        } else {
            text
        }
    })?;
    if config.command.trim().is_empty() {
        return Err("`command` ist leer".into());
    }
    Ok(config)
}

/// Zeile des Eintrags `<slug>:` unter `acp:` → `agents:` in einer YAML-Datei.
pub fn line_of(text: &str, slug: &str) -> Option<usize> {
    let mut in_acp = None;
    let mut agents_line = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with("acp:") {
            in_acp = Some(indent);
            continue;
        }
        let Some(acp_indent) = in_acp else { continue };
        if !trimmed.is_empty() && !trimmed.starts_with('#') && indent <= acp_indent {
            in_acp = None;
            continue;
        }
        if trimmed.starts_with("agents:") {
            agents_line = Some(i + 1);
            continue;
        }
        if agents_line.is_some() {
            let key = trimmed.trim_start_matches(['"', '\'']);
            if let Some(rest) = key.strip_prefix(slug)
                && rest.trim_start_matches(['"', '\'']).starts_with(':')
            {
                return Some(i + 1);
            }
        }
    }
    agents_line
}

fn problem(file: Option<&Path>, slug: &str, message: String) -> ConfigProblem {
    let line = file
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|text| line_of(&text, slug));
    ConfigProblem {
        file: file.map(Path::to_path_buf),
        line,
        key: format!("harnesses.acp.agents.{slug}"),
        message,
    }
}

/// Alle Agents aus Presets, User- und Projekt-Konfiguration, sortiert nach Slug, und die
/// übersprungenen Einträge.
pub fn agents(layers: &HarnessLayers) -> (Vec<AcpAgent>, Vec<ConfigProblem>) {
    let mut out: BTreeMap<String, AcpAgent> = PRESETS
        .iter()
        .map(|p| {
            (
                p.slug.to_owned(),
                AcpAgent {
                    slug: p.slug.to_owned(),
                    config: AcpAgentConfig {
                        command: p.command.to_owned(),
                        args: p.args.iter().map(|a| (*a).to_owned()).collect(),
                        ..AcpAgentConfig::default()
                    },
                    origin: Origin::Preset,
                },
            )
        })
        .collect();
    let mut problems = Vec::new();
    let levels: [(&HarnessesConfig, Option<&PathBuf>, Origin); 2] = [
        (&layers.user, layers.user_file.as_ref(), Origin::User),
        (
            &layers.project,
            layers.project_file.as_ref(),
            Origin::Project,
        ),
    ];
    for (config, file, origin) in levels {
        for (slug, raw) in &config.acp.agents {
            match parse_entry(slug, raw) {
                Ok(config) => {
                    out.insert(
                        slug.clone(),
                        AcpAgent {
                            slug: slug.clone(),
                            config,
                            origin,
                        },
                    );
                }
                Err(message) => {
                    let p = problem(file.map(PathBuf::as_path), slug, message);
                    tracing::warn!("ACP-Agent übersprungen: {p}");
                    problems.push(p);
                }
            }
        }
    }
    (out.into_values().collect(), problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn har_008_slugs_follow_the_rule() {
        for ok in ["gemini", "mein-agent", "a1"] {
            assert!(valid_slug(ok), "{ok}");
        }
        for bad in ["", "Mein", "mein_agent", "a b", "ä"] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn line_of_finds_the_entry() {
        let text = "harnesses:\n  claude:\n    command: claude\n  acp:\n    agents:\n      gut:\n        command: x\n      \"Kaputt_1\":\n        args: []\n";
        assert_eq!(line_of(text, "gut"), Some(6));
        assert_eq!(line_of(text, "Kaputt_1"), Some(8));
        assert_eq!(line_of(text, "fehlt"), Some(5), "sonst die agents-Zeile");
    }

    #[test]
    fn har_008_custom_entry_overrides_preset() {
        let mut layers = HarnessLayers::default();
        layers.user.acp.agents.insert(
            "gemini".into(),
            serde_json::json!({"command": "/opt/gemini", "args": ["--acp"]}),
        );
        let (agents, problems) = agents(&layers);
        assert!(problems.is_empty());
        let gemini = agents.iter().find(|a| a.slug == "gemini").unwrap();
        assert_eq!(gemini.config.command, "/opt/gemini");
        assert_eq!(gemini.origin, Origin::User);
        assert_eq!(
            agents.iter().map(|a| a.slug.as_str()).collect::<Vec<_>>(),
            ["gemini", "goose", "qwen"]
        );
    }
}
