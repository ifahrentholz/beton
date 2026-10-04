//! Tests des Export-/Import-Formats (DATA-010, Teile von SES-009).

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;

use beton_core::event::{
    Actor, ApprovalKind, ApprovalRequested, CostDelta, Event, EventPayload, GitWorktreeCreated,
    Notice, RawJson, SessionKind, SessionStatus, SessionStatusChanged, SessionTitleChanged,
    SessionTrigger, SystemComponent, TimeoutAction, TitleSource,
};
use beton_core::id::{ApprovalId, PrincipalId, SessionId, UserId};
use beton_core::time::Timestamp;
use serde_json::{Value, json};

use super::*;
use crate::testutil::{TestStore, org, store};
use crate::{DeleteAuthority, NewSession, SessionRecord};

fn agent() -> Actor {
    Actor::Agent {
        id: None,
        harness: "claude".into(),
        agent_ref: None,
    }
}

async fn append(t: &TestStore, s: &SessionRecord, mut events: Vec<Event>) {
    for e in &mut events {
        e.session_id = s.id;
    }
    let head = t.store.session(s.org_id, s.id).await.unwrap().head_seq;
    t.store
        .append(s.org_id, s.id, head, s.epoch, events)
        .await
        .unwrap();
}

/// Eine Session mit Titel, Nachrichten, Kausalität, Turn, `raw`, großer (ausgelagerter)
/// Payload, Kosten, offener Freigabe und Worktree.
async fn sample(t: &TestStore) -> SessionRecord {
    let s = t
        .store
        .create_session(
            org(t),
            NewSession {
                id: SessionId::new(),
                owner: t.local.user,
                kind: SessionKind::Main,
                harness: "claude".into(),
                cwd: "/tmp/quelle".into(),
                model: Some("sonnet".into()),
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
        .unwrap();
    let title = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::SessionTitleChanged(SessionTitleChanged {
            title: "Rate-Limiter".into(),
            source: TitleSource::User,
        }),
    );
    let mut user = Event::new(
        s.id,
        0,
        Actor::User {
            id: PrincipalId::User(t.local.user),
            device_id: None,
        },
        EventPayload::Notice(Notice {
            text: "Bitte bauen".into(),
            ..Notice::default()
        }),
    );
    user.turn_id = Some(beton_core::id::TurnId::new());
    user.raw = Some(RawJson::from_string(r#"{"original":true}"#.into()).unwrap());
    let mut answer = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::Notice(Notice {
            // Größer als 64 KiB: wird ausgelagert (PROTO-001 AC4).
            text: "x".repeat(70 * 1024),
            ..Notice::default()
        }),
    );
    answer.causation_id = Some(user.id);
    answer.turn_id = user.turn_id;
    let cost = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::CostDelta(CostDelta {
            harness: "claude".into(),
            model: "sonnet".into(),
            input_tokens: 10,
            output_tokens: 5,
            cost_micro: Some(1200),
            currency: "USD".into(),
            ..CostDelta::default()
        }),
    );
    let approval = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::ApprovalRequested(ApprovalRequested {
            approval_id: ApprovalId::new(),
            kind: ApprovalKind::default(),
            subject: json!({"tool": "bash"}),
            options: vec!["allow".into(), "deny".into()],
            expires_at: Timestamp::now(),
            on_timeout: TimeoutAction::default(),
        }),
    );
    let worktree = Event::new(
        s.id,
        0,
        Actor::System {
            component: SystemComponent::Server,
        },
        EventPayload::GitWorktreeCreated(GitWorktreeCreated {
            path: "/tmp/quelle-worktree".into(),
            branch: "beton/x".into(),
            base: "main".into(),
            base_sha: "abc".into(),
        }),
    );
    let running = Event::new(
        s.id,
        0,
        agent(),
        EventPayload::SessionStatus(SessionStatusChanged {
            status: SessionStatus::Running,
            reason: None,
        }),
    );
    append(
        t,
        &s,
        vec![title, user, answer, cost, approval, worktree, running],
    )
    .await;
    t.store.session(org(t), s.id).await.unwrap()
}

