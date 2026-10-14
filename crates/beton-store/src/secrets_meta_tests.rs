//! Tests der Secret-Metadaten (SEC-001): Metadaten und Audit-Eintrag entstehen gemeinsam.

use beton_core::id::PrincipalId;
use beton_core::time::Timestamp;
use serde_json::json;

use crate::audit::NewAuditEntry;
use crate::secrets_meta::SecretMetaRecord;
use crate::testutil::{TestStore, org, store};

fn record(r: &str) -> SecretMetaRecord {
    let now = Timestamp::now();
    SecretMetaRecord {
        secret_ref: r.into(),
        scope: "user".into(),
        provider: "openai".into(),
        account: "default".into(),
        name: String::new(),
        kind: "token".into(),
        backend: "file".into(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
    }
}

fn audit(t: &TestStore, action: &str, r: &str) -> NewAuditEntry {
    NewAuditEntry {
        at: Timestamp::now(),
        actor: PrincipalId::User(t.local.user),
        action: action.into(),
        target_kind: "secret".into(),
        target_id: r.into(),
        session_id: None,
        host_id: None,
        details: json!({}),
    }
}

#[tokio::test]
async fn sec_001_metadata_round_trip_with_audit() {
    let t = store().await;
    let r = "secret://user/openai/default";
    t.store
        .insert_secret_meta(org(&t), &record(r), audit(&t, "secret.created", r))
        .await
        .unwrap();
    let err = t
        .store
        .insert_secret_meta(org(&t), &record(r), audit(&t, "secret.created", r))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "conflict");
    let list = t.store.list_secret_meta(org(&t)).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].secret_ref, r);
    assert_eq!(list[0].last_used_at, None);

    let used = Timestamp::now();
    t.store.touch_secret_used(org(&t), r, used).await.unwrap();
    let rotated = Timestamp::now();
    t.store
        .rotate_secret_meta(org(&t), r, rotated, audit(&t, "secret.rotated", r))
        .await
        .unwrap();
    let m = t.store.secret_meta(org(&t), r).await.unwrap().unwrap();
    assert_eq!(m.last_used_at, Some(used));
    assert_eq!(m.updated_at, rotated);

    assert!(
        t.store
            .delete_secret_meta(org(&t), r, audit(&t, "secret.deleted", r))
            .await
            .unwrap()
    );
    assert!(t.store.secret_meta(org(&t), r).await.unwrap().is_none());
    assert!(
        !t.store
            .delete_secret_meta(org(&t), r, audit(&t, "secret.deleted", r))
            .await
            .unwrap(),
        "zweites Löschen findet nichts und auditiert nichts"
    );
    let actions: Vec<String> = t
        .store
        .audit_entries(org(&t), 10)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert_eq!(
        actions,
        vec!["secret.deleted", "secret.rotated", "secret.created"]
    );
    let err = t
        .store
        .rotate_secret_meta(org(&t), r, rotated, audit(&t, "secret.rotated", r))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "not_found");
}
