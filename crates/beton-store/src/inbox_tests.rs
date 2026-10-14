//! UX-001: Inbox-Projektion aus dem Event-Log; POL-009: Zustand `user_day`/`user` im Store.

use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, Event,
    EventPayload, PolicyNotified, ResolvedVia, SessionKind, SessionTrigger, TimeoutAction,
};
use beton_core::id::{ApprovalId, PrincipalId, SessionId};
use serde_json::{Value, json};

use crate::testutil::{TestStore, org, store};
use crate::{InboxQuery, NewSession, SessionRecord};

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
            },
        )
        .await
        .unwrap()
}

fn ev(session: SessionId, payload: EventPayload) -> Event {
    Event::new(session, 0, Actor::default(), payload)
}

fn requested(id: ApprovalId, kind: ApprovalKind) -> EventPayload {
    EventPayload::ApprovalRequested(ApprovalRequested {
        approval_id: id,
        kind,
        subject: if kind == ApprovalKind::Question {
            json!({"question": "Pro IP oder pro Account?", "choices": ["Pro IP"]})
        } else {
            json!({"title": "Shell-Kommando bestätigen", "summary": "git push origin main",
                   "tool": {"name": "Bash", "kind": "shell", "args": {"command": "git push origin main"}}})
        },
        options: vec!["approve".into(), "approve_session".into(), "deny".into()],
        on_timeout: TimeoutAction::Abort,
        ..ApprovalRequested::default()
    })
}

fn resolved(t: &TestStore, id: ApprovalId) -> EventPayload {
    EventPayload::ApprovalResolved(ApprovalResolved {
        approval_id: id,
        decision: ApprovalDecision::Allow,
        via: ResolvedVia::User,
        actor: Actor::User {
            id: PrincipalId::User(t.local.user),
            device_id: None,
        },
        comment: Some("passt".into()),
        ..ApprovalResolved::default()
    })
}

async fn append(t: &TestStore, s: &SessionRecord, events: Vec<Event>) {
    let head = t.store.session(org(t), s.id).await.unwrap().head_seq;
    t.store
        .append(org(t), s.id, head, s.epoch, events)
        .await
        .unwrap();
}

