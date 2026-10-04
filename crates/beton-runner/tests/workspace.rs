//! Worktrees pro Session und Workspace-API über REST und WebSocket mit echtem Runner und
//! Fake-Harness (SES-015, SES-016, SES-017, SES-018). Git-Repositories sind temporär, Remotes
//! lokale Pfade: kein Netzwerk.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_host::LocalProvider;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{Store, StoreOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

struct Daemonized {
    daemon: Daemon,
    token: String,
    addr: SocketAddr,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn daemon(data: &Path) -> Daemonized {
    let store = Store::open(data, StoreOptions::default()).await.unwrap();
    let mut cfg = ServerConfig::local(data.to_path_buf());
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.socket = None;
    let runners = data.join("runners");
    let daemon = start_with(cfg, store, move |mut r| {
        r.sessions.provider =
            std::sync::Arc::new(LocalProvider::new(vec![runner_bin().into()], runners));
        r.sessions.dev = true;
        r
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(data.join("auth/local.token"))
        .unwrap()
        .trim()
        .to_owned();
    let addr = daemon.addrs[0];
    Daemonized {
        daemon,
        token,
        addr,
    }
}

impl Daemonized {
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, &str)],
    ) -> Reply {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n",
            port = self.addr.port(),
            token = self.token,
            len = body.len()
        );
        for (k, v) in extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        stream
            .write_all(format!("{head}\r\n{body}").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        let chunked = headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
        });
        let payload = if chunked {
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        Reply {
            status,
            headers,
            body: serde_json::from_str(&payload).unwrap_or(Value::Null),
        }
    }

    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        self.request(method, path, body, &[]).await
    }

    async fn ws(
        &self,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut req = format!("ws://127.0.0.1:{}/v1/ws", self.addr.port())
            .into_client_request()
            .unwrap();
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        req.headers_mut()
            .insert("sec-websocket-protocol", "beton.v1".parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws.send(Message::Text(
            json!({"t": "hello", "protocol": "1.0", "client": {"kind": "test", "version": "0"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let _welcome = ws.next().await;
        ws
    }

    async fn create(&self, body: Value) -> Reply {
        self.http("POST", "/v1/sessions", Some(body)).await
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let mut after = 0;
        loop {
            let page = self
                .http(
                    "GET",
                    &format!("/v1/sessions/{id}/events?after_seq={after}&limit=200"),
                    None,
                )
                .await;
            assert_eq!(page.status, 200, "{}", page.body);
            let items = page.body["items"].as_array().unwrap().clone();
            if items.is_empty() {
                return out;
            }
            after = items.last().unwrap()["seq"].as_u64().unwrap();
            out.extend(items);
        }
    }

    async fn wait_for(&self, id: &str, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            if events.iter().any(&pred) {
                return events;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Event fehlt: {:?}",
                events.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn wait_idle(&self, id: &str) {
        let start = Instant::now();
        loop {
            let s = self.http("GET", &format!("/v1/sessions/{id}"), None).await;
            if s.body["status"] == "idle" {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "nicht idle: {}",
                s.body
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Eingabe senden und auf das Turn-Ende warten; liefert die Turn-ID.
    async fn turn(&self, id: &str, text: &str) -> String {
        let before = self.events(id).await.len();
        let r = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({ "text": text })),
            )
            .await;
        assert_eq!(r.status, 202, "{}", r.body);
        let turn = r.body["turn_id"].as_str().unwrap().to_owned();
        let t = turn.clone();
        self.wait_for(id, move |e| {
            e["type"] == "turn.completed" && e["payload"]["turn_id"] == t.as_str()
        })
        .await;
        assert!(self.events(id).await.len() > before);
        turn
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&tail[..n.min(tail.len())]);
        rest = tail.get(n + 2..).unwrap_or("");
    }
    out
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("bt")
        .tempdir_in("/tmp")
        .unwrap()
}

fn scenario(dir: &Path, yaml: &str) -> PathBuf {
    let p = dir.join(format!("{}.yaml", beton_core::id::RunnerId::new()));
    std::fs::write(&p, yaml).unwrap();
    p
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Datenverzeichnis des Daemons und ein Repository mit `main`.
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tmp();
    let data = dir.path().join("home");
    let repo = dir.path().join("projekt");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    write(&repo, "README.md", "eins\nzwei\ndrei\n");
    write(&repo, "src/lib.rs", "fn a() {}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "start"]);
    let repo = repo.canonicalize().unwrap();
    (dir, data, repo)
}

fn session_body(cwd: &Path, scenario: &Path, worktree: Option<Value>) -> Value {
    let mut body = json!({"target": "fake", "cwd": cwd, "harness_opts": {"scenario": scenario}});
    if let Some(w) = worktree {
        body["worktree"] = w;
    }
    body
}

fn path_of(summary: &Value) -> PathBuf {
    PathBuf::from(summary["worktree"]["path"].as_str().unwrap())
}

// ---------------------------------------------------------------------------
// SES-015: Worktree pro Session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ses_015_ac1_two_worktree_sessions_work_in_separate_directories_and_branches() {
    let (dir, data, repo) = setup();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let a = d.create(session_body(&repo, &sc, Some(json!({})))).await;
    assert_eq!(a.status, 201, "{}", a.body);
    let b = d
        .create(session_body(
            &repo,
            &sc,
            Some(json!({"branch": "feature/zwei"})),
        ))
        .await;
    assert_eq!(b.status, 201, "{}", b.body);
    let (pa, pb) = (path_of(&a.body), path_of(&b.body));
    assert_ne!(pa, pb);
    assert!(pa.starts_with(data.join("worktrees").canonicalize().unwrap()));
    let (ba, bb) = (
        a.body["worktree"]["branch"].as_str().unwrap(),
        b.body["worktree"]["branch"].as_str().unwrap(),
    );
    assert!(ba.starts_with("beton/session-"), "{ba}");
    assert_eq!(bb, "feature/zwei");
    let list = git(&repo, &["worktree", "list", "--porcelain"]);
    for (p, br) in [(&pa, ba), (&pb, bb)] {
        assert!(
            list.contains(&format!("worktree {}", p.display())),
            "{list}"
        );
        assert!(list.contains(&format!("branch refs/heads/{br}")), "{list}");
        assert_eq!(git(p, &["branch", "--show-current"]), br);
    }
    // Die Sessions laufen; `cwd` bleibt das Projekt (für `beton run -c`).
    for s in [&a, &b] {
        let id = s.body["id"].as_str().unwrap();
        d.wait_idle(id).await;
        let events = d.events(id).await;
        assert_eq!(events[0]["payload"]["cwd"], repo.display().to_string());
    }
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_015_ac3_unresolvable_base_fails_and_no_session_starts() {
    let (dir, data, repo) = setup();
    git(&repo, &["branch", "feature/x"]);
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d
        .create(session_body(
            &repo,
            &sc,
            Some(json!({"base": "gibt-es-nicht"})),
        ))
        .await;
    assert_eq!(r.status, 422, "{}", r.body);
    assert_eq!(r.body["code"], "base_not_found");
    let detail = r.body["detail"].as_str().unwrap();
    assert!(
        detail.contains("gibt-es-nicht") && detail.contains("main") && detail.contains("feature/x"),
        "{detail}"
    );
    // Keine Session, kein Worktree.
    let list = d.http("GET", "/v1/sessions", None).await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 0);
    assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);

    // Ohne Git-Repository gibt es keinen Worktree (und keine Session ohne ihn).
    let plain = dir.path().join("ohne-git");
    std::fs::create_dir_all(&plain).unwrap();
    let r = d.create(session_body(&plain, &sc, Some(json!({})))).await;
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(r.body["code"], "not_a_git_repo");
    let list = d.http("GET", "/v1/sessions", None).await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 0);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_015_ac4_worktree_created_event_is_logged() {
    let (dir, data, repo) = setup();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d
        .create(
            json!({"target": "fake", "cwd": repo, "title": "Login fixen",
                       "harness_opts": {"scenario": sc}, "worktree": {"base": "main"}}),
        )
        .await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    let events = d.events(id).await;
    let created: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "git.worktree_created")
        .collect();
    assert_eq!(created.len(), 1);
    let p = &created[0]["payload"];
    assert_eq!(p["path"], r.body["worktree"]["path"]);
    assert_eq!(p["base"], "main");
    assert_eq!(p["base_sha"], git(&repo, &["rev-parse", "main"]).as_str());
    let branch = p["branch"].as_str().unwrap();
    assert!(branch.starts_with("beton/login-fixen-"), "{branch}");
    assert_eq!(branch.len(), "beton/login-fixen-".len() + 4);
    assert_eq!(
        created[0]["actor"],
        json!({"kind": "system", "component": "server"})
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_015_ac5_unreachable_remote_uses_local_base_and_notes_the_skipped_fetch() {
    let (dir, data, upstream) = setup();
    git(
        dir.path(),
        &["clone", "--quiet", &upstream.display().to_string(), "klon"],
    );
    let clone = dir.path().join("klon").canonicalize().unwrap();
    // „Offline“: das Remote ist nicht mehr erreichbar.
    std::fs::rename(&upstream, dir.path().join("weg")).unwrap();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&clone, &sc, Some(json!({})))).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    assert_eq!(r.body["worktree"]["base"], "origin/main");
    d.wait_idle(id).await;
    let events = d.events(id).await;
    let notice = events
        .iter()
        .find(|e| e["type"] == "notice")
        .expect("Hinweis auf den nicht ausgeführten Fetch");
    assert_eq!(notice["payload"]["level"], "warn");
    let text = notice["payload"]["text"].as_str().unwrap();
    assert!(text.contains("git fetch origin main"), "{text}");
    assert!(!text.contains("weg"), "keine Remote-URL im Hinweis: {text}");
    assert!(path_of(&r.body).join("README.md").is_file());
    d.daemon.shutdown().await;
}

// ---------------------------------------------------------------------------
// SES-016: Worktree-Aufräumen
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ses_016_ac1_delete_with_uncommitted_changes_removes_nothing_and_returns_409() {
    let (dir, data, repo) = setup();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&repo, &sc, Some(json!({})))).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    let wt = path_of(&r.body);
    let branch = r.body["worktree"]["branch"].as_str().unwrap().to_owned();
    d.wait_idle(id).await;
    write(&wt, "angefangen.txt", "halb fertig\n");

    let del = d.http("DELETE", &format!("/v1/sessions/{id}"), None).await;
    assert_eq!(del.status, 409, "{}", del.body);
    assert_eq!(del.body["code"], "worktree_dirty");
    assert_eq!(del.header("content-type"), Some("application/problem+json"));
    // Nichts entfernt: Session, Worktree, Datei und Branch existieren, der Runner läuft.
    assert_eq!(
        d.http("GET", &format!("/v1/sessions/{id}"), None)
            .await
            .status,
        200
    );
    assert!(wt.join("angefangen.txt").is_file());
    assert_eq!(
        git(
            &repo,
            &["branch", "--list", &branch, "--format=%(refname:short)"]
        ),
        branch
    );
    assert!(d.daemon.runtime.runners.connected(id.parse().unwrap()));

    // Mit WIP-Commit folgt die zweite Rückfrage (ungepushte Commits), Branch behalten.
    let del = d
        .http(
            "DELETE",
            &format!("/v1/sessions/{id}?uncommitted=commit"),
            None,
        )
        .await;
    assert_eq!(del.status, 409, "{}", del.body);
    assert_eq!(del.body["code"], "worktree_unpushed");
    assert!(wt.join("angefangen.txt").is_file());
    let del = d
        .http(
            "DELETE",
            &format!("/v1/sessions/{id}?uncommitted=commit&branch=keep"),
            None,
        )
        .await;
    assert_eq!(del.status, 204, "{}", del.body);
    assert!(!wt.exists());
    assert_eq!(
        git(&repo, &["show", &format!("{branch}:angefangen.txt")]),
        "halb fertig",
        "die Arbeit steckt im WIP-Commit"
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_016_ac2_delete_with_merged_branch_removes_directory_and_branch() {
    let (dir, data, repo) = setup();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&repo, &sc, Some(json!({})))).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    let wt = path_of(&r.body);
    let branch = r.body["worktree"]["branch"].as_str().unwrap().to_owned();
    d.wait_idle(id).await;
    write(&wt, "feature.txt", "fertig\n");
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "--quiet", "-m", "feature"]);
    git(&repo, &["merge", "--quiet", "--ff-only", &branch]);

    let del = d.http("DELETE", &format!("/v1/sessions/{id}"), None).await;
    assert_eq!(del.status, 204, "{}", del.body);
    assert!(!wt.exists(), "Verzeichnis entfernt");
    assert!(
        git(&repo, &["branch", "--list", &branch]).is_empty(),
        "Branch gelöscht"
    );
    assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
    assert!(
        !data.join("snapshots").join(format!("{id}.git")).exists(),
        "Turn-Snapshots gelöscht"
    );
    d.daemon.shutdown().await;
}

// ---------------------------------------------------------------------------
// SES-017: Files & Suche
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ses_017_ac1_dotdot_and_symlinks_out_of_the_workspace_are_403() {
    let dir = tmp();
    let data = dir.path().join("home");
    let ws = dir.path().join("ws");
    let outside = dir.path().join("draussen");
    std::fs::create_dir_all(&data).unwrap();
    write(&ws, "src/lib.rs", "fn a() {}\n");
    write(&outside, "geheim.txt", "geheim");
    std::os::unix::fs::symlink(&outside, ws.join("raus")).unwrap();
    std::os::unix::fs::symlink(outside.join("geheim.txt"), ws.join("link.txt")).unwrap();
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    std::fs::write(ws.join(".git/config"), "[core]").unwrap();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&ws, &sc, None)).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    let base = format!("/v1/sessions/{id}/workspace");
    for path in [
        format!("{base}/files/../draussen/geheim.txt"),
        format!("{base}/files/src/../../draussen/geheim.txt"),
        format!("{base}/files/%2e%2e/draussen/geheim.txt"),
        format!("{base}/files/src%2f..%2f..%2fdraussen%2fgeheim.txt"),
        format!("{base}/files/raus/geheim.txt"),
        format!("{base}/files/link.txt"),
        format!("{base}/files/.git/config"),
        format!("{base}/tree?path=..%2Fdraussen"),
        format!("{base}/tree?path=raus"),
        format!("{base}/diff?path=..%2Fdraussen%2Fgeheim.txt&scope=turn"),
    ] {
        let res = d.http("GET", &path, None).await;
        assert_eq!(res.status, 403, "GET {path}: {}", res.body);
        assert_eq!(res.body["code"], "path_outside_workspace", "{path}");
        assert!(!res.body.to_string().contains("geheim\""), "kein Inhalt");
    }
    for path in [
        format!("{base}/files/raus/neu.txt"),
        format!("{base}/files/../draussen/neu.txt"),
        format!("{base}/files/link.txt"),
        format!("{base}/files/.git/hooks/pre-commit"),
    ] {
        let res = d
            .http("PUT", &path, Some(json!({"content": "boese"})))
            .await;
        assert_eq!(res.status, 403, "PUT {path}: {}", res.body);
    }
    assert!(!outside.join("neu.txt").exists());
    assert_eq!(
        std::fs::read_to_string(outside.join("geheim.txt")).unwrap(),
        "geheim"
    );
    assert!(!ws.join(".git/hooks").exists());
    // Innerhalb des Workspace geht es.
    let ok = d
        .http("GET", &format!("{base}/files/src/lib.rs"), None)
        .await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(ok.body["content"], "fn a() {}\n");
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_017_ac2_put_with_stale_if_match_is_412_and_leaves_the_file() {
    let dir = tmp();
    let data = dir.path().join("home");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    write(&ws, "notiz.md", "alt\n");
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&ws, &sc, None)).await;
    let id = r.body["id"].as_str().unwrap();
    let url = format!("/v1/sessions/{id}/workspace/files/notiz.md");
    let read = d.http("GET", &url, None).await;
    assert_eq!(read.status, 200, "{}", read.body);
    let etag = read.header("etag").unwrap().to_owned();
    assert_eq!(
        etag,
        format!("\"{}\"", read.body["sha256"].as_str().unwrap())
    );

    let saved = d
        .request(
            "PUT",
            &url,
            Some(json!({"content": "neu\n"})),
            &[("If-Match", &etag)],
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(saved.body["created"], false);
    // Der zweite Client hat noch den alten Stand.
    let stale = d
        .request(
            "PUT",
            &url,
            Some(json!({"content": "konflikt\n"})),
            &[("If-Match", &etag)],
        )
        .await;
    assert_eq!(stale.status, 412, "{}", stale.body);
    assert_eq!(stale.body["code"], "precondition_failed");
    assert_eq!(
        std::fs::read_to_string(ws.join("notiz.md")).unwrap(),
        "neu\n"
    );
    // Ohne `"…"`: die nackte SHA-256 aus der Spec gilt auch.
    let sha = saved.body["sha256"].as_str().unwrap().to_owned();
    let again = d
        .request(
            "PUT",
            &url,
            Some(json!({"content": "drei\n"})),
            &[("If-Match", &sha)],
        )
        .await;
    assert_eq!(again.status, 200, "{}", again.body);

    // Schreibzugriffe des Users erzeugen `fs.changed` mit `actor=user`.
    let events = d.events(id).await;
    let changes: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "fs.changed")
        .collect();
    assert_eq!(changes.len(), 2, "nur erfolgreiche Schreibzugriffe");
    assert_eq!(changes[0]["actor"]["kind"], "user");
    assert_eq!(changes[0]["payload"]["source"], "api");
    assert_eq!(
        changes[0]["payload"]["changes"],
        json!([{"path": "notiz.md", "change": "modified"}])
    );
    let created = d
        .http(
            "PUT",
            &format!("/v1/sessions/{id}/workspace/files/neu/datei.txt"),
            Some(json!({"content": "x"})),
        )
        .await;
    assert_eq!(created.status, 201, "{}", created.body);
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_017_tree_and_search() {
    let (dir, data, repo) = setup();
    write(&repo, ".gitignore", "target/\n");
    write(&repo, "target/lib.rs", "fn a() {}\n");
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&repo, &sc, None)).await;
    let id = r.body["id"].as_str().unwrap();
    let info = d
        .http("GET", &format!("/v1/sessions/{id}/workspace"), None)
        .await;
    assert_eq!(info.status, 200, "{}", info.body);
    assert_eq!(info.body, json!({"git_repo": true}));
    let tree = d
        .http("GET", &format!("/v1/sessions/{id}/workspace/tree"), None)
        .await;
    assert_eq!(tree.status, 200, "{}", tree.body);
    let names: Vec<&str> = tree.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec![".gitignore", "README.md", "src", "target"]);
    let page = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/tree?limit=2"),
            None,
        )
        .await;
    let cursor = page.body["next_cursor"].as_str().unwrap();
    let rest = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/tree?limit=2&cursor={cursor}"),
            None,
        )
        .await;
    assert_eq!(rest.body["items"][0]["name"], "src");
    let search = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/search?q=fn%20a&mode=content"),
            None,
        )
        .await;
    assert_eq!(search.status, 200, "{}", search.body);
    assert_eq!(
        search.body["items"],
        json!([{"path": "src/lib.rs", "line": 1, "column": 1, "text": "fn a() {}"}]),
        ".gitignore respektiert"
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_017_ac3_agent_write_reaches_all_clients_as_fs_changed_within_one_second() {
    let dir = tmp();
    let data = dir.path().join("home");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    write(&ws, "a.txt", "a\n");
    let d = daemon(&data).await;
    let sc = scenario(
        dir.path(),
        r#"
turns:
  - emit:
      - { message: "Ich schreibe." }
      - { write_file: { path: "neu/agent.txt", content: "vom Agent\n" }, delay_ms: 300 }
      - { message: "Fertig.", delay_ms: 3000 }
"#,
    );
    let r = d.create(session_body(&ws, &sc, None)).await;
    let id = r.body["id"].as_str().unwrap().to_owned();
    d.wait_idle(&id).await;
    let mut readers = Vec::new();
    for _ in 0..2 {
        let mut client = d.ws().await;
        client
            .send(Message::Text(
                json!({"t": "attach", "id": "a", "session_id": id, "from_seq": 0})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        readers.push(tokio::spawn(async move {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(20) {
                let Ok(Some(Ok(Message::Text(t)))) =
                    tokio::time::timeout(Duration::from_secs(5), client.next()).await
                else {
                    continue;
                };
                let m: Value = serde_json::from_str(&t).unwrap();
                for e in m["events"].as_array().into_iter().flatten() {
                    if e["type"] == "fs.changed"
                        && e["payload"]["changes"][0]["path"] == "neu/agent.txt"
                    {
                        return (Instant::now(), e.clone());
                    }
                }
            }
            panic!("kein fs.changed");
        }));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let file = ws.join("neu/agent.txt");
    let watcher = tokio::spawn(async move {
        loop {
            if file.is_file() {
                return Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    let input = d
        .http(
            "POST",
            &format!("/v1/sessions/{id}/input"),
            Some(json!({"text": "los"})),
        )
        .await;
    assert_eq!(input.status, 202, "{}", input.body);
    let written = watcher.await.unwrap();
    for reader in readers {
        let (seen, event) = reader.await.unwrap();
        let delay = seen.saturating_duration_since(written);
        assert!(delay <= Duration::from_secs(1), "fs.changed nach {delay:?}");
        assert_eq!(event["actor"]["kind"], "agent");
        assert_eq!(event["payload"]["source"], "watcher");
        assert_eq!(event["payload"]["changes"][0]["change"], "added");
        assert_eq!(event["turn_id"], input.body["turn_id"]);
    }
    d.daemon.shutdown().await;
}

// ---------------------------------------------------------------------------
// SES-018: Changes & Diffs
// ---------------------------------------------------------------------------

const THREE_FILES: &str = r#"
turns:
  - emit:
      - { write_file: { path: "README.md", content: "eins\nZWEI\ndrei\n" } }
      - { write_file: { path: "neu.txt", content: "a\nb\n" } }
      - { write_file: { path: "docs/x.md", content: "x\n" } }
      - { message: "Drei Dateien geändert." }
  - emit:
      - { write_file: { path: "src/lib.rs", content: "fn b() {}\n" } }
      - { message: "Noch eine." }
"#;

#[tokio::test]
async fn ses_018_ac1_turn_scope_lists_exactly_the_three_changed_files_with_line_counts() {
    let (dir, data, repo) = setup();
    // Eine Änderung des Users vor dem Turn gehört nicht zum Turn.
    write(&repo, "vorher.txt", "vom User\n");
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), THREE_FILES);
    let r = d.create(session_body(&repo, &sc, None)).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap().to_owned();
    d.wait_idle(&id).await;
    let first = d.turn(&id, "eins").await;
    let second = d.turn(&id, "zwei").await;

    let turn = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/changes?scope=turn&turn={first}"),
            None,
        )
        .await;
    assert_eq!(turn.status, 200, "{}", turn.body);
    assert_eq!(turn.body["turn_id"], first.as_str());
    assert_eq!(
        turn.body["items"],
        json!([
            {"path": "README.md", "status": "modified", "additions": 1, "deletions": 1},
            {"path": "docs/x.md", "status": "added", "additions": 1, "deletions": 0},
            {"path": "neu.txt", "status": "added", "additions": 2, "deletions": 0},
        ])
    );
    // Ohne `turn`: der letzte Turn.
    let last = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/changes?scope=turn"),
            None,
        )
        .await;
    assert_eq!(last.body["turn_id"], second.as_str());
    assert_eq!(last.body["items"].as_array().unwrap().len(), 1);
    assert_eq!(last.body["items"][0]["path"], "src/lib.rs");

    // Diff zeilengenau adressierbar.
    let diff = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/diff?scope=turn&turn={first}&path=README.md"),
            None,
        )
        .await;
    assert_eq!(diff.status, 200, "{}", diff.body);
    assert_eq!(diff.body["base_sha"], turn.body["base_sha"]);
    assert_eq!(diff.body["head_sha"], turn.body["head_sha"]);
    let lines = &diff.body["hunks"][0]["lines"];
    assert_eq!(
        lines[1],
        json!({"kind": "delete", "old_line": 2, "text": "zwei", "no_newline": false})
    );
    assert_eq!(
        lines[2],
        json!({"kind": "add", "new_line": 2, "text": "ZWEI", "no_newline": false})
    );
    // Die `fs.changed`-Events tragen den Turn.
    let events = d.events(&id).await;
    assert!(
        events
            .iter()
            .filter(|e| e["type"] == "fs.changed")
            .all(|e| e["turn_id"] == first.as_str() || e["turn_id"] == second.as_str())
    );
    // Uncommitted sieht alles inklusive der Änderung des Users.
    let unc = d
        .http("GET", &format!("/v1/sessions/{id}/workspace/changes"), None)
        .await;
    let paths: Vec<&str> = unc.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec![
            "README.md",
            "docs/x.md",
            "neu.txt",
            "src/lib.rs",
            "vorher.txt"
        ]
    );
    assert_eq!(
        unc.body["base_sha"],
        git(&repo, &["rev-parse", "HEAD"]).as_str()
    );
    assert!(unc.body.get("head_sha").is_none_or(Value::is_null));
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_018_ac2_branch_scope_matches_git_diff_merge_base_triple_dot() {
    let (dir, data, repo) = setup();
    let d = daemon(&data).await;
    let sc = scenario(dir.path(), "turns: []");
    let r = d.create(session_body(&repo, &sc, Some(json!({})))).await;
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap();
    let wt = path_of(&r.body);
    write(&wt, "README.md", "eins\nZWEI\ndrei\nvier\n");
    write(&wt, "neu/modul.rs", "pub fn x() {}\npub fn y() {}\n");
    std::fs::remove_file(wt.join("src/lib.rs")).unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "--quiet", "-m", "arbeit"]);
    // Die Base läuft weiter.
    write(&repo, "main.txt", "nur auf main\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "main"]);

    let res = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/changes?scope=branch"),
            None,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.body);
    let mut expected: Vec<Value> = git(&wt, &["diff", "--numstat", "main...HEAD"])
        .lines()
        .map(|l| {
            let mut p = l.split('\t');
            let add: u32 = p.next().unwrap().parse().unwrap();
            let del: u32 = p.next().unwrap().parse().unwrap();
            json!([p.next().unwrap(), add, del])
        })
        .collect();
    expected.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    let actual: Vec<Value> = res.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!([f["path"], f["additions"], f["deletions"]]))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(
        res.body["base_sha"],
        git(&wt, &["merge-base", "main", "HEAD"]).as_str()
    );
    assert_eq!(
        res.body["head_sha"],
        git(&wt, &["rev-parse", "HEAD"]).as_str()
    );

    let diff = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/diff?scope=branch&path=README.md"),
            None,
        )
        .await;
    assert_eq!(diff.status, 200, "{}", diff.body);
    let expected_patch = git(
        &wt,
        &["diff", "--no-color", "main...HEAD", "--", "README.md"],
    );
    assert_eq!(
        diff.body["patch"].as_str().unwrap().trim_end(),
        expected_patch
    );
    d.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_018_ac3_without_git_turn_diffs_work_and_branch_is_409() {
    let dir = tmp();
    let data = dir.path().join("home");
    let ws = dir.path().join("ohne-git");
    std::fs::create_dir_all(&data).unwrap();
    write(&ws, "plan.md", "alt\n");
    let d = daemon(&data).await;
    let sc = scenario(
        dir.path(),
        r#"
turns:
  - emit:
      - { write_file: { path: "plan.md", content: "neu\nmehr\n" } }
      - { message: "ok" }
"#,
    );
    let r = d.create(session_body(&ws, &sc, None)).await;
    let id = r.body["id"].as_str().unwrap().to_owned();
    d.wait_idle(&id).await;
    let turn = d.turn(&id, "los").await;
    let changes = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/changes?scope=turn&turn={turn}"),
            None,
        )
        .await;
    assert_eq!(changes.status, 200, "{}", changes.body);
    assert_eq!(
        changes.body["items"],
        json!([{"path": "plan.md", "status": "modified", "additions": 2, "deletions": 1}])
    );
    let diff = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/diff?scope=turn&path=plan.md"),
            None,
        )
        .await;
    assert_eq!(diff.status, 200, "{}", diff.body);
    assert!(
        diff.body["patch"]
            .as_str()
            .unwrap()
            .contains("-alt\n+neu\n+mehr")
    );
    for scope in ["branch", "uncommitted"] {
        let res = d
            .http(
                "GET",
                &format!("/v1/sessions/{id}/workspace/changes?scope={scope}"),
                None,
            )
            .await;
        assert_eq!(res.status, 409, "{scope}: {}", res.body);
        assert_eq!(res.body["code"], "not_a_git_repo");
    }
    let res = d
        .http(
            "GET",
            &format!("/v1/sessions/{id}/workspace/diff?scope=branch&path=plan.md"),
            None,
        )
        .await;
    assert_eq!(res.status, 409);
    // Clients erfahren vorab, dass nur die Sicht pro Turn geht (WEB-011, ohne 409).
    let info = d
        .http("GET", &format!("/v1/sessions/{id}/workspace"), None)
        .await;
    assert_eq!(info.body, json!({"git_repo": false}));
    assert!(!ws.join(".git").exists(), "kein Git im Workspace angelegt");
    d.daemon.shutdown().await;
}
