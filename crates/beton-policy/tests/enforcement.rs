//! POL-024: Enforcement-Matrix und Prüfung beim Session-Start. Die Matrix selbst entsteht aus
//! den Harness-Capabilities (`beton-harness`); hier geht es um die Prüfung der Regeln gegen eine
//! gegebene Matrix und um die statische Analyse der betroffenen Tool-Klassen.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::enforcement::{KINDS, Level, Matrix, StartCheck, check_start, targets};
use beton_policy::engine::EffectiveSet;
use beton_policy::load::{LoadedSet, Scope, parse_set};

fn set(body: &str) -> Arc<LoadedSet> {
    let text = format!("spec_version: 1\nname: guard\nrules:\n{body}");
    Arc::new(
        parse_set(Scope::Project, "guard.yaml", &text).unwrap_or_else(|e| panic!("{e}\n{text}")),
    )
}

fn rule(id: &str, when: &str, action: &str) -> String {
    format!("  - id: {id}\n    on: tool_call\n    when: '{when}'\n    action: {action}\n")
}

/// Wie Codex nativ (POL-024, Tabelle): `shell`/`file_edit` über Freigaben, Lesen nur beobachtbar,
/// MCP voll über den beton-MCP-Proxy.
fn codex_like() -> Matrix {
    let mut m = Matrix::full();
    for kind in KINDS {
        m.tool_call.insert(kind.to_string(), Level::ObserveOnly);
        m.tool_result.insert(kind.to_string(), Level::ObserveOnly);
    }
    for kind in ["shell", "file_edit", "file_write"] {
        m.tool_call.insert(kind.into(), Level::ApprovalOnly);
    }
    for kind in ["mcp", "system", "browser"] {
        m.tool_call.insert(kind.into(), Level::Full);
        m.tool_result.insert(kind.into(), Level::Full);
    }
    m
}

fn kinds(list: &[&str]) -> Option<BTreeSet<String>> {
    Some(list.iter().map(|s| (*s).to_owned()).collect())
}

#[test]
fn pol_024_ac1_gap_on_reading_class_only_degrades() {
    let effective = EffectiveSet::new(vec![set(&rule(
        "no-secrets",
        "tool.kind == \"file_read\"",
        "deny",
    ))]);
    let check = check_start(&effective, &codex_like());
    assert!(check.refused.is_empty(), "{check:?}");
    assert_eq!(check.degraded.len(), 1, "{check:?}");
    assert_eq!(check.degraded[0].rule.rule_id, "no-secrets");
    assert_eq!(check.degraded[0].kinds, vec!["file_read".to_owned()]);
}

#[test]
fn pol_024_ac1_gap_on_mutating_class_refuses_start_and_names_the_rule() {
    let mut fake = Matrix::full();
    fake.tool_call
        .insert("file_edit".into(), Level::ObserveOnly);
    let effective = EffectiveSet::new(vec![set(&rule(
        "no-edits",
        "tool.kind == \"file_edit\"",
        "deny",
    ))]);
    let check = check_start(&effective, &fake);
    assert_eq!(check.refused.len(), 1, "{check:?}");
    assert_eq!(check.refused_rules(), vec!["no-edits".to_owned()]);
    assert_eq!(check.refused[0].phase, Phase::ToolCall);
    assert_eq!(check.refused[0].level, Level::ObserveOnly);
    // Mit voller Durchsetzung keine Lücke.
    assert!(check_start(&effective, &Matrix::full()).is_clean());
}

#[test]
fn pol_024_best_effort_notify_dry_run_and_allow_never_refuse() {
    let mut fake = Matrix::full();
    fake.tool_call.insert("shell".into(), Level::ObserveOnly);
    let body = [
        "  - id: soft\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: deny\n    enforcement: best_effort\n".to_owned(),
        rule("tell", "tool.kind == \"shell\"", "notify"),
        rule("ok", "tool.kind == \"shell\"", "allow"),
        "  - id: shadow\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: deny\n    dry_run: true\n".to_owned(),
    ]
    .concat();
    let check = check_start(&EffectiveSet::new(vec![set(&body)]), &fake);
    assert!(check.refused.is_empty(), "{check:?}");
    assert_eq!(
        check
            .degraded
            .iter()
            .map(|g| g.rule.rule_id.as_str())
            .collect::<Vec<_>>(),
        vec!["soft"]
    );
}

