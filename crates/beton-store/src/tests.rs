//! Tests über die Store-API hinweg (DATA-001, DATA-002, DATA-005, DATA-006, DATA-008, PROTO-001).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use beton_core::event::{
    Actor, ApprovalDecision, ApprovalKind, ApprovalRequested, ApprovalResolved, CostDelta, Empty,
    Event, EventBody, EventPayload, Notice, RawJson, ResolvedVia, SessionKind, SessionStatus,
    SessionStatusChanged, SessionTitleChanged, SessionTrigger, TextDelta, TitleSource,
    ToolCallCompleted, ToolStatus,
};
use beton_core::id::{ApprovalId, OrgId, PrincipalId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};

use crate::testutil::{TestStore, org, store, store_with};
use crate::{DeleteAuthority, Error, NewSession, RawRedactor, SessionRecord, Store, StoreOptions};

fn new_session(t: &TestStore) -> NewSession {
    NewSession {
        id: SessionId::new(),
        owner: t.local.user,
        kind: SessionKind::Main,
        harness: "claude".into(),
        cwd: "/tmp/projekt".into(),
        model: Some("sonnet".into()),
        agent_ref: None,
        project_id: None,
        parent_id: None,
        trigger: SessionTrigger::User,
        home_node: t.local.node,
    }
}

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "claude".into(),
        agent_ref: None,
    }
}

fn notice(session: SessionId, text: &str) -> Event {
    Event::new(
        session,
        0,
        agent(),
        EventPayload::Notice(Notice {
            text: text.into(),
            ..Notice::default()
        }),
    )
}

fn ev(session: SessionId, payload: EventPayload) -> Event {
    Event::new(session, 0, agent(), payload)
}

async fn append_one(store: &Store, s: &SessionRecord, event: Event) -> Event {
    let head = store.session(s.org_id, s.id).await.unwrap().head_seq;
    store
        .append(s.org_id, s.id, head, s.epoch, vec![event])
        .await
        .unwrap()
        .remove(0)
}

// ---------------------------------------------------------------------------
// DATA-002: Append-only-Log
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_002_session_created_is_seq_one() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    assert_eq!(s.head_seq, 1);
    assert_eq!(s.epoch, 1);
    let events = t.store.events(s.org_id, s.id, 0, 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].type_name(), "session.created");
    assert_eq!(events[0].seq, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_002_ac1_concurrent_appends_one_wins() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    for round in 0..20 {
        let head = t.store.session(s.org_id, s.id).await.unwrap().head_seq;
        let a = t.store.clone();
        let b = t.store.clone();
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move {
                a.append(s.org_id, s.id, head, 1, vec![notice(s.id, "a")])
                    .await
            }),
            tokio::spawn(async move {
                b.append(s.org_id, s.id, head, 1, vec![notice(s.id, "b")])
                    .await
            }),
        );
        let results = [ra.unwrap(), rb.unwrap()];
        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1, "Runde {round}: genau ein Append gelingt");
        let err = results.into_iter().find_map(Result::err).unwrap();
        assert_eq!(err.code(), "seq_conflict", "{err}");
        assert!(
            matches!(err, Error::SeqConflict { expected, actual } if expected == head && actual == head + 1)
        );
    }
    let after = t.store.session(s.org_id, s.id).await.unwrap();
    assert_eq!(after.head_seq, 21);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_002_parallel_writers_keep_log_gapless() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let conflicts = Arc::new(AtomicUsize::new(0));
    let writers: Vec<_> = (0..8)
        .map(|w| {
            let store = t.store.clone();
            let conflicts = conflicts.clone();
            tokio::spawn(async move {
                for i in 0..25 {
                    loop {
                        let head = store.session(s.org_id, s.id).await.unwrap().head_seq;
                        match store
                            .append(
                                s.org_id,
                                s.id,
                                head,
                                1,
                                vec![notice(s.id, &format!("{w}/{i}"))],
                            )
                            .await
                        {
                            Ok(_) => break,
                            Err(Error::SeqConflict { .. }) => {
                                conflicts.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => panic!("{e}"),
                        }
                    }
                }
            })
        })
        .collect();
    for w in writers {
        w.await.unwrap();
    }
    let events = t.store.events(s.org_id, s.id, 0, 1_000).await.unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=201).collect::<Vec<_>>());
    assert_eq!(t.store.session(s.org_id, s.id).await.unwrap().head_seq, 201);
}

