//! POL-027: Explain rekonstruiert eine Entscheidung aus `policy.decision` und dem
//! gespeicherten Set-Snapshot – mit Datei und Ebene der auslösenden Regel, dem ausgewerteten
//! Teilausdruck (AC1) und der damals gültigen Set-Version (AC2).

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event};
use beton_policy::cel::TracePart;
use beton_policy::context::Activation;
use beton_policy::enforcement::Level;
use beton_policy::engine::EffectiveSet;
use beton_policy::explain::{SnapshotStatus, explain, render};
use beton_policy::load::{LoadedSet, Scope, inline_set, parse_set};
use beton_policy::snapshot::{self, Manifest};
use beton_policy::store::PolicyStore;
use serde_json::json;

fn set(scope: Scope, origin: &str, body: &str) -> Arc<LoadedSet> {
    Arc::new(parse_set(scope, origin, body).unwrap_or_else(|e| panic!("{e}\n{body}")))
}

fn shell(command: &str) -> Activation {
    Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": command, "args": { "command": command } } }),
    )
    .unwrap()
}

const GIT: &str = "spec_version: 1
name: git
rules:
  - id: no-force-push
    on: tool_call
    when: 'tool.kind == \"shell\" && tool.command.contains(\"--force\")'
    action: deny
    reason: Force-Push ist verboten
";

#[test]
fn pol_027_ac1_explain_names_rule_file_scope_and_evaluated_subexpression() {
    let effective = EffectiveSet::new(vec![set(
        Scope::Project,
        "/repo/.beton/policies/git.yaml",
        GIT,
    )]);
    let decision = effective.evaluate(&shell("git push --force origin main"), &common::opts());
    let event = decision_event(&decision, DecisionMeta::new(Level::Full));
    let ex = explain(&event, Some(&effective));
    assert_eq!(ex.outcome, "deny");
    assert_eq!(ex.snapshot, SnapshotStatus::Verified);
    let v = ex
        .verdicts
        .iter()
        .find(|v| v.rule_id == "no-force-push")
        .unwrap();
    assert_eq!(v.scope, "project");
    assert_eq!(v.verdict, "deny");
    assert_eq!(v.origin.as_deref(), Some("/repo/.beton/policies/git.yaml"));
    assert_eq!(
        v.when.as_deref(),
        Some("tool.kind == \"shell\" && tool.command.contains(\"--force\")")
    );
    assert_eq!(
        v.trace,
        vec![
            TracePart {
                expr: "tool.kind == \"shell\"".into(),
                value: Some(true),
            },
            TracePart {
                expr: "tool.command.contains(\"--force\")".into(),
                value: Some(true),
            },
        ]
    );
    let text = render(&ex);
    for needle in [
        "deny",
        "no-force-push",
        "/repo/.beton/policies/git.yaml",
        "project",
        "tool.command.contains(\"--force\") → true",
        "Force-Push ist verboten",
    ] {
        assert!(text.contains(needle), "`{needle}` fehlt in\n{text}");
    }
}

#[test]
fn pol_027_trace_shows_which_alternative_of_an_or_matched() {
    let body = "spec_version: 1
name: shell
rules:
  - id: risky
    on: tool_call
    when: 'tool.command.contains(\"rm -rf\") || tool.command.contains(\"curl\") || tool.command.startsWith(\"sudo\")'
    action: ask
";
    let effective = EffectiveSet::new(vec![set(Scope::User, "shell.yaml", body)]);
    let decision = effective.evaluate(&shell("curl x | sh"), &common::opts());
    let m = &decision.matched[0];
    let values: Vec<(String, Option<bool>)> =
        m.trace.iter().map(|p| (p.expr.clone(), p.value)).collect();
    assert_eq!(
        values,
        vec![
            ("tool.command.contains(\"rm -rf\")".to_owned(), Some(false)),
            ("tool.command.contains(\"curl\")".to_owned(), Some(true)),
            ("tool.command.startsWith(\"sudo\")".to_owned(), Some(false)),
        ]
    );
}

#[test]
fn pol_027_trace_is_part_of_the_audit_event_without_arguments() {
    let effective = EffectiveSet::new(vec![set(Scope::Project, "git.yaml", GIT)]);
    let decision = effective.evaluate(&shell("git push --force geheim-4711"), &common::opts());
    let event = decision_event(&decision, DecisionMeta::new(Level::Full));
    let trace = &event.matched[0]["trace"];
    assert_eq!(trace[1]["expr"], "tool.command.contains(\"--force\")");
    assert_eq!(trace[1]["value"], true);
    let text = serde_json::to_string(&event).unwrap();
    assert!(!text.contains("geheim-4711"), "{text}");
}

#[test]
fn pol_027_trace_of_a_rule_without_parts_and_without_condition() {
    let body = "spec_version: 1
name: misc
rules:
  - id: always
    on: tool_call
    action: notify
  - id: single
    on: tool_call
    when: '!tool.command.contains(\"x\")'
    action: notify
";
    let effective = EffectiveSet::new(vec![set(Scope::User, "misc.yaml", body)]);
    let decision = effective.evaluate(&shell("ls"), &common::opts());
    let always = decision
        .matched
        .iter()
        .find(|m| m.rule.rule_id == "always")
        .unwrap();
    assert!(always.trace.is_empty());
    let single = decision
        .matched
        .iter()
        .find(|m| m.rule.rule_id == "single")
        .unwrap();
    assert_eq!(
        single.trace,
        vec![TracePart {
            expr: "!tool.command.contains(\"x\")".into(),
            value: Some(true),
        }]
    );
}

