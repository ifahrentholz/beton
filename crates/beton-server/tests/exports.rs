//! Session-Export und -Import über die API (SES-009, DATA-010). Die importierte Datei ist
//! nicht vertrauenswürdig: Die Tests prüfen Ablehnung mit Zeilennummer und dass eine
//! importierte Session auf dem Host nichts ausführt und keine Pfade aus der Datei benutzt.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use beton_core::event::{
    Actor, Event, EventPayload, GitWorktreeCreated, Notice, SessionKind, SessionTitleChanged,
    SessionTrigger, SystemComponent, TitleSource,
};
use beton_core::id::SessionId;
use beton_server::ws::Authorizer;
use beton_store::{NewSession, SessionRecord};
use serde_json::{Value, json};

use common::{Res, TestApp, app, app_runtime};

struct Fixture {
    t: TestApp,
    source: SessionRecord,
    /// Verzeichnis, auf das der Worktree der Quelle zeigt (darf nie gelöscht werden).
    victim: tempfile::TempDir,
    _work: tempfile::TempDir,
}

async fn fixture_with(t: TestApp) -> Fixture {
    let work = tempfile::tempdir().unwrap();
    let victim = tempfile::tempdir().unwrap();
    std::fs::write(victim.path().join("wichtig.txt"), "bleibt").unwrap();
    let local = t.store.ensure_local().await.unwrap();
    let s = t
        .store
        .create_session(
            local.org,
            NewSession {
                id: SessionId::new(),
                owner: local.user,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: work.path().display().to_string(),
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
        .unwrap();
    let agent = Actor::Agent {
        id: None,
        harness: "fake".into(),
        agent_ref: None,
    };
    let events = vec![
        Event::new(
            s.id,
            0,
            agent.clone(),
            EventPayload::SessionTitleChanged(SessionTitleChanged {
                title: "Rate-Limiter für die Login-API".into(),
                source: TitleSource::User,
            }),
        ),
        Event::new(
            s.id,
            0,
            agent.clone(),
            EventPayload::Notice(Notice {
                text: "Hallo".into(),
                ..Notice::default()
            }),
        ),
        Event::new(
            s.id,
            0,
            Actor::System {
                component: SystemComponent::Server,
            },
            EventPayload::GitWorktreeCreated(GitWorktreeCreated {
                path: victim.path().display().to_string(),
                branch: "beton/opfer".into(),
                base: "main".into(),
                base_sha: "0000000".into(),
            }),
        ),
    ];
    t.store
        .append(local.org, s.id, s.head_seq, s.epoch, events)
        .await
        .unwrap();
    let source = t.store.session(local.org, s.id).await.unwrap();
    Fixture {
        t,
        source,
        victim,
        _work: work,
    }
}

async fn fixture() -> Fixture {
    fixture_with(app().await).await
}

async fn export(t: &TestApp, id: &str, query: &str) -> Res {
    t.send(
        t.authed("GET", &format!("/v1/sessions/{id}/export{query}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

async fn import(t: &TestApp, bytes: Vec<u8>, query: &str) -> Res {
    t.send(
        t.authed("POST", &format!("/v1/sessions/import{query}"))
            .header("content-type", "application/x-ndjson")
            .body(Body::from(bytes))
            .unwrap(),
    )
    .await
}

fn lines(body: &[u8]) -> (Value, Vec<Value>) {
    let text = std::str::from_utf8(body).unwrap();
    let mut it = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    (it.next().unwrap(), it.collect())
}

/// Event-Zeilen ohne Session- und Event-IDs (vergibt der Import neu).
fn normalized(events: &[Value]) -> Vec<Value> {
    let seq_of: HashMap<String, Value> = events
        .iter()
        .map(|e| (e["id"].as_str().unwrap().to_owned(), e["seq"].clone()))
        .collect();
    events
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e["session_id"] = Value::Null;
            e["id"] = Value::Null;
            if let Some(c) = e.get("causation_id").and_then(Value::as_str) {
                e["causation_id"] = seq_of[c].clone();
            }
            e
        })
        .collect()
}

async fn session_count(t: &TestApp) -> usize {
    let local = t.store.ensure_local().await.unwrap();
    t.store.sessions(local.org, true).await.unwrap().len()
}

#[tokio::test]
async fn ses_009_ac1_export_import_export_via_api_yields_identical_event_lines() {
    let f = fixture().await;
    let id = f.source.id.to_string();
    let first = export(&f.t, &id, "").await;
    assert_eq!(first.status, 200);
    assert_eq!(first.header("content-type"), Some("application/x-ndjson"));
    assert_eq!(first.header("beton-export-events"), Some("4"));
    assert_eq!(first.header("beton-export-blobs"), Some("0"));
    let res = import(&f.t, first.body.clone(), "").await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let result = res.json();
    assert_eq!(result["status"], "imported");
    assert_eq!(result["title"], "Rate-Limiter für die Login-API");
    assert_eq!(result["events"], 4);
    assert_eq!(result["imported_from"]["session_id"], id.as_str());
    let new_id = result["session_id"].as_str().unwrap().to_owned();
    assert_ne!(new_id, id);
    let second = export(&f.t, &new_id, "").await;
    assert_eq!(second.status, 200);
    let (_, e1) = lines(&first.body);
    let (h2, e2) = lines(&second.body);
    assert_eq!(normalized(&e1), normalized(&e2));
    assert_eq!(h2["session"]["imported_from"]["session_id"], id.as_str());

    // Mit Blobs: `.tar.zst`, ebenfalls wieder importierbar.
    let archive = export(&f.t, &id, "?with_blobs=true&with_raw=true").await;
    assert_eq!(archive.status, 200);
    assert_eq!(archive.header("content-type"), Some("application/zstd"));
    assert!(archive.body.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]));
    let res =
        f.t.send(
            f.t.authed("POST", "/v1/sessions/import")
                .header("content-type", "application/zstd")
                .body(Body::from(archive.body.clone()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    assert_eq!(res.json()["status"], "imported");
}

#[tokio::test]
async fn ses_009_second_import_of_the_same_file_is_skipped_unless_forced() {
    let f = fixture().await;
    let file = export(&f.t, &f.source.id.to_string(), "").await.body;
    let first = import(&f.t, file.clone(), "").await.json();
    let again = import(&f.t, file.clone(), "").await.json();
    assert_eq!(again["status"], "skipped");
    assert_eq!(again["reason"], "already_imported");
    assert_eq!(again["session_id"], first["session_id"]);
    let forced = import(&f.t, file, "?force=true").await.json();
    assert_eq!(forced["status"], "imported");
    assert_ne!(forced["session_id"], first["session_id"]);
}

#[tokio::test]
async fn ses_009_ac3_a_newer_format_version_is_rejected_with_a_clear_message() {
    let f = fixture().await;
    let body = export(&f.t, &f.source.id.to_string(), "").await.body;
    let (mut header, events) = lines(&body);
    header["format_version"] = json!(2);
    header["beton_version"] = json!("1.4.0");
    let mut file = header.to_string();
    for e in &events {
        file.push('\n');
        file.push_str(&e.to_string());
    }
    let before = session_count(&f.t).await;
    let res = import(&f.t, file.into_bytes(), "").await;
    assert_eq!(res.status, 422);
    res.assert_problem("unsupported_format_version");
    let detail = res.json()["detail"].as_str().unwrap().to_owned();
    assert!(
        detail.contains("Exportformat 2 (aus beton 1.4.0)")
            && detail.contains("bis Format 1")
            && detail.contains("Es wurde nichts angelegt"),
        "{detail}"
    );
    assert_eq!(
        res.json()["errors"][0]["pointer"],
        "/lines/1/format_version"
    );
    assert_eq!(session_count(&f.t).await, before);
}

#[tokio::test]
async fn data_010_ac2_a_seq_gap_is_rejected_via_api_with_line_and_pointer() {
    let f = fixture().await;
    let body = export(&f.t, &f.source.id.to_string(), "").await.body;
    let text = String::from_utf8(body).unwrap();
    let mut parts: Vec<&str> = text.lines().collect();
    parts.remove(3); // seq 3 fehlt
    let before = session_count(&f.t).await;
    let res = import(&f.t, parts.join("\n").into_bytes(), "").await;
    assert_eq!(res.status, 422);
    res.assert_problem("import_seq_gap");
    let p = res.json();
    assert_eq!(p["errors"][0]["pointer"], "/lines/4/seq");
    assert!(p["detail"].as_str().unwrap().starts_with("Zeile 4:"));
    assert_eq!(session_count(&f.t).await, before, "nichts angelegt");

    // Ungültige Zeile
    let mut parts: Vec<String> = text.lines().map(str::to_owned).collect();
    parts[2] = "{\"v\":1}".into();
    let res = import(&f.t, parts.join("\n").into_bytes(), "").await;
    assert_eq!(res.status, 422);
    res.assert_problem("import_invalid");
    assert!(
        res.json()["errors"][0]["pointer"]
            .as_str()
            .unwrap()
            .starts_with("/lines/3")
    );
}

#[tokio::test]
async fn ses_009_import_rejects_foreign_media_types() {
    let t = app().await;
    let res = t
        .send(
            t.authed("POST", "/v1/sessions/import")
                .header("content-type", "text/html")
                .body(Body::from("<html>"))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 415);
    res.assert_problem("unsupported_media_type");
}

struct Nobody;

impl Authorizer for Nobody {
    fn can_read(&self, _: beton_server::security::Authenticated, _: &SessionRecord) -> bool {
        false
    }
}

/// API-002 AC1: Happy Path (oben), 403 und 404 für die neue Ressource.
#[tokio::test]
async fn api_002_ac1_contract_for_session_export() {
    let f = fixture().await;
    let res = export(&f.t, &SessionId::new().to_string(), "").await;
    assert_eq!(res.status, 404);
    res.assert_problem("not_found");
    let res = export(&f.t, "kein-id", "").await;
    assert_eq!(res.status, 404);
    let denied = fixture_with(
        app_runtime(|mut r| {
            r.authorizer = Arc::new(Nobody);
            r
        })
        .await,
    )
    .await;
    let res = export(&denied.t, &denied.source.id.to_string(), "").await;
    assert_eq!(res.status, 403);
    res.assert_problem("forbidden");
}

/// Eine importierte Session führt auf dem Host nichts aus und verwendet keine Pfade aus der
/// Datei: kein Runner (Resume, Eingabe), kein Fork, kein Workspace-Zugriff, und Löschen
/// berührt den Worktree-Pfad aus der Datei nicht.
#[tokio::test]
async fn ses_009_imported_session_never_uses_paths_from_the_file() {
    let f = fixture().await;
    let file = export(&f.t, &f.source.id.to_string(), "").await.body;
    let imported = import(&f.t, file, "").await.json();
    let id = imported["session_id"].as_str().unwrap().to_owned();
    let local = f.t.store.ensure_local().await.unwrap();
    let record =
        f.t.store
            .session(local.org, id.parse().unwrap())
            .await
            .unwrap();
    assert!(record.worktree.is_none());
    assert_eq!(record.status, beton_core::event::SessionStatus::Stopped);

    let post = |path: String, body: Value| {
        let t = &f.t;
        async move {
            t.send(
                t.authed("POST", &path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
        }
    };
    let res = post(format!("/v1/sessions/{id}/resume"), json!({})).await;
    assert_eq!(res.status, 409, "{}", String::from_utf8_lossy(&res.body));
    res.assert_problem("imported_read_only");
    let res = post(
        format!("/v1/sessions/{id}/input"),
        json!({"text": "weiter"}),
    )
    .await;
    assert_eq!(res.status, 409);
    res.assert_problem("imported_read_only");
    let res = post(format!("/v1/sessions/{id}/fork"), json!({})).await;
    assert_eq!(res.status, 409);
    res.assert_problem("imported_read_only");
    let res =
        f.t.send(
            f.t.authed("GET", &format!("/v1/sessions/{id}/workspace/tree"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 409);
    res.assert_problem("imported_read_only");
    // Keine Eingabe ist im Verlauf gelandet.
    let events =
        f.t.store
            .events(local.org, id.parse().unwrap(), 0, 100)
            .await
            .unwrap();
    assert_eq!(events.len(), 4);

    // Löschen entfernt nur die Session, nie das Verzeichnis aus der Datei.
    let res =
        f.t.send(
            f.t.authed("DELETE", &format!("/v1/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(res.status == 204 || res.status == 200, "{}", res.status);
    assert!(f.victim.path().join("wichtig.txt").is_file());
    // Die Quelle ist weiterhin ausführbar (kein `imported_read_only`).
    let res =
        f.t.send(
            f.t.authed(
                "GET",
                &format!("/v1/sessions/{}/workspace/tree", f.source.id),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_ne!(res.status, 409);
}
