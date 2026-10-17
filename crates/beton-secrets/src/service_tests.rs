//! Tests des Secret-Dienstes (SEC-001, SEC-004, SEC-012 AC2).

use std::sync::Arc;

use beton_core::id::SessionId;
use beton_core::time::Timestamp;
use beton_store::{AUDIT_USAGE_WINDOW, AuditQuery, Store, StoreOptions};

use crate::backend::{Backend, FileBackend, KdfParams, MemoryBackend};
use crate::value::drop_hook;
use crate::{NewSecret, SecretKind, SecretRef, SecretValue, Secrets, UseContext};

const MARKER: &str = "MARKER-sk-proj-9f8e7d6c5b4a3210";

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    secrets: Secrets,
    caps: crate::UseCapabilities,
}

async fn fixture_with(backend: Arc<dyn Backend>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let (secrets, caps) = Secrets::new(store.clone(), local, backend);
    Fixture {
        dir,
        store,
        secrets,
        caps,
    }
}

async fn fixture() -> Fixture {
    fixture_with(Arc::new(MemoryBackend::default())).await
}

fn openai() -> SecretRef {
    SecretRef::user("openai", "default", None).unwrap()
}

fn token(r: &SecretRef, value: &str) -> NewSecret {
    NewSecret {
        secret_ref: r.clone(),
        kind: SecretKind::Token,
        value: SecretValue::from(value),
    }
}

fn ctx(session: Option<SessionId>) -> UseContext {
    UseContext {
        session_id: session,
        host_id: "hst_local".into(),
    }
}

async fn audit(f: &Fixture) -> Vec<beton_store::AuditEntry> {
    let org = f.secrets.org();
    f.store
        .audit_page(
            org,
            &AuditQuery {
                limit: 1000,
                ..AuditQuery::default()
            },
        )
        .await
        .unwrap()
        .items
}

#[tokio::test]
async fn sec_001_ac1_list_meta_never_returns_values() {
    let f = fixture().await;
    f.secrets.put(token(&openai(), MARKER)).await.unwrap();
    let list = f.secrets.list_meta().await.unwrap();
    assert_eq!(list.len(), 1);
    let m = &list[0];
    assert_eq!(m.secret_ref, "secret://user/openai/default");
    assert_eq!(m.kind, SecretKind::Token);
    assert_eq!(m.provider, "openai");
    assert_eq!(m.account, "default");
    assert_eq!(m.last_used_at, None);
    let json = serde_json::to_string(&list).unwrap();
    assert!(!json.contains(MARKER), "{json}");
    assert!(!json.contains("\"value\""), "{json}");
    assert!(!format!("{list:?}").contains(MARKER));
}

#[tokio::test]
async fn sec_001_ac3_every_use_is_audited_with_purpose_session_and_host() {
    let f = fixture().await;
    f.secrets.put(token(&openai(), "wert")).await.unwrap();
    let session = SessionId::new();

    let v = f
        .caps
        .direct_api
        .get_for_use(&f.secrets, &openai(), ctx(Some(session)))
        .await
        .unwrap();
    assert_eq!(v.expose(), b"wert");
    f.caps
        .git_provider
        .get_for_use(&f.secrets, &openai(), ctx(None))
        .await
        .unwrap();
    let entries = audit(&f).await;
    let used: Vec<_> = entries
        .iter()
        .filter(|e| e.action == "secret.used")
        .collect();
    assert_eq!(used.len(), 2, "{entries:?}");
    let direct = used
        .iter()
        .find(|e| e.details["purpose"] == "direct_api")
        .unwrap();
    assert_eq!(direct.session_id, Some(session));
    assert_eq!(direct.host_id.as_deref(), Some("hst_local"));
    assert_eq!(direct.target_id, "secret://user/openai/default");
    assert!(used.iter().any(|e| e.details["purpose"] == "git_provider"));

    // Proxy-Nutzungen werden in 5-min-Fenstern gezählt und dann als ein Eintrag versiegelt.
    for _ in 0..3 {
        f.caps
            .proxy
            .get_for_use(&f.secrets, &openai(), ctx(Some(session)))
            .await
            .unwrap();
    }
    let later = Timestamp::from(Timestamp::now().as_offset_date_time() + AUDIT_USAGE_WINDOW);
    f.secrets.seal_usage(later).await.unwrap();
    let proxy: Vec<_> = audit(&f)
        .await
        .into_iter()
        .filter(|e| e.details["purpose"] == "proxy")
        .collect();
    assert_eq!(proxy.len(), 1);
    assert_eq!(proxy[0].count, 3);
    assert_eq!(proxy[0].session_id, Some(session));
    assert_eq!(proxy[0].host_id.as_deref(), Some("hst_local"));

    // „Zuletzt genutzt“ ist gesetzt (SEC-004 AC1).
    let meta = f.secrets.list_meta().await.unwrap();
    assert!(meta[0].last_used_at.is_some());
}

