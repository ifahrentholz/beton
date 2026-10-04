//! PROTO-002 AC2: Die Golden-Transcripts des Codex-Adapters erzeugen für bekannte
//! Vendor-Events ausschließlich Katalog-Typen; unbekannte erscheinen als `harness.unmapped`.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_core::event::{EventPayload, EventType};
use beton_harness_codex::mapping::{MapState, map_notification};

#[test]
fn proto_002_ac2_golden_events_are_catalog_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut cases = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path().join("expected.events.jsonl");
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        cases += 1;
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let ty = v["type"].as_str().unwrap();
            assert!(
                EventType::parse(ty).is_some(),
                "{}: {ty} ist kein Katalog-Typ",
                path.display()
            );
        }
    }
    assert!(cases >= 5, "Golden-Fälle gefunden: {cases}");
}

#[test]
fn proto_002_ac2_unknown_vendor_message_becomes_unmapped() {
    let events = map_notification(
        "future/notification",
        &serde_json::json!({"x": 1}),
        &mut MapState::default(),
    );
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(
        matches!(events[0], EventPayload::HarnessUnmapped(_)),
        "{events:?}"
    );
}