#[tokio::test]
async fn data_002_stale_epoch_is_rejected() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let err = t
        .store
        .append(s.org_id, s.id, 1, 0, vec![notice(s.id, "alt")])
        .await
        .unwrap_err();
    assert_eq!(err.code(), "stale_epoch");
}

#[tokio::test]
async fn data_002_transient_and_foreign_events_are_rejected() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let delta = ev(s.id, EventPayload::MessageDelta(TextDelta::default()));
    let err = t
        .store
        .append(s.org_id, s.id, 1, 1, vec![delta])
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid_event");
    let other = notice(SessionId::new(), "falsche Session");
    let err = t
        .store
        .append(s.org_id, s.id, 1, 1, vec![other])
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid_event");
    assert_eq!(t.store.session(s.org_id, s.id).await.unwrap().head_seq, 1);
}

#[tokio::test]
async fn data_002_events_roundtrip_envelope_fields() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let mut e = notice(s.id, "hallo");
    e.turn_id = Some(beton_core::id::TurnId::new());
    e.causation_id = Some(beton_core::id::EventId::new());
    let written = append_one(&t.store, &s, e.clone()).await;
    let read = t.store.events(s.org_id, s.id, 1, 10).await.unwrap();
    assert_eq!(read, vec![written.clone()]);
    let mut expected = e;
    expected.seq = 2;
    assert_eq!(written, expected);
}

#[tokio::test]
async fn data_002_ac3_batch_append_throughput() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let total = 20_000u64;
    let batch = 500u64;
    let mut head = 1;
    let started = Instant::now();
    for _ in 0..total / batch {
        let events = (0..batch)
            .map(|i| notice(s.id, &format!("Zeile {i}")))
            .collect();
        head = t
            .store
            .append(s.org_id, s.id, head, 1, events)
            .await
            .unwrap()
            .last()
            .unwrap()
            .seq;
    }
    let elapsed = started.elapsed();
    let rate = total as f64 / elapsed.as_secs_f64();
    eprintln!("DATA-002 AC3: {rate:.0} Events/s ({total} in {elapsed:?})");
    assert_eq!(head, total + 1);
    assert!(rate >= 5_000.0, "nur {rate:.0} Events/s");
}

// ---------------------------------------------------------------------------
// PROTO-001: raw und große Payloads
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CountingRedactor(AtomicUsize);

impl RawRedactor for CountingRedactor {
    fn redact(&self, raw: &mut Value) {
        self.0.fetch_add(1, Ordering::Relaxed);
        if let Some(obj) = raw.as_object_mut() {
            obj.insert("token".into(), "[REDACTED]".into());
        }
    }
}

#[tokio::test]
async fn proto_001_ac3_raw_passes_redaction_hook_before_persistence() {
    let redactor = Arc::new(CountingRedactor::default());
    let t = store_with(StoreOptions {
        redactor: redactor.clone(),
        ..StoreOptions::default()
    })
    .await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let mut e = notice(s.id, "mit raw");
    e.raw = Some(RawJson::from_value(
        &json!({"type": "assistant", "token": "geheim"}),
    ));
    let written = append_one(&t.store, &s, e).await;
    assert_eq!(redactor.0.load(Ordering::Relaxed), 1);
    assert!(written.raw.is_none(), "raw geht nicht auf den Draht");
    let raw = t
        .store
        .event_raw(s.org_id, s.id, written.seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(raw, json!({"type": "assistant", "token": "[REDACTED]"}));
}

#[tokio::test]
async fn har_001_ac3_raw_lands_byte_identical_in_log() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let line = r#"{"type":"assistant", "z":1,"a":"\u00e4","n":1.50}"#;
    let mut e = notice(s.id, "raw");
    e.raw = Some(RawJson::from_string(line.to_owned()).unwrap());
    let written = append_one(&t.store, &s, e).await;
    let stored: String =
        sqlx::query_scalar("SELECT raw FROM event_raw WHERE org_id = ? AND seq = ?")
            .bind(org(&t).to_string())
            .bind(written.seq as i64)
            .fetch_one(&t.store.pool)
            .await
            .unwrap();
    assert_eq!(stored, line);
}

