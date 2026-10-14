//! Redaction über die API (SEC-013 AC2, SES-009 AC2) mit der Pipeline aus `beton-secrets`.
//! Test-Secrets sind Fixtures; das Speicher-Backend steht für Keychain bzw. Datei.

#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use beton_core::event::{
    Actor, Event, EventPayload, MessageCompleted, MessageRole, RawJson, SessionKind, SessionTrigger,
};
use beton_core::id::SessionId;
use beton_secrets::backend::MemoryBackend;
use beton_secrets::{NewSecret, SecretKind, SecretRef, SecretValue};
use beton_store::{NewSession, SessionRecord, Store, StoreOptions};
use serde_json::{Value, json};

use common::{TestApp, app_redacting};

/// Bekannter Wert ohne erkennbares Token-Format.
const KNOWN: &str = "Zx9-known-SECRET-value-4711";

const PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACDw7z6dZSb0L2Q9P3k8m3m8x8Y5Q6m0yHq1aW5mb3JtYXRpb24AAAAA\n-----END OPENSSH PRIVATE KEY-----";

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "fake".into(),
        agent_ref: None,
    }
}

fn message(session: SessionId, role: MessageRole, text: &str) -> Event {
    Event::new(
        session,
        0,
        agent(),
        EventPayload::MessageCompleted(MessageCompleted {
            message_id: format!("m_{}", SessionId::new()),
            role,
            content: vec![json!({ "type": "text", "text": text })],
            author: None,
        }),
    )
}

async fn session(store: &Store) -> SessionRecord {
    let local = store.ensure_local().await.unwrap();
    store
        .create_session(
            local.org,
            NewSession {
                id: SessionId::new(),
                owner: local.user,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/tmp/redaction".into(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: Value::Null,
            },
        )
        .await
        .unwrap()
}

async fn append(store: &Store, s: &SessionRecord, events: Vec<Event>) {
    let head = store.session(s.org_id, s.id).await.unwrap().head_seq;
    store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

async fn get(t: &TestApp, path: &str) -> common::Res {
    t.send(t.authed("GET", path).body(Body::empty()).unwrap())
        .await
}

#[tokio::test]
async fn sec_013_ac2_pem_private_key_in_agent_message_is_replaced_completely() {
    let (t, _redactor, _secrets) = app_redacting(Arc::new(MemoryBackend::default())).await;
    let s = session(&t.store).await;
    append(
        &t.store,
        &s,
        vec![message(
            s.id,
            MessageRole::Assistant,
            &format!("Hier der Schlüssel:\n{PEM}\nBitte sicher ablegen."),
        )],
    )
    .await;
    let res = get(&t, &format!("/v1/sessions/{}/events?after_seq=1", s.id)).await;
    assert_eq!(res.status, 200);
    let text = res.json()["items"][0]["payload"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        text,
        "Hier der Schlüssel:\n[REDACTED:private_key]\nBitte sicher ablegen."
    );
    for part in ["BEGIN", "END", "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQ"] {
        assert!(!text.contains(part), "{part} steht noch in {text}");
    }
}

#[tokio::test]
async fn ses_009_ac2_export_contains_no_known_test_secrets() {
    let (t, _redactor, secrets) = app_redacting(Arc::new(MemoryBackend::default())).await;
    // Präpariertes Log (ohne Redaction geschrieben), bevor der Wert als Secret bekannt war.
    let raw_store = Store::open(t.dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let s = session(&raw_store).await;
    let mut leaked = message(
        s.id,
        MessageRole::Assistant,
        &format!("Passwort ist {KNOWN}"),
    );
    leaked.raw = Some(RawJson::from_value(&json!({ "text": KNOWN })));
    append(&raw_store, &s, vec![leaked]).await;
    secrets
        .put(NewSecret {
            secret_ref: SecretRef::user("custom:db", "default", None).unwrap(),
            kind: SecretKind::Token,
            value: SecretValue::from(KNOWN),
        })
        .await
        .unwrap();

    for query in ["", "?with_raw=true"] {
        let res = get(&t, &format!("/v1/sessions/{}/export{query}", s.id)).await;
        assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
        let text = String::from_utf8_lossy(&res.body);
        assert!(!text.contains(KNOWN), "Export{query}: {text}");
        assert!(text.contains("[REDACTED:secret]"), "Export{query}: {text}");
    }
}
