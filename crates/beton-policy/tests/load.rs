//! POL-001: Policy-Dokumentformat, Ladeorte & Hot-Reload.

use std::path::Path;

use beton_policy::Phase;
use beton_policy::context::Activation;
use beton_policy::doc::{SCHEMA_COMMENT, schema_json};
use beton_policy::engine::{EvalOptions, Outcome};
use beton_policy::load::{Scope, inline_set, parse_set};
use beton_policy::store::{PolicyStore, StoreEvent};
use serde_json::json;

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Das Beispiel-Set aus docs/spec/03-policies.md („Policy-Dokument (Beispiel)“).
fn spec_example() -> String {
    let spec = std::fs::read_to_string(repo().join("docs/spec/03-policies.md")).unwrap();
    let start = spec.find("### Policy-Dokument (Beispiel)").expect("Abschnitt fehlt");
    let block = &spec[start..];
    let open = block.find("```yaml\n").expect("YAML-Block fehlt") + "```yaml\n".len();
    let close = block[open..].find("```").expect("Blockende fehlt");
    block[open..open + close].to_owned()
}

#[test]
fn pol_001_ac1_spec_example_loads() {
    let text = spec_example();
    let set = parse_set(Scope::Project, ".beton/policies/git-and-budget.yaml", &text)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(set.name, "git-and-budget");
    assert_eq!(set.scope, Scope::Project);
    let ids: Vec<&str> = set.rules.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "confirm-destructive-shell",
            "route-when-expensive",
            "count-pushes",
            "allow-docs",
            "budget"
        ]
    );
    assert!(set.version.starts_with("sha256:"));
    // Der Kommentar im Beispiel zeigt auf die lokale Schema-Datei (ADR-0033).
    assert!(text.contains(SCHEMA_COMMENT), "{text}");
}

#[test]
fn pol_001_ac1_generated_schema_matches_the_checked_in_file() {
    let path = repo().join("schemas/v1/policy.schema.json");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} fehlt: cargo xtask codegen", path.display()));
    assert_eq!(
        committed,
        schema_json(),
        "schemas/v1/policy.schema.json ist veraltet: cargo xtask codegen"
    );
    let schema: serde_json::Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    for field in ["spec_version", "name", "mode", "defaults", "rules"] {
        assert!(schema["properties"][field].is_object(), "{field} fehlt im Schema");
    }
}

#[test]
fn pol_001_schema_comment_is_a_local_path() {
    assert!(!SCHEMA_COMMENT.contains("://"));
    assert!(SCHEMA_COMMENT.ends_with("schemas/v1/policy.schema.json"));
}

#[test]
fn pol_001_ac4_duplicate_rule_ids_are_a_load_error() {
    let text = "spec_version: 1\nname: dup\nrules:\n  - id: a\n    on: tool_call\n    action: deny\n  - id: a\n    on: tool_call\n    action: allow\n";
    let err = parse_set(Scope::Project, "dup.yaml", text).unwrap_err();
    assert_eq!(err.file, "dup.yaml");
    assert_eq!(err.line, 7, "{err}");
    assert!(err.message.contains("doppelt") && err.message.contains('a'), "{err}");
}

fn load_err(text: &str) -> beton_policy::load::LoadError {
    parse_set(Scope::User, "x.yaml", text).expect_err("Ladefehler erwartet")
}

#[test]
fn pol_001_load_errors_carry_position() {
    // CEL-Typfehler: Zeile der Regel, Spalte im Ausdruck
    let err = load_err(
        "spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    when: session.cost_usd > \"5\"\n    action: deny\n",
    );
    assert_eq!((err.line, err.column), (6, 28), "{err}");
    assert!(err.message.contains("rules[0].when"), "{err}");

    // unbekanntes Feld
    let err = load_err("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: deny\n    wenn: true\n");
    assert_eq!(err.line, 7, "{err}");

    // YAML-Syntax
    let err = load_err("spec_version: 1\nname: [x\n");
    assert!(err.line >= 2, "{err}");
}

