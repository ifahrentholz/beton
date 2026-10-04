//! Akzeptanztests für Format, Schema und Validierung (AGT-001, AGT-002).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_agents::load::parse_agent_yaml;
use beton_agents::{
    AgentDir, AgentRef, Builtins, HarnessCatalog, Located, Report, SearchPath, Severity, Source,
    Validator, schema_json,
};
use beton_harness::HarnessId;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Der erste YAML-Block nach „Vollständiges Beispiel“ in docs/spec/02-agents.md.
fn spec_example() -> String {
    let text = std::fs::read_to_string(repo().join("docs/spec/02-agents.md")).unwrap();
    let start = text.find("### Vollständiges Beispiel").unwrap();
    let block = &text[start..];
    let open = block.find("```yaml\n").unwrap() + "```yaml\n".len();
    let close = block[open..].find("```").unwrap();
    block[open..open + close].to_owned()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn catalog() -> HarnessCatalog {
    let mut c = HarnessCatalog::new();
    c.insert(&"claude".parse::<HarnessId>().unwrap(), Vec::new());
    c.insert(
        &"fake".parse::<HarnessId>().unwrap(),
        vec!["low".into(), "medium".into(), "high".into()],
    );
    c
}

fn validate_dir(dir: &Path, search: &SearchPath) -> Report {
    let harnesses = catalog();
    let validator = Validator {
        search,
        harnesses: &harnesses,
        skill_roots: Vec::new(),
    };
    let located = Located {
        name: dir.file_name().unwrap().to_string_lossy().into_owned(),
        source: Source::Path,
        dir: AgentDir::Fs(dir.to_path_buf()),
        shadows: None,
    };
    validator.validate(&located)
}

fn minimal(name: &str, extra: &str) -> String {
    format!("spec_version: 1\nname: {name}\nexecutor:\n  harness: fake\n{extra}")
}

#[test]
fn agt_001_ac1_spec_example_loads_without_errors() {
    let example = spec_example();
    let parsed = parse_agent_yaml(&example, "agent.yaml");
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let spec = parsed.spec.unwrap();
    assert_eq!(spec.name.as_str(), "pr-fixer");
    assert_eq!(spec.schedules.len(), 1);
    assert!(spec.async_.is_some() && spec.sandbox.is_some() && spec.timers == Some(true));

    // Mit den referenzierten Dateien besteht es auch die semantische Prüfung.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pr-fixer");
    write(&dir, "agent.yaml", &example);
    write(&dir, "prompts/system.md", "Du behebst CI-Fehler.\n");
    write(
        &dir,
        "skills/fix-ci/SKILL.md",
        "---\nname: fix-ci\ndescription: x\n---\n",
    );
    write(
        &dir,
        "skills/ci-triage/SKILL.md",
        "---\nname: ci-triage\ndescription: x\n---\n",
    );
    write(&dir, "agents/reviewer/agent.yaml", &minimal("reviewer", ""));
    write(
        &dir,
        "policies/strict.yaml",
        "spec_version: 1\nname: strict\n",
    );
    let report = validate_dir(&dir, &SearchPath::default());
    assert!(report.is_valid(), "{:#?}", report.diagnostics);
    // Codex ist in dieser Installation (Test-Katalog) nicht registriert: nur eine Warnung.
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
}

#[test]
fn agt_001_ac2_unknown_field_reports_file_line_column_and_suggestion() {
    let text = "spec_version: 1\nname: pr-fixer\nexecutor:\n  harness: claude\ninstruction:\n  text: hallo\n";
    let parsed = parse_agent_yaml(text, ".beton/agents/pr-fixer/agent.yaml");
    let d = parsed
        .diagnostics
        .iter()
        .find(|d| d.code == "unknown_field")
        .unwrap();
    assert_eq!(d.file, ".beton/agents/pr-fixer/agent.yaml");
    assert_eq!((d.line, d.column), (5, 1));
    assert_eq!(d.path, "instruction");
    assert!(
        d.message.contains("Meintest du „instructions“?"),
        "{}",
        d.message
    );
    assert_eq!(d.severity, Severity::Error);

    // Auch verschachtelt und mehrere auf einmal.
    let text =
        "spec_version: 1\nname: x\nexecutor:\n  harness: claude\n  modell: opus\ntoolz: {}\n";
    let parsed = parse_agent_yaml(text, "agent.yaml");
    let found: Vec<(&str, usize, usize)> = parsed
        .diagnostics
        .iter()
        .map(|d| (d.path.as_str(), d.line, d.column))
        .collect();
    assert_eq!(found, [("executor.modell", 5, 3), ("toolz", 6, 1)]);
    assert!(parsed.diagnostics[0].message.contains("„model“"));

    // `x-`-Felder sind auf oberster Ebene erlaubt.
    let parsed = parse_agent_yaml(&minimal("x", "x-team: platform\n"), "agent.yaml");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.spec.unwrap().extensions["x-team"], "platform");
    // `os_env` ist kein Alias für `sandbox`.
    let parsed = parse_agent_yaml(&minimal("x", "os_env: {}\n"), "agent.yaml");
    assert_eq!(parsed.diagnostics[0].code, "unknown_field");
}

