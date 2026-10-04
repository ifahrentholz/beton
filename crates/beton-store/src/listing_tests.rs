//! Session-Liste, Suche, Anpinnen und Gelesen-Stand (SES-012).

use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Event, EventPayload, MessageCompleted, MessageRole, SessionKind, SessionTitleChanged,
    SessionTrigger, TitleSource,
};
use beton_core::id::SessionId;
use beton_core::time::Timestamp;
use serde_json::json;

use crate::listing::{SessionFilter, SessionScope, fts_query};
use crate::testutil::{TestStore, org, store};
use crate::{NewSession, SessionRecord};

fn new_session(t: &TestStore, harness: &str) -> NewSession {
    NewSession {
        id: SessionId::new(),
        owner: t.local.user,
        kind: SessionKind::Main,
        harness: harness.into(),
        cwd: "/tmp/projekt".into(),
        model: None,
        agent_ref: None,
        project_id: None,
        parent_id: None,
        trigger: SessionTrigger::User,
        home_node: t.local.node,
        harness_opts: serde_json::Value::Null,
    }
}

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "claude".into(),
        agent_ref: None,
    }
}

fn reply(text: &str) -> EventPayload {
    EventPayload::MessageCompleted(MessageCompleted {
        message_id: "msg_1".into(),
        role: MessageRole::Assistant,
        content: vec![json!({"type": "text", "text": text})],
        author: None,
    })
}

async fn append(t: &TestStore, s: &SessionRecord, payload: EventPayload) {
    let head = t.store.session(s.org_id, s.id).await.unwrap().head_seq;
    t.store
        .append(
            s.org_id,
            s.id,
            head,
            s.epoch,
            vec![Event::new(s.id, 0, agent(), payload)],
        )
        .await
        .unwrap();
}

async fn create(t: &TestStore, harness: &str) -> SessionRecord {
    t.store
        .create_session(org(t), new_session(t, harness))
        .await
        .unwrap()
}

fn query(q: &str) -> SessionFilter {
    SessionFilter {
        query: Some(q.into()),
        ..SessionFilter::default()
    }
}

#[test]
fn fts_query_quotes_every_term_as_prefix() {
    assert_eq!(fts_query("  "), None);
    assert_eq!(
        fts_query("Retry-After OR \"x"),
        Some("\"Retry-After\"* \"OR\"* \"\"\"x\"*".into())
    );
}