/// Kopf und Event-Zeilen einer Exportdatei (JSONL oder `.tar.zst`).
fn lines(bytes: &[u8]) -> (Value, Vec<Value>) {
    let jsonl = if bytes.starts_with(&ZSTD_MAGIC) {
        unpack(bytes).unwrap().0
    } else {
        bytes.to_vec()
    };
    let text = String::from_utf8(jsonl).unwrap();
    let mut it = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let header = it.next().unwrap();
    (header, it.collect())
}

/// Event-Zeilen ohne die Felder, die ein Import neu vergibt: Session-ID und Event-IDs
/// (`causation_id` zeigt danach auf die `seq` des Ziels).
fn normalized(events: &[Value]) -> Vec<Value> {
    let seq_of: HashMap<String, u64> = events
        .iter()
        .map(|e| {
            (
                e["id"].as_str().unwrap().to_owned(),
                e["seq"].as_u64().unwrap(),
            )
        })
        .collect();
    events
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e["session_id"] = json!("<session>");
            e["id"] = json!(format!("<event {}>", e["seq"]));
            if let Some(c) = e.get("causation_id").and_then(Value::as_str) {
                e["causation_id"] = json!(format!("<event {}>", seq_of[c]));
            }
            e
        })
        .collect()
}

async fn import(t: &TestStore, bytes: &[u8], force: bool) -> FileImportOutcome {
    let parsed = parse_export(bytes).unwrap();
    t.store
        .import_export(org(t), t.local.user, t.local.node, parsed, force)
        .await
        .unwrap()
}

async fn session_count(t: &TestStore) -> usize {
    t.store.sessions(org(t), true).await.unwrap().len()
}

// ---------------------------------------------------------------------------
// AC1: Export → Import → Export
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_010_ac1_export_import_export_yields_identical_event_lines() {
    for opts in [
        ExportOptions::default(),
        ExportOptions {
            with_raw: true,
            with_blobs: false,
        },
        ExportOptions {
            with_raw: true,
            with_blobs: true,
        },
    ] {
        let t = store().await;
        let s = sample(&t).await;
        let first = t.store.export_session(org(&t), s.id, opts).await.unwrap();
        assert_eq!(first.archive, opts.with_blobs);
        assert_eq!(first.events, s.head_seq);
        let outcome = import(&t, &first.bytes, false).await;
        assert_eq!(outcome.status, FileImportStatus::Imported);
        assert_ne!(outcome.session_id, s.id, "neue Session-ID");
        assert_eq!(outcome.title, "Rate-Limiter");
        let second = t
            .store
            .export_session(org(&t), outcome.session_id, opts)
            .await
            .unwrap();
        let (h1, e1) = lines(&first.bytes);
        let (h2, e2) = lines(&second.bytes);
        assert_eq!(e1.len(), 8);
        assert_eq!(normalized(&e1), normalized(&e2), "{opts:?}");
        // Gleich bleiben seq, ts, actor, type, payload, turn_id und (mit Flag) raw.
        for (a, b) in e1.iter().zip(&e2) {
            assert_eq!(a["seq"], b["seq"]);
            assert_eq!(a["ts"], b["ts"]);
            assert_ne!(a["id"], b["id"]);
            assert_eq!(b["session_id"], outcome.session_id.to_string());
        }
        // Kopf: bis auf Session-ID, Exportzeitpunkt und Herkunft identisch.
        let strip = |mut h: Value| {
            h["exported_at"] = Value::Null;
            h["session"]["id"] = Value::Null;
            h["session"]
                .as_object_mut()
                .unwrap()
                .remove("imported_from");
            h
        };
        assert_eq!(
            h2["session"]["imported_from"]["session_id"],
            s.id.to_string()
        );
        assert_eq!(
            h2["session"]["imported_from"]["export_sha256"],
            outcome.imported_from.export_sha256
        );
        // Die importierte Session ist gestoppt (sie läuft auf diesem Host nicht).
        let mut h1 = strip(h1);
        h1["session"]["status"] = json!("stopped");
        assert_eq!(h1, strip(h2));
        if opts.with_blobs {
            assert_eq!(first.blobs, 1, "der ausgelagerte Payload liegt im Archiv");
            assert!(e1[3].get("payload_ref").is_some());
        } else {
            assert!(e1[3].get("payload_ref").is_none(), "JSONL ist vollständig");
            assert_eq!(e1[3]["payload"]["text"].as_str().unwrap().len(), 70 * 1024);
        }
    }
}

