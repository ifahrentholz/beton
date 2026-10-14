//! QA-004: Gate über alle mitgelieferten Policies und Built-in-Agents. Die Policy-Tests unter
//! `policies/tests/` laufen grün (in CI zusätzlich über `beton policy test` mit JUnit), jede
//! mitgelieferte Policy hat mindestens eine Testdatei (AC2), und jeder entzuckerte eingebaute
//! Regeltyp hat seine mitgelieferte Testdatei im POL-026-Format (POL-026 AC4).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use beton_policy::doc::RuleType;
use beton_policy::load::{RuleKind, Scope, parse_set};
use beton_policy::policytest::{Summary, TestOptions, discover, layer_files, render, run_paths};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn tests_dir() -> PathBuf {
    root().join("policies/tests")
}

/// Mitgelieferte Policy-Dateien: `policies/*.yaml` (ohne Tests und Testdaten).
fn shipped_policy_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root().join("policies"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e == "yaml")
                && !p.to_string_lossy().ends_with(".policytest.yaml")
        })
        .collect();
    out.sort();
    out
}

/// Built-in-Agents mit `policies:`-Block (auch Sub-Agents).
fn agents_with_policies() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.file_name().is_some_and(|n| n == "agent.yaml") {
                let text = std::fs::read_to_string(&path).unwrap();
                if text.lines().any(|l| l.starts_with("policies:")) {
                    out.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join("agents"), &mut out);
    out.sort();
    out
}

/// Built-in-Agents (oberste Ebene unter `agents/`).
fn builtin_agents() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root().join("agents"))
        .unwrap()
        .map(|e| e.unwrap().path().join("agent.yaml"))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

fn covered() -> BTreeSet<PathBuf> {
    discover(&[tests_dir()])
        .unwrap()
        .iter()
        .flat_map(|t| layer_files(t, &TestOptions::default()).unwrap())
        .map(|p| p.canonicalize().unwrap())
        .collect()
}

#[test]
fn qa_004_shipped_policy_tests_pass() {
    let reports = run_paths(&[tests_dir()], &TestOptions::default()).unwrap();
    let summary = Summary::of(&reports);
    assert!(summary.passed > 0, "keine Policy-Tests gefunden");
    assert_eq!(summary.exit_code(), 0, "{}", render(&reports));
}

#[test]
fn qa_004_ac2_every_shipped_policy_and_builtin_agent_has_a_test() {
    let covered = covered();
    let mut missing = Vec::new();
    for file in shipped_policy_files()
        .into_iter()
        .chain(agents_with_policies())
        .chain(builtin_agents())
    {
        if !covered.contains(&file.canonicalize().unwrap()) {
            missing.push(file.strip_prefix(root()).unwrap().display().to_string());
        }
    }
    missing.dedup();
    assert!(
        missing.is_empty(),
        "ohne Policy-Test unter policies/tests/: {missing:?}"
    );
}

#[test]
fn qa_004_hierarchy_cases_are_part_of_the_gate() {
    let file = tests_dir().join("hierarchy.policytest.yaml");
    let text = std::fs::read_to_string(&file).unwrap();
    for scope in ["user:", "project:", "agent:"] {
        assert!(
            text.contains(scope),
            "Ebene {scope} fehlt in {}",
            file.display()
        );
    }
}

/// Regeltypen, die noch nicht zu CEL entzuckert werden (mit ihrem Arbeitspaket). Wird ein Typ
/// entzuckert, schlägt dieser Test fehl, bis er hier entfernt ist und seine Testdatei hat.
const PENDING: [(RuleType, &str); 9] = [
    (RuleType::SpendCap, "WP-43, #166"),
    (RuleType::DailyBudget, "WP-43, #166"),
    (RuleType::ModelRoute, "WP-43, #166"),
    (RuleType::ToolAllowlist, "WP-42, #165"),
    (RuleType::PathGuard, "WP-42, #165"),
    (RuleType::GitGuard, "WP-42, #165"),
    (RuleType::LoopDetection, "WP-42, #165"),
    (RuleType::PiiRedaction, "WP-42, #165"),
    (RuleType::BrowserGuard, "M3"),
];

const ALL_TYPES: [RuleType; 10] = [
    RuleType::SpendCap,
    RuleType::DailyBudget,
    RuleType::ModelRoute,
    RuleType::RequireApproval,
    RuleType::ToolAllowlist,
    RuleType::PathGuard,
    RuleType::GitGuard,
    RuleType::LoopDetection,
    RuleType::PiiRedaction,
    RuleType::BrowserGuard,
];

/// Wird der Regeltyp beim Laden entzuckert? Ein noch nicht umgesetzter Typ bleibt
/// `RuleKind::Builtin`; ein umgesetzter wird zu CEL oder prüft seine Parameter.
fn desugared(t: RuleType) -> bool {
    let text = format!(
        "spec_version: 1\nname: t\nrules:\n  - {{ id: x, type: {}, params: {{ kinds: [shell] }} }}\n",
        t.as_str()
    );
    match parse_set(Scope::User, "t.yaml", &text) {
        Ok(set) => !matches!(set.rules[0].kind, RuleKind::Builtin(_)),
        Err(_) => true,
    }
}

#[test]
fn pol_026_ac4_every_builtin_rule_type_has_a_shipped_test_file() {
    let dir = tests_dir().join("rule-types");
    for t in ALL_TYPES {
        let pending = PENDING.iter().find(|(p, _)| *p == t);
        let file = dir.join(format!("{}.policytest.yaml", t.as_str()));
        match pending {
            Some((_, wp)) => assert!(
                !desugared(t),
                "`{}` wird entzuckert: Testdatei {} anlegen und aus PENDING ({wp}) entfernen",
                t.as_str(),
                file.display()
            ),
            None => {
                assert!(desugared(t), "`{}` ist nicht entzuckert", t.as_str());
                assert!(file.is_file(), "Testdatei fehlt: {}", file.display());
                let reports = run_paths(&[file], &TestOptions::default()).unwrap();
                let summary = Summary::of(&reports);
                assert!(summary.passed > 0, "{}", t.as_str());
                assert_eq!(summary.exit_code(), 0, "{}", render(&reports));
            }
        }
    }
}
