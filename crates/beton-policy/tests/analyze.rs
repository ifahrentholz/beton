//! Statische Analyse geprüfter Ausdrücke: benutzte Kontextfelder (POL-004 AC3, der Runner holt
//! `session.cost_usd` und `user.daily_cost_usd` nur bei Bedarf) und betroffene Tool-Klassen
//! (POL-024).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use beton_policy::Phase;
use beton_policy::cel::compile_bool;

fn fields(src: &str, phase: Phase) -> BTreeSet<String> {
    compile_bool(src, phase)
        .unwrap_or_else(|e| panic!("{src}: {e}"))
        .fields()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn kinds(src: &str) -> Option<BTreeSet<String>> {
    compile_bool(src, Phase::ToolCall)
        .unwrap_or_else(|e| panic!("{src}: {e}"))
        .tool_kinds()
}

#[test]
fn pol_004_ac3_fields_name_the_cost_variables_a_rule_uses() {
    let f = fields(
        "session.cost_usd > 20.0 && (user.daily_cost_usd > 5.0 || !(request.model == \"x\"))",
        Phase::ModelRequest,
    );
    for want in [
        "session",
        "session.cost_usd",
        "user",
        "user.daily_cost_usd",
        "request.model",
    ] {
        assert!(f.contains(want), "{want} fehlt in {f:?}");
    }
    // Eine Regel ohne Kostenvariablen braucht keine Abfrage beim Server.
    let f = fields("tool.kind == \"shell\"", Phase::ToolCall);
    assert_eq!(f, set(&["tool", "tool.kind"]));
}

#[test]
fn pol_004_ac3_fields_walk_conditionals_lists_maps_and_macros() {
    let f = fields(
        "result.ok ? tool.kind in [\"shell\", session.harness] : tool.name == \"Bash\"",
        Phase::ToolResult,
    );
    for want in ["result.ok", "tool.kind", "session.harness", "tool.name"] {
        assert!(f.contains(want), "{want} fehlt in {f:?}");
    }
    let f = fields(
        "tool.paths.exists(p, p.startsWith(session.worktree))",
        Phase::ToolCall,
    );
    assert!(f.contains("tool.paths"), "{f:?}");
    assert!(f.contains("session.worktree"), "{f:?}");
    let f = fields("{\"k\": user.id}[\"k\"] == \"usr_local\"", Phase::ToolCall);
    assert!(f.contains("user.id"), "{f:?}");
}

#[test]
fn pol_024_tool_kinds_edge_cases() {
    let one = |k: &str| Some(set(&[k]));
    // `false` trifft keine Klasse.
    assert_eq!(kinds("false"), Some(BTreeSet::new()));
    // Unbestimmte Seite einer Konjunktion schränkt nicht ein.
    assert_eq!(kinds("true && tool.kind == \"shell\""), one("shell"));
    assert_eq!(
        kinds("tool.kind == \"shell\" && tool.kind == \"mcp\""),
        Some(BTreeSet::new())
    );
    // Unbekannter Tool-Name: könnte jedes (MCP-)Tool sein.
    assert_eq!(kinds("tool.name == \"gibt_es_nicht\""), None);
    // Liste mit Nicht-Literal, anderes Feld, Vergleich zweier Felder: nicht bestimmbar.
    assert_eq!(kinds("tool.kind in [\"shell\", tool.name]"), None);
    assert_eq!(kinds("tool.server == \"builtin\""), None);
    assert_eq!(kinds("tool.kind == tool.name"), None);
    assert_eq!(kinds("tool.kind in tool.paths"), None);
    assert_eq!(
        kinds("tool.name in [\"Bash\", \"Read\"]"),
        Some(set(&["file_read", "shell"]))
    );
}
