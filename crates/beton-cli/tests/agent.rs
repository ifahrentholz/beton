//! `beton agent` (CLI-009; Semantik AGT-002, AGT-003, AGT-013). Ohne Daemon.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::process::Command;

use common::{run, stderr, stdout};
use serde_json::Value;

/// Projekt (eigenes Git-Repo) und Datenverzeichnis; `HOME` zeigt in den Test, damit keine
/// Skills des echten Users die Prüfung beeinflussen.
struct Env {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(env.project.path().join(".git")).unwrap();
        env
    }

    fn beton(&self) -> Command {
        let mut c = common::beton(self.home.path());
        c.current_dir(self.project.path())
            .env("HOME", self.home.path());
        c
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.project.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn json(o: &std::process::Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("kein JSON ({e}): {}", stdout(o)))
}

#[test]
fn cli_009_ac1_validate_reports_schema_error_with_path_and_exit_1() {
    let env = Env::new();
    env.write(
        "my-agent/agent.yaml",
        "spec_version: 1\nname: my-agent\nexecutor:\n  harness: claude-code\n",
    );
    let out = run(env.beton().args(["agent", "validate", "./my-agent"]));
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("agent.yaml:4:3"), "{text}");
    assert!(text.contains("executor.harness"), "{text}");
    assert!(text.contains("Meintest du „claude“?"), "{text}");
    assert!(stderr(&out).contains("1 Fehler"), "{}", stderr(&out));
}

#[test]
fn agt_002_ac2_validate_exit_codes_and_json_diagnostics() {
    let env = Env::new();
    env.write(
        "ok/agent.yaml",
        "spec_version: 1\nname: ok\nexecutor:\n  harness: claude\n",
    );
    let out = run(env.beton().args(["agent", "validate", "./ok", "--json"]));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(json(&out), Value::Array(Vec::new()));
    let out = run(env.beton().args(["agent", "validate", "ok"]));
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("✓ ok ist gültig"));

    env.write(
        "bad/agent.yaml",
        "spec_version: 1\nname: bad\nexecutor:\n  harness: claude\ninstruction:\n  text: x\nskills: [ci-triage]\n",
    );
    let out = run(env.beton().args(["agent", "validate", "bad", "--json"]));
    assert_eq!(out.status.code(), Some(1));
    let items = json(&out);
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 2, "{items:#?}");
    for item in items {
        for key in ["path", "line", "column", "code", "message"] {
            assert!(!item[key].is_null(), "{key} fehlt: {item}");
        }
    }
    assert_eq!(items[0]["code"], "unknown_field");
    assert_eq!(items[0]["line"], 5);
    assert_eq!(items[1]["code"], "skill_not_found");

    // Ein nicht vorhandener Agent ist ebenfalls Exit 1 mit Befund.
    let out = run(env.beton().args(["agent", "validate", "./fehlt", "--json"]));
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&out)[0]["code"], "missing_agent_yaml");
}

#[test]
fn agt_013_ac1_new_demo_creates_a_structure_that_validates() {
    let env = Env::new();
    let out = run(env.beton().args(["agent", "new", "demo"]));
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );
    let dir = env.project.path().join(".beton/agents/demo");
    for f in ["agent.yaml", "prompts/system.md", "skills/example/SKILL.md"] {
        assert!(dir.join(f).is_file(), "{f} fehlt");
    }
    let yaml = std::fs::read_to_string(dir.join("agent.yaml")).unwrap();
    let first = yaml.lines().next().unwrap();
    assert_eq!(
        first,
        "# yaml-language-server: $schema=../../schemas/v1/agent.schema.json"
    );
    assert!(!yaml.contains("http"), "{yaml}");
    // Die referenzierte Schema-Datei liegt lokal und ist das aktuelle Schema.
    let schema = dir.join("../../schemas/v1/agent.schema.json");
    assert_eq!(
        std::fs::read_to_string(schema).unwrap(),
        beton_agents::schema_json()
    );
    assert!(stdout(&out).contains("gültig"));

    let out = run(env
        .beton()
        .args(["agent", "validate", ".beton/agents/demo"]));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    // Ein zweites `new` überschreibt nichts.
    let out = run(env.beton().args(["agent", "new", "demo"]));
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("existiert bereits"));
}