#[tokio::test]
async fn data_010_ac1_unknown_users_become_system_import_and_the_local_user_stays() {
    let t = store().await;
    let s = sample(&t).await;
    let stranger = Event::new(
        s.id,
        0,
        Actor::User {
            id: PrincipalId::User(UserId::new()),
            device_id: None,
        },
        EventPayload::Notice(Notice {
            text: "von woanders".into(),
            ..Notice::default()
        }),
    );
    append(&t, &s, vec![stranger]).await;
    let file = t
        .store
        .export_session(org(&t), s.id, ExportOptions::default())
        .await
        .unwrap();
    let outcome = import(&t, &file.bytes, false).await;
    let events = t
        .store
        .events(org(&t), outcome.session_id, 0, 100)
        .await
        .unwrap();
    assert_eq!(
        events.last().unwrap().actor,
        Actor::System {
            component: SystemComponent::Import
        }
    );
    assert_eq!(
        events[2].actor,
        Actor::User {
            id: PrincipalId::User(t.local.user),
            device_id: None
        }
    );
    // Die importierte Session gehört dem importierenden User.
    let record = t.store.session(org(&t), outcome.session_id).await.unwrap();
    assert_eq!(record.owner, t.local.user);
    assert_eq!(record.kind, SessionKind::Main);
}

