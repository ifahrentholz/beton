//! Instructions eines Agents zusammensetzen (AGT-005): `instructions.text` oder
//! `instructions.file`, dazu `append` (mit Parametern, AGT-010) und die Projektdateien nach
//! `project_files`. Wie der Text beim Harness ankommt, entscheidet dessen Capability
//! `instructions_delivery` (Adapter); hier entsteht nur der Text, genau einmal.
//!
//! `project_files: auto` (Default) sorgt dafür, dass `AGENTS.md` und `CLAUDE.md` des Projekts
//! bei jedem Harness ankommen: Was der Harness selbst liest (Capability
//! `native_project_files`, z. B. Claude `CLAUDE.md`, Codex `AGENTS.md`), fügt beton nicht noch
//! einmal an.

use std::path::{Component, Path};

use crate::dir::AgentDir;
use crate::params::ParamValues;
use crate::spec::{AgentSpec, ProjectFiles, ProjectFilesMode};
use crate::template;

/// Projektdateien bei `project_files: auto`.
pub const AUTO_PROJECT_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
/// Obergrenze je Projektdatei; längere werden gekürzt (mit Hinweis).
pub const MAX_PROJECT_FILE_BYTES: usize = 256 * 1024;

/// Eingaben für [`compose`].
#[derive(Debug, Clone, Copy)]
pub struct ComposeInput<'a> {
    pub spec: &'a AgentSpec,
    /// Verzeichnis des Agents (für `instructions.file`).
    pub dir: &'a AgentDir,
    pub params: &'a ParamValues,
    /// Wert für `{{ now }}`.
    pub now: &'a str,
    /// Arbeitsverzeichnis der Session (Projektdateien).
    pub workdir: &'a Path,
    /// Projektdateien, die der Harness selbst liest.
    pub harness_reads: &'a [String],
}

/// Ergebnis: Text für den Harness (oder keiner) und Hinweise für das Event-Log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composed {
    pub text: Option<String>,
    /// Gelieferte Projektdateien in Reihenfolge.
    pub project_files: Vec<String>,
    pub notices: Vec<String>,
}

/// Welche Projektdateien in Frage kommen.
fn wanted(spec: &AgentSpec) -> Vec<String> {
    match spec
        .instructions
        .as_ref()
        .and_then(|i| i.project_files.as_ref())
    {
        None | Some(ProjectFiles::Mode(ProjectFilesMode::Auto)) => {
            AUTO_PROJECT_FILES.iter().map(|s| (*s).to_owned()).collect()
        }
        Some(ProjectFiles::Mode(ProjectFilesMode::None)) => Vec::new(),
        Some(ProjectFiles::List(list)) => list.clone(),
    }
}