/// `n` Sessions mit Titel und je einer Antwort, per SQL in einem Rutsch angelegt (Tempo des
/// Aufbaus); der FTS-Index wird danach wie bei `rebuild` aus `search_docs` neu aufgebaut.
async fn bulk_sessions(t: &TestStore, n: usize) {
    let mut tx = t.store.write_tx().await.unwrap();
    let now = Timestamp::now().to_string();
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < ? - 1) \
         INSERT INTO sessions (id, org_id, owner_id, kind, harness, home_node_id, epoch, \
         head_seq, created_at, updated_at, title, status, archived, cost_micro, \
         last_activity_at) SELECT printf('ses_01J%023d', i), ?, ?, 'main', 'claude', ?, 1, 2, \
         ?, ?, 'Aufgabe ' || i || ' im Modul ' || (i % 97), 'idle', 0, 0, ? FROM n",
    )
    .bind(n as i64)
    .bind(org(t).to_string())
    .bind(t.local.user.to_string())
    .bind(t.local.node.to_string())
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .unwrap();
    for (doc, body) in [
        ("title", "title"),
        (
            "msg:2",
            "'Antwort ' || substr(id, 8) || ': Ich habe die Tests in Datei modul_' || \
             (CAST(substr(id, 8) AS INTEGER) % 113) || '.rs angepasst und laufen lassen.'",
        ),
    ] {
        sqlx::query(&format!(
            "INSERT INTO search_docs (org_id, session_id, doc, body) \
             SELECT org_id, id, '{doc}', {body} FROM sessions WHERE org_id = ?"
        ))
        .bind(org(t).to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO search_fts (search_fts) VALUES ('rebuild')")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn ses_012_ac1_search_finds_word_from_agent_reply_fast_with_10000_sessions() {
    let t = store().await;
    bulk_sessions(&t, 10_000).await;
    // Die gesuchte Session entsteht über den normalen Event-Pfad.
    let target = create(&t, "codex").await;
    append(
        &t,
        &target,
        reply("Der Rate-Limiter antwortet jetzt mit 429 und einem Retry-After-Header."),
    )
    .await;

    let start = Instant::now();
    let (hits, _) = t
        .store
        .session_list(org(&t), t.local.user, &query("Retry-After"), 50, None)
        .await
        .unwrap();
    let took = start.elapsed();
    assert_eq!(
        hits.iter().map(|h| h.session.id).collect::<Vec<_>>(),
        vec![target.id]
    );
    assert!(
        took < Duration::from_millis(300),
        "Suche dauerte {took:?} bei 10 000 Sessions"
    );
    // Präfix, Groß-/Kleinschreibung und mehrere Wörter (UND, im selben Dokument).
    for q in ["retry", "RATE-limiter 429", "header"] {
        let (hits, _) = t
            .store
            .session_list(org(&t), t.local.user, &query(q), 50, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "{q}");
    }
    // Ein häufiges Wort: viele Treffer, trotzdem schnell, Seite begrenzt.
    let start = Instant::now();
    let (hits, next) = t
        .store
        .session_list(org(&t), t.local.user, &query("modul_7"), 50, None)
        .await
        .unwrap();
    assert!(start.elapsed() < Duration::from_millis(300));
    assert_eq!(hits.len(), 50);
    assert!(next.is_some());
    // Titel sind ebenfalls durchsuchbar.
    let (hits, _) = t
        .store
        .session_list(org(&t), t.local.user, &query("Aufgabe 4711"), 50, None)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn ses_012_filters_by_scope_harness_and_status() {
    let t = store().await;
    let a = create(&t, "claude").await;
    let b = create(&t, "codex").await;
    append(
        &t,
        &b,
        EventPayload::SessionArchived(beton_core::event::Empty {}),
    )
    .await;
    let list = |f: SessionFilter| {
        let t = &t;
        async move {
            t.store
                .session_list(org(t), t.local.user, &f, 50, None)
                .await
                .unwrap()
                .0
                .into_iter()
                .map(|v| v.session.id)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(list(SessionFilter::default()).await, vec![a.id]);
    assert_eq!(
        list(SessionFilter {
            scope: SessionScope::Archived,
            ..SessionFilter::default()
        })
        .await,
        vec![b.id]
    );
    assert_eq!(
        list(SessionFilter {
            scope: SessionScope::Own,
            ..SessionFilter::default()
        })
        .await,
        vec![a.id]
    );
    assert!(
        list(SessionFilter {
            scope: SessionScope::Shared,
            ..SessionFilter::default()
        })
        .await
        .is_empty()
    );
    assert_eq!(
        list(SessionFilter {
            scope: SessionScope::All,
            harness: Some("codex".into()),
            ..SessionFilter::default()
        })
        .await,
        vec![b.id]
    );
    assert_eq!(
        list(SessionFilter {
            status: Some(beton_core::event::SessionStatus::Starting),
            ..SessionFilter::default()
        })
        .await,
        vec![a.id]
    );
}

#[tokio::test]
async fn ses_012_ac3_pinned_sessions_come_first_regardless_of_sort() {
    let t = store().await;
    let oldest = create(&t, "claude").await;
    let mut others = Vec::new();
    for _ in 0..4 {
        others.push(create(&t, "claude").await);
    }
    // Die älteste Session hat die älteste ID und die älteste Aktivität.
    let pinned = t
        .store
        .set_pinned(org(&t), t.local.user, oldest.id, true)
        .await
        .unwrap();
    assert!(pinned.pinned);
    let (page, next) = t
        .store
        .session_list(org(&t), t.local.user, &SessionFilter::default(), 2, None)
        .await
        .unwrap();
    assert_eq!(page[0].session.id, oldest.id, "angepinnt oben");
    assert!(page[0].pinned && !page[1].pinned);
    assert_eq!(page.len(), 3, "Angepinnte zusätzlich zur Seitengröße");
    // Folgeseiten wiederholen die angepinnte Session nicht.
    let (rest, _) = t
        .store
        .session_list(org(&t), t.local.user, &SessionFilter::default(), 10, next)
        .await
        .unwrap();
    assert!(rest.iter().all(|v| v.session.id != oldest.id));
    assert_eq!(page.len() + rest.len(), 5);
    // Lösen: wieder normal einsortiert.
    t.store
        .set_pinned(org(&t), t.local.user, oldest.id, false)
        .await
        .unwrap();
    let (page, _) = t
        .store
        .session_list(org(&t), t.local.user, &SessionFilter::default(), 10, None)
        .await
        .unwrap();
    assert_eq!(page.last().unwrap().session.id, oldest.id);
}

#[tokio::test]
async fn ses_012_ac2_read_state_is_monotonic_and_appears_in_deltas() {
    let t = store().await;
    let s = create(&t, "claude").await;
    append(&t, &s, reply("Fertig.")).await;
    let view = t
        .store
        .session_view(org(&t), t.local.user, s.id)
        .await
        .unwrap();
    assert!(view.unread());
    let since = view.changed_at;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let read = t
        .store
        .mark_read(org(&t), t.local.user, s.id, 99)
        .await
        .unwrap();
    assert_eq!(read.read_seq, 2, "höchstens head_seq");
    assert!(!read.unread());
    // Ein älterer Stand (anderes Gerät, verspätet) setzt nicht zurück.
    let again = t
        .store
        .mark_read(org(&t), t.local.user, s.id, 1)
        .await
        .unwrap();
    assert_eq!(again.read_seq, 2);
    // Das zweite Gerät sieht die Änderung im Delta.
    let delta = t
        .store
        .sessions_changed_after(org(&t), t.local.user, since, 50)
        .await
        .unwrap();
    assert_eq!(delta.len(), 1);
    assert!(!delta[0].unread());
    assert!(delta[0].changed_at > since);
}

#[tokio::test]
async fn ses_012_delete_and_rebuild_keep_search_index_consistent() {
    let t = store().await;
    let s = create(&t, "claude").await;
    append(&t, &s, reply("Einmaliges Stichwort Zebrastreifen")).await;
    t.store
        .mark_read(org(&t), t.local.user, s.id, 2)
        .await
        .unwrap();
    let before = t.store.projection_dump(org(&t)).await.unwrap();
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(t.store.projection_dump(org(&t)).await.unwrap(), before);
    let (hits, _) = t
        .store
        .session_list(org(&t), t.local.user, &query("zebra"), 10, None)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    t.store
        .delete_session(
            org(&t),
            s.id,
            beton_core::id::PrincipalId::User(t.local.user),
            crate::DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    let (hits, _) = t
        .store
        .session_list(org(&t), t.local.user, &query("zebra"), 10, None)
        .await
        .unwrap();
    assert!(hits.is_empty());
    let docs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_docs")
        .fetch_one(&t.store.pool)
        .await
        .unwrap();
    assert_eq!(docs, 0);
}

#[tokio::test]
async fn ses_004_last_event_of_type_finds_latest() {
    let t = store().await;
    let s = create(&t, "claude").await;
    assert!(
        t.store
            .last_event_of_type(org(&t), s.id, "session.title_changed")
            .await
            .unwrap()
            .is_none()
    );
    for title in ["eins", "zwei"] {
        append(
            &t,
            &s,
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: title.into(),
                source: TitleSource::User,
            }),
        )
        .await;
    }
    let last = t
        .store
        .last_event_of_type(org(&t), s.id, "session.title_changed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.seq, 3);
}

#[tokio::test]
async fn ses_012_view_carries_the_full_session_record() {
    let t = store().await;
    let s = create(&t, "claude").await;
    let view = t
        .store
        .session_view(org(&t), t.local.user, s.id)
        .await
        .unwrap();
    assert_eq!(view.session, t.store.session(org(&t), s.id).await.unwrap());
}