#[tokio::test]
async fn proto_001_ac3_store_raw_false_keeps_no_raw() {
    let redactor = Arc::new(CountingRedactor::default());
    let t = store_with(StoreOptions {
        store_raw: false,
        redactor: redactor.clone(),
        ..StoreOptions::default()
    })
    .await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let mut e = notice(s.id, "mit raw");
    e.raw = Some(RawJson::from_value(&json!({"token": "geheim"})));
    let written = append_one(&t.store, &s, e).await;
    assert!(
        t.store
            .event_raw(s.org_id, s.id, written.seq)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(redactor.0.load(Ordering::Relaxed), 0);
    let db = std::fs::read(t.dir.path().join("beton.db")).unwrap();
    let wal = std::fs::read(t.dir.path().join("beton.db-wal")).unwrap_or_default();
    for bytes in [db, wal] {
        assert!(!bytes.windows(6).any(|w| w == b"geheim"));
    }
}

fn big_tool_result(session: SessionId, kib: usize) -> Event {
    ev(
        session,
        EventPayload::ToolCallCompleted(ToolCallCompleted {
            call_id: "call_1".into(),
            status: ToolStatus::Ok,
            result: Some(Value::String("x".repeat(kib * 1024))),
            result_ref: None,
            duration_ms: 12,
        }),
    )
}

#[tokio::test]
async fn proto_001_ac4_large_payload_is_offloaded_to_blob() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let original = big_tool_result(s.id, 100);
    let EventBody::Inline(original_payload) = original.body.clone() else {
        unreachable!()
    };
    let written = append_one(&t.store, &s, original).await;

    let EventBody::Offloaded(o) = &written.body else {
        panic!("100 KiB müssen ausgelagert werden");
    };
    let wire = serde_json::to_value(&written).unwrap();
    assert!(wire.get("payload").is_none());
    assert_eq!(wire["payload_ref"], o.payload_ref.to_string());
    assert_eq!(wire["type"], "tool.call.completed");

    let read = t.store.events(s.org_id, s.id, 1, 10).await.unwrap();
    assert_eq!(read[0].body, written.body);
    // Inhalt über die Blob-API der Session.
    let blob = t
        .store
        .session_blob(s.org_id, s.id, &o.payload_ref)
        .await
        .unwrap();
    let from_blob: Value = serde_json::from_slice(&blob).unwrap();
    assert_eq!(from_blob["call_id"], "call_1");
    assert_eq!(
        t.store.resolve_payload(s.org_id, &read[0]).await.unwrap(),
        original_payload
    );

    // Knapp unter der Grenze bleibt die Nutzlast inline.
    let small = append_one(&t.store, &s, big_tool_result(s.id, 60)).await;
    assert!(matches!(small.body, EventBody::Inline(_)));
}

// ---------------------------------------------------------------------------
// DATA-001: Org-Isolation und Fremdschlüssel
// ---------------------------------------------------------------------------