fn inside(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Setzt die Instructions zusammen. Fehler (z. B. fehlende `instructions.file`) verhindern
/// den Start (fail closed).
pub fn compose(input: ComposeInput<'_>) -> Result<Composed, String> {
    let mut out = Composed::default();
    let mut parts: Vec<String> = Vec::new();
    if let Some(i) = &input.spec.instructions {
        match (&i.text, &i.file) {
            (Some(text), _) => parts.push(text.trim_end().to_owned()),
            (None, Some(file)) => {
                if !AgentDir::contains(file) {
                    return Err(format!(
                        "instructions.file: {file} liegt außerhalb des Agent-Verzeichnisses"
                    ));
                }
                let bytes = input.dir.read(file).ok_or_else(|| {
                    format!(
                        "instructions.file: {file} existiert nicht ({})",
                        input.dir.display()
                    )
                })?;
                parts.push(String::from_utf8_lossy(&bytes).trim_end().to_owned());
            }
            (None, None) => {}
        }
        if let Some(append) = &i.append {
            let rendered = template::render(append, input.params, input.now);
            if !rendered.trim().is_empty() {
                parts.push(rendered.trim_end().to_owned());
            }
        }
    }
    let explicit = matches!(
        input
            .spec
            .instructions
            .as_ref()
            .and_then(|i| i.project_files.as_ref()),
        Some(ProjectFiles::List(_))
    );
    for name in wanted(input.spec) {
        if input.harness_reads.iter().any(|r| r == &name) {
            continue;
        }
        if !inside(&name) {
            out.notices.push(format!(
                "project_files: {name} liegt außerhalb des Projekts und wird nicht geliefert"
            ));
            continue;
        }
        let path = input.workdir.join(&name);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if explicit {
                    out.notices
                        .push(format!("project_files: {name} fehlt im Projekt"));
                }
                continue;
            }
            Err(e) => {
                out.notices
                    .push(format!("project_files: {name} nicht lesbar: {e}"));
                continue;
            }
        };
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() > MAX_PROJECT_FILE_BYTES {
            let mut cut = MAX_PROJECT_FILE_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            out.notices.push(format!(
                "project_files: {name} ist größer als {} KiB und wurde gekürzt",
                MAX_PROJECT_FILE_BYTES / 1024
            ));
        }
        parts.push(format!(
            "--- Projektdatei {name} ---\n{}\n--- Ende {name} ---",
            text.trim_end()
        ));
        out.project_files.push(name);
    }
    if !parts.is_empty() {
        out.text = Some(parts.join("\n\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;
    use crate::load::parse_agent_yaml;

    fn spec(yaml: &str) -> AgentSpec {
        let p = parse_agent_yaml(yaml, "agent.yaml");
        assert!(p.diagnostics.iter().all(|d| !d.is_error()), "{p:?}");
        p.spec.unwrap()
    }

    struct Fx {
        _tmp: tempfile::TempDir,
        agent: AgentDir,
        work: std::path::PathBuf,
    }

    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(agent.join("prompts")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(agent.join("prompts/system.md"), "Du behebst CI-Fehler.\n").unwrap();
        std::fs::write(work.join("AGENTS.md"), "Regeln aus AGENTS.md").unwrap();
        std::fs::write(work.join("CLAUDE.md"), "Regeln aus CLAUDE.md").unwrap();
        Fx {
            _tmp: tmp,
            agent: AgentDir::Fs(agent),
            work,
        }
    }

    fn run(fx: &Fx, yaml: &str, reads: &[&str], params: &ParamValues) -> Composed {
        let reads: Vec<String> = reads.iter().map(|s| (*s).to_owned()).collect();
        compose(ComposeInput {
            spec: &spec(yaml),
            dir: &fx.agent,
            params,
            now: "2026-10-04T12:00:00Z",
            workdir: &fx.work,
            harness_reads: &reads,
        })
        .unwrap()
    }

    const BASE: &str = "spec_version: 1\nname: a\nexecutor: { harness: claude }\n";

    #[test]
    fn agt_005_ac2_project_files_auto_skips_what_the_harness_reads_itself() {
        let fx = fx();
        let yaml =
            format!("{BASE}instructions: {{ file: prompts/system.md, project_files: auto }}\n");
        // Claude liest CLAUDE.md selbst und bekommt AGENTS.md von beton.
        let claude = run(&fx, &yaml, &["CLAUDE.md"], &ParamValues::new());
        let text = claude.text.unwrap();
        assert_eq!(text.matches("Regeln aus AGENTS.md").count(), 1);
        assert!(!text.contains("Regeln aus CLAUDE.md"));
        assert_eq!(claude.project_files, ["AGENTS.md"]);
        // Codex liest AGENTS.md selbst und bekommt CLAUDE.md von beton.
        let codex = run(&fx, &yaml, &["AGENTS.md"], &ParamValues::new());
        let text = codex.text.unwrap();
        assert_eq!(text.matches("Regeln aus CLAUDE.md").count(), 1);
        assert!(!text.contains("Regeln aus AGENTS.md"));
        // Ein Harness, der keine liest, bekommt beide; `none` liefert keine.
        assert_eq!(
            run(&fx, &yaml, &[], &ParamValues::new()).project_files,
            ["AGENTS.md", "CLAUDE.md"]
        );
        let none = format!("{BASE}instructions: {{ text: x, project_files: none }}\n");
        assert_eq!(run(&fx, &none, &[], &ParamValues::new()).text.unwrap(), "x");
    }

    #[test]
    fn agt_005_instructions_come_from_file_or_text_plus_rendered_append() {
        let fx = fx();
        let yaml = format!(
            "{BASE}params: {{ branch: {{ type: string, default: main }} }}\ninstructions:\n  file: prompts/system.md\n  append: \"Branch: {{{{ params.branch }}}}\"\n  project_files: none\n"
        );
        let params = ParamValues::from([("branch".to_owned(), json!("develop"))]);
        let c = run(&fx, &yaml, &[], &params);
        assert_eq!(c.text.unwrap(), "Du behebst CI-Fehler.\n\nBranch: develop");
    }

    #[test]
    fn missing_instructions_file_fails_closed() {
        let fx = fx();
        let yaml = format!("{BASE}instructions: {{ file: prompts/fehlt.md }}\n");
        let err = compose(ComposeInput {
            spec: &spec(&yaml),
            dir: &fx.agent,
            params: &ParamValues::new(),
            now: "",
            workdir: &fx.work,
            harness_reads: &[],
        })
        .unwrap_err();
        assert!(err.contains("prompts/fehlt.md"), "{err}");
    }

    #[test]
    fn explicit_project_files_must_stay_inside_and_missing_ones_are_noted() {
        let fx = fx();
        let yaml = format!(
            "{BASE}instructions: {{ project_files: [../geheim.md, FEHLT.md, AGENTS.md] }}\n"
        );
        let c = run(&fx, &yaml, &[], &ParamValues::new());
        assert_eq!(c.project_files, ["AGENTS.md"]);
        assert_eq!(c.notices.len(), 2, "{:?}", c.notices);
    }
}
