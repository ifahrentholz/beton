//! Tests des hash-verketteten Audit-Logs (SEC-012).

use std::time::Duration;

use beton_core::id::{PrincipalId, SessionId};
use beton_core::time::Timestamp;
use serde_json::json;

use crate::audit::{AUDIT_USAGE_WINDOW, AuditQuery, NewAuditEntry, UsageKey};
use crate::testutil::{TestStore, org, store};

fn at(base: Timestamp, offset: Duration) -> Timestamp {
    Timestamp::from(base.as_offset_date_time() + offset)
}

fn entry(t: &TestStore, action: &str, n: u32) -> NewAuditEntry {
    NewAuditEntry {
        at: Timestamp::now(),
        actor: PrincipalId::User(t.local.user),
        action: action.into(),
        target_kind: "secret".into(),
        target_id: format!("secret://user/openai/k{n}"),
        session_id: None,
        host_id: None,
        details: json!({ "n": n }),
    }
}

async fn append_n(t: &TestStore, n: u32) {
    for i in 1..=n {
        t.store
            .append_audit(org(t), entry(t, "secret.created", i))
            .await
            .unwrap();
    }
}

async fn raw(t: &TestStore, sql: &str) {
    sqlx::query(sql).execute(&t.store.pool).await.unwrap();
}

#[tokio::test]
async fn sec_012_entries_form_a_hash_chain() {
    let t = store().await;
    append_n(&t, 3).await;
    let page = t
        .store
        .audit_page(org(&t), &AuditQuery::default())
        .await
        .unwrap();
    // Neueste zuerst.
    let seqs: Vec<u64> = page.items.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![3, 2, 1]);
    let oldest = &page.items[2];
    assert_eq!(oldest.prev_hash, "0".repeat(64));
    assert_eq!(page.items[1].prev_hash, oldest.hash);
    assert_eq!(page.items[0].prev_hash, page.items[1].hash);
    let report = t.store.verify_audit(org(&t)).await.unwrap();
    assert_eq!(report.entries, 3);
    assert_eq!(report.first_invalid, None, "{report:?}");
}

#[tokio::test]
async fn sec_012_ac1_verify_names_first_modified_row() {
    let t = store().await;
    append_n(&t, 4).await;
    raw(
        &t,
        "UPDATE audit_entries SET details = '{\"n\":99}' WHERE seq = 2",
    )
    .await;
    let report = t.store.verify_audit(org(&t)).await.unwrap();
    assert_eq!(report.first_invalid, Some(2), "{report:?}");
    assert!(report.reason.is_some());

    // Auch eine geänderte Aktion oder ein geänderter Akteur fällt auf.
    let t = store().await;
    append_n(&t, 3).await;
    raw(
        &t,
        "UPDATE audit_entries SET action = 'secret.harmless' WHERE seq = 3",
    )
    .await;
    assert_eq!(
        t.store.verify_audit(org(&t)).await.unwrap().first_invalid,
        Some(3)
    );
}

#[tokio::test]
async fn sec_012_ac1_verify_names_first_deleted_row() {
    let t = store().await;
    append_n(&t, 4).await;
    raw(&t, "DELETE FROM audit_entries WHERE seq = 2").await;
    let report = t.store.verify_audit(org(&t)).await.unwrap();
    assert_eq!(report.first_invalid, Some(2), "{report:?}");

    // Das Löschen des letzten Eintrags erkennt der Kopf der Kette.
    let t = store().await;
    append_n(&t, 3).await;
    raw(&t, "DELETE FROM audit_entries WHERE seq = 3").await;
    assert_eq!(
        t.store.verify_audit(org(&t)).await.unwrap().first_invalid,
        Some(3)
    );

    // Ein komplett geleertes Log ebenso.
    let t = store().await;
    append_n(&t, 2).await;
    raw(&t, "DELETE FROM audit_entries").await;
    assert_eq!(
        t.store.verify_audit(org(&t)).await.unwrap().first_invalid,
        Some(1)
    );
}