/// Legt in einer zweiten Org eine Session mit Events, Blob, Approval und Usage an.
async fn populate_second_org(t: &TestStore) -> (OrgId, SessionRecord, beton_core::event::BlobRef) {
    let org_b = OrgId::new();
    let user_b = UserId::new();
    let node_b = beton_core::id::NodeId::new();
    t.store.create_org(org_b, "Zweite").await.unwrap();
    t.store.create_user(org_b, user_b, "B").await.unwrap();
    t.store.create_node(org_b, node_b, "b").await.unwrap();
    let mut new = new_session(t);
    new.owner = user_b;
    new.home_node = node_b;
    let s = t.store.create_session(org_b, new).await.unwrap();
    let approval = ApprovalId::new();
    let mut raw_event = notice(s.id, "raw");
    raw_event.raw = Some(RawJson::from_value(&json!({"x": 1})));
    t.store
        .append(
            org_b,
            s.id,
            1,
            1,
            vec![
                ev(
                    s.id,
                    EventPayload::ApprovalRequested(ApprovalRequested {
                        approval_id: approval,
                        kind: ApprovalKind::Tool,
                        ..ApprovalRequested::default()
                    }),
                ),
                ev(
                    s.id,
                    EventPayload::CostDelta(CostDelta {
                        harness: "claude".into(),
                        model: "sonnet".into(),
                        cost_micro: Some(5),
                        ..CostDelta::default()
                    }),
                ),
                raw_event,
                big_tool_result(s.id, 70),
            ],
        )
        .await
        .unwrap();
    let blob = t
        .store
        .put_session_blob(org_b, s.id, b"anhang")
        .await
        .unwrap();
    (org_b, s, blob)
}

