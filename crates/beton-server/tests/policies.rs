//! Policies über die API (POL-027, API-002): effektives Set, seiteneffektfreie Auswertung,
//! Explain gespeicherter Entscheidungen mit Set-Snapshot, Shadow-Ansicht. Dazu HAR-021 AC3:
//! `session.cost_usd` folgt jedem `cost.delta` (geprüft über die Erklärung von `eval`).

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{Request, header};
use beton_core::event::{
    Actor, AuthSource, CostDelta, CostSource, Event, EventPayload, SessionKind, SessionTrigger,
    ToolCallRequested, ToolSource,
};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_policy::Phase;
use beton_policy::audit::{DecisionMeta, decision_event};
use beton_policy::context::Activation;
use beton_policy::enforcement::Level;
use beton_policy::engine::{EffectiveSet, EvalOptions};
use beton_policy::load::{Scope, load_file};
use beton_server::app::AppState;
use beton_server::tunnel::SystemCalls as _;
use beton_store::NewSession;
use common::TestApp;
use serde_json::{Value, json};

const POLICY: &str = "spec_version: 1
name: git
rules:
  - id: no-force-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\") && shell.has_flag(tool.argv, \"--force\")'
    action: deny
    reason: Force-Push ist verboten
  - id: confirm-push
    on: tool_call
    when: 'shell.has_command(tool.argv, \"git\", \"push\")'
    action: ask
    reason: Push bestätigen
    effects:
      - { op: increment, scope: user, key: pushes }
  - id: tell
    on: tool_call
    action: notify
    notify: { channels: [inbox] }
  - id: expensive
    on: model_request
    when: 'session.cost_usd > 1.0'
    action: deny
    reason: zu teuer
  - id: shadow-publish
    on: tool_call
    when: 'tool.command.contains(\"publish\")'
    action: deny
    dry_run: true
";

struct Setup {
    t: TestApp,
    project: tempfile::TempDir,
    _home: tempfile::TempDir,
}

