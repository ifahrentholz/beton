//! Vertragstests der mitgelieferten Agents `maestra` (AGT-011) und `duetto` (AGT-012): Sie
//! sind gültig, haben die Struktur aus der Spec und ihre Anweisungen enthalten die Regeln, die
//! die Akzeptanzkriterien verlangen. Den Ablauf mit Fake-Harnesses prüfen die
//! Integrationstests in `crates/beton-runner/tests/subagents.rs`.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use beton_agents::spec::{SkillSelection, WorktreeMode};
use beton_agents::{
    AgentRef, AgentSnapshot, AgentSpec, Builtins, HarnessCatalog, SearchPath, Validator,
};
use beton_harness::{HarnessId, PermissionMode};

fn catalog() -> HarnessCatalog {
    let mut c = HarnessCatalog::new();
    for h in ["claude", "codex"] {
        c.insert(&h.parse::<HarnessId>().unwrap(), Vec::new());
    }
    c
}

fn search() -> SearchPath {
    SearchPath {
        builtins: Builtins::embedded(),
        ..SearchPath::default()
    }
}

/// Lädt ein Built-in samt Snapshot und prüft es wie `beton agent validate`.
fn builtin(name: &str) -> (AgentSpec, AgentSnapshot) {
    let search = search();
    let located = search
        .resolve(
            &AgentRef::parse(&format!("builtin:{name}")).unwrap(),
            Path::new("."),
        )
        .unwrap();
    let harnesses = catalog();
    let report = Validator {
        search: &search,
        harnesses: &harnesses,
        skill_roots: Vec::new(),
    }
    .validate(&located);
    assert!(report.is_valid(), "{name}: {:#?}", report.diagnostics);
    assert!(
        report.diagnostics.is_empty(),
        "{name} ohne Warnungen: {:#?}",
        report.diagnostics
    );
    let snapshot =
        AgentSnapshot::capture(&located, &format!("builtin:{name}"), &Builtins::embedded())
            .unwrap();
    (report.spec.unwrap(), snapshot)
}

fn text(name: &str, rel: &str) -> String {
    let bytes = Builtins::embedded()
        .dir(name)
        .read(rel)
        .unwrap_or_else(|| panic!("{name}/{rel} fehlt"));
    String::from_utf8(bytes).unwrap()
}

fn sub(snapshot: &AgentSnapshot, name: &str) -> AgentSpec {
    snapshot.subagent(name).unwrap().agent().unwrap().0
}

// ------------------------------------------------------------------------------- AGT-011

#[test]
fn agt_011_maestra_is_valid_and_matches_the_spec() {
    let (spec, snapshot) = builtin("maestra");
    assert_eq!(spec.executor.harness.as_str(), "claude");
    assert_eq!(spec.executor.permission_mode, Some(PermissionMode::Plan));
    let spawn = spec.spawn.as_ref().unwrap();
    assert_eq!(
        spawn.agents,
        ["impl-claude", "impl-codex", "review-claude", "review-codex"]
    );
    assert_eq!(spawn.max_depth, Some(1));
    assert_eq!(spawn.max_concurrent, Some(4));
    assert_eq!(spawn.worktree, Some(WorktreeMode::New));
    assert_eq!(
        spec.skills,
        Some(SkillSelection::List(vec![
            "plan".into(),
            "fanout".into(),
            "cross-review".into(),
            "investigate".into()
        ]))
    );
    let ids: Vec<_> = spec.policies.iter().filter_map(|p| p.id.clone()).collect();
    assert_eq!(ids, ["maestra-no-code", "maestra-no-merge"]);

    // Implementer schreiben, Reviewer nur lesend; je ein Paar pro Vendor.
    for (name, harness, mode) in [
        ("impl-claude", "claude", PermissionMode::AcceptEdits),
        ("impl-codex", "codex", PermissionMode::AcceptEdits),
        ("review-claude", "claude", PermissionMode::Plan),
        ("review-codex", "codex", PermissionMode::Plan),
    ] {
        let s = sub(&snapshot, name);
        assert_eq!(s.executor.harness.as_str(), harness, "{name}");
        assert_eq!(s.executor.permission_mode, Some(mode), "{name}");
    }
    // Gleiche Rolle, gleiche Anweisungen (kein Auseinanderlaufen der Kopien).
    assert_eq!(
        text("maestra", "agents/impl-claude/prompts/system.md"),
        text("maestra", "agents/impl-codex/prompts/system.md")
    );
    assert_eq!(
        text("maestra", "agents/review-claude/prompts/system.md"),
        text("maestra", "agents/review-codex/prompts/system.md")
    );
}

#[test]
fn agt_011_ac1_maestra_reviews_across_vendors_and_never_merges() {
    let system = text("maestra", "prompts/system.md");
    assert!(system.contains("`impl-claude` → `review-codex`, `impl-codex` → `review-claude`"));
    assert!(system.contains("Du merged nie"));
    assert!(system.contains("schreibst keinen Code"));
    let fanout = text("maestra", "skills/fanout/SKILL.md");
    assert!(fanout.contains("worktree: \"new\""), "{fanout}");
    assert!(fanout.contains("async: true"));
    let implementer = text("maestra", "agents/impl-claude/prompts/system.md");
    assert!(implementer.contains("**Committe**"));
    assert!(implementer.contains("kein `git merge`"));
    let reviewer = text("maestra", "agents/review-claude/prompts/system.md");
    assert!(
        reviewer.contains("VERDICT: approve") && reviewer.contains("VERDICT: changes_requested")
    );
    assert!(reviewer.contains("**änderst nichts**"));
}

