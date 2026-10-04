//! Composer-Endpunkte (WEB-006): `@`-Dateisuche und Skills für das Slash-Menü.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use axum::body::Body;
use beton_core::event::{SessionKind, SessionTrigger};
use beton_core::id::{OrgId, SessionId, UserId};
use beton_server::session_skills::SkillPaths;
use beton_store::NewSession;
use common::{TestApp, app_runtime};

async fn session(t: &TestApp, cwd: &Path, agent: Option<&str>) -> String {
    let local = t.store.ensure_local().await.unwrap();
    t.store
        .create_session(
            OrgId::LOCAL,
            NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "claude".into(),
                cwd: cwd.display().to_string(),
                model: None,
                effort: None,
                permission_mode: None,
                agent_ref: agent.map(str::to_owned),
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::User,
                home_node: local.node,
                harness_opts: serde_json::Value::Null,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Nutzerverzeichnisse ohne Skills, damit die Skills des Entwicklers nicht mitlaufen.
async fn app(home: &Path) -> TestApp {
    let paths = SkillPaths {
        beton_home: home.join(".beton"),
        home: Some(home.to_path_buf()),
    };
    app_runtime(move |mut r| {
        r.sessions.skill_paths = paths;
        r
    })
    .await
}

#[tokio::test]
async fn web_006_ac2_mention_lists_middleware_within_150_ms_in_20000_files() {
    let home = tempfile::tempdir().unwrap();
    let t = app(home.path()).await;
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    for d in 0..200 {
        let dir = root.join(format!("pkg{d:03}/src"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("modul_{f:03}.rs")), "").unwrap();
        }
    }
    write(&root.join("src/middleware.rs"), "pub fn layer() {}\n");
    let id = session(&t, root, None).await;
    let path = format!("/v1/sessions/{id}/workspace/search?mode=fuzzy&limit=20&q=");
    // Der Composer lädt den Index beim Öffnen (leere Anfrage).
    let warm = t
        .send(t.authed("GET", &path).body(Body::empty()).unwrap())
        .await;
    assert_eq!(warm.status, 200, "{}", String::from_utf8_lossy(&warm.body));
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let started = Instant::now();
        let res = t
            .send(
                t.authed("GET", &format!("{path}mid"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        best = best.min(started.elapsed());
        assert_eq!(res.status, 200);
        let items = res.json()["items"].as_array().unwrap().clone();
        assert_eq!(items[0]["path"], "src/middleware.rs", "{items:?}");
    }
    assert!(best < Duration::from_millis(150), "{best:?}");
}

#[tokio::test]
async fn web_006_ac3_skills_of_the_active_agent_with_description() {
    let home = tempfile::tempdir().unwrap();
    let t = app(home.path()).await;
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(
        &root.join(".beton/agents/implementer/agent.yaml"),
        "spec_version: 1\nname: implementer\nexecutor: { harness: claude }\nskills: [review, changelog]\n",
    );
    write(
        &root.join(".beton/agents/implementer/skills/review/SKILL.md"),
        "---\nname: review\ndescription: Prüft die Änderungen wie ein Reviewer\n---\n# Review\n",
    );
    write(
        &root.join(".beton/skills/changelog/SKILL.md"),
        "---\nname: changelog\ndescription: Schreibt einen CHANGELOG-Eintrag\n---\n# Changelog\n",
    );
    // Nicht ausgewählt bzw. nicht vom Menschen aufrufbar.
    write(
        &root.join(".beton/skills/deploy/SKILL.md"),
        "---\nname: deploy\ndescription: Deployt\n---\n",
    );
    write(
        &root.join(".beton/agents/implementer/skills/intern/SKILL.md"),
        "---\nname: intern\ndescription: Nur für das Modell\nuser-invocable: false\n---\n",
    );
    let id = session(&t, root, Some("implementer")).await;
    let res = t
        .send(
            t.authed("GET", &format!("/v1/sessions/{id}/skills"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let body = res.json();
    assert_eq!(body["agent"], "implementer");
    let items = body["items"].as_array().unwrap();
    let pairs: Vec<(&str, &str, &str)> = items
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap(),
                s["description"].as_str().unwrap(),
                s["origin"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("review", "Prüft die Änderungen wie ein Reviewer", "agent"),
            ("changelog", "Schreibt einen CHANGELOG-Eintrag", "project"),
        ]
    );

    // Ohne Agent: alle Skills des Workspace.
    let plain = session(&t, root, None).await;
    let res = t
        .send(
            t.authed("GET", &format!("/v1/sessions/{plain}/skills"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let names: Vec<String> = res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, vec!["changelog", "deploy"]);
    assert!(res.json().get("agent").is_none());
}

#[tokio::test]
async fn web_006_upload_needs_a_known_session() {
    let home = tempfile::tempdir().unwrap();
    let t = app(home.path()).await;
    let res = t
        .send(
            t.authed(
                "POST",
                "/v1/sessions/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C/attachments?name=x.png",
            )
            .header("content-type", "image/png")
            .body(Body::from(vec![1u8, 2, 3]))
            .unwrap(),
        )
        .await;
    assert_eq!(res.status, 404);
    res.assert_problem("not_found");
}
