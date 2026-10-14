//! #182 (POL-005 AC3): Der ACP-Adapter leitet `tool.call.requested.kind` aus der
//! Abbildungstabelle von `beton-policy` ab, nicht aus einer eigenen Tabelle.

#![allow(clippy::unwrap_used)]

use beton_core::event::EventPayload;
use beton_harness_acp::mapping::MapState;
use serde_json::json;

fn kind_of(acp_kind: &str) -> Option<String> {
    let mut st = MapState::default();
    st.begin_turn();
    match st
        .requested(&json!({ "toolCallId": "t1", "title": "x", "kind": acp_kind, "rawInput": {} }))
    {
        Some(EventPayload::ToolCallRequested(r)) => r.kind,
        other => panic!("{other:?}"),
    }
}

#[test]
fn pol_005_acp_kinds_come_from_the_policy_tool_map() {
    for (acp, expected) in [
        ("execute", "shell"),
        ("read", "file_read"),
        ("edit", "file_edit"),
        // Spec und Tabelle: `delete`/`move` → `file_edit` (früher abweichend `file_write`).
        ("delete", "file_edit"),
        ("move", "file_edit"),
        ("search", "search"),
        ("fetch", "web_fetch"),
        ("think", "other"),
        ("unbekannt", "other"),
    ] {
        assert_eq!(kind_of(acp).as_deref(), Some(expected), "{acp}");
        let table = beton_policy::normalize::tool_map()
            .unwrap()
            .kind_of("acp", "x", Some(acp), None)
            .unwrap_or("other");
        assert_eq!(table, expected, "{acp}");
    }
}
