//! Gerüst für `beton agent new` (AGT-013 AC1): `agent.yaml` mit `$schema`-Kommentar, der auf
//! die lokal abgelegte Schema-Datei zeigt (nie auf eine URL, ADR-0033), Prompt-Datei und
//! Beispiel-Skill.

use std::path::{Path, PathBuf};

use crate::dir::{AGENT_YAML, AgentDir};
use crate::spec::{AgentName, schema_json};

/// Ort der Schema-Datei relativ zu `<projekt>/.beton`.
pub const SCHEMA_PATH: &str = "schemas/v1/agent.schema.json";

/// Kommentarzeile für Editoren (yaml-language-server), relativ zu
/// `.beton/agents/<name>/agent.yaml`.
pub const SCHEMA_COMMENT: &str =
    "# yaml-language-server: $schema=../../schemas/v1/agent.schema.json";

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("{0} existiert bereits")]
    Exists(PathBuf),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Was `new` angelegt hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scaffold {
    /// `<projekt>/.beton/agents/<name>`.
    pub dir: PathBuf,
    /// Angelegte Dateien relativ zu `dir`.
    pub files: Vec<String>,
    /// Geschriebene Schema-Datei.
    pub schema: PathBuf,
}

fn agent_yaml(name: &AgentName) -> String {
    format!(
        "{SCHEMA_COMMENT}
spec_version: 1
name: {name}
description: Beschreibe in einem Satz, wofür dieser Agent da ist.
version: 0.1.0

executor:
  harness: claude
  permission_mode: default

instructions:
  file: prompts/system.md

skills: [example]
"
    )
}

fn system_md(name: &AgentName) -> String {
    format!(
        "Du bist {name}, ein Agent in beton.\n\nBeschreibe hier Rolle, Ziel und Grenzen des Agents.\n"
    )
}

const EXAMPLE_SKILL: &str = "---
name: example
description: Beispiel-Skill. Ersetze ihn oder entferne ihn samt Eintrag unter `skills`.
---

# Beispiel

Schritte, die der Agent bei Bedarf lädt.
";

/// Setzt `name` und den `$schema`-Kommentar in einer kopierten `agent.yaml`; Kommentare und
/// Reihenfolge bleiben erhalten.
fn retarget(text: &str, name: &AgentName) -> String {
    let mut out = vec![SCHEMA_COMMENT.to_owned()];
    let mut renamed = false;
    for line in text.lines() {
        if line.starts_with("# yaml-language-server:") {
            continue;
        }
        if !renamed && line.starts_with("name:") {
            out.push(format!("name: {name}"));
            renamed = true;
        } else {
            out.push(line.to_owned());
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), ScaffoldError> {
    let io = |source| ScaffoldError::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(path, bytes).map_err(io)
}

/// Legt `<beton_dir>/agents/<name>/` an (aus der Vorlage oder als Kopie von `from`) und
/// schreibt das aktuelle Schema nach `<beton_dir>/schemas/v1/agent.schema.json`.
pub fn new_agent(
    beton_dir: &Path,
    name: &AgentName,
    from: Option<&AgentDir>,
) -> Result<Scaffold, ScaffoldError> {
    let dir = beton_dir.join("agents").join(name.as_str());
    if dir.exists() {
        return Err(ScaffoldError::Exists(dir));
    }
    let files: Vec<(String, Vec<u8>)> = match from {
        Some(source) => source
            .files()
            .into_iter()
            .map(|(rel, bytes)| {
                if rel == AGENT_YAML {
                    let text = String::from_utf8_lossy(&bytes);
                    (rel, retarget(&text, name).into_bytes())
                } else {
                    (rel, bytes)
                }
            })
            .collect(),
        None => vec![
            (AGENT_YAML.to_owned(), agent_yaml(name).into_bytes()),
            ("prompts/system.md".to_owned(), system_md(name).into_bytes()),
            (
                "skills/example/SKILL.md".to_owned(),
                EXAMPLE_SKILL.as_bytes().to_vec(),
            ),
        ],
    };
    let schema = beton_dir.join(SCHEMA_PATH);
    write(&schema, schema_json().as_bytes())?;
    for (rel, bytes) in &files {
        write(&dir.join(rel), bytes)?;
    }
    Ok(Scaffold {
        dir,
        files: files.into_iter().map(|(rel, _)| rel).collect(),
        schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retarget_replaces_name_and_schema_comment_only() {
        let text = "# yaml-language-server: $schema=https://example.com/x.json\n# Kommentar\nspec_version: 1\nname: maestra\nexecutor: { harness: claude }\n";
        let name: AgentName = "release-notes".parse().unwrap_or_else(|_| unreachable!());
        let out = retarget(text, &name);
        assert_eq!(
            out,
            format!(
                "{SCHEMA_COMMENT}\n# Kommentar\nspec_version: 1\nname: release-notes\nexecutor: {{ harness: claude }}\n"
            )
        );
        assert!(!out.contains("https://"));
    }
}