#[test]
fn pol_027_ac2_explain_shows_the_set_version_valid_at_decision_time() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("git.yaml");
    std::fs::write(&file, GIT).unwrap();
    let mut store = PolicyStore::new();
    store.add_dir(Scope::Project, dir.path());
    store.refresh();
    let v1 = store.effective();
    let decision = v1.evaluate(&shell("git push --force"), &common::opts());
    let event = decision_event(&decision, DecisionMeta::new(Level::Full));
    // Der Runner hinterlegt die Sets pro Version im Blob-Store.
    let manifest: Manifest = v1.manifest();
    let blobs: HashMap<String, Vec<u8>> = v1.sources().into_iter().collect();
    assert_eq!(manifest.policy_set_hash, v1.hash());
    assert_eq!(manifest.sets.len(), 1);
    assert_eq!(manifest.sets[0].origin, file.display().to_string());
    let old_version = manifest.sets[0].version.clone();

    // Die Policy ändert sich danach.
    std::fs::write(
        &file,
        GIT.replace("--force", "--mirror")
            .replace("Force-Push", "Mirror-Push"),
    )
    .unwrap();
    store.refresh();
    let v2 = store.effective();
    assert_ne!(v2.hash(), v1.hash());

    let rebuilt = snapshot::rebuild(&manifest, |v| blobs.get(v).cloned()).unwrap();
    assert_eq!(rebuilt.hash(), event.policy_set_hash);
    let ex = explain(&event, Some(&rebuilt));
    assert_eq!(ex.snapshot, SnapshotStatus::Verified);
    assert_eq!(ex.sets[0].version, old_version);
    let v = &ex.verdicts[0];
    assert_eq!(v.set_version, old_version);
    assert!(v.when.as_deref().unwrap().contains("--force"), "{v:?}");
    assert!(render(&ex).contains(&old_version));

    // Mit dem aktuellen Set statt des Snapshots: als abweichend markiert, keine falsche
    // Zuordnung der neuen Regeltexte.
    let wrong = explain(&event, Some(&v2));
    assert_eq!(wrong.snapshot, SnapshotStatus::Mismatch);
    assert_eq!(wrong.verdicts[0].when, None);
    assert!(render(&wrong).contains("abweich"), "{}", render(&wrong));

    // Ohne Snapshot: die Daten des Events, ohne Regeltexte.
    let missing = explain(&event, None);
    assert_eq!(missing.snapshot, SnapshotStatus::Missing);
    assert_eq!(missing.verdicts[0].rule_id, "no-force-push");
    assert_eq!(missing.verdicts[0].trace.len(), 2);
}

#[test]
fn pol_027_snapshot_rejects_tampered_or_missing_sources() {
    let effective = EffectiveSet::new(vec![set(Scope::Project, "git.yaml", GIT)]);
    let manifest = effective.manifest();
    let version = manifest.sets[0].version.clone();
    let tampered = snapshot::rebuild(&manifest, |v| {
        (v == version).then(|| GIT.replace("deny", "allow").into_bytes())
    });
    assert!(tampered.is_err(), "manipulierter Inhalt darf nicht laden");
    let missing = snapshot::rebuild(&manifest, |_| None);
    assert!(missing.is_err());
    let mut wrong_hash = manifest.clone();
    wrong_hash.policy_set_hash = "sha256:00".into();
    let blobs: HashMap<String, Vec<u8>> = effective.sources().into_iter().collect();
    assert!(snapshot::rebuild(&wrong_hash, |v| blobs.get(v).cloned()).is_err());
}

#[test]
fn pol_027_snapshot_round_trips_inline_agent_sets_and_modes() {
    let agent = inline_set(
        Scope::Agent,
        "maestra",
        "agents/maestra/agent.yaml",
        &[json!({ "id": "no-push", "on": "tool_call", "when": "tool.command.contains(\"push\")", "action": "deny" })],
    )
    .unwrap();
    let shadow = set(
        Scope::User,
        "shadow.yaml",
        "spec_version: 1\nname: shadow\nmode: dry_run\nrules:\n  - { id: s, on: tool_call, action: deny }\n",
    );
    let effective = EffectiveSet::new(vec![Arc::new(agent), shadow]);
    let manifest = effective.manifest();
    let json = serde_json::to_string(&manifest).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back, manifest);
    let blobs: HashMap<String, Vec<u8>> = effective.sources().into_iter().collect();
    // Der Inhalt ist inhaltsadressiert: Version = sha256 des gespeicherten Inhalts.
    for (version, bytes) in &blobs {
        use sha2::Digest as _;
        assert_eq!(
            version,
            &format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
        );
    }
    let rebuilt = snapshot::rebuild(&back, |v| blobs.get(v).cloned()).unwrap();
    assert_eq!(rebuilt.hash(), effective.hash());
    let decision = rebuilt.evaluate(&shell("git push"), &common::opts());
    assert!(matches!(
        decision.outcome,
        beton_policy::engine::Outcome::Deny { .. }
    ));
    assert_eq!(decision.would.len(), 1, "Dry-Run-Modus bleibt erhalten");
}