#[test]
fn cli_009_ac2_new_reviewer_passes_validate() {
    let env = Env::new();
    let out = run(env.beton().args(["agent", "new", "reviewer"]));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = run(env
        .beton()
        .args(["agent", "validate", "./.beton/agents/reviewer"]));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    // Ungültige Namen sind Usage-Fehler.
    let out = run(env.beton().args(["agent", "new", "Bad_Name"]));
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn agt_013_new_from_copies_an_existing_agent_under_a_new_name() {
    let env = Env::new();
    assert!(
        run(env.beton().args(["agent", "new", "base"]))
            .status
            .success()
    );
    let out = run(env.beton().args(["agent", "new", "copy", "--from", "base"]));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let yaml =
        std::fs::read_to_string(env.project.path().join(".beton/agents/copy/agent.yaml")).unwrap();
    assert!(yaml.contains("\nname: copy\n"), "{yaml}");
    assert!(
        env.project
            .path()
            .join(".beton/agents/copy/skills/example/SKILL.md")
            .is_file()
    );
}

#[test]
fn agt_013_ac2_show_json_returns_the_resolved_agent_with_hash() {
    let env = Env::new();
    assert!(
        run(env.beton().args(["agent", "new", "maestra"]))
            .status
            .success()
    );
    let out = run(env.beton().args(["agent", "show", "maestra", "--json"]));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let agent = json(&out);
    assert_eq!(agent["name"], "maestra");
    assert_eq!(agent["source"], "project");
    assert_eq!(agent["version"], "0.1.0");
    let hash = agent["hash"].as_str().unwrap();
    assert!(
        hash.starts_with("sha256:") && hash.len() == 7 + 64,
        "{hash}"
    );
    assert_eq!(agent["spec"]["executor"]["harness"], "claude");
    assert_eq!(agent["origins"]["executor.harness"], "agent.yaml:8");
    assert_eq!(agent["origins"]["executor.mode"], "default");

    // Gleiche Dateien, gleicher Hash; eine geänderte Prompt-Datei ändert ihn.
    let again = json(&run(env
        .beton()
        .args(["agent", "show", "maestra", "--json"])));
    assert_eq!(again["hash"], agent["hash"]);
    env.write(".beton/agents/maestra/prompts/system.md", "Neu.\n");
    let changed = json(&run(env
        .beton()
        .args(["agent", "show", "maestra", "--json"])));
    assert_ne!(changed["hash"], agent["hash"]);

    // Textausgabe: Kopfzeile und Herkunft je Feld.
    let out = run(env.beton().args(["agent", "show", "maestra"]));
    let text = stdout(&out);
    assert!(text.starts_with("maestra 0.1.0 · project · "), "{text}");
    assert!(text.contains("executor.harness"), "{text}");
    assert!(text.contains("agent.yaml:8"), "{text}");
}

#[test]
fn agt_003_ac2_list_shows_name_source_version_harness_and_path() {
    let env = Env::new();
    assert!(
        run(env.beton().args(["agent", "new", "pr-fixer"]))
            .status
            .success()
    );
    let user = env.home.path().join("agents/dep-updater");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(
        user.join("agent.yaml"),
        "spec_version: 1\nname: dep-updater\nversion: 1.2.0\nexecutor:\n  harness: codex\n",
    )
    .unwrap();
    let out = run(env.beton().args(["agent", "list"]));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    let header: Vec<&str> = text.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(header, ["NAME", "QUELLE", "VERSION", "HARNESS", "PFAD"]);
    let row = |name: &str| -> Vec<String> {
        text.lines()
            .find(|l| l.starts_with(name))
            .unwrap()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        row("pr-fixer"),
        [
            "pr-fixer",
            "project",
            "0.1.0",
            "claude",
            ".beton/agents/pr-fixer"
        ]
    );
    assert_eq!(
        row("dep-updater"),
        [
            "dep-updater",
            "user",
            "1.2.0",
            "codex",
            "~/agents/dep-updater"
        ]
    );
    let list = json(&run(env.beton().args(["agent", "list", "--json"])));
    let first = &list[0];
    for key in ["name", "source", "version", "harness", "path"] {
        assert!(!first[key].is_null(), "{key} fehlt: {first}");
    }
}

#[test]
fn agt_003_ac3_list_all_shows_a_dir_without_agent_yaml_as_invalid() {
    let env = Env::new();
    assert!(
        run(env.beton().args(["agent", "new", "pr-fixer"]))
            .status
            .success()
    );
    std::fs::create_dir_all(
        env.project
            .path()
            .join(".beton/agents/old-reviewer/prompts"),
    )
    .unwrap();
    let out = run(env.beton().args(["agent", "list"]));
    assert!(!stdout(&out).contains("old-reviewer"), "{}", stdout(&out));
    let out = run(env.beton().args(["agent", "list", "--all"]));
    let line = stdout(&out)
        .lines()
        .find(|l| l.starts_with("old-reviewer"))
        .map(str::to_owned)
        .unwrap();
    assert!(line.contains("ungültig"), "{line}");
    assert!(line.contains("keine agent.yaml"), "{line}");
    let all = json(&run(env.beton().args(["agent", "list", "--all", "--json"])));
    let old = all
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "old-reviewer")
        .unwrap();
    assert_eq!(old["valid"], false);
}

#[test]
fn cli_009_schema_prints_the_published_schema() {
    let env = Env::new();
    let out = run(env.beton().args(["agent", "schema"]));
    assert_eq!(out.status.code(), Some(0));
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1/agent.schema.json"),
    )
    .unwrap();
    assert_eq!(stdout(&out), committed);
}

#[test]
fn cli_009_agent_commands_do_not_create_log_files() {
    let env = Env::new();
    run(env.beton().args(["agent", "list"]));
    assert!(!env.home.path().join("logs").exists());
}