#[tokio::test]
async fn data_001_ac2_data_of_another_org_is_unreachable() {
    let t = store().await;
    let (org_b, s, blob) = populate_second_org(&t).await;
    let a = org(&t);
    assert_eq!(a, OrgId::LOCAL);

    // Gegenprobe: in Org B ist alles sichtbar.
    assert_eq!(t.store.sessions(org_b, true).await.unwrap().len(), 1);
    assert_eq!(t.store.open_approvals(org_b).await.unwrap().len(), 1);
    assert_eq!(
        t.store
            .usage_daily(org_b, "0000", "9999")
            .await
            .unwrap()
            .len(),
        1
    );

    // Aus Org A ist nichts davon erreichbar.
    assert!(t.store.sessions(a, true).await.unwrap().is_empty());
    assert_eq!(
        t.store.session(a, s.id).await.unwrap_err().code(),
        "not_found"
    );
    assert_eq!(
        t.store.events(a, s.id, 0, 10).await.unwrap_err().code(),
        "not_found"
    );
    assert!(t.store.event_raw(a, s.id, 4).await.unwrap().is_none());
    assert_eq!(
        t.store
            .session_blob(a, s.id, &blob)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        t.store
            .put_session_blob(a, s.id, b"x")
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert!(t.store.open_approvals(a).await.unwrap().is_empty());
    assert!(
        t.store
            .usage_daily(a, "0000", "9999")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.store
            .append(a, s.id, s.head_seq, 1, vec![notice(s.id, "x")])
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        t.store
            .rebuild_projections(a, Some(s.id))
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(t.store.rebuild_projections(a, None).await.unwrap(), 0);
    assert_eq!(
        t.store
            .delete_session(
                a,
                s.id,
                PrincipalId::User(UserId::LOCAL),
                DeleteAuthority::OrgAdmin
            )
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        t.store.delete_user(a, s.owner).await.unwrap_err().code(),
        "not_found"
    );

    // Löschen in B: Tombstones und Audit nur in B sichtbar.
    t.store
        .delete_session(
            org_b,
            s.id,
            PrincipalId::User(s.owner),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    assert!(
        t.store
            .tombstones_since(a, Timestamp::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.store.audit_entries(a, 10).await.unwrap().is_empty());
    assert_eq!(
        t.store
            .tombstones_since(org_b, Timestamp::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Lint-Regel zu DATA-001 AC2: Jedes SQL in `beton-store`, das Domänentabellen liest oder
/// schreibt, nennt `org_id`. Ausnahmen müssen als `org-übergreifend` markiert sein.
#[test]
fn data_001_ac2_every_query_filters_by_org() {
    let re = regex_lite_sql();
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut checked = 0;
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|n| n == "tests.rs" || n == "schema.rs" || n == "migrate.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text.split("#[cfg(test)]").next().unwrap_or_default();
        for sql in string_literals(text) {
            let upper = sql.to_uppercase();
            if !re.iter().any(|kw| upper.contains(kw)) {
                continue;
            }
            checked += 1;
            // Die Org-Tabelle selbst: ihr `id` ist die org_id.
            if upper.contains("INTO ORGS ") {
                continue;
            }
            assert!(
                sql.contains("org_id") || sql.contains("org-übergreifend"),
                "{}: SQL ohne org_id-Filter:\n{sql}",
                path.display()
            );
        }
    }
    assert!(checked > 30, "nur {checked} SQL-Anweisungen gefunden");
}

fn regex_lite_sql() -> [&'static str; 4] {
    ["SELECT ", "UPDATE ", "DELETE FROM", "INSERT INTO"]
}

/// Inhalte von `"…"`-Literalen (mit `\`-Zeilenfortsetzung), grob genug für SQL-Strings.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        s.push(n);
                    }
                }
                '"' => break,
                _ => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

#[tokio::test]
async fn data_001_ac3_user_with_sessions_cannot_be_deleted() {
    let t = store().await;
    let user = UserId::new();
    t.store.create_user(org(&t), user, "Kim").await.unwrap();
    let mut new = new_session(&t);
    new.owner = user;
    let s = t.store.create_session(org(&t), new).await.unwrap();
    let err = t.store.delete_user(org(&t), user).await.unwrap_err();
    assert_eq!(err.code(), "conflict", "{err}");
    t.store
        .delete_session(
            org(&t),
            s.id,
            PrincipalId::User(user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    t.store.delete_user(org(&t), user).await.unwrap();
}

#[tokio::test]
async fn data_001_local_identity_is_stable() {
    let t = store().await;
    assert_eq!(t.local.org.to_string(), "org_local");
    assert_eq!(t.local.user.to_string(), "usr_local");
    assert_eq!(t.store.ensure_local().await.unwrap(), t.local);
    drop(t.store);
    let reopened = Store::open(t.dir.path(), StoreOptions::default())
        .await
        .unwrap();
    assert_eq!(reopened.ensure_local().await.unwrap(), t.local);
}

// ---------------------------------------------------------------------------
// DATA-005: Projektionen
// ---------------------------------------------------------------------------

async fn session_with_history(t: &TestStore, title: &str) -> SessionRecord {
    let s = t
        .store
        .create_session(org(t), new_session(t))
        .await
        .unwrap();
    let approval_open = ApprovalId::new();
    let approval_done = ApprovalId::new();
    let events = vec![
        ev(
            s.id,
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: title.into(),
                source: TitleSource::Generated,
            }),
        ),
        ev(
            s.id,
            EventPayload::SessionStatus(SessionStatusChanged {
                status: SessionStatus::Running,
                reason: None,
            }),
        ),
        ev(
            s.id,
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: approval_done,
                kind: ApprovalKind::Tool,
                subject: json!({"tool": "bash"}),
                options: vec!["allow".into(), "deny".into()],
                ..ApprovalRequested::default()
            }),
        ),
        ev(
            s.id,
            EventPayload::ApprovalResolved(ApprovalResolved {
                approval_id: approval_done,
                decision: ApprovalDecision::Allow,
                via: ResolvedVia::User,
                actor: Actor::User {
                    id: PrincipalId::User(t.local.user),
                    device_id: None,
                },
                ..ApprovalResolved::default()
            }),
        ),
        ev(
            s.id,
            EventPayload::ApprovalRequested(ApprovalRequested {
                approval_id: approval_open,
                kind: ApprovalKind::Policy,
                subject: json!({"rule": "spend_cap"}),
                ..ApprovalRequested::default()
            }),
        ),
        ev(
            s.id,
            EventPayload::CostDelta(CostDelta {
                harness: "claude".into(),
                model: "sonnet".into(),
                input_tokens: 1000,
                output_tokens: 200,
                cost_micro: Some(12_500),
                ..CostDelta::default()
            }),
        ),
        ev(
            s.id,
            EventPayload::CostDelta(CostDelta {
                harness: "claude".into(),
                model: "sonnet".into(),
                input_tokens: 10,
                output_tokens: 2,
                cost_micro: Some(500),
                ..CostDelta::default()
            }),
        ),
        big_tool_result(s.id, 80),
        ev(s.id, EventPayload::SessionArchived(Empty {})),
    ];
    t.store.append(s.org_id, s.id, 1, 1, events).await.unwrap();
    t.store.session(s.org_id, s.id).await.unwrap()
}

#[tokio::test]
async fn data_005_projections_follow_appends_in_same_transaction() {
    let t = store().await;
    let s = session_with_history(&t, "Tests reparieren").await;
    assert_eq!(s.title, "Tests reparieren");
    assert_eq!(s.status, SessionStatus::Running);
    assert!(s.archived);
    assert_eq!(s.cost_micro, 13_000);
    assert_eq!(s.head_seq, 10);
    let approvals = t.store.open_approvals(org(&t)).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].kind, ApprovalKind::Policy);
    assert_eq!(approvals[0].requested_seq, 6);
    let usage = t.store.usage_daily(org(&t), "0000", "9999").await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        (usage[0].input_tokens, usage[0].cost_micro, usage[0].events),
        (1010, 13_000, 2)
    );

    // Archivieren ist reversibel.
    assert!(t.store.sessions(org(&t), false).await.unwrap().is_empty());
    append_one(
        &t.store,
        &s,
        ev(s.id, EventPayload::SessionUnarchived(Empty {})),
    )
    .await;
    assert_eq!(t.store.sessions(org(&t), false).await.unwrap().len(), 1);
}

