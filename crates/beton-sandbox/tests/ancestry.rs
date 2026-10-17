//! Prozess-Abstammung für die Peer-Prüfung am Hook-Socket (#247, HAR-005).

#![allow(clippy::unwrap_used)]
#![allow(
    clippy::disallowed_methods,
    reason = "Kindprozesse ohne Sandbox, nur für die Abstammung"
)]
#![cfg(any(target_os = "linux", target_os = "macos"))]

use beton_sandbox::ancestry::{is_descendant, parent_of};

#[test]
fn har_005_ancestry_follows_parents_and_fails_closed() {
    let me = std::process::id();
    // Ein Enkel: sh startet sleep als Kind.
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .unwrap();
    assert_eq!(parent_of(child.id()), Some(me));
    assert!(is_descendant(child.id(), me));
    assert!(is_descendant(me, me), "der Prozess selbst zählt");
    assert!(
        !is_descendant(me, child.id()),
        "Eltern sind keine Nachfahren"
    );
    // Unbekannte PID: keine Auskunft, kein Nachfahre (fail closed).
    assert_eq!(parent_of(u32::MAX), None);
    assert!(!is_descendant(u32::MAX, me));
    let _ = child.kill();
    let _ = child.wait();
}