#[tokio::test]
async fn sec_001_ac4_values_are_zeroed_after_use() {
    let f = fixture().await;
    f.secrets
        .put(token(&openai(), "zu-nullender-wert"))
        .await
        .unwrap();
    drop_hook::reset();
    let v = f
        .caps
        .direct_api
        .get_for_use(&f.secrets, &openai(), ctx(None))
        .await
        .unwrap();
    assert_eq!(v.len(), 17);
    drop(v);
    let seen = drop_hook::take();
    assert!(
        seen.iter().any(|d| d.len == 17),
        "der ausgegebene Wert wurde beobachtet: {seen:?}"
    );
    assert!(
        seen.iter().all(|d| d.all_zero),
        "jede Kopie wird genullt: {seen:?}"
    );
}

#[tokio::test]
async fn sec_001_use_fails_closed() {
    // Gesperrter Datei-Store: kein Wert, Code `secrets_locked`.
    let dir = tempfile::tempdir().unwrap();
    let file = Arc::new(FileBackend::new(
        dir.path().join("store.v1"),
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    ));
    file.unlock(&SecretValue::from("pass")).unwrap();
    let f = fixture_with(file.clone()).await;
    f.secrets.put(token(&openai(), "wert")).await.unwrap();
    let locked = Arc::new(FileBackend::new(
        dir.path().join("store.v1"),
        KdfParams::default(),
    ));
    let local = f.store.ensure_local().await.unwrap();
    let (secrets, caps) = Secrets::new(f.store.clone(), local, locked);
    let err = caps
        .proxy
        .get_for_use(&secrets, &openai(), ctx(None))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "secrets_locked");

    // Eine Ref ohne Metadaten wird nicht ausgeliefert, auch wenn das Backend einen Wert hat.
    let stray = SecretRef::user("npm", "default", None).unwrap();
    file.put(&stray, &SecretValue::from("wert")).unwrap();
    let err = f
        .caps
        .proxy
        .get_for_use(&f.secrets, &stray, ctx(None))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "not_found");
    assert!(
        audit(&f).await.iter().all(|e| e.action != "secret.used"),
        "fehlgeschlagene Abrufe gelten nicht als Nutzung"
    );
}

