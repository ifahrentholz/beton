//! POL-004: Kontext-Variablen als Rust-Typen. Aus ihnen entstehen die CEL-Deklaration und die
//! Referenztabelle der Doku (`docs/generated/policy-context.md`).

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_policy::Phase;
use beton_policy::cel::{Meter, compile_bool};
use beton_policy::context::Activation;
use beton_policy::types::{DECLS, Ty};
use beton_policy::vars::{
    CelTimestamp, RequestVars, SessionVars, StartSandboxVars, StartVars, ToolVars, UserVars, Vars,
    reference_markdown,
};

fn row<'a>(
    rows: &'a [beton_policy::types::DeclRow],
    path: &str,
) -> &'a beton_policy::types::DeclRow {
    rows.iter()
        .find(|r| r.path == path)
        .unwrap_or_else(|| panic!("{path} fehlt in der Deklaration"))
}

#[test]
fn pol_004_ac1_declaration_is_derived_from_the_rust_types() {
    let rows = DECLS.rows();
    // Gemeinsame Variablen gelten in allen Phasen.
    let cost = row(&rows, "session.cost_usd");
    assert_eq!(cost.ty, "double");
    assert!(cost.phases.is_empty(), "gemeinsam: alle Phasen");
    assert!(!cost.doc.is_empty(), "Doku-Kommentar wird übernommen");
    assert_eq!(row(&rows, "now").ty, "timestamp");
    assert_eq!(row(&rows, "session.started_at").ty, "timestamp");
    assert_eq!(row(&rows, "session.labels").ty, "list(string)");
    assert_eq!(row(&rows, "state.session").ty, "map(string, dyn)");
    assert_eq!(row(&rows, "user.daily_cost_usd").ty, "double");
    // Phasenspezifisch.
    let argv = row(&rows, "tool.argv");
    assert_eq!(argv.ty, "list(list(string))");
    assert_eq!(argv.phases, vec![Phase::ToolCall, Phase::ToolResult]);
    assert_eq!(row(&rows, "tool.args").ty, "map(string, dyn)");
    assert_eq!(row(&rows, "result.text").phases, vec![Phase::ToolResult]);
    assert_eq!(row(&rows, "start.sandbox.enabled").ty, "bool");
    assert_eq!(row(&rows, "request.messages").ty, "list(dyn)");
    // `browser` gibt es in zwei Phasen mit verschiedenen Feldern.
    assert_eq!(
        row(&rows, "browser.initiator").phases,
        vec![Phase::BrowserNavigate]
    );
    assert_eq!(
        row(&rows, "browser.is_form_submit").phases,
        vec![Phase::BrowserAction]
    );
    // Die Typprüfung nutzt dieselbe Deklaration.
    assert_eq!(DECLS.field("tool", "parse_uncertain"), Some(&Ty::Bool));
    assert_eq!(
        DECLS.var("result", Phase::ToolCall).unwrap_err(),
        Some(&[Phase::ToolResult][..])
    );
}

#[test]
fn pol_004_ac1_reference_table_is_generated_and_checked_in() {
    let table = reference_markdown();
    assert!(table.contains("| `session.cost_usd` | `double` | alle |"));
    assert!(table.contains("| `tool.argv` | `list(list(string))` | `tool_call`, `tool_result` |"));
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/generated/policy-context.md");
    let checked_in = std::fs::read_to_string(&file).unwrap_or_default();
    assert_eq!(
        checked_in, table,
        "docs/generated/policy-context.md ist veraltet: `cargo xtask codegen` ausführen"
    );
}

#[test]
fn pol_004_ac2_phase_variable_outside_its_phase_is_a_load_error_with_hint() {
    let err = compile_bool("result.text == \"x\"", Phase::ToolCall).unwrap_err();
    assert!(err.message.contains("tool_result"), "{err}");
    assert!(compile_bool("result.text == \"x\"", Phase::ToolResult).is_ok());
}

#[test]
fn pol_004_typed_context_feeds_the_activation() {
    let vars = Vars {
        now: Some(CelTimestamp::from_unix(1_767_268_800)),
        session: Some(SessionVars {
            id: Some("ses_1".into()),
            harness: Some("claude".into()),
            cost_usd: Some(1.25),
            turns: Some(3),
            labels: Some(vec!["a".into()]),
            ..SessionVars::default()
        }),
        user: Some(UserVars {
            daily_cost_usd: Some(12.0),
            timezone: Some("Europe/Berlin".into()),
            ..UserVars::default()
        }),
        tool: Some(ToolVars {
            name: Some("Bash".into()),
            kind: Some("shell".into()),
            argv: Some(vec![vec!["git".into(), "push".into()]]),
            ..ToolVars::default()
        }),
        ..Vars::default()
    };
    let act = Activation::new(Phase::ToolCall, vars.to_json()).unwrap();
    let when = compile_bool(
        "session.cost_usd > 1.0 && user.daily_cost_usd == 12.0 && shell.has_command(tool.argv, \"git\", \"push\") && now > timestamp(\"2025-01-01T00:00:00Z\")",
        Phase::ToolCall,
    )
    .unwrap();
    assert!(when.eval_bool(&act, &mut Meter::unlimited()).unwrap());

    let start = Vars {
        start: Some(StartVars {
            model: Some("opus".into()),
            sandbox: Some(StartSandboxVars {
                enabled: Some(false),
                ..StartSandboxVars::default()
            }),
            ..StartVars::default()
        }),
        ..Vars::default()
    };
    let act = Activation::new(Phase::SessionStart, start.to_json()).unwrap();
    let when = compile_bool(
        "!start.sandbox.enabled && start.model == \"opus\"",
        Phase::SessionStart,
    )
    .unwrap();
    assert!(when.eval_bool(&act, &mut Meter::unlimited()).unwrap());

    let request = Vars {
        request: Some(RequestVars {
            model: Some("sonnet".into()),
            granularity: Some("turn".into()),
            ..RequestVars::default()
        }),
        ..Vars::default()
    };
    assert!(Activation::new(Phase::ModelRequest, request.to_json()).is_ok());
    // Eine Phasenvariable in der falschen Phase ist ein Fehler (fail closed).
    assert!(Activation::new(Phase::ToolCall, request.to_json()).is_err());
}