#[test]
fn pol_024_undeterminable_kinds_mean_all_classes() {
    let effective = EffectiveSet::new(vec![set(&rule(
        "generic",
        "tool.command.contains(\"rm\")",
        "deny",
    ))]);
    let check = check_start(&effective, &codex_like());
    assert_eq!(check.refused_rules(), vec!["generic".to_owned()]);
    let gap = &check.refused[0];
    assert!(gap.kinds.contains(&"other".to_owned()), "{gap:?}");
    assert!(
        !gap.kinds.contains(&"mcp".to_owned()),
        "MCP ist voll durchsetzbar"
    );
}

#[test]
fn pol_024_ask_and_modify_need_enforcement_too() {
    let mut fake = Matrix::full();
    fake.tool_call.insert("shell".into(), Level::ObserveOnly);
    let body = [
        rule("confirm", "tool.kind == \"shell\"", "ask"),
        "  - id: rewrite\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: modify\n    modify: { args: { command: ls } }\n".to_owned(),
    ]
    .concat();
    let check = check_start(&EffectiveSet::new(vec![set(&body)]), &fake);
    assert_eq!(
        check.refused_rules(),
        vec!["confirm".to_owned(), "rewrite".to_owned()]
    );
}

#[test]
fn pol_024_other_phases_use_their_level() {
    let mut m = Matrix::full();
    m.model_request = Level::ObserveOnly;
    let body = "  - id: no-opus\n    on: model_request\n    when: 'request.model.startsWith(\"opus\")'\n    action: deny\n  - id: start-ok\n    on: session_start\n    action: deny\n    when: 'false'\n";
    let check = check_start(&EffectiveSet::new(vec![set(body)]), &m);
    assert_eq!(check.refused_rules(), vec!["no-opus".to_owned()]);
    assert_eq!(check.refused[0].phase, Phase::ModelRequest);
    assert!(check.refused[0].kinds.is_empty());
}

#[test]
fn pol_024_builtin_rule_types_are_checked_for_all_classes_of_their_phases() {
    let body = "  - id: guard\n    type: git_guard\n";
    let check = check_start(&EffectiveSet::new(vec![set(body)]), &codex_like());
    assert_eq!(check.refused_rules(), vec!["guard".to_owned()]);
}

#[test]
fn pol_024_static_analysis_of_tool_classes() {
    let t = |when: &str| {
        let s = set(&rule("r", when, "deny"));
        targets(&s.rules[0])
    };
    assert_eq!(t("tool.kind == \"shell\"").kinds, kinds(&["shell"]));
    assert_eq!(t("\"shell\" == tool.kind").kinds, kinds(&["shell"]));
    assert_eq!(
        t("tool.kind in [\"shell\", \"file_edit\"]").kinds,
        kinds(&["file_edit", "shell"])
    );
    assert_eq!(
        t("tool.kind == \"shell\" || tool.kind == \"mcp\"").kinds,
        kinds(&["mcp", "shell"])
    );
    assert_eq!(
        t("tool.kind == \"shell\" && tool.command.contains(\"git\")").kinds,
        kinds(&["shell"])
    );
    // Tool-Namen über die Abbildungstabelle (POL-005).
    assert_eq!(t("tool.name == \"Bash\"").kinds, kinds(&["shell"]));
    assert_eq!(t("tool.name == \"Read\"").kinds, kinds(&["file_read"]));
    // Nicht bestimmbar: alle Klassen.
    assert_eq!(t("tool.command.contains(\"x\")").kinds, None);
    assert_eq!(t("tool.kind != \"shell\"").kinds, None);
    assert_eq!(t("tool.kind == \"shell\" || true").kinds, None);
    assert_eq!(t("tool.kind == \"shell\"").phases, vec![Phase::ToolCall]);
    // `kinds:` geht vor.
    let s = set(
        "  - id: r\n    on: tool_call\n    when: 'tool.command.contains(\"x\")'\n    action: deny\n    kinds: [shell]\n",
    );
    assert_eq!(targets(&s.rules[0]).kinds, kinds(&["shell"]));
}

