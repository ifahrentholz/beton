//! Import fremder Chats über die API (SES-008). Die Vendor-Verzeichnisse sind temporär und
//! enthalten nur die synthetischen Fixtures der Adapter-Crates; echte Verläufe unter `~/.claude`
//! oder `~/.codex` werden nie gelesen.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};

use axum::body::Body;
use beton_core::event::EventPayload;
use beton_core::id::SessionId;
use serde_json::{Value, json};

use common::{TestApp, app_runtime};

const CLAUDE_ID: &str = "3f1c2a9e-5b7d-4c8e-9a1f-2d3e4f5a6b7c";
const CLAUDE_OLDER: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const CODEX_ID: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Temporäre Vendor-Verzeichnisse mit je einer Claude- und einer Codex-Session.
struct Vendor {
    dir: tempfile::TempDir,
}

impl Vendor {
    fn new(cwd: &Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let slug: String = cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let claude = dir.path().join("claude/projects").join(slug);
        std::fs::create_dir_all(&claude).unwrap();
        let fixture = std::fs::read_to_string(crates().join(
            "beton-harness-claude/tests/golden/import/claude-1.0.98-interrupt-mcp/session.jsonl",
        ))
        .unwrap()
        .replace("/beton-golden/workdir", &cwd.display().to_string());
        std::fs::write(claude.join(format!("{CLAUDE_ID}.jsonl")), fixture).unwrap();
        // Eine zweite, ältere Claude-Session (für die Pagination).
        let older = claude.join(format!("{CLAUDE_OLDER}.jsonl"));
        std::fs::copy(
            crates().join(
                "beton-harness-claude/tests/golden/import/claude-2.1.285-rewind-subagent/session.jsonl",
            ),
            &older,
        )
        .unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_modified(past)
            .unwrap();
        std::fs::write(dir.path().join("claude/.credentials.json"), "{}").unwrap();
        let codex = dir.path().join("codex/sessions/2026/09/30");
        std::fs::create_dir_all(&codex).unwrap();
        let fixture = std::fs::read_to_string(crates().join(
            "beton-harness-codex/tests/golden/import/codex-0.46.0-patch-abort/rollout.jsonl",
        ))
        .unwrap()
        .replace("/beton-golden/workdir", &cwd.display().to_string());
        std::fs::write(
            codex.join(format!("rollout-2026-09-30T08-15-22-{CODEX_ID}.jsonl")),
            fixture,
        )
        .unwrap();
        Self { dir }
    }

    fn env(&self) -> beton_harness::HostEnv {
        let mut env = beton_harness::HostEnv::default();
        env.vars.insert(
            "CLAUDE_CONFIG_DIR".into(),
            self.dir.path().join("claude").display().to_string(),
        );
        env.vars.insert(
            "CODEX_HOME".into(),
            self.dir.path().join("codex").display().to_string(),
        );
        env
    }
}

async fn setup() -> (TestApp, Vendor, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let vendor = Vendor::new(work.path());
    let env = vendor.env();
    let app = app_runtime(move |mut r| {
        r.sessions.vendor_env = env;
        r
    })
    .await;
    (app, vendor, work)
}

async fn get(t: &TestApp, path: &str) -> (u16, Value) {
    let res = t
        .send(t.authed("GET", path).body(Body::empty()).unwrap())
        .await;
    (res.status, res.json())
}