#[test]
fn agt_001_ac4_spec_version_2_is_rejected_as_unsupported() {
    let text = "spec_version: 2\nname: x\nexecutor:\n  harness: claude\nneues_feld: 1\n";
    let parsed = parse_agent_yaml(text, "agent.yaml");
    assert!(parsed.spec.is_none());
    assert_eq!(parsed.diagnostics.len(), 1, "{:?}", parsed.diagnostics);
    let d = &parsed.diagnostics[0];
    assert_eq!(d.code, "unsupported_spec_version");
    assert!(
        d.message
            .to_lowercase()
            .contains("nicht unterstützte spec-version"),
        "{}",
        d.message
    );
    assert_eq!((d.line, d.column), (1, 1));
}

#[test]
fn agt_002_ac1_generated_schema_matches_the_checked_in_file() {
    let path = repo().join("schemas/v1/agent.schema.json");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} fehlt: cargo xtask codegen", path.display()));
    assert_eq!(
        committed,
        schema_json(),
        "schemas/v1/agent.schema.json ist veraltet: cargo xtask codegen"
    );
    let schema: serde_json::Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["patternProperties"]["^x-"].is_object());
    for field in [
        "spec_version",
        "name",
        "description",
        "version",
        "executor",
        "instructions",
        "params",
        "tools",
        "skills",
        "agents",
        "spawn",
        "timers",
        "schedules",
        "async",
        "policies",
        "sandbox",
    ] {
        assert!(
            schema["properties"][field].is_object(),
            "{field} fehlt im Schema"
        );
    }
}

#[test]
fn agt_002_ac2_diagnostics_have_path_line_column_code_and_message() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("my-agent");
    write(
        &dir,
        "agent.yaml",
        "spec_version: 1\nname: my-agent\nexecutor:\n  harness: fake\nskills: [fix-ci]\npolicies:\n  - ref: ./policies/strict.yaml\n",
    );
    let report = validate_dir(&dir, &SearchPath::default());
    assert!(!report.is_valid());
    let json = serde_json::to_value(&report.diagnostics).unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 2, "{json:#}");
    for item in items {
        for key in ["path", "line", "column", "code", "message"] {
            assert!(!item[key].is_null(), "{key} fehlt in {item}");
        }
    }
    assert_eq!(items[0]["code"], "skill_not_found");
    assert_eq!(items[0]["path"], "skills[0]");
    assert_eq!(
        (items[0]["line"].as_u64(), items[0]["column"].as_u64()),
        (Some(5), Some(10))
    );
    assert_eq!(items[1]["code"], "file_not_found");
    assert_eq!(items[1]["path"], "policies[0].ref");
}

#[test]
fn agt_002_ac3_subagent_cycle_is_reported_as_agent_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("loop-a");
    let b = tmp.path().join("loop-b");
    write(
        &a,
        "agent.yaml",
        &minimal("loop-a", "agents:\n  b: { ref: ../loop-b }\n"),
    );
    write(
        &b,
        "agent.yaml",
        &minimal("loop-b", "agents:\n  a: { ref: ../loop-a }\n"),
    );
    let report = validate_dir(&a, &SearchPath::default());
    let cycle: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "agent_cycle")
        .collect();
    assert_eq!(cycle.len(), 1, "{:#?}", report.diagnostics);
    assert!(
        cycle[0].message.contains("loop-a → loop-b → loop-a"),
        "{}",
        cycle[0].message
    );
    assert_eq!(cycle[0].path, "agents.a.ref");
    assert!(
        cycle[0].file.ends_with("loop-b/agent.yaml"),
        "{}",
        cycle[0].file
    );
    assert!(!report.is_valid());

    // Auch über den Suchpfad und bei Selbstbezug.
    let project = tmp.path().join("proj");
    write(
        &project.join("self"),
        "agent.yaml",
        &minimal("self", "agents:\n  me: { ref: self }\n"),
    );
    let search = SearchPath {
        project: Some(project.clone()),
        ..SearchPath::default()
    };
    let report = validate_dir(&project.join("self"), &search);
    assert!(report.diagnostics.iter().any(|d| d.code == "agent_cycle"));
}