async fn setup() -> Setup {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    write(&project.path().join(".beton/policies/git.yaml"), POLICY);
    let beton_home = home.path().to_path_buf();
    // Testroute für System-Calls des Runners (`policy.snapshot` über den Tunnel).
    let t = common::app_full(
        |r| r.route("/test/system/{id}/{tool}", axum::routing::post(system_call)),
        "127.0.0.1:50000",
        move |mut r| {
            r.sessions.beton_home = beton_home;
            r
        },
    )
    .await;
    Setup {
        t,
        project,
        _home: home,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

async fn session(t: &TestApp, cwd: &Path) -> SessionId {
    let local = t.store.ensure_local().await.unwrap();
    t.store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: cwd.display().to_string(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap()
        .id
}

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "fake".into(),
        agent_ref: None,
    }
}

async fn append(t: &TestApp, id: SessionId, payloads: Vec<EventPayload>) -> Vec<u64> {
    let record = t.store.session(OrgId::LOCAL, id).await.unwrap();
    let events = payloads
        .into_iter()
        .map(|p| Event::new(id, 0, agent(), p))
        .collect();
    t.store
        .append(OrgId::LOCAL, id, record.head_seq, record.epoch, events)
        .await
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect()
}

async fn get(t: &TestApp, path: &str) -> common::Res {
    t.send(t.authed("GET", path).body(Body::empty()).unwrap())
        .await
}

async fn post(t: &TestApp, path: &str, body: Value) -> common::Res {
    t.send(
        t.authed("POST", path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn api_002_ac1_contract_for_policy_resources() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    for path in [
        "/v1/policies".to_owned(),
        "/v1/policies/shadow".to_owned(),
        format!("/v1/policies?session_id={id}"),
    ] {
        assert_eq!(get(t, &path).await.status, 200, "{path}");
        let forbidden = t
            .send(
                Request::get(&path)
                    .header(header::HOST, "evil.test")
                    .header(header::AUTHORIZATION, format!("Bearer {}", t.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(forbidden.status, 403, "{path}");
    }
    let missing = get(t, &format!("/v1/policies?session_id={}", SessionId::new())).await;
    assert_eq!(missing.status, 404);
    let missing = get(
        t,
        &format!("/v1/sessions/{}/policy-decisions/1", SessionId::new()),
    )
    .await;
    assert_eq!(missing.status, 404);
    let no_decision = get(t, &format!("/v1/sessions/{id}/policy-decisions/1")).await;
    assert_eq!(no_decision.status, 404);
    let bad = get(t, "/v1/policies/shadow?days=0").await;
    bad.assert_problem("validation_failed");
    let bad = post(
        t,
        "/v1/policies/evaluate",
        json!({ "phase": "bogus", "input": {} }),
    )
    .await;
    bad.assert_problem("validation_failed");
}

#[tokio::test]
async fn pol_027_show_lists_rules_with_scope_and_origin() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let user_only = get(t, "/v1/policies").await.json();
    assert_eq!(user_only["sets"], json!([]));
    let v = get(t, &format!("/v1/policies?session_id={id}"))
        .await
        .json();
    let set = &v["sets"][0];
    assert_eq!(set["scope"], "project");
    assert_eq!(set["name"], "git");
    assert!(
        set["origin"]
            .as_str()
            .unwrap()
            .ends_with(".beton/policies/git.yaml")
    );
    assert_eq!(set["rules"][0]["id"], "no-force-push");
    assert!(
        v["policy_set_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[tokio::test]
async fn pol_027_ac3_evaluate_creates_no_events_approvals_or_state() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let before = t.store.session(OrgId::LOCAL, id).await.unwrap().head_seq;
    let dump = t.store.projection_dump(OrgId::LOCAL).await.unwrap();
    for (command, outcome) in [
        ("git push origin main", "ask"),
        ("git push --force", "deny"),
        ("npm publish", "allow"),
    ] {
        let r = post(
            t,
            "/v1/policies/evaluate",
            json!({ "session_id": id.to_string(), "phase": "tool_call",
                    "input": { "tool": { "name": "Bash", "args": { "command": command } } } }),
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        assert_eq!(v["outcome"], outcome, "{command}: {v}");
        assert_eq!(v["enforcement"], "full");
    }
    // Shadow-Treffer erscheinen in der Auswertung, ohne das Ergebnis zu ändern.
    let r = post(
        t,
        "/v1/policies/evaluate",
        json!({ "session_id": id.to_string(), "phase": "tool_call",
                "input": { "tool": { "name": "Bash", "args": { "command": "npm publish" } } } }),
    )
    .await
    .json();
    assert!(r["explain"].as_str().unwrap().contains("would_deny"), "{r}");
    // Keine Events, keine Approvals, keine Inbox, kein Zustand, keine Projektion.
    assert_eq!(
        t.store.session(OrgId::LOCAL, id).await.unwrap().head_seq,
        before
    );
    assert!(
        t.store
            .open_approvals(OrgId::LOCAL)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(t.store.projection_dump(OrgId::LOCAL).await.unwrap(), dump);
    let state = t
        .store
        .policy_user_state(OrgId::LOCAL, UserId::LOCAL, "")
        .await
        .unwrap();
    assert!(state.1.is_empty() && state.0.is_empty(), "{state:?}");
    let inbox = get(t, "/v1/inbox").await.json();
    assert_eq!(inbox["items"], json!([]));
}

/// API-006 AC4: Das Session-Budget (`max_cost_usd`) ist eine `spend_cap`-Regel der User-Ebene,
/// die auch `show --effective` und `evaluate` mit Session kennen.
#[tokio::test]
async fn api_006_ac4_the_session_budget_is_a_spend_cap_rule() {
    let s = setup().await;
    let t = &s.t;
    let local = t.store.ensure_local().await.unwrap();
    let id = t
        .store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: s.project.path().display().to_string(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
                max_cost_usd: Some(0.5),
            },
        )
        .await
        .unwrap()
        .id;
    let shown = get(t, &format!("/v1/policies?session_id={id}"))
        .await
        .json();
    let budget = shown["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|set| set["name"] == "session-budget")
        .unwrap_or_else(|| panic!("{shown}"));
    assert_eq!(budget["scope"], "user");
    assert_eq!(budget["rules"][0]["id"], "max-cost");
    assert_eq!(budget["rules"][0]["rule_type"], "spend_cap");
    let turn = json!({ "session_id": id.to_string(), "phase": "model_request",
                       "input": { "request": { "model": "x", "granularity": "turn" } } });
    let v = post(t, "/v1/policies/evaluate", turn.clone()).await.json();
    assert_eq!(v["outcome"], "allow", "{v}");
    append(t, id, vec![cost(1_000_000)]).await;
    let v = post(t, "/v1/policies/evaluate", turn).await.json();
    assert_eq!(v["outcome"], "deny", "{v}");
    assert_eq!(v["decision"]["matched"][0]["rule_id"], "max-cost", "{v}");
    assert_eq!(v["budget_exhausted"][0]["rule_id"], "max-cost", "{v}");
    // Ein Budget muss positiv sein.
    let r = post(
        t,
        "/v1/sessions",
        json!({ "target": "fake", "cwd": s.project.path(), "max_cost_usd": 0 }),
    )
    .await;
    assert_eq!(r.status, 422, "{}", String::from_utf8_lossy(&r.body));
}

#[tokio::test]
async fn pol_027_evaluate_with_fixtures_without_session() {
    let s = setup().await;
    let t = &s.t;
    let r = post(
        t,
        "/v1/policies/evaluate",
        json!({ "phase": "model_request", "fixtures": { "session": { "cost_usd": 3.0 } },
                "input": { "request": { "model": "x", "granularity": "turn" } } }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        r.json()["outcome"],
        "allow",
        "ohne Session nur die User-Ebene"
    );
    let wrong = post(
        t,
        "/v1/policies/evaluate",
        json!({ "phase": "model_request", "input": { "tool": { "name": "Bash" } } }),
    )
    .await;
    wrong.assert_problem("validation_failed");
}

fn cost(micro: i64) -> EventPayload {
    EventPayload::CostDelta(CostDelta {
        harness: "fake".into(),
        model: "m".into(),
        currency: "USD".into(),
        source: CostSource::Reported,
        auth_source: AuthSource::ApiKey,
        cost_micro: Some(micro),
        ..CostDelta::default()
    })
}

#[tokio::test]
async fn har_021_ac3_session_cost_usd_follows_every_cost_delta() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let eval = || async {
        post(
            t,
            "/v1/policies/evaluate",
            json!({ "session_id": id.to_string(), "phase": "model_request",
                    "input": { "request": { "model": "m", "granularity": "turn" } } }),
        )
        .await
        .json()
    };
    let v = eval().await;
    assert_eq!(v["explanation"]["context"]["session.cost_usd"], json!(0.0));
    assert_eq!(v["outcome"], "allow");
    append(t, id, vec![cost(600_000)]).await;
    let v = eval().await;
    assert_eq!(v["explanation"]["context"]["session.cost_usd"], json!(0.6));
    assert_eq!(v["outcome"], "allow");
    append(t, id, vec![cost(500_000)]).await;
    let v = eval().await;
    assert_eq!(v["explanation"]["context"]["session.cost_usd"], json!(1.1));
    assert_eq!(v["outcome"], "deny", "{v}");
    assert!(
        v["explain"]
            .as_str()
            .unwrap()
            .contains("session.cost_usd = 1.1")
    );
}

/// Entscheidung wie im Runner: auswerten, Snapshot ablegen, `policy.decision` schreiben.
async fn decide(t: &TestApp, id: SessionId, set: &EffectiveSet, command: &str) -> (u64, u64) {
    let manifest = set.manifest();
    let sources: Vec<Value> = set
        .sources()
        .into_iter()
        .map(|(v, b)| json!({ "version": v, "text": String::from_utf8(b).unwrap() }))
        .collect();
    let r = post(
        t,
        &format!("/test/system/{id}/policy.snapshot"),
        json!({ "manifest": manifest, "sources": sources }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(r.json()["stored"].is_boolean());
    let act = Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": command,
                          "argv": [command.split(' ').collect::<Vec<_>>()],
                          "args": { "command": command } } }),
    )
    .unwrap();
    let decision = set.evaluate(&act, &EvalOptions::default());
    let call_id = format!("call-{}", command.len());
    let mut event = decision_event(&decision, DecisionMeta::new(Level::Full));
    event.subject_ref = json!({ "call_id": call_id });
    let seqs = append(
        t,
        id,
        vec![
            EventPayload::ToolCallRequested(ToolCallRequested {
                call_id: call_id.clone(),
                tool: "Bash".into(),
                kind: Some("shell".into()),
                mcp_server: None,
                args: json!({ "command": command }),
                source: ToolSource::Harness,
                parent_call_id: None,
            }),
            EventPayload::PolicyDecision(event),
        ],
    )
    .await;
    (seqs[0], seqs[1])
}

async fn system_call(
    State(state): State<AppState>,
    UrlPath((id, tool)): UrlPath<(String, String)>,
    axum::Json(args): axum::Json<Value>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    let calls = beton_server::sessions::ServerSystemCalls(state);
    match calls.call(id.parse().unwrap(), &tool, args).await {
        Ok(v) => (axum::http::StatusCode::OK, axum::Json(v)),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, axum::Json(e)),
    }
}

#[tokio::test]
async fn pol_027_ac2_snapshot_call_rejects_content_that_does_not_match_its_version() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let file = s.project.path().join(".beton/policies/git.yaml");
    let set = EffectiveSet::new(vec![Arc::new(load_file(Scope::Project, &file).unwrap())]);
    let manifest = set.manifest();
    let version = manifest.sets[0].version.clone();
    let r = post(
        t,
        &format!("/test/system/{id}/policy.snapshot"),
        json!({ "manifest": manifest, "sources": [{ "version": version, "text": "manipuliert" }] }),
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "validation_failed");
    let r = post(
        t,
        &format!("/test/system/{id}/policy.snapshot"),
        json!({ "manifest": { "kaputt": true }, "sources": [] }),
    )
    .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn pol_027_ac1_ac2_explain_shows_rule_file_and_version_of_the_decision() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let file = s.project.path().join(".beton/policies/git.yaml");
    let v1 = EffectiveSet::new(vec![Arc::new(load_file(Scope::Project, &file).unwrap())]);
    let (call_seq, decision_seq) = decide(t, id, &v1, "git push --force").await;
    // Danach ändert sich die Policy.
    write(
        &file,
        &POLICY.replace("Force-Push ist verboten", "Niemals Force-Push"),
    );
    let v2 = EffectiveSet::new(vec![Arc::new(load_file(Scope::Project, &file).unwrap())]);
    let _ = decide(t, id, &v2, "git push --force origin").await;

    for seq in [decision_seq, call_seq] {
        let r = get(t, &format!("/v1/sessions/{id}/policy-decisions/{seq}")).await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        assert_eq!(v["seq"], decision_seq);
        assert_eq!(v["snapshot_error"], Value::Null);
        let ex = &v["explanation"];
        assert_eq!(ex["snapshot"], "verified");
        assert_eq!(ex["outcome"], "deny");
        let verdict = ex["verdicts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["rule_id"] == "no-force-push")
            .unwrap();
        assert_eq!(verdict["scope"], "project");
        assert_eq!(verdict["origin"], file.display().to_string());
        assert_eq!(verdict["set_version"], v1.manifest().sets[0].version);
        assert!(
            verdict["trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |p| p["expr"] == "shell.has_flag(tool.argv, \"--force\")" && p["value"] == true
                ),
            "{verdict}"
        );
        let text = v["explain"].as_str().unwrap();
        assert!(text.contains("Force-Push ist verboten"), "{text}");
        assert!(
            !text.contains("Niemals"),
            "alte Version, nicht die aktuelle: {text}"
        );
    }
}

#[tokio::test]
async fn pol_027_ac2_explain_without_snapshot_names_the_gap() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let file = s.project.path().join(".beton/policies/git.yaml");
    let set = EffectiveSet::new(vec![Arc::new(load_file(Scope::Project, &file).unwrap())]);
    let act = Activation::new(
        Phase::ToolCall,
        json!({ "tool": { "name": "Bash", "kind": "shell", "command": "ls", "args": { "command": "ls" } } }),
    )
    .unwrap();
    let decision = set.evaluate(&act, &EvalOptions::default());
    let seq = append(
        t,
        id,
        vec![EventPayload::PolicyDecision(decision_event(
            &decision,
            DecisionMeta::new(Level::Full),
        ))],
    )
    .await[0];
    let v = get(t, &format!("/v1/sessions/{id}/policy-decisions/{seq}"))
        .await
        .json();
    assert_eq!(v["explanation"]["snapshot"], "missing");
    assert!(v["snapshot_error"].as_str().unwrap().contains("Snapshot"));
}

#[tokio::test]
async fn pol_027_ac4_shadow_view_counts_would_deny_per_rule_and_day() {
    let s = setup().await;
    let t = &s.t;
    let id = session(t, s.project.path()).await;
    let file = s.project.path().join(".beton/policies/git.yaml");
    let set = EffectiveSet::new(vec![Arc::new(load_file(Scope::Project, &file).unwrap())]);
    decide(t, id, &set, "npm publish").await;
    decide(t, id, &set, "pnpm publish --dry-run").await;
    decide(t, id, &set, "ls").await;
    let v = get(t, "/v1/policies/shadow?days=7").await.json();
    assert_eq!(v["days"].as_array().unwrap().len(), 7);
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1, "{v}");
    let r = &rules[0];
    assert_eq!(r["rule_id"], "shadow-publish");
    assert_eq!(r["verdict"], "deny");
    assert_eq!(r["scope"], "project");
    assert_eq!(r["set"], "git");
    assert_eq!(r["total"], 2);
    assert_eq!(r["counts"][6], 2, "heute");
    assert_eq!(r["last"]["session_id"], id.to_string());
    assert_eq!(r["last"]["summary"], "Bash: pnpm publish --dry-run");
}