#[tokio::test]
async fn sec_004_create_rotate_delete_are_audited() {
    let f = fixture().await;
    let meta = f.secrets.put(token(&openai(), "eins")).await.unwrap();
    assert_eq!(meta.backend, "memory");
    let err = f.secrets.put(token(&openai(), "zwei")).await.unwrap_err();
    assert_eq!(err.code(), "conflict");
    // Ein abgelehntes Anlegen überschreibt den Wert nicht.
    assert_eq!(
        f.caps
            .direct_api
            .get_for_use(&f.secrets, &openai(), ctx(None))
            .await
            .unwrap()
            .expose(),
        b"eins"
    );
    f.secrets
        .rotate(&openai(), SecretValue::from("drei"))
        .await
        .unwrap();
    assert_eq!(
        f.caps
            .direct_api
            .get_for_use(&f.secrets, &openai(), ctx(None))
            .await
            .unwrap()
            .expose(),
        b"drei"
    );
    let missing = SecretRef::user("npm", "default", None).unwrap();
    assert_eq!(
        f.secrets
            .rotate(&missing, SecretValue::from("x"))
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    f.secrets.delete(&openai()).await.unwrap();
    assert!(f.secrets.list_meta().await.unwrap().is_empty());
    assert_eq!(
        f.secrets.delete(&openai()).await.unwrap_err().code(),
        "not_found"
    );
    let actions: Vec<String> = audit(&f)
        .await
        .into_iter()
        .rev()
        .map(|e| e.action)
        .filter(|a| a != "secret.used")
        .collect();
    assert_eq!(
        actions,
        vec!["secret.created", "secret.rotated", "secret.deleted"]
    );
}

#[tokio::test]
async fn sec_004_values_are_validated_per_kind_without_echo() {
    let f = fixture().await;
    let basic = |v: &str| NewSecret {
        secret_ref: SecretRef::user("npm", "default", None).unwrap(),
        kind: SecretKind::Basic,
        value: SecretValue::from(v),
    };
    let err = f.secrets.put(basic(MARKER)).await.unwrap_err();
    assert_eq!(err.code(), "validation_failed");
    assert!(!err.to_string().contains(MARKER), "{err}");
    f.secrets
        .put(basic(r#"{"username":"u","password":"p"}"#))
        .await
        .unwrap();
    let oauth = NewSecret {
        secret_ref: SecretRef::user("gitlab.com", "me", None).unwrap(),
        kind: SecretKind::Oauth,
        value: SecretValue::from(r#"{"refresh_token":"r"}"#),
    };
    assert_eq!(
        f.secrets.put(oauth).await.unwrap_err().code(),
        "validation_failed"
    );
    let empty = token(&openai(), "");
    assert_eq!(
        f.secrets.put(empty).await.unwrap_err().code(),
        "validation_failed"
    );
}

#[tokio::test]
async fn sec_012_ac2_marked_value_appears_in_no_audit_entry() {
    let f = fixture().await;
    let r = openai();
    let session = SessionId::new();
    // Alle Pfade mit dem markierten Wert, auch fehlschlagende.
    f.secrets.put(token(&r, MARKER)).await.unwrap();
    let mut errors = vec![
        f.secrets.put(token(&r, MARKER)).await.unwrap_err(),
        f.secrets
            .put(NewSecret {
                secret_ref: SecretRef::user("npm", "x", None).unwrap(),
                kind: SecretKind::Basic,
                value: SecretValue::from(MARKER),
            })
            .await
            .unwrap_err(),
        f.secrets
            .rotate(
                &SecretRef::user("npm", "y", None).unwrap(),
                SecretValue::from(MARKER),
            )
            .await
            .unwrap_err(),
    ];
    f.secrets
        .rotate(&r, SecretValue::from(format!("{MARKER}-2")))
        .await
        .unwrap();
    for _ in 0..3 {
        f.caps
            .proxy
            .get_for_use(&f.secrets, &r, ctx(Some(session)))
            .await
            .unwrap();
    }
    f.caps
        .direct_api
        .get_for_use(&f.secrets, &r, ctx(Some(session)))
        .await
        .unwrap();
    let later = Timestamp::from(Timestamp::now().as_offset_date_time() + AUDIT_USAGE_WINDOW);
    f.secrets.seal_usage(later).await.unwrap();
    f.secrets.delete(&r).await.unwrap();
    errors.push(f.secrets.delete(&r).await.unwrap_err());
    errors.push(
        f.caps
            .proxy
            .get_for_use(&f.secrets, &r, ctx(None))
            .await
            .unwrap_err(),
    );

    let entries = audit(&f).await;
    assert!(entries.len() >= 5, "{entries:?}");
    for e in &entries {
        let text = serde_json::to_string(e).unwrap();
        assert!(!text.contains("MARKER"), "Wert im Audit-Eintrag: {text}");
    }
    for e in &errors {
        assert!(
            !e.to_string().contains("MARKER"),
            "Wert in Fehlermeldung: {e}"
        );
        assert!(!format!("{e:?}").contains("MARKER"), "Wert in Debug: {e:?}");
    }
    // Auch roh in der Datenbank (inkl. WAL) steht der Wert nirgends.
    f.store.close().await;
    for entry in std::fs::read_dir(f.dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes.windows(6).any(|w| w == b"MARKER"),
                "Wert in {}",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Redaction-Capability (SEC-013, #186)

/// Ein Wert ohne erkennbares Token-Format: nur der Matcher der bekannten Werte findet ihn.
const KNOWN: &str = "Zx9-known-SECRET-value-4711";

/// Nimmt die Capabilities aus der Fixture (`attach` verbraucht die Redaction-Capability).
fn take_caps(f: &mut Fixture) -> crate::UseCapabilities {
    std::mem::replace(&mut f.caps, crate::UseCapabilities::new())
}

/// Die Einträge der Matcher-Aufbauten, älteste zuerst.
fn redaction_entries(entries: &[beton_store::AuditEntry]) -> Vec<&beton_store::AuditEntry> {
    let mut out: Vec<_> = entries
        .iter()
        .filter(|e| e.action == "secrets.redaction_loaded")
        .collect();
    out.sort_by_key(|e| e.seq);
    out
}

#[tokio::test]
async fn sec_013_redaction_cap_loads_known_values_as_matcher() {
    let f = fixture().await;
    f.secrets.put(token(&openai(), KNOWN)).await.unwrap();
    let redactor = Arc::new(crate::Redactor::pending());
    assert!(
        redactor.redact_str(KNOWN).is_err(),
        "vor dem Laden fail closed"
    );
    f.caps
        .redaction
        .attach(&f.secrets, redactor.clone())
        .await
        .unwrap();
    assert_eq!(
        redactor
            .redact_str(&format!("Fehler mit {KNOWN} im Text"))
            .unwrap(),
        "Fehler mit [REDACTED:secret] im Text"
    );
    assert_eq!(
        redactor.status(),
        crate::RedactionStatus::Ready {
            values: 1,
            source: crate::redact::KnownSource::Store
        }
    );
}

#[tokio::test]
async fn sec_001_redaction_cap_audits_each_build_once_not_per_match() {
    let mut f = fixture().await;
    f.secrets.put(token(&openai(), KNOWN)).await.unwrap();
    let redactor = Arc::new(crate::Redactor::pending());
    take_caps(&mut f)
        .redaction
        .attach(&f.secrets, redactor.clone())
        .await
        .unwrap();
    for _ in 0..100 {
        redactor.redact_str(KNOWN).unwrap();
    }
    let entries = audit(&f).await;
    let loaded = redaction_entries(&entries);
    assert_eq!(loaded.len(), 1, "ein Eintrag je Aufbau, keiner je Treffer");
    assert_eq!(loaded[0].target_kind, "secret_store");
    assert_eq!(loaded[0].details["reason"], "attach");
    assert_eq!(loaded[0].details["values"], 1);
    assert!(
        entries.iter().all(|e| e.action != "secret.used"),
        "der Matcher ist keine Nutzung eines einzelnen Secrets"
    );
    let all = serde_json::to_string(&entries).unwrap();
    assert!(!all.contains(KNOWN), "kein Wert im Audit-Log");
}

#[tokio::test]
async fn sec_013_matcher_follows_create_rotate_and_delete() {
    let mut f = fixture().await;
    let redactor = Arc::new(crate::Redactor::pending());
    take_caps(&mut f)
        .redaction
        .attach(&f.secrets, redactor.clone())
        .await
        .unwrap();
    assert_eq!(redactor.redact_str(KNOWN).unwrap(), KNOWN);

    f.secrets.put(token(&openai(), KNOWN)).await.unwrap();
    assert_eq!(redactor.redact_str(KNOWN).unwrap(), "[REDACTED:secret]");

    let rotated = "Rotated-known-SECRET-0815";
    f.secrets
        .rotate(&openai(), SecretValue::from(rotated))
        .await
        .unwrap();
    assert_eq!(redactor.redact_str(rotated).unwrap(), "[REDACTED:secret]");

    f.secrets.delete(&openai()).await.unwrap();
    let reasons: Vec<_> = redaction_entries(&audit(&f).await)
        .iter()
        .map(|e| e.details["reason"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(reasons, ["attach", "created", "rotated", "deleted"]);
    assert_eq!(
        redactor.status(),
        crate::RedactionStatus::Ready {
            values: 0,
            source: crate::redact::KnownSource::Store
        }
    );
}

#[tokio::test]
async fn sec_013_locked_file_store_keeps_redaction_ready_and_loads_after_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let params = KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    let open = Arc::new(FileBackend::new(dir.path().join("store.v1"), params));
    open.unlock(&SecretValue::from("pass")).unwrap();
    let f = fixture_with(open).await;
    f.secrets.put(token(&openai(), KNOWN)).await.unwrap();

    // Neuer Dienst über denselben, noch gesperrten Store (wie nach einem Neustart).
    let locked = Arc::new(FileBackend::new(dir.path().join("store.v1"), params));
    let local = f.store.ensure_local().await.unwrap();
    let (secrets, caps) = Secrets::new(f.store.clone(), local, locked);
    let redactor = Arc::new(crate::Redactor::pending());
    caps.redaction
        .attach(&secrets, redactor.clone())
        .await
        .unwrap();
    assert_eq!(
        redactor.status(),
        crate::RedactionStatus::Ready {
            values: 0,
            source: crate::redact::KnownSource::StoreUnavailable
        },
        "gesperrt: bereit mit den zuletzt bekannten Werten (keine), Store gibt auch nichts heraus"
    );
    secrets.unlock(SecretValue::from("pass")).await.unwrap();
    assert_eq!(redactor.redact_str(KNOWN).unwrap(), "[REDACTED:secret]");
    let entries = audit(&f).await;
    let loaded = redaction_entries(&entries);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].details["store"], "secrets_locked");
    assert_eq!(loaded[1].details["reason"], "unlocked");
}

#[tokio::test]
async fn sec_013_attach_fails_closed_when_the_store_breaks() {
    let mut f = fixture().await;
    f.store.close().await;
    let redactor = Arc::new(crate::Redactor::pending());
    let err = take_caps(&mut f)
        .redaction
        .attach(&f.secrets, redactor.clone())
        .await
        .unwrap_err();
    assert_ne!(err.code(), "secrets_locked", "{err}");
    assert_eq!(redactor.status(), crate::RedactionStatus::Failed);
    assert!(redactor.redact_log_str("x").is_err());
}

/// #216: Werte aus `sandbox.env.passthrough` gelten als bekannte Werte (mit Varianten), auch
/// wenn der Store gesperrt ist; das Audit nennt Grund und Namen, nie Werte.
#[tokio::test]
async fn sec_013_passthrough_values_are_known_values_and_audited_without_values() {
    const ENV: &str = "Fixture-passthrough-4711-abcdef";
    let mut f = fixture().await;
    let redactor = Arc::new(crate::Redactor::pending());
    take_caps(&mut f)
        .redaction
        .attach(&f.secrets, redactor.clone())
        .await
        .unwrap();
    f.secrets
        .add_redaction_values(
            "env_passthrough",
            vec![("GITHUB_TOKEN".into(), SecretValue::from(ENV))],
        )
        .await
        .unwrap();
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(ENV);
    let out = redactor
        .redact_str(&format!("a {ENV} b {b64} c"))
        .unwrap()
        .into_owned();
    assert!(!out.contains(ENV) && !out.contains(&b64[..20]), "{out}");
    assert!(out.contains("[REDACTED:secret]"), "{out}");
    // Ein neuer Aufbau aus dem Store verliert die Werte nicht.
    f.secrets.put(token(&openai(), KNOWN)).await.unwrap();
    assert!(!redactor.redact_str(ENV).unwrap().contains(ENV));
    let entries = audit(&f).await;
    let loaded: Vec<_> = redaction_entries(&entries)
        .into_iter()
        .filter(|e| e.details["reason"] == "env_passthrough")
        .collect();
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].details["names"],
        serde_json::json!(["GITHUB_TOKEN"])
    );
    assert_eq!(loaded[0].details["values"], 1);
    assert!(!format!("{:?}", loaded[0].details).contains(ENV));
}