#[test]
fn pol_001_semantic_validation() {
    for (text, needle) in [
        ("spec_version: 2\nname: x\n", "spec_version"),
        ("spec_version: 1\nname: Gross\n", "name"),
        ("spec_version: 1\nname: x\nrules:\n  - id: Ungültig\n    on: tool_call\n    action: deny\n", "id"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    action: deny\n", "on"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n", "action"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: modify\n", "modify"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: model_request\n    action: modify\n    modify: { args: { a: 1 } }\n", "args"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_result\n    action: ask\n", "ask"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: deny\n    approval: { timeout: 30m }\n", "approval"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: ask\n    approval: { timeout: bald }\n", "dauer"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    type: git_guard\n    on: tool_call\n", "type"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: notify\n    notify: { message: \"{{ tool.nope }}\" }\n", "nope"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: allow\n    effects: [{ op: set, scope: session, key: k, value: \"1 +\" }]\n", "effects"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: allow\n    kinds: [shel]\n", "shel"),
        ("spec_version: 1\nname: x\nrules:\n  - id: r\n    on: tool_call\n    action: modify\n    modify: { redact: [\"(\"] }\n", "regex"),
    ] {
        let err = load_err(text);
        assert!(
            err.message.to_lowercase().contains(needle),
            "{text}\n→ {err} (erwartet `{needle}`)"
        );
    }
}

#[test]
fn pol_001_agent_level_loads_inline_rules() {
    let rules = vec![json!({ "id": "no-shell", "on": "tool_call", "when": "tool.kind == \"shell\"", "action": "deny" })];
    let set = inline_set(Scope::Agent, "pr-fixer", "agents/pr-fixer/agent.yaml", &rules).unwrap();
    assert_eq!(set.scope, Scope::Agent);
    assert_eq!(set.rules.len(), 1);
    let bad = vec![json!({ "id": "x", "on": "tool_call", "action": "deny", "when": "1" })];
    let err = inline_set(Scope::Agent, "pr-fixer", "agents/pr-fixer/agent.yaml", &bad).unwrap_err();
    assert!(err.message.contains("policies[0].when"), "{err}");
}

fn shell_call() -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": "ls" } }),
    )
    .unwrap()
}

const DENY_SHELL: &str = "spec_version: 1\nname: guard\nrules:\n  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"shell\"\n    action: deny\n    reason: kein Shell\n";
const ALLOW_ALL: &str = "spec_version: 1\nname: guard\nrules:\n  - id: no-shell\n    on: tool_call\n    when: tool.kind == \"nie\"\n    action: deny\n";

#[test]
fn pol_001_ac2_changed_project_file_applies_to_next_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let policies = dir.path().join(".beton/policies");
    std::fs::create_dir_all(&policies).unwrap();
    let file = policies.join("guard.yaml");
    std::fs::write(&file, DENY_SHELL).unwrap();

    let mut store = PolicyStore::new();
    store.add_dir(Scope::Project, &policies);
    let events = store.refresh();
    assert!(matches!(&events[..], [StoreEvent::Changed(c)] if c.scope == "project" && c.set == "guard"), "{events:?}");
    let first = store.effective();
    let d = first.evaluate(&shell_call(), &EvalOptions::default());
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "{d:?}");

    // Datei ändern, Session läuft weiter: nächste Auswertung mit neuer Version.
    std::fs::write(&file, ALLOW_ALL).unwrap();
    let events = store.refresh();
    assert!(matches!(&events[..], [StoreEvent::Changed(c)] if c.version.starts_with("sha256:")), "{events:?}");
    let second = store.effective();
    assert_ne!(first.hash(), second.hash());
    let d = second.evaluate(&shell_call(), &EvalOptions::default());
    assert_eq!(d.outcome, Outcome::Allow, "{d:?}");
    assert_eq!(d.policy_set_hash, second.hash());

    // Unveränderte Dateien erzeugen keine Ereignisse.
    assert!(store.refresh().is_empty());
}

#[test]
fn pol_001_ac3_broken_change_keeps_old_version_and_reports_file_line_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("guard.yaml");
    std::fs::write(&file, DENY_SHELL).unwrap();
    let mut store = PolicyStore::new();
    store.add_dir(Scope::Project, dir.path());
    store.refresh();
    let before = store.effective();

    let broken = DENY_SHELL.replace("tool.kind == \"shell\"", "tool.kind > 5");
    std::fs::write(&file, broken).unwrap();
    let events = store.refresh();
    let [StoreEvent::Invalid(inv)] = &events[..] else {
        panic!("{events:?}");
    };
    assert_eq!(inv.file, file.display().to_string());
    assert_eq!(inv.line, 6);
    assert!(inv.error.contains("rules[0].when"), "{}", inv.error);

    let after = store.effective();
    assert_eq!(before.hash(), after.hash());
    let d = after.evaluate(&shell_call(), &EvalOptions::default());
    assert!(matches!(d.outcome, Outcome::Deny { .. }), "alte Version bleibt aktiv");
    assert!(after.invalid().iter().any(|f| f == &file.display().to_string()));

    // Reparatur: wieder gültig, Fehlerliste leer.
    std::fs::write(&file, ALLOW_ALL).unwrap();
    store.refresh();
    assert!(store.effective().invalid().is_empty());
}

#[test]
fn pol_001_store_ignores_policy_tests_and_handles_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("guard.yaml"), DENY_SHELL).unwrap();
    std::fs::write(dir.path().join("x.policytest.yaml"), "kein: set").unwrap();
    std::fs::write(dir.path().join("notiz.txt"), "egal").unwrap();
    let mut store = PolicyStore::new();
    store.add_dir(Scope::User, dir.path());
    store.add_dir(Scope::Project, dir.path().join("fehlt"));
    let events = store.refresh();
    assert_eq!(events.len(), 1, "{events:?}");
    std::fs::remove_file(dir.path().join("guard.yaml")).unwrap();
    let events = store.refresh();
    assert!(matches!(&events[..], [StoreEvent::Changed(c)] if c.version == "deleted"), "{events:?}");
    assert_eq!(
        store.effective().evaluate(&shell_call(), &EvalOptions::default()).outcome,
        Outcome::Allow
    );
}
