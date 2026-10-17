//! Policy-Daten im Store: Set-Snapshots (POL-027 AC2) und Shadow-Treffer (POL-027 AC4).

use beton_core::event::{
    Actor, Event, EventPayload, PolicyDecision, PolicyOutcome, SessionKind, SessionTrigger,
};
use beton_core::id::{PrincipalId, SessionId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::testutil::{TestStore, org, store};
use crate::{DeleteAuthority, Error, NewSession, SessionRecord};

async fn session(t: &TestStore) -> SessionRecord {
    t.store
        .create_session(
            org(t),
            NewSession {
                id: SessionId::new(),
                owner: t.local.user,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/tmp/projekt".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: t.local.node,
                harness_opts: Value::Null,
                max_cost_usd: None,
            },
        )
        .await
        .unwrap()
}

fn version(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

const TEXT: &str = "spec_version: 1\nname: git\nrules: []\n";

fn manifest(hash: &str, v: &str) -> Value {
    json!({
        "policy_set_hash": hash,
        "sets": [{ "scope": "project", "name": "git", "origin": "/repo/.beton/policies/git.yaml",
                   "version": v, "format": "yaml" }]
    })
}

#[tokio::test]
async fn pol_027_ac2_snapshot_round_trips_through_the_session_blob_store() {
    let t = store().await;
    let s = session(&t).await;
    let v = version(TEXT.as_bytes());
    let m = manifest("sha256:aa", &v);
    let sources = vec![(v.clone(), TEXT.as_bytes().to_vec())];
    assert!(
        t.store
            .put_policy_snapshot(org(&t), s.id, "sha256:aa", &m, &sources)
            .await
            .unwrap()
    );
    assert!(
        !t.store
            .put_policy_snapshot(org(&t), s.id, "sha256:aa", &m, &sources)
            .await
            .unwrap(),
        "zweimal derselbe Snapshot"
    );
    let (back, blobs) = t
        .store
        .policy_snapshot(org(&t), s.id, "sha256:aa")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back, m);
    assert_eq!(blobs, sources);
    assert!(
        t.store
            .policy_snapshot(org(&t), s.id, "sha256:bb")
            .await
            .unwrap()
            .is_none()
    );
    // Nur über die Session, die ihn gespeichert hat (DATA-006 AC1).
    let other = session(&t).await;
    assert!(
        t.store
            .policy_snapshot(org(&t), other.id, "sha256:aa")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn pol_027_ac2_snapshot_rejects_wrong_or_missing_content() {
    let t = store().await;
    let s = session(&t).await;
    let v = version(TEXT.as_bytes());
    let wrong = t
        .store
        .put_policy_snapshot(
            org(&t),
            s.id,
            "sha256:aa",
            &manifest("sha256:aa", &v),
            &[(v.clone(), b"anders".to_vec())],
        )
        .await;
    assert!(matches!(wrong, Err(Error::InvalidEvent(_))), "{wrong:?}");
    let missing = t
        .store
        .put_policy_snapshot(org(&t), s.id, "sha256:aa", &manifest("sha256:aa", &v), &[])
        .await;
    assert!(
        matches!(missing, Err(Error::InvalidEvent(_))),
        "{missing:?}"
    );
    let other_hash = t
        .store
        .put_policy_snapshot(
            org(&t),
            s.id,
            "sha256:aa",
            &manifest("sha256:bb", &v),
            &[(v.clone(), TEXT.as_bytes().to_vec())],
        )
        .await;
    assert!(matches!(other_hash, Err(Error::InvalidEvent(_))));
    let no_session = t
        .store
        .put_policy_snapshot(
            org(&t),
            SessionId::new(),
            "sha256:aa",
            &manifest("sha256:aa", &v),
            &[(v, TEXT.as_bytes().to_vec())],
        )
        .await;
    assert!(no_session.is_err());
}

fn decision(matched: Vec<Value>) -> EventPayload {
    EventPayload::PolicyDecision(PolicyDecision {
        decision_id: "pd_1".into(),
        phase: "tool_call".into(),
        outcome: PolicyOutcome::Allow,
        matched,
        enforcement: "full".into(),
        policy_set_hash: "sha256:aa".into(),
        ..PolicyDecision::default()
    })
}

fn hit(rule: &str, verdict: &str, dry_run: bool) -> Value {
    json!({ "rule_id": rule, "scope": "project", "set": "shadow", "set_version": "sha256:v",
            "verdict": verdict, "dry_run": dry_run })
}

async fn append(t: &TestStore, s: &SessionRecord, payloads: Vec<EventPayload>) {
    let head = t.store.session(s.org_id, s.id).await.unwrap().head_seq;
    let actor = Actor::Agent {
        id: None,
        harness: "fake".into(),
        agent_ref: None,
    };
    let events = payloads
        .into_iter()
        .map(|p| Event::new(s.id, 0, actor.clone(), p))
        .collect();
    t.store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

#[tokio::test]
async fn pol_027_ac4_shadow_hits_are_counted_per_rule_and_day_and_rebuildable() {
    let t = store().await;
    let a = session(&t).await;
    let b = session(&t).await;
    append(
        &t,
        &a,
        vec![
            decision(vec![hit("no-npm-publish", "deny", true)]),
            decision(vec![
                hit("no-npm-publish", "deny", true),
                hit("real", "deny", false),
            ]),
            decision(vec![hit("real", "deny", false)]),
        ],
    )
    .await;
    append(
        &t,
        &b,
        vec![decision(vec![
            hit("no-npm-publish", "deny", true),
            hit("curl", "ask", true),
        ])],
    )
    .await;
    let rows = t
        .store
        .policy_shadow(org(&t), "0000-00-00", "9999-12-31")
        .await
        .unwrap();
    let summary: Vec<(String, String, u64)> = rows
        .iter()
        .map(|r| (r.rule_id.clone(), r.verdict.clone(), r.hits))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("curl".to_owned(), "ask".to_owned(), 1),
            ("no-npm-publish".to_owned(), "deny".to_owned(), 3),
        ]
    );
    let npm = &rows[1];
    assert_eq!(npm.last_session_id, b.id, "letzter Treffer");
    assert_eq!(npm.scope, "project");
    assert_eq!(npm.set_name, "shadow");
    assert_eq!(npm.day.len(), 10);
    assert!(
        t.store
            .policy_shadow(org(&t), "0000-00-00", "0000-00-01")
            .await
            .unwrap()
            .is_empty()
    );

    // DATA-005: Rebuild ergibt dieselbe Projektion.
    let before = t.store.projection_dump(org(&t)).await.unwrap();
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(t.store.projection_dump(org(&t)).await.unwrap(), before);

    // Gelöschte Sessions zählen nicht mehr.
    t.store
        .delete_session(
            org(&t),
            b.id,
            PrincipalId::User(t.local.user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    let rows = t
        .store
        .policy_shadow(org(&t), "0000-00-00", "9999-12-31")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hits, 2);
}

#[tokio::test]
async fn pol_027_session_with_snapshot_can_be_deleted() {
    let t = store().await;
    let s = session(&t).await;
    let v = version(TEXT.as_bytes());
    t.store
        .put_policy_snapshot(
            org(&t),
            s.id,
            "sha256:aa",
            &manifest("sha256:aa", &v),
            &[(v, TEXT.as_bytes().to_vec())],
        )
        .await
        .unwrap();
    t.store
        .delete_session(
            org(&t),
            s.id,
            PrincipalId::User(t.local.user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
}