#[tokio::test]
async fn ux_001_ac1_requested_is_open_and_resolved_is_done() {
    let t = store().await;
    let s = session(&t).await;
    let a = ApprovalId::new();
    append(&t, &s, vec![ev(s.id, requested(a, ApprovalKind::Policy))]).await;

    let open = t
        .store
        .inbox(
            org(&t),
            &InboxQuery {
                status: Some("open".into()),
                ..InboxQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    let item = &open[0];
    assert_eq!(item.kind, "approval");
    assert_eq!(item.session_id, s.id);
    assert_eq!(item.approval_id.as_deref(), Some(a.to_string().as_str()));
    assert_eq!(item.title, "Shell-Kommando bestätigen");
    assert_eq!(item.summary, "git push origin main");
    assert_eq!(item.detail["options"][1], "approve_session");
    assert_eq!(item.detail["on_timeout"], "abort");
    assert_eq!(t.store.inbox_open_count(org(&t)).await.unwrap(), 1);

    append(&t, &s, vec![ev(s.id, resolved(&t, a))]).await;
    let done = t.store.inbox_item(org(&t), &item.id).await.unwrap();
    assert_eq!(done.status, "done");
    let r = done.resolution.unwrap();
    assert_eq!(r["decision"], "allow");
    assert_eq!(r["via"], "user");
    assert_eq!(r["comment"], "passt");
    assert_eq!(r["actor"]["kind"], "user");
    assert_eq!(t.store.inbox_open_count(org(&t)).await.unwrap(), 0);
}

#[tokio::test]
async fn ux_001_ac3_counter_counts_open_approvals_and_questions() {
    let t = store().await;
    let s = session(&t).await;
    let q = ApprovalId::new();
    append(
        &t,
        &s,
        vec![
            ev(s.id, requested(q, ApprovalKind::Question)),
            ev(
                s.id,
                EventPayload::PolicyNotified(PolicyNotified {
                    rule_id: "count-pushes".into(),
                    scope: "project".into(),
                    set: "git".into(),
                    channels: vec!["inbox".into()],
                    message: Some("Push Nr. 3".into()),
                }),
            ),
        ],
    )
    .await;
    let all = t
        .store
        .inbox(org(&t), &InboxQuery::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let notice = all.iter().find(|i| i.kind == "notice").unwrap();
    assert_eq!(notice.status, "info");
    assert_eq!(notice.title, "Push Nr. 3");
    assert_eq!(notice.detail["rule_id"], "count-pushes");
    let question = all.iter().find(|i| i.kind == "question").unwrap();
    assert_eq!(question.title, "Frage des Agents");
    assert_eq!(question.summary, "Pro IP oder pro Account?");
    // Nur Freigaben und Fragen zählen.
    assert_eq!(t.store.inbox_open_count(org(&t)).await.unwrap(), 1);
    let only_notices = t
        .store
        .inbox(
            org(&t),
            &InboxQuery {
                kind: Some("notice".into()),
                ..InboxQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(only_notices.len(), 1);
}

#[tokio::test]
async fn ux_001_ac4_rebuild_reproduces_the_inbox() {
    let t = store().await;
    let s = session(&t).await;
    let (a, b) = (ApprovalId::new(), ApprovalId::new());
    append(
        &t,
        &s,
        vec![
            ev(s.id, requested(a, ApprovalKind::Policy)),
            ev(s.id, requested(b, ApprovalKind::Tool)),
            ev(s.id, resolved(&t, a)),
        ],
    )
    .await;
    let before = t
        .store
        .inbox(org(&t), &InboxQuery::default())
        .await
        .unwrap();
    let dump = t.store.projection_dump(org(&t)).await.unwrap();
    // Beschädigen und neu aufbauen.
    sqlx::query("UPDATE inbox_items SET status = 'kaputt' WHERE org_id = ?")
        .bind(org(&t).to_string())
        .execute(&t.store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM inbox_items WHERE org_id = ? AND approval_id = ?")
        .bind(org(&t).to_string())
        .bind(b.to_string())
        .execute(&t.store.pool)
        .await
        .unwrap();
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(
        t.store
            .inbox(org(&t), &InboxQuery::default())
            .await
            .unwrap(),
        before
    );
    assert_eq!(t.store.projection_dump(org(&t)).await.unwrap(), dump);
}

#[tokio::test]
async fn pol_009_ac3_user_day_starts_empty_on_a_new_day() {
    let t = store().await;
    let inc = |v: Option<Value>| -> Result<Option<Value>, String> {
        Ok(Some(json!(v.and_then(|v| v.as_i64()).unwrap_or(0) + 1)))
    };
    for _ in 0..2 {
        t.store
            .update_policy_user_state(
                org(&t),
                t.local.user,
                "user_day",
                "2026-01-01",
                "pushes",
                inc,
            )
            .await
            .unwrap();
    }
    t.store
        .update_policy_user_state(org(&t), t.local.user, "user", "2026-01-01", "total", inc)
        .await
        .unwrap();
    let (day, user) = t
        .store
        .policy_user_state(org(&t), t.local.user, "2026-01-01")
        .await
        .unwrap();
    assert_eq!(day["pushes"], json!(2));
    assert_eq!(user["total"], json!(1));
    // Neuer Tag: user_day leer, user bleibt.
    let (day, user) = t
        .store
        .policy_user_state(org(&t), t.local.user, "2026-01-02")
        .await
        .unwrap();
    assert!(day.is_empty());
    assert_eq!(user["total"], json!(1));
    // Löschen und Fehler.
    t.store
        .update_policy_user_state(org(&t), t.local.user, "user", "", "total", |_| Ok(None))
        .await
        .unwrap();
    let (_, user) = t
        .store
        .policy_user_state(org(&t), t.local.user, "2026-01-02")
        .await
        .unwrap();
    assert!(user.is_empty());
    assert!(
        t.store
            .update_policy_user_state(org(&t), t.local.user, "session", "", "x", inc)
            .await
            .is_err()
    );
    assert!(
        t.store
            .update_policy_user_state(org(&t), t.local.user, "user", "", "x", |_| Err(
                "zu groß".into()
            ))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ux_001_inbox_pages_with_a_cursor() {
    let t = store().await;
    let s = session(&t).await;
    let events: Vec<Event> = (0..5)
        .map(|_| ev(s.id, requested(ApprovalId::new(), ApprovalKind::Policy)))
        .collect();
    append(&t, &s, events).await;
    let first = t
        .store
        .inbox(
            org(&t),
            &InboxQuery {
                limit: 3,
                ..InboxQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 3);
    let last = first.last().unwrap();
    let rest = t
        .store
        .inbox(
            org(&t),
            &InboxQuery {
                limit: 3,
                before: Some((last.created_at.to_string(), last.id.clone())),
                ..InboxQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.len(), 2);
    let mut all: Vec<_> = first.iter().chain(&rest).map(|i| i.id.clone()).collect();
    all.dedup();
    assert_eq!(all.len(), 5, "jeder Eintrag genau einmal");
}

#[tokio::test]
async fn pol_009_user_state_values_pass_the_redaction() {
    struct Hide;
    impl crate::ValueRedactor for Hide {
        fn redact(&self, v: &mut Value) -> std::result::Result<bool, crate::RedactionUnavailable> {
            let text = v.to_string();
            if text.contains("ghp_geheim") {
                *v = serde_json::from_str(&text.replace("ghp_geheim", "[REDACTED]")).unwrap();
                return Ok(true);
            }
            Ok(false)
        }
    }
    let t = crate::testutil::store_with(crate::StoreOptions {
        redactor: std::sync::Arc::new(Hide),
        ..crate::StoreOptions::default()
    })
    .await;
    let stored = t
        .store
        .update_policy_user_state(org(&t), t.local.user, "user", "", "last", |_| {
            Ok(Some(json!("token ghp_geheim")))
        })
        .await
        .unwrap();
    assert_eq!(stored, Some(json!("token [REDACTED]")));
    let (_, user) = t
        .store
        .policy_user_state(org(&t), t.local.user, "2026-01-01")
        .await
        .unwrap();
    assert_eq!(user["last"], json!("token [REDACTED]"));
}