async fn post(t: &TestApp, path: &str, body: Value) -> (u16, Value) {
    let res = t
        .send(
            t.authed("POST", path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    (res.status, res.json())
}

async fn events(t: &TestApp, id: &str) -> Vec<beton_core::event::Event> {
    let local = t.store.ensure_local().await.unwrap();
    let id: SessionId = id.parse().unwrap();
    t.store.events(local.org, id, 0, 1000).await.unwrap()
}

#[tokio::test]
async fn ses_008_ac1_second_import_without_force_is_skipped_with_the_existing_session() {
    let (t, _vendor, work) = setup().await;
    let (status, body) = get(&t, "/v1/imports/candidates?harness=claude").await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["vendor_session_id"], CLAUDE_ID,
        "zuletzt geänderte zuerst"
    );
    assert_eq!(items[1]["vendor_session_id"], CLAUDE_OLDER);
    assert!(body["next_cursor"].is_null());
    let (_, page1) = get(&t, "/v1/imports/candidates?harness=claude&limit=1").await;
    assert_eq!(page1["items"][0]["vendor_session_id"], CLAUDE_ID);
    let cursor = page1["next_cursor"].as_str().unwrap();
    let (_, page2) = get(
        &t,
        &format!("/v1/imports/candidates?harness=claude&limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(page2["items"][0]["vendor_session_id"], CLAUDE_OLDER);
    assert!(page2["next_cursor"].is_null());
    assert_eq!(items[0]["title"], "Flaky Payment-Test stabilisieren");
    assert!(items[0].get("imported_session_id").is_none());

    let (status, body) = post(
        &t,
        "/v1/imports",
        json!({"harness": "claude", "refs": [CLAUDE_ID]}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let first = &body["results"][0];
    assert_eq!(first["status"], "imported", "{body}");
    let session = first["session_id"].as_str().unwrap().to_owned();

    // Erstes Event nach `session.created`: `session.imported`; Ursprungszeiten bleiben in `ts`.
    let log = events(&t, &session).await;
    assert_eq!(log[0].type_name(), "session.created");
    let Some(EventPayload::SessionImported(imp)) = log[1].payload() else {
        panic!("{:?}", log[1])
    };
    assert_eq!(
        (imp.source.as_str(), imp.vendor_session_id.as_str()),
        ("claude", CLAUDE_ID)
    );
    let Some(EventPayload::SessionCreated(created)) = log[0].payload() else {
        unreachable!()
    };
    assert_eq!(created.cwd, work.path().display().to_string());
    let first_prompt = log
        .iter()
        .find(|e| matches!(e.payload(), Some(EventPayload::MessageCompleted(_))))
        .unwrap();
    assert_eq!(first_prompt.ts.to_string(), "2026-09-24T14:05:00.000Z");
    assert_eq!(log.last().unwrap().type_name(), "session.status");
    let (_, summary) = get(&t, &format!("/v1/sessions/{session}")).await;
    assert_eq!(summary["title"], "Flaky Payment-Test stabilisieren");
    assert_eq!(summary["status"], "stopped");

    let (_, body) = get(&t, "/v1/imports/candidates?harness=claude").await;
    assert_eq!(body["items"][0]["imported_session_id"], session.as_str());

    // Zweiter Import ohne `force`: übersprungen, mit der bestehenden Session.
    let (status, body) = post(
        &t,
        "/v1/imports",
        json!({"harness": "claude", "refs": [CLAUDE_ID]}),
    )
    .await;
    assert_eq!(status, 200);
    let second = &body["results"][0];
    assert_eq!(second["status"], "skipped");
    assert_eq!(second["reason"], "already_imported");
    assert_eq!(second["session_id"], session.as_str());

    // Mit `force`: neue Session; der Schlüssel zeigt danach auf sie.
    let (_, body) = post(
        &t,
        "/v1/imports",
        json!({"harness": "claude", "refs": [CLAUDE_ID], "force": true}),
    )
    .await;
    let forced = &body["results"][0];
    assert_eq!(forced["status"], "imported");
    assert_ne!(forced["session_id"], session.as_str());
    let (_, body) = post(&t, "/v1/imports", json!({"harness": "claude", "last_n": 1})).await;
    assert_eq!(body["results"][0]["status"], "skipped");
    assert_eq!(body["results"][0]["session_id"], forced["session_id"]);
}

#[tokio::test]
async fn ses_008_codex_import_by_last_n_carries_warnings_without_content() {
    let (t, _vendor, _work) = setup().await;
    let (status, body) = post(&t, "/v1/imports", json!({"harness": "codex", "last_n": 5})).await;
    assert_eq!(status, 200, "{body}");
    let r = &body["results"][0];
    assert_eq!(r["vendor_session_id"], CODEX_ID);
    assert_eq!(r["status"], "imported");
    assert_eq!(r["warnings"][0]["kind"], "unmapped");
    let log = events(&t, r["session_id"].as_str().unwrap()).await;
    let types: Vec<&str> = log.iter().map(|e| e.type_name()).collect();
    assert!(types.contains(&"fs.changed"), "{types:?}");
    assert!(types.contains(&"notice"));
    // Der Harness der Session ist Codex, die Nutzer-Nachricht stammt vom Nutzer.
    let (_, summary) = get(
        &t,
        &format!("/v1/sessions/{}", r["session_id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(summary["harness"], "codex");
    let user_msg = log
        .iter()
        .find(|e| matches!(e.payload(), Some(EventPayload::MessageCompleted(m)) if m.role == beton_core::event::MessageRole::User))
        .unwrap();
    assert!(matches!(
        user_msg.actor,
        beton_core::event::Actor::User { .. }
    ));
}

#[tokio::test]
async fn ses_008_refs_are_ids_not_paths_and_bad_requests_are_rejected() {
    let (t, _vendor, _work) = setup().await;
    // Unbekannte bzw. pfadartige Referenzen werden nie als Pfad benutzt.
    let (status, body) = post(
        &t,
        "/v1/imports",
        json!({"harness": "claude", "refs": ["../../.credentials", "11111111-2222-4333-8444-555555555555"]}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    for r in body["results"].as_array().unwrap() {
        assert_eq!(r["status"], "failed");
        assert_eq!(r["reason"], "not_found");
    }
    for bad in [
        json!({"harness": "claude"}),
        json!({"harness": "claude", "refs": [CLAUDE_ID], "last_n": 1}),
        json!({"harness": "claude", "last_n": 0}),
        json!({"harness": "kein harness!", "last_n": 1}),
    ] {
        let (status, body) = post(&t, "/v1/imports", bad.clone()).await;
        assert_eq!(status, 400, "{bad} → {body}");
    }
    // Harness ohne Import: capability_unsupported.
    let (status, body) = get(&t, "/v1/imports/candidates?harness=fake").await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["code"], "capability_unsupported");
}

#[tokio::test]
async fn ses_008_missing_vendor_dirs_give_an_empty_list() {
    let empty = tempfile::tempdir().unwrap();
    let mut env = beton_harness::HostEnv::default();
    env.vars
        .insert("HOME".into(), empty.path().display().to_string());
    let t = app_runtime(move |mut r| {
        r.sessions.vendor_env = env;
        r
    })
    .await;
    for harness in ["claude", "codex"] {
        let (status, body) = get(&t, &format!("/v1/imports/candidates?harness={harness}")).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["items"], json!([]));
    }
}