#[tokio::test]
async fn sec_012_ac3_thousand_uses_within_five_minutes_make_one_entry() {
    let t = store().await;
    let session = SessionId::new();
    let key = UsageKey {
        secret_ref: "secret://user/openai/default".into(),
        session_id: Some(session),
        host_id: "hst_local".into(),
        actor: PrincipalId::User(t.local.user),
        purpose: "proxy".into(),
    };
    let start = Timestamp::now();
    for i in 0..1000u64 {
        let when = at(start, Duration::from_millis(i * 250)); // 250 s insgesamt
        t.store
            .record_audit_usage(org(&t), &key, when)
            .await
            .unwrap();
    }
    // Solange das Fenster offen ist, gibt es keinen Eintrag.
    let open = t
        .store
        .seal_audit_windows(org(&t), at(start, Duration::from_secs(299)))
        .await
        .unwrap();
    assert_eq!(open, 0);
    let sealed = t
        .store
        .seal_audit_windows(org(&t), at(start, AUDIT_USAGE_WINDOW))
        .await
        .unwrap();
    assert_eq!(sealed, 1);
    let items = t
        .store
        .audit_page(org(&t), &AuditQuery::default())
        .await
        .unwrap()
        .items;
    assert_eq!(items.len(), 1, "{items:?}");
    let e = &items[0];
    assert_eq!(e.action, "secret.used");
    assert_eq!(e.count, 1000);
    assert_eq!(e.target_id, "secret://user/openai/default");
    assert_eq!(e.session_id, Some(session));
    assert_eq!(e.host_id.as_deref(), Some("hst_local"));
    assert_eq!(e.details["purpose"], "proxy");

    // Eine Nutzung nach dem Fenster beginnt ein neues; das alte bleibt unverändert.
    t.store
        .record_audit_usage(org(&t), &key, at(start, Duration::from_secs(400)))
        .await
        .unwrap();
    t.store
        .seal_audit_windows(org(&t), at(start, Duration::from_secs(1000)))
        .await
        .unwrap();
    let items = t
        .store
        .audit_page(org(&t), &AuditQuery::default())
        .await
        .unwrap()
        .items;
    assert_eq!(
        items.iter().map(|e| e.count).collect::<Vec<_>>(),
        vec![1, 1000]
    );
    assert_eq!(
        t.store.verify_audit(org(&t)).await.unwrap().first_invalid,
        None
    );
}

#[tokio::test]
async fn sec_012_usage_per_session_and_host_is_counted_separately() {
    let t = store().await;
    let key = |host: &str| UsageKey {
        secret_ref: "secret://user/openai/default".into(),
        session_id: None,
        host_id: host.into(),
        actor: PrincipalId::User(t.local.user),
        purpose: "proxy".into(),
    };
    let start = Timestamp::now();
    for host in ["a", "b", "a"] {
        t.store
            .record_audit_usage(org(&t), &key(host), start)
            .await
            .unwrap();
    }
    assert_eq!(
        t.store
            .seal_audit_windows(org(&t), at(start, AUDIT_USAGE_WINDOW))
            .await
            .unwrap(),
        2
    );
    let mut counts: Vec<(Option<String>, u64)> = t
        .store
        .audit_page(org(&t), &AuditQuery::default())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|e| (e.host_id, e.count))
        .collect();
    counts.sort();
    assert_eq!(counts, vec![(Some("a".into()), 2), (Some("b".into()), 1)]);
}

#[tokio::test]
async fn sec_012_page_filters_by_action_and_pages_by_seq() {
    let t = store().await;
    append_n(&t, 5).await;
    t.store
        .append_audit(org(&t), entry(&t, "secret.deleted", 6))
        .await
        .unwrap();
    let first = t
        .store
        .audit_page(
            org(&t),
            &AuditQuery {
                limit: 2,
                ..AuditQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        first.items.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![6, 5]
    );
    assert_eq!(first.next_before, Some(5));
    let second = t
        .store
        .audit_page(
            org(&t),
            &AuditQuery {
                limit: 2,
                before_seq: first.next_before,
                ..AuditQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        second.items.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![4, 3]
    );
    let deleted = t
        .store
        .audit_page(
            org(&t),
            &AuditQuery {
                action: Some("secret.deleted".into()),
                ..AuditQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(deleted.items.len(), 1);
    assert_eq!(deleted.next_before, None);
}

#[tokio::test]
async fn sec_012_entries_from_before_the_chain_are_chained_on_open() {
    let t = store().await;
    // Ein Eintrag wie vor Migration 0010 (DATA-008): ohne seq und Hash.
    raw(
        &t,
        &format!(
            "INSERT INTO audit_entries (id, org_id, at, actor_id, action, target_kind, \
             target_id, details) VALUES ('old1', '{}', '2026-01-01T00:00:00Z', '{}', \
             'session.delete', 'session', 'ses_x', '{{}}')",
            org(&t),
            t.local.user
        ),
    )
    .await;
    t.store.close().await;
    let store = crate::Store::open(t.dir.path(), crate::StoreOptions::default())
        .await
        .unwrap();
    let report = store.verify_audit(org(&t)).await.unwrap();
    assert_eq!(report.entries, 1);
    assert_eq!(report.first_invalid, None, "{report:?}");
    // Neue Einträge hängen sich hinten an.
    store
        .append_audit(org(&t), entry(&t, "secret.created", 1))
        .await
        .unwrap();
    let report = store.verify_audit(org(&t)).await.unwrap();
    assert_eq!((report.entries, report.first_invalid), (2, None));
}

#[tokio::test]
async fn sec_012_concurrent_appends_keep_the_chain_intact() {
    let t = store().await;
    let mut tasks = Vec::new();
    for i in 0..20 {
        let s = t.store.clone();
        let e = entry(&t, "secret.created", i);
        let o = org(&t);
        tasks.push(tokio::spawn(async move { s.append_audit(o, e).await }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let report = t.store.verify_audit(org(&t)).await.unwrap();
    assert_eq!((report.entries, report.first_invalid), (20, None));
}
