//! QA-009: Policy-YAML-Loader (POL-001, POL-003, POL-005 …).
//!
//! Gefuzzt: `load::parse_set` (eigener YAML-Parser mit Positionen über `yaml-rust2`,
//! Schema-Prüfung, Regeltypen, CEL-Bedingungen) und die Auswertung eines geladenen Sets
//! (`engine::EffectiveSet::evaluate`) in jeder Phase mit leerem bzw. festem Kontext.
//!
//! Invarianten: kein Panic; Laden ist deterministisch (gleicher Fehlertext); die
//! Auswertung eines geladenen Sets ist deterministisch (Uhr eingefroren).

#![no_main]

use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::cel::Clock;
use beton_policy::context::Activation;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::load::{Scope, parse_set};
use libfuzzer_sys::fuzz_target;
use serde_json::json;

struct Frozen;

impl Clock for Frozen {
    fn now_us(&self) -> u64 {
        0
    }
}

const SCOPES: [Scope; 5] = [
    Scope::Org,
    Scope::Team,
    Scope::User,
    Scope::Project,
    Scope::Agent,
];

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let scope = SCOPES[text.len() % SCOPES.len()];
    let first = parse_set(scope, "fuzz.yaml", text);
    let second = parse_set(scope, "fuzz.yaml", text);
    assert_eq!(
        first.as_ref().err().map(ToString::to_string),
        second.as_ref().err().map(ToString::to_string),
        "Laden nicht deterministisch"
    );
    let Ok(set) = first else {
        return;
    };
    let effective = EffectiveSet::new(vec![Arc::new(set)]);
    let clock = Frozen;
    let opts = EvalOptions {
        clock: Some(&clock),
        ..EvalOptions::default()
    };
    for phase in Phase::ALL {
        let contexts = [
            json!({}),
            json!({
                "tool": { "name": "Bash", "kind": "shell", "command": "git push --force",
                          "argv": [["git", "push", "--force"]], "paths": ["/etc/passwd"],
                          "args": { "command": "git push --force" } },
                "session": { "cost_usd": 3.5, "harness": "claude" },
            }),
        ];
        for ctx in contexts {
            let Ok(act) = Activation::new(phase, ctx) else {
                continue;
            };
            let a = serde_json::to_string(&effective.evaluate(&act, &opts)).ok();
            let b = serde_json::to_string(&effective.evaluate(&act, &opts)).ok();
            assert_eq!(a, b, "Auswertung nicht deterministisch ({phase})");
        }
    }
});