#[tokio::test]
async fn data_005_ac2_cost_delta_is_visible_right_after_append() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let written = append_one(
        &t.store,
        &s,
        ev(
            s.id,
            EventPayload::CostDelta(CostDelta {
                harness: "codex".into(),
                model: "gpt-5".into(),
                output_tokens: 7,
                cost_micro: Some(42),
                ..CostDelta::default()
            }),
        ),
    )
    .await;
    let day = &written.ts.to_string()[..10];
    let usage = t.store.usage_daily(org(&t), day, day).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        (usage[0].harness.as_str(), usage[0].cost_micro),
        ("codex", 42)
    );
    assert_eq!(t.store.session(org(&t), s.id).await.unwrap().cost_micro, 42);
}

#[tokio::test]
async fn data_005_ac1_rebuild_reproduces_projections() {
    let t = store().await;
    let a = session_with_history(&t, "Erste").await;
    session_with_history(&t, "Zweite").await;
    let before = t.store.projection_dump(org(&t)).await;
    assert!(before.len() > 4);

    assert_eq!(t.store.rebuild_projections(org(&t), None).await.unwrap(), 2);
    assert_eq!(t.store.projection_dump(org(&t)).await, before);

    // Auch nach Beschädigung stellt der Rebuild den Stand wieder her.
    sqlx::query("UPDATE sessions SET title = 'kaputt', cost_micro = 99 WHERE org_id = ?")
        .bind(org(&t).to_string())
        .execute(&t.store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM approvals WHERE org_id = ?")
        .bind(org(&t).to_string())
        .execute(&t.store.pool)
        .await
        .unwrap();
    assert_eq!(
        t.store
            .rebuild_projections(org(&t), Some(a.id))
            .await
            .unwrap(),
        1
    );
    assert_ne!(t.store.projection_dump(org(&t)).await, before);
    t.store.rebuild_projections(org(&t), None).await.unwrap();
    assert_eq!(t.store.projection_dump(org(&t)).await, before);
}

// ---------------------------------------------------------------------------
// DATA-006: Blob-Zugriff, GC, Beschädigung
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_006_ac1_blob_only_reachable_through_referencing_session() {
    let t = store().await;
    let own = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let other = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let blob = t
        .store
        .put_session_blob(org(&t), other.id, b"gleicher Inhalt")
        .await
        .unwrap();
    assert_eq!(
        t.store
            .session_blob(org(&t), own.id, &blob)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    // Derselbe Inhalt in der eigenen Session ist nur über diese erreichbar.
    assert_eq!(
        t.store
            .put_session_blob(org(&t), own.id, b"gleicher Inhalt")
            .await
            .unwrap(),
        blob
    );
    assert_eq!(
        t.store.session_blob(org(&t), own.id, &blob).await.unwrap(),
        b"gleicher Inhalt"
    );
}

#[tokio::test]
async fn data_006_ac2_blob_of_deleted_session_is_collected_after_grace() {
    let t = store().await;
    let keep = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let doomed = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let shared = t
        .store
        .put_session_blob(org(&t), keep.id, b"geteilt")
        .await
        .unwrap();
    t.store
        .put_session_blob(org(&t), doomed.id, b"geteilt")
        .await
        .unwrap();
    let only = t
        .store
        .put_session_blob(org(&t), doomed.id, b"nur hier")
        .await
        .unwrap();
    let written = append_one(&t.store, &doomed, big_tool_result(doomed.id, 70)).await;
    let EventBody::Offloaded(o) = written.body else {
        panic!()
    };

    t.store
        .delete_session(
            org(&t),
            doomed.id,
            PrincipalId::User(t.local.user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    // Innerhalb der Frist bleibt alles liegen.
    let report = t.store.gc_blobs(SystemTime::now()).await.unwrap();
    assert_eq!(report.files_deleted, 0);
    assert!(t.store.blob_file_exists(&only));

    let later = SystemTime::now() + Duration::from_secs(25 * 3600);
    let report = t.store.gc_blobs(later).await.unwrap();
    assert_eq!(report.files_deleted, 2, "{report:?}");
    assert!(!t.store.blob_file_exists(&only));
    assert!(!t.store.blob_file_exists(&o.payload_ref));
    assert!(t.store.blob_file_exists(&shared), "noch referenziert");
    assert_eq!(
        t.store
            .session_blob(org(&t), keep.id, &shared)
            .await
            .unwrap(),
        b"geteilt"
    );
}

#[tokio::test]
async fn data_006_ac3_corrupt_blob_reports_blob_corrupt() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let blob = t
        .store
        .put_session_blob(org(&t), s.id, b"original")
        .await
        .unwrap();
    std::fs::write(t.store.blobs.path(&blob), b"bitfehler").unwrap();
    let err = t
        .store
        .session_blob(org(&t), s.id, &blob)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "blob_corrupt");
}

// ---------------------------------------------------------------------------
// DATA-008: Archivieren und Löschen
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_008_ac1_deleted_session_is_gone_and_tombstoned() {
    let t = store().await;
    let root = session_with_history(&t, "Wurzel").await;
    let mut side = new_session(&t);
    side.kind = SessionKind::SideChat;
    side.parent_id = Some(root.id);
    let side = t.store.create_session(org(&t), side).await.unwrap();
    let mut sub = new_session(&t);
    sub.kind = SessionKind::Subagent;
    sub.parent_id = Some(side.id);
    let sub = t.store.create_session(org(&t), sub).await.unwrap();
    let blob = t
        .store
        .put_session_blob(org(&t), root.id, b"anhang")
        .await
        .unwrap();
    let mut raw = notice(root.id, "raw");
    raw.raw = Some(RawJson::from_value(&json!({"a": 1})));
    let raw = append_one(&t.store, &root, raw).await;
    let before = Timestamp::now();

    let tombstones = t
        .store
        .delete_session(
            org(&t),
            root.id,
            PrincipalId::User(t.local.user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    assert_eq!(
        tombstones.len(),
        3,
        "Side-Chat und Sub-Session werden mitgelöscht"
    );

    for id in [root.id, side.id, sub.id] {
        assert_eq!(
            t.store.session(org(&t), id).await.unwrap_err().code(),
            "not_found"
        );
        assert_eq!(
            t.store.events(org(&t), id, 0, 10).await.unwrap_err().code(),
            "not_found"
        );
    }
    assert_eq!(
        t.store
            .session_blob(org(&t), root.id, &blob)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert!(
        t.store
            .event_raw(org(&t), root.id, raw.seq)
            .await
            .unwrap()
            .is_none()
    );
    assert!(t.store.open_approvals(org(&t)).await.unwrap().is_empty());
    assert!(
        t.store
            .usage_daily(org(&t), "0000", "9999")
            .await
            .unwrap()
            .is_empty()
    );

    let since = Timestamp::from(before.as_offset_date_time() - time::Duration::seconds(1));
    let visible = t.store.tombstones_since(org(&t), since).await.unwrap();
    let mut ids: Vec<_> = visible.iter().map(|t| t.id.clone()).collect();
    ids.sort();
    let mut expected = vec![root.id.to_string(), side.id.to_string(), sub.id.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
    let by = PrincipalId::User(UserId::LOCAL);
    assert!(
        visible
            .iter()
            .all(|ts| ts.kind == "session" && ts.deleted_by == by)
    );
}

#[tokio::test]
async fn data_008_ac2_replica_deletes_its_copy_on_tombstone() {
    let home = store().await;
    let replica = store().await;
    let mut new = new_session(&home);
    let s = home
        .store
        .create_session(org(&home), new.clone())
        .await
        .unwrap();
    // Replica hält eine Kopie mit derselben ID (Replikation selbst kommt mit SYNC-002).
    new.home_node = replica.local.node;
    replica
        .store
        .create_session(org(&replica), new.clone())
        .await
        .unwrap();

    let tombstones = home
        .store
        .delete_session(
            org(&home),
            s.id,
            PrincipalId::User(home.local.user),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap();
    for t in &tombstones {
        replica
            .store
            .apply_tombstone(org(&replica), t)
            .await
            .unwrap();
        replica
            .store
            .apply_tombstone(org(&replica), t)
            .await
            .unwrap();
    }
    assert_eq!(
        replica
            .store
            .session(org(&replica), s.id)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    // Erneute Replikation derselben Session wird verhindert.
    let err = replica
        .store
        .create_session(org(&replica), new.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "tombstoned");
    let err = home
        .store
        .create_session(org(&home), new)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "tombstoned");
}

#[tokio::test]
async fn data_008_ac3_only_owner_or_org_admin_may_delete_and_it_is_audited() {
    let t = store().await;
    let s = t
        .store
        .create_session(org(&t), new_session(&t))
        .await
        .unwrap();
    let stranger = UserId::new();
    t.store
        .create_user(org(&t), stranger, "Fremd")
        .await
        .unwrap();

    let err = t
        .store
        .delete_session(
            org(&t),
            s.id,
            PrincipalId::User(stranger),
            DeleteAuthority::Owner,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "forbidden");
    assert!(t.store.session(org(&t), s.id).await.is_ok());
    assert!(t.store.audit_entries(org(&t), 10).await.unwrap().is_empty());

    t.store
        .delete_session(
            org(&t),
            s.id,
            PrincipalId::User(stranger),
            DeleteAuthority::OrgAdmin,
        )
        .await
        .unwrap();
    let audit = t.store.audit_entries(org(&t), 10).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].action, "session.delete");
    assert_eq!(audit[0].actor, PrincipalId::User(stranger));
    assert_eq!(audit[0].target_id, s.id.to_string());
    assert_eq!(audit[0].details["authority"], "org_admin");
}