#[tokio::test]
async fn data_010_import_is_idempotent_by_export_hash_and_force_creates_another() {
    let t = store().await;
    let s = sample(&t).await;
    let file = t
        .store
        .export_session(org(&t), s.id, ExportOptions::default())
        .await
        .unwrap();
    let first = import(&t, &file.bytes, false).await;
    let before = session_count(&t).await;
    let again = import(&t, &file.bytes, false).await;
    assert_eq!(again.status, FileImportStatus::Skipped);
    assert_eq!(again.session_id, first.session_id);
    assert_eq!(session_count(&t).await, before);
    let forced = import(&t, &file.bytes, true).await;
    assert_eq!(forced.status, FileImportStatus::Imported);
    assert_ne!(forced.session_id, first.session_id);
    // Beide bleiben als Import markiert.
    for id in [first.session_id, forced.session_id] {
        assert!(t.store.file_import(org(&t), id).await.unwrap().is_some());
    }
    // Nach dem Löschen der Session gibt der Hash wieder frei.
    for id in [first.session_id, forced.session_id] {
        t.store
            .delete_session(
                org(&t),
                id,
                PrincipalId::User(t.local.user),
                DeleteAuthority::Owner,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        import(&t, &file.bytes, false).await.status,
        FileImportStatus::Imported
    );
}

#[tokio::test]
async fn data_010_imported_session_is_sealed_also_after_a_projection_rebuild() {
    let t = store().await;
    let s = sample(&t).await;
    assert!(s.worktree.is_some());
    assert_eq!(s.status, SessionStatus::Running);
    let file = t
        .store
        .export_session(org(&t), s.id, ExportOptions::default())
        .await
        .unwrap();
    let outcome = import(&t, &file.bytes, false).await;
    for round in 0..2 {
        let r = t.store.session(org(&t), outcome.session_id).await.unwrap();
        assert!(r.worktree.is_none(), "kein Worktree-Pfad aus der Datei");
        assert_eq!(r.status, SessionStatus::Stopped);
        assert_eq!(r.cost_micro, 1200, "Kosten der Session bleiben sichtbar");
        let open = t.store.open_approvals(org(&t)).await.unwrap();
        assert!(open.iter().all(|a| a.session_id != outcome.session_id));
        let usage = t
            .store
            .usage_daily(org(&t), "2000-01-01", "2999-12-31")
            .await
            .unwrap();
        assert_eq!(
            usage.iter().map(|u| u.events).sum::<u64>(),
            1,
            "nur die Quelle zählt zur Usage"
        );
        if round == 0 {
            t.store.rebuild_projections(org(&t), None).await.unwrap();
        }
    }
    // Die Quelle bleibt unverändert.
    let source = t.store.session(org(&t), s.id).await.unwrap();
    assert!(source.worktree.is_some());
}

// ---------------------------------------------------------------------------
// AC2: Lücken und ungültige Zeilen
// ---------------------------------------------------------------------------

fn jsonl_of(header: &Value, events: &[Value]) -> Vec<u8> {
    let mut out = String::new();
    for v in std::iter::once(header).chain(events) {
        out.push_str(&v.to_string());
        out.push('\n');
    }
    out.into_bytes()
}

async fn exported(t: &TestStore) -> (Value, Vec<Value>) {
    let s = sample(t).await;
    let file = t
        .store
        .export_session(org(t), s.id, ExportOptions::default())
        .await
        .unwrap();
    lines(&file.bytes)
}

#[tokio::test]
async fn data_010_ac2_a_seq_gap_rejects_the_whole_file_with_line_number() {
    let t = store().await;
    let (header, mut events) = exported(&t).await;
    events.remove(2); // seq 3 fehlt, Zeile 4 trägt seq 4
    let before = session_count(&t).await;
    let err = parse_export(&jsonl_of(&header, &events)).unwrap_err();
    assert_eq!(err.kind, InvalidKind::SeqGap);
    assert_eq!(err.kind.code(), "import_seq_gap");
    assert_eq!(err.line, Some(4));
    assert_eq!(err.pointer.as_deref(), Some("/lines/4/seq"));
    assert!(
        err.detail.contains("Zeile 4") && err.detail.contains("Ereignis 3 fehlt"),
        "{}",
        err.detail
    );
    assert_eq!(session_count(&t).await, before, "nichts angelegt");
}

#[tokio::test]
async fn data_010_ac2_an_invalid_line_rejects_the_whole_file_with_line_number() {
    let t = store().await;
    let (header, events) = exported(&t).await;
    // Ungültige Nutzlast in Zeile 3 (seq 2): Titel ist keine Zeichenkette.
    let mut bad = events.clone();
    bad[1]["payload"]["title"] = json!(42);
    let err = parse_export(&jsonl_of(&header, &bad)).unwrap_err();
    assert_eq!(err.kind, InvalidKind::Invalid);
    assert_eq!(err.kind.code(), "import_invalid");
    assert_eq!(err.line, Some(3));
    assert!(err.pointer.as_deref().unwrap().starts_with("/lines/3"));
    assert!(err.detail.starts_with("Zeile 3:"), "{}", err.detail);

    // Kein JSON in Zeile 5.
    let mut text = String::from_utf8(jsonl_of(&header, &events)).unwrap();
    let mut parts: Vec<String> = text.lines().map(str::to_owned).collect();
    parts[4] = "{kaputt".into();
    text = parts.join("\n");
    let err = parse_export(text.as_bytes()).unwrap_err();
    assert_eq!(err.line, Some(5));

    // Unbekannter Event-Typ, fremde Session, transient, doppelte ID, Zeile zu viel.
    type Mutation = Box<dyn Fn(&mut Vec<Value>)>;
    let cases: Vec<(Mutation, u64)> = vec![
        (Box::new(|e| e[2]["type"] = json!("gibt.es.nicht")), 4),
        (
            Box::new(|e| e[3]["session_id"] = json!(SessionId::new().to_string())),
            5,
        ),
        (Box::new(|e| e[1]["transient"] = json!(true)), 3),
        (
            Box::new(|e| {
                let id = e[0]["id"].clone();
                e[4]["id"] = id;
            }),
            6,
        ),
        (
            Box::new(|e| {
                let mut extra = e[7].clone();
                extra["seq"] = json!(9);
                extra["id"] = json!(beton_core::id::EventId::new().to_string());
                e.push(extra);
            }),
            10,
        ),
    ];
    for (mutate, line) in cases {
        let mut e = events.clone();
        mutate(&mut e);
        let err = parse_export(&jsonl_of(&header, &e)).unwrap_err();
        assert_eq!(err.line, Some(line), "{}", err.detail);
    }
    // Abgeschnitten: der Kopf nennt mehr Events als folgen.
    let err = parse_export(&jsonl_of(&header, &events[..5])).unwrap_err();
    assert_eq!(err.kind, InvalidKind::Invalid);
    assert_eq!(err.line, Some(7));
    // Keine Exportdatei.
    let err = parse_export(b"{\"hallo\":1}\n").unwrap_err();
    assert_eq!(err.line, Some(1));
    assert!(parse_export(b"").is_err());
}

// ---------------------------------------------------------------------------
// AC3: raw nur mit --with-raw
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_010_ac3_raw_is_only_exported_with_the_flag() {
    let t = store().await;
    let s = sample(&t).await;
    let plain = t
        .store
        .export_session(org(&t), s.id, ExportOptions::default())
        .await
        .unwrap();
    let (h, events) = lines(&plain.bytes);
    assert_eq!(h["session"]["raw"], false);
    assert!(events.iter().all(|e| e.get("raw").is_none()));
    let with = t
        .store
        .export_session(
            org(&t),
            s.id,
            ExportOptions {
                with_raw: true,
                with_blobs: false,
            },
        )
        .await
        .unwrap();
    let (h, events) = lines(&with.bytes);
    assert_eq!(h["session"]["raw"], true);
    assert_eq!(events[2]["raw"], json!({"original": true}));
}

/// Der Redaction-Hook greift auch beim Export von `raw`.
#[tokio::test]
async fn data_010_ac3_exported_raw_passes_the_redaction_hook() {
    struct Redact;
    impl crate::RawRedactor for Redact {
        fn redact(&self, raw: &mut Value) {
            if let Some(o) = raw.as_object_mut() {
                o.insert("original".into(), json!("[REDACTED:test]"));
            }
        }
    }
    let t = store().await;
    let s = sample(&t).await;
    // Ein zweiter Store auf derselben Datenbank, aber mit Redaction-Hook.
    let redacting = Store::open(
        t.dir.path(),
        crate::StoreOptions {
            redactor: std::sync::Arc::new(Redact),
            ..crate::StoreOptions::default()
        },
    )
    .await
    .unwrap();
    let file = redacting
        .export_session(
            org(&t),
            s.id,
            ExportOptions {
                with_raw: true,
                with_blobs: false,
            },
        )
        .await
        .unwrap();
    let (_, events) = lines(&file.bytes);
    assert_eq!(events[2]["raw"]["original"], "[REDACTED:test]");
}

// ---------------------------------------------------------------------------
// AC4: Formatversion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_010_ac4_format_version_2_is_rejected_as_unsupported() {
    let t = store().await;
    let (mut header, events) = exported(&t).await;
    header["format_version"] = json!(2);
    header["beton_version"] = json!("1.4.0");
    // Ein v2-Kopf darf neue Felder haben; abgelehnt wird trotzdem wegen der Version.
    header["neues_feld"] = json!(true);
    let err = parse_export(&jsonl_of(&header, &events)).unwrap_err();
    assert_eq!(err.kind, InvalidKind::UnsupportedFormatVersion);
    assert_eq!(err.kind.code(), "unsupported_format_version");
    assert_eq!(err.line, Some(1));
    assert!(
        err.detail.contains("Exportformat 2 (aus beton 1.4.0)")
            && err.detail.contains("bis Format 1"),
        "{}",
        err.detail
    );
    header["format_version"] = json!(0);
    let err = parse_export(&jsonl_of(&header, &events)).unwrap_err();
    assert_eq!(err.kind, InvalidKind::Invalid);
}