#[test]
fn pol_024_matrix_reports_levels_and_modify_support() {
    let m = codex_like();
    assert_eq!(m.level(Phase::ToolCall, Some("shell")), Level::ApprovalOnly);
    assert_eq!(
        m.level(Phase::ToolCall, Some("file_read")),
        Level::ObserveOnly
    );
    assert_eq!(
        m.level(Phase::ToolCall, Some("unbekannt")),
        Level::ObserveOnly
    );
    assert_eq!(m.level(Phase::SessionStart, None), Level::Full);
    assert_eq!(Level::ApprovalOnly.as_str(), "approval_only");
    assert!(Level::ApprovalOnly.enforces());
    assert!(!Level::ObserveOnly.enforces());
    let full = Matrix::full();
    assert!(full.capabilities(Phase::ToolCall).args);
}

#[test]
fn pol_024_gaps_describe_themselves_without_file_contents() {
    let mut m = codex_like();
    m.model_request = Level::ObserveOnly;
    let body = [
        rule("no-push", "tool.kind == \"shell\"", "deny"),
        "  - id: no-opus\n    on: model_request\n    when: 'request.model == \"opus\"'\n    action: deny\n    reason: geheim-123\n".to_owned(),
        "  - id: soft-model\n    on: model_request\n    when: 'request.model == \"x\"'\n    action: deny\n    enforcement: best_effort\n".to_owned(),
    ]
    .concat();
    let check = check_start(&EffectiveSet::new(vec![set(&body)]), &m);
    assert!(!check.is_clean());
    assert_eq!(check.refused_rules(), vec!["no-opus".to_owned()]);
    assert_eq!(check.degraded_rules(), vec!["soft-model".to_owned()]);
    let text = check.refused[0].describe();
    assert!(
        text.contains("no-opus") && text.contains("observe_only"),
        "{text}"
    );
    assert!(
        !text.contains("geheim-123"),
        "keine Inhalte der Datei: {text}"
    );
    // Mit Klassen: Lücke auf lesenden Klassen nennt sie.
    let read = rule("no-read", "tool.kind == \"file_read\"", "deny");
    let check = check_start(&EffectiveSet::new(vec![set(&read)]), &codex_like());
    let text = check.degraded[0].describe();
    assert!(text.contains("file_read"), "{text}");
    assert!(StartCheck::default().is_clean());
}

#[test]
fn pol_024_dry_run_sets_and_weakest_level_across_classes() {
    let mut fake = Matrix::full();
    fake.tool_call.insert("shell".into(), Level::ObserveOnly);
    let shadow = "spec_version: 1\nname: shadow\nmode: dry_run\nrules:\n  - id: r\n    on: tool_call\n    when: 'tool.kind == \"shell\"'\n    action: deny\n";
    let loaded = Arc::new(parse_set(Scope::Project, "shadow.yaml", shadow).unwrap());
    assert!(check_start(&EffectiveSet::new(vec![loaded]), &fake).is_clean());
    // Ohne Klasse gilt die schwächste Stufe aller Klassen.
    assert_eq!(fake.level(Phase::ToolCall, None), Level::ObserveOnly);
    assert_eq!(Matrix::full().level(Phase::ToolResult, None), Level::Full);
    assert_eq!(
        Matrix::full().level(Phase::BrowserNavigate, None),
        Level::Full
    );
    assert_eq!(Level::Full.to_string(), "full");
    assert_eq!(Level::ObserveOnly.to_string(), "observe_only");
}
