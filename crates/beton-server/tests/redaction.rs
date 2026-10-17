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
                max_cost_usd: None,
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

// ---------------------------------------------------------------------------------------
// DATA-012: manuelle Nachredaktion

/// Ein Leak ohne erkennbares Format, das die automatische Redaction nicht kennt.
const LEAK: &str = "hunter2-geheim-007";

async fn post(t: &TestApp, path: &str, body: Value) -> common::Res {
    t.send(
        t.authed("POST", path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

async fn events(t: &TestApp, s: &SessionRecord) -> Vec<Value> {
    let res = get(
        t,
        &format!("/v1/sessions/{}/events?after_seq=0&limit=200", s.id),
    )
    .await;
    assert_eq!(res.status, 200);
    res.json()["items"].as_array().unwrap().clone()
}

/// Session mit einem Leak in Nachricht, `raw` und einer ausgelagerten Tool-Ausgabe.
async fn leaked_session(t: &TestApp) -> SessionRecord {
    let s = session(&t.store).await;
    let mut msg = message(
        s.id,
        MessageRole::Assistant,
        &format!("Mein Passwort ist {LEAK}"),
    );
    msg.raw = Some(RawJson::from_value(&json!({ "text": LEAK })));
    let big = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::ToolCallCompleted(beton_core::event::ToolCallCompleted {
            call_id: "call_1".into(),
            status: beton_core::event::ToolStatus::Ok,
            result: Some(json!({ "stdout": format!("{}{LEAK}", "x".repeat(70_000)) })),
            result_ref: None,
            duration_ms: 5,
        }),
    );
    append(&t.store, &s, vec![msg, big]).await;
    s
}

fn on_disk(t: &TestApp, needle: &str) -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(t.dir.path(), &mut files);
    files
        .into_iter()
        .filter(|p| {
            std::fs::read(p)
                .unwrap_or_default()
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        })
        .map(|p| p.display().to_string())
        .collect()
}

#[tokio::test]
async fn data_012_ac1_redact_paths_replaces_content_for_attach_and_export() {
    let (t, _redactor, _secrets) = app_redacting(Arc::new(MemoryBackend::default())).await;
    let s = leaked_session(&t).await;
    assert!(
        !on_disk(&t, LEAK).is_empty(),
        "Vorbedingung: Leak liegt vor"
    );

    let res = post(
        &t,
        &format!("/v1/sessions/{}/events/2/redact", s.id),
        json!({ "paths": ["/content/0/text"], "reason": "Passwort im Chat" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let body = res.json();
    assert_eq!(body["target"]["seq"], 2);
    assert_eq!(body["event"]["type"], "event.redacted");
    assert_eq!(body["event"]["payload"]["target_seq"], 2);
    let res = post(
        &t,
        &format!("/v1/sessions/{}/events/3/redact", s.id),
        json!({ "reason": "Ausgabe mit Passwort" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));

    // Neuer Attach: redigierte Fassung, `seq`, `type`, `actor`, `ts` und IDs bleiben.
    let after = events(&t, &s).await;
    let msg = &after[1];
    assert_eq!(msg["seq"], 2);
    assert_eq!(msg["type"], "message.completed");
    assert_eq!(msg["payload"]["role"], "assistant");
    assert_eq!(msg["payload"]["content"][0]["text"], "[REDACTED:manual]");
    let tool = &after[2];
    assert_eq!(tool["type"], "tool.call.completed");
    assert_eq!(tool["payload"]["call_id"], "call_1", "IDs bleiben");
    assert_eq!(tool["payload"]["status"], "ok", "Typen bleiben");
    assert!(
        tool.get("payload_ref").is_none(),
        "ausgelagerter Blob ist weg"
    );
    assert_eq!(
        after
            .iter()
            .filter(|e| e["type"] == "event.redacted")
            .count(),
        2
    );
    assert!(
        t.store
            .event_raw(s.org_id, s.id, 2)
            .await
            .unwrap()
            .is_none()
    );

    // Export ebenfalls, und nichts davon liegt noch auf der Platte (DB, WAL, Blobs).
    let res = get(&t, &format!("/v1/sessions/{}/export?with_raw=true", s.id)).await;
    assert!(!String::from_utf8_lossy(&res.body).contains(LEAK));
    assert!(on_disk(&t, LEAK).is_empty(), "{:?}", on_disk(&t, LEAK));
}

#[tokio::test]
async fn data_012_ac1_invalid_requests_are_rejected() {
    let (t, _redactor, _secrets) = app_redacting(Arc::new(MemoryBackend::default())).await;
    let s = leaked_session(&t).await;
    let url = format!("/v1/sessions/{}/events/2/redact", s.id);
    for (body, code) in [
        (
            json!({ "paths": ["/gibt/es/nicht"], "reason": "x" }),
            "validation_failed",
        ),
        (
            json!({ "paths": ["/role"], "reason": "x" }),
            "validation_failed",
        ),
        (
            json!({ "paths": ["content"], "reason": "x" }),
            "validation_failed",
        ),
        (json!({ "reason": "" }), "validation_failed"),
    ] {
        let res = post(&t, &url, body.clone()).await;
        res.assert_problem(code);
    }
    let res = post(
        &t,
        &format!("/v1/sessions/{}/events/99/redact", s.id),
        json!({ "reason": "x" }),
    )
    .await;
    res.assert_problem("not_found");
    // Nichts wurde geändert.
    assert_eq!(
        events(&t, &s).await[1]["payload"]["content"][0]["text"],
        format!("Mein Passwort ist {LEAK}")
    );
}

#[tokio::test]
async fn data_012_ac2_connected_clients_receive_event_redacted() {
    let (t, _redactor, _secrets, hub) =
        common::app_redacting_hub(Arc::new(MemoryBackend::default())).await;
    let s = leaked_session(&t).await;
    let (mut live, _) = hub.subscribe(s.id);
    let res = post(
        &t,
        &format!("/v1/sessions/{}/events/2/redact", s.id),
        json!({ "reason": "Leak" }),
    )
    .await;
    assert_eq!(res.status, 200);
    let e = tokio::time::timeout(std::time::Duration::from_secs(5), live.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.type_name(), "event.redacted");
    let v = serde_json::to_value(&*e).unwrap();
    assert_eq!(v["payload"]["target_seq"], 2);
    assert_eq!(v["payload"]["by"], "usr_local");
    assert_eq!(v["payload"]["reason"], "Leak");
}

#[tokio::test]
async fn data_012_ac3_redaction_is_audited_for_the_owner() {
    let (t, _redactor, _secrets) = app_redacting(Arc::new(MemoryBackend::default())).await;
    let s = leaked_session(&t).await;
    let res = post(
        &t,
        &format!("/v1/sessions/{}/events/2/redact", s.id),
        json!({ "paths": ["/content/0/text"], "reason": "Passwort im Chat" }),
    )
    .await;
    assert_eq!(res.status, 200);
    let entries = t.store.audit_entries(s.org_id, 100).await.unwrap();
    let e = entries
        .iter()
        .find(|e| e.action == "event.redacted")
        .expect("Audit-Eintrag fehlt");
    assert_eq!(e.target_kind, "event");
    assert_eq!(e.target_id, format!("{}/2", s.id));
    assert_eq!(e.session_id, Some(s.id));
    assert_eq!(e.actor.to_string(), "usr_local");
    assert_eq!(e.details["paths"], json!(["/content/0/text"]));
    assert_eq!(e.details["reason"], "Passwort im Chat");
    assert!(!serde_json::to_string(&entries).unwrap().contains(LEAK));
}

/// #216 (SEC-013): Werte durchgereichter Variablen mit Secret-Namen (`*_TOKEN`, `*_KEY`,
/// `*_SECRET`, `*PASSWORD*`) und das lokale Token des Daemons sind bekannte Werte, auch nach
/// einer Rotation des Tokens. Andere durchgereichte Werte bleiben lesbar.
#[tokio::test]
async fn sec_013_passthrough_secrets_and_the_local_token_are_redacted() {
    const ENV: &str = "Fixture-passthrough-4711-abcdef";
    const PLAIN: &str = "Fixture-plain-value-0815-xyz";
    let dir = tempfile::tempdir().unwrap();
    let redactor = Arc::new(beton_secrets::Redactor::pending());
    let store = Store::open(
        dir.path(),
        StoreOptions {
            redactor: redactor.clone(),
            ..StoreOptions::default()
        },
    )
    .await
    .unwrap();
    let mut cfg = beton_server::ServerConfig::local(dir.path().to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let for_runtime = redactor.clone();
    let daemon = beton_server::start_with(cfg, store.clone(), move |mut r| {
        r.redactor = Some(for_runtime);
        r.secret_backend = Arc::new(MemoryBackend::default());
        r.sessions.sandbox = beton_sandbox::SandboxSpec::from_yaml(
            "env:\n  passthrough: [GITHUB_TOKEN, PLAIN_VAR, DB_PASSWORD_FILE]\n",
        )
        .unwrap();
        r.sessions.daemon_env = Arc::new(std::collections::BTreeMap::from([
            ("GITHUB_TOKEN".to_owned(), ENV.to_owned()),
            ("PLAIN_VAR".to_owned(), PLAIN.to_owned()),
            (
                "DB_PASSWORD_FILE".to_owned(),
                "Fixture-pw-path-value".to_owned(),
            ),
        ]));
        r
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(dir.path().join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    let s = session(&store).await;
    let text = format!("{ENV} | {PLAIN} | Fixture-pw-path-value | {token}");
    append(
        &store,
        &s,
        vec![message(s.id, MessageRole::Assistant, &text)],
    )
    .await;
    let stored = store.events(s.org_id, s.id, 1, 10).await.unwrap();
    let got = serde_json::to_string(&stored[0]).unwrap();
    assert!(!got.contains(ENV), "{got}");
    assert!(!got.contains("Fixture-pw-path-value"), "{got}");
    assert!(!got.contains(&token), "{got}");
    assert!(
        got.contains(PLAIN),
        "kein Secret-Name: bleibt lesbar: {got}"
    );
    assert!(got.contains("[REDACTED:beton]"), "{got}");
    // Das Audit nennt die Namen, nie die Werte.
    let audit = store.audit_entries(s.org_id, 100).await.unwrap();
    let loaded = audit
        .iter()
        .find(|e| e.details["reason"] == "env_passthrough")
        .expect("Audit der durchgereichten Werte");
    assert_eq!(
        loaded.details["names"],
        json!(["DB_PASSWORD_FILE", "GITHUB_TOKEN"])
    );
    assert!(!loaded.details.to_string().contains("Fixture"));
    // Rotation (`beton auth rotate-local` in einem anderen Prozess): das neue Token zählt.
    daemon.token.rotate().unwrap();
    let rotated = std::fs::read_to_string(dir.path().join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    assert!(daemon.token.verify(&rotated));
    append(
        &store,
        &s,
        vec![message(s.id, MessageRole::Assistant, &rotated)],
    )
    .await;
    let stored = store.events(s.org_id, s.id, 2, 10).await.unwrap();
    assert!(
        !serde_json::to_string(&stored[0])
            .unwrap()
            .contains(&rotated)
    );
    daemon.shutdown().await;
}