#[test]
fn agt_002_ac4_unsupported_effort_warns_with_effective_value() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("x");
    write(
        &dir,
        "agent.yaml",
        "spec_version: 1\nname: x\nexecutor:\n  harness: fake\n  model: fake-1\n  reasoning_effort: xhigh\n",
    );
    let report = validate_dir(&dir, &SearchPath::default());
    assert!(report.is_valid(), "{:#?}", report.diagnostics);
    let d = &report.diagnostics[0];
    assert_eq!(d.code, "effort_mapped");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(d.path, "executor.reasoning_effort");
    assert!(d.message.contains("„xhigh“ → „high“"), "{}", d.message);
    assert!(d.message.contains("effektiv „high“"), "{}", d.message);

    // Ein Harness ohne Effort-Stufen wendet den Wert nicht an.
    write(
        &dir,
        "agent.yaml",
        "spec_version: 1\nname: x\nexecutor:\n  harness: claude\n  reasoning_effort: xhigh\n",
    );
    let report = validate_dir(&dir, &SearchPath::default());
    assert_eq!(report.diagnostics[0].code, "effort_mapped");
    assert!(report.diagnostics[0].message.contains("nicht angewendet"));
}

#[test]
fn agt_002_semantic_rules_cover_files_subagents_and_spawn() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("x");
    write(
        &dir,
        "agent.yaml",
        "spec_version: 1
name: x
executor:
  harness: fake
instructions:
  file: ../outside.md
params:
  n: { type: integer, default: 20, minimum: 1, maximum: 10 }
  mode: { type: enum }
agents:
  missing: { ref: ./agents/missing }
  inline: { instructions: { text: hi } }
spawn:
  agents: [missing, ghost]
policies:
  - { id: Bad_Id, type: git_guard }
",
    );
    let report = validate_dir(&dir, &SearchPath::default());
    let codes: Vec<(&str, &str)> = report
        .diagnostics
        .iter()
        .map(|d| (d.code, d.path.as_str()))
        .collect();
    for expected in [
        ("path_outside_agent", "instructions.file"),
        ("invalid_param", "params.n.default"),
        ("invalid_param", "params.mode"),
        ("agent_not_found", "agents.missing.ref"),
        ("missing_field", "agents.inline"),
        ("unknown_subagent", "spawn.agents[1]"),
        ("invalid_value", "policies[0].id"),
    ] {
        assert!(
            codes.contains(&expected),
            "{expected:?} fehlt in {codes:#?}"
        );
    }
}

#[test]
fn agt_002_unknown_harness_is_a_schema_error_with_suggestion() {
    let parsed = parse_agent_yaml(
        "spec_version: 1\nname: x\nexecutor:\n  harness: claude-code\n",
        "agent.yaml",
    );
    let d = &parsed.diagnostics[0];
    assert_eq!(d.code, "unknown_harness");
    assert_eq!(d.path, "executor.harness");
    assert_eq!((d.line, d.column), (4, 3));
    assert!(d.message.contains("Meintest du „claude“?"), "{}", d.message);
}

#[test]
fn agt_003_builtin_subagent_refs_resolve_inside_the_embedded_tree() {
    let builtins = Builtins::from_files([
        (
            "maestra/agent.yaml",
            "spec_version: 1\nname: maestra\nexecutor:\n  harness: fake\nagents:\n  impl: { ref: ./agents/impl }\n",
        ),
        (
            "maestra/agents/impl/agent.yaml",
            "spec_version: 1\nname: impl\nexecutor:\n  harness: fake\n",
        ),
    ]);
    let search = SearchPath {
        builtins,
        ..SearchPath::default()
    };
    let located = search
        .resolve(&AgentRef::parse("builtin:maestra").unwrap(), Path::new("."))
        .unwrap();
    let harnesses = catalog();
    let report = Validator {
        search: &search,
        harnesses: &harnesses,
        skill_roots: Vec::new(),
    }
    .validate(&located);
    assert!(report.is_valid(), "{:#?}", report.diagnostics);
}

// --------------------------------------------------------------------------- AGT-005 / AGT-010

#[test]
fn agt_005_ac3_missing_instructions_file_fails_validation_with_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("x");
    write(
        &dir,
        "agent.yaml",
        &minimal("x", "instructions:\n  file: prompts/system.md\n"),
    );
    let report = validate_dir(&dir, &SearchPath::default());
    assert!(!report.is_valid());
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.code == "file_not_found")
        .unwrap();
    assert_eq!(d.path, "instructions.file");
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("prompts/system.md"), "{}", d.message);
    assert_eq!((d.line, d.column), (6, 3));
}

#[test]
fn agt_010_templates_only_in_append_and_only_with_declared_params() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("x");
    write(
        &dir,
        "agent.yaml",
        &minimal(
            "x",
            "params:\n  branch: { type: string, default: main }\ninstructions:\n  text: \"Branch {{ params.branch }}\"\n  append: \"{{ params.branch }} {{ params.fehlt }} {{ now }} {{ env.HOME }}\"\n",
        ),
    );
    let report = validate_dir(&dir, &SearchPath::default());
    let found: Vec<(&str, &str)> = report
        .diagnostics
        .iter()
        .map(|d| (d.code, d.path.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("invalid_value", "instructions.text"),
            ("invalid_param", "instructions.append"),
            ("invalid_value", "instructions.append"),
        ],
        "{:#?}",
        report.diagnostics
    );
}