// ---------------------------------------------------------------------------
// Archiv: nicht vertrauenswürdige Eingabe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_010_archive_rejects_links_paths_foreign_entries_and_bad_blobs() {
    let t = store().await;
    let s = sample(&t).await;
    let file = t
        .store
        .export_session(
            org(&t),
            s.id,
            ExportOptions {
                with_raw: false,
                with_blobs: true,
            },
        )
        .await
        .unwrap();
    let (jsonl, blobs) = unpack(&file.bytes).unwrap();
    let (blob, data) = blobs.iter().next().unwrap();
    let blob_name = format!("blobs/{}", blob.hex());
    let ok = pack_for_tests(&jsonl, &[(&blob_name, data, tar::EntryType::Regular)]);
    assert!(parse_export(&ok).is_ok());

    let regular = tar::EntryType::Regular;
    let cases: Vec<Vec<(&str, &[u8], tar::EntryType)>> = vec![
        // Symlink und Hardlink
        vec![
            (&blob_name, data, regular),
            ("blobs/link", b"", tar::EntryType::Symlink),
        ],
        vec![
            (&blob_name, data, regular),
            ("session2", b"", tar::EntryType::Link),
        ],
        // Pfade aus dem Archiv
        vec![
            (&blob_name, data, regular),
            ("../../etc/passwd", b"x", regular),
        ],
        vec![(&blob_name, data, regular), ("/tmp/x", b"x", regular)],
        // fremde Datei
        vec![(&blob_name, data, regular), ("README", b"x", regular)],
        // Blob passt nicht zum Hash
        vec![(&blob_name, b"anderer Inhalt", regular)],
        // Blob fehlt (payload_ref zeigt ins Leere)
        vec![],
        // doppelte session.jsonl
        vec![(&blob_name, data, regular), (JSONL_NAME, &jsonl, regular)],
    ];
    for entries in cases {
        let bytes = pack_for_tests(&jsonl, &entries);
        let err = parse_export(&bytes).unwrap_err();
        assert_eq!(err.kind, InvalidKind::Invalid, "{}", err.detail);
    }
    // Ein zusätzlicher Blob, den der Kopf nicht nennt.
    let other = b"anhang";
    let other_name = format!("blobs/{}", hex::encode(Sha256::digest(other)));
    let bytes = pack_for_tests(
        &jsonl,
        &[(&blob_name, data, regular), (&other_name, other, regular)],
    );
    assert!(parse_export(&bytes).is_err());
    // Kein gültiges zstd hinter den magischen Bytes.
    let mut garbage = ZSTD_MAGIC.to_vec();
    garbage.extend_from_slice(b"kein zstd");
    assert_eq!(
        parse_export(&garbage).unwrap_err().kind,
        InvalidKind::Invalid
    );
}

#[test]
fn data_010_unpacking_stops_at_the_size_limit() {
    let mut limited = Limited {
        inner: std::io::repeat(0),
        left: 10,
    };
    let mut buf = [0u8; 8];
    assert_eq!(limited.read(&mut buf).unwrap(), 8);
    let err = limited.read(&mut buf).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::FileTooLarge);
    assert_eq!(unpack_error(&err).kind, InvalidKind::TooLarge);
    let too_big = vec![0u8; MAX_IMPORT_BYTES + 1];
    assert_eq!(
        parse_export(&too_big).unwrap_err().kind,
        InvalidKind::TooLarge
    );
}

#[tokio::test]
async fn data_010_export_of_unknown_session_is_not_found() {
    let t = store().await;
    let err = t
        .store
        .export_session(org(&t), SessionId::new(), ExportOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}