#[test]
fn agt_011_ac3_cross_review_retasks_the_implementer_and_stops_after_three_rounds() {
    let skill = text("maestra", "skills/cross-review/SKILL.md");
    assert!(skill.contains("**denselben** Implementer"), "{skill}");
    assert!(skill.contains("session_send"));
    assert!(skill.contains("Nach **3 Runden**"));
    assert!(skill.contains("`needs_human`"));
    let system = text("maestra", "prompts/system.md");
    assert!(system.contains("`needs_human`: Nach 3 Review-Runden"));
}

#[test]
fn agt_011_ac4_maestra_falls_back_to_one_vendor_and_says_so() {
    let system = text("maestra", "prompts/system.md");
    assert!(system.contains("Rufe `session_list` auf"));
    assert!(system.contains("„same-vendor review“"));
    let skill = text("maestra", "skills/cross-review/SKILL.md");
    assert!(skill.contains("„same-vendor review“"));
}

/// AGT-011 AC5, AGT-012 AC4: Kein Built-in setzt einen API-Key, `providers` oder den
/// Direkt-API-Harness voraus (ADR-0034).
#[test]
fn agt_011_ac5_agt_012_ac4_builtins_need_no_api_key() {
    let all = Builtins::embedded();
    assert_eq!(all.names(), ["duetto", "maestra"]);
    for (path, bytes) in all.files() {
        let content = String::from_utf8_lossy(bytes);
        for bad in ["_API_KEY", "providers", "direct:", "${secret:"] {
            assert!(!content.contains(bad), "{path} enthält {bad}");
        }
    }
    for name in ["maestra", "duetto"] {
        let (spec, snapshot) = builtin(name);
        let mut harnesses = vec![spec.executor.harness.to_string()];
        // Nie `yolo` (HAR-027: braucht Sandbox und Egress-Proxy).
        assert_ne!(spec.executor.permission_mode, Some(PermissionMode::Yolo));
        for a in spec.spawn.as_ref().unwrap().agents.clone() {
            let s = sub(&snapshot, &a);
            assert_ne!(
                s.executor.permission_mode,
                Some(PermissionMode::Yolo),
                "{a}"
            );
            harnesses.push(s.executor.harness.to_string());
        }
        assert!(
            harnesses.iter().all(|h| h == "claude" || h == "codex"),
            "{name}: {harnesses:?}"
        );
    }
}

// ------------------------------------------------------------------------------- AGT-012

#[test]
fn agt_012_duetto_is_valid_with_two_read_only_voices_on_different_harnesses() {
    let (spec, snapshot) = builtin("duetto");
    let spawn = spec.spawn.as_ref().unwrap();
    assert_eq!(spawn.agents, ["voce-claude", "voce-codex"]);
    assert_eq!(spawn.max_concurrent, Some(2));
    assert_eq!(spawn.worktree, Some(WorktreeMode::None));
    let voices: Vec<String> = spawn
        .agents
        .iter()
        .map(|a| {
            let s = sub(&snapshot, a);
            assert_eq!(
                s.executor.permission_mode,
                Some(PermissionMode::Plan),
                "{a}"
            );
            s.executor.harness.to_string()
        })
        .collect();
    assert_eq!(voices, ["claude", "codex"]);
    let rounds = &spec.params["rounds"];
    assert_eq!(rounds.default, Some(serde_json::json!(2)));
    assert_eq!(rounds.maximum, Some(5.0));
}

#[test]
fn agt_012_ac1_duetto_asks_both_voices_in_parallel() {
    let system = text("duetto", "prompts/system.md");
    assert!(system.contains("async: true"));
    assert!(system.contains("session_wait(ids: [beide], mode: all"));
}

#[test]
fn agt_012_ac2_debate_has_exact_rounds_and_a_synthesis_with_consensus_and_dissent() {
    let skill = text("duetto", "skills/debate/SKILL.md");
    assert!(skill.contains("Genau N Runden"), "{skill}");
    assert!(skill.contains("## Konsens") && skill.contains("## Dissens"));
    assert!(skill.contains("Werte über 5 setzt du auf 5"));
    assert!(text("duetto", "agent.yaml").contains("{{ params.rounds }}"));
}

#[test]
fn agt_012_ac3_duetto_refuses_to_debate_with_one_voice() {
    let system = text("duetto", "prompts/system.md");
    assert!(system.contains("Debattiere nie mit nur einer Stimme"));
    assert!(system.contains("duetto braucht zwei Stimmen auf verschiedenen Harnesses"));
}

#[test]
fn agt_011_plan_mode_agents_never_call_exit_plan_mode_and_delegate_instead() {
    // #148: maestra hat nach dem Planen `ExitPlanMode` aufgerufen statt zu delegieren. Die
    // Anweisungen verbieten das; der Claude-Adapter lehnt es zusätzlich ab (HAR-027).
    let maestra = text("maestra", "prompts/system.md");
    assert!(
        maestra.contains("Rufe **nie** `ExitPlanMode` auf"),
        "{maestra}"
    );
    assert!(
        maestra.contains("sofort die\n  Umsetzung mit `session_spawn`"),
        "{maestra}"
    );
    let plan = text("maestra", "skills/plan/SKILL.md");
    assert!(plan.contains("nie über `ExitPlanMode`"), "{plan}");
    let duetto = text("duetto", "prompts/system.md");
    assert!(duetto.contains("Rufe nie `ExitPlanMode` auf"), "{duetto}");
    assert!(duetto.contains("per `session_spawn`"), "{duetto}");
    for (agent, rel) in [
        ("maestra", "agents/review-claude/prompts/system.md"),
        ("duetto", "agents/voce-claude/prompts/system.md"),
    ] {
        assert!(
            text(agent, rel).contains("nie `ExitPlanMode`"),
            "{agent}/{rel}"
        );
    }
}
