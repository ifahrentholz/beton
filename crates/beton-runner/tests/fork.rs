//! Fork ab Event X und Harness-Wechsel per Fork mit echtem Daemon und Runner (SES-006,
//! SES-007, HAR-018, HAR-019). Claude Code und Codex sind die Fake-CLI (QA-002), der
//! Verlauf der Claude-Fake-CLI liegt mit `--persist` in einem eigenen `CLAUDE_CONFIG_DIR`.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beton_core::event::{
    Actor, Empty, Event, EventPayload, MessageCompleted, MessageRole, Notice, NoticeLevel,
    SessionImported, SessionKind, SessionTitleChanged, SessionTrigger, TitleSource, TurnCompleted,
    TurnStarted,
};
use beton_core::id::{PrincipalId, SessionId, TurnId, UserId};
use beton_host::LocalProvider;
use beton_server::{Daemon, ServerConfig, start_with};
use beton_store::{NewSession, Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_beton-runner")
}

fn fake_cli() -> PathBuf {
    let fake = PathBuf::from(runner_bin()).with_file_name("beton-fake-cli");
    if !fake.is_file() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "beton-fake-cli"])
            .status()
            .unwrap();
        assert!(status.success());
    }
    fake
}

/// Testumgebung: eigenes Home, Arbeitsverzeichnis `work`, Daemon im Entwicklermodus.
struct Env {
    dir: tempfile::TempDir,
    daemon: Daemon,
    store: Store,
    token: String,
    addr: SocketAddr,
}

impl Env {
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    async fn start(vars: impl FnOnce(&Path) -> Vec<(&'static str, String)>) -> Self {
        // Kurzer Pfad: Unix-Sockets vertragen keine langen Pfade.
        let dir = tempfile::Builder::new()
            .prefix("bt")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".beton")).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        let mut cfg = ServerConfig::local(dir.path().to_path_buf());
        cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
        cfg.socket = None;
        let mut inherit: BTreeMap<String, String> = std::env::vars()
            .filter(|(k, _)| {
                !k.starts_with("BETON_") && k != "CLAUDE_CONFIG_DIR" && k != "CODEX_HOME"
            })
            .collect();
        inherit.insert("HOME".into(), home.display().to_string());
        inherit.insert(
            "BETON_HOME".into(),
            home.join(".beton").display().to_string(),
        );
        for (k, v) in vars(dir.path()) {
            inherit.insert(k.to_owned(), v);
        }
        let runners = dir.path().join("runners");
        let daemon = start_with(cfg, store.clone(), move |mut r| {
            r.sessions.provider = std::sync::Arc::new(
                LocalProvider::new(vec![runner_bin().into()], runners).with_inherited_env(inherit),
            );
            r.sessions.dev = true;
            r
        })
        .await
        .unwrap();
        let token = std::fs::read_to_string(dir.path().join("auth/local.token"))
            .unwrap()
            .trim()
            .to_owned();
        let addr = daemon.addrs[0];
        Self {
            dir,
            daemon,
            store,
            token,
            addr,
        }
    }

    async fn http(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            port = self.addr.port(),
            token = self.token,
            len = body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let payload = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        (
            status,
            serde_json::from_str(&payload).unwrap_or(Value::Null),
        )
    }

    async fn create(&self, body: Value) -> String {
        let (status, body) = self.http("POST", "/v1/sessions", Some(body)).await;
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn fork(&self, id: &str, body: Value) -> Value {
        let (status, body) = self
            .http("POST", &format!("/v1/sessions/{id}/fork"), Some(body))
            .await;
        assert_eq!(status, 201, "{body}");
        body
    }

    async fn events(&self, id: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let mut after = 0;
        loop {
            let (status, page) = self
                .http(
                    "GET",
                    &format!("/v1/sessions/{id}/events?after_seq={after}&limit=200"),
                    None,
                )
                .await;
            assert_eq!(status, 200, "{page}");
            let items = page["items"].as_array().unwrap().clone();
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
                start.elapsed() < Duration::from_secs(30),
                "Event fehlt: {}",
                serde_json::to_string_pretty(&events).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Sendet eine Eingabe und wartet auf das Ende des Turns; liefert die Texte des Agents.
    async fn turn(&self, id: &str, text: &str) -> Vec<String> {
        let before = self.events(id).await.len();
        let (status, body) = self
            .http(
                "POST",
                &format!("/v1/sessions/{id}/input"),
                Some(json!({"text": text})),
            )
            .await;
        assert_eq!(status, 202, "{body}");
        let start = Instant::now();
        loop {
            let events = self.events(id).await;
            let new = &events[before.min(events.len())..];
            if new.iter().any(|e| {
                matches!(
                    e["type"].as_str(),
                    Some("turn.completed" | "turn.failed" | "turn.interrupted")
                )
            }) {
                return new
                    .iter()
                    .filter(|e| {
                        e["type"] == "message.completed" && e["payload"]["role"] == "assistant"
                    })
                    .map(|e| {
                        e["payload"]["content"][0]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .collect();
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "Turn endet nicht: {}",
                serde_json::to_string_pretty(&events).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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

fn write(path: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    path.to_owned()
}

fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

fn one<'a>(events: &'a [Value], t: &str) -> &'a Value {
    let found = of_type(events, t);
    assert_eq!(
        found.len(),
        1,
        "{t}: {}",
        serde_json::to_string_pretty(events).unwrap()
    );
    found[0]
}

fn claude_cmd(scenario: &Path, persist: bool) -> String {
    format!(
        "{} --protocol stream-json --scenario {}{}",
        fake_cli().display(),
        scenario.display(),
        if persist { " --persist" } else { "" }
    )
}

fn codex_cmd(scenario: &Path) -> String {
    // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
    let link = scenario.with_file_name("codex");
    if !link.exists() {
        std::os::unix::fs::symlink(fake_cli(), &link).unwrap();
    }
    format!(
        "{} --protocol app-server --scenario {}",
        link.display(),
        scenario.display()
    )
}

const CONTENT: [&str; 10] = [
    "message.completed",
    "reasoning.completed",
    "tool.call.requested",
    "tool.call.started",
    "tool.call.completed",
    "turn.started",
    "turn.completed",
    "turn.failed",
    "turn.interrupted",
    "fs.changed",
];

fn content(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|e| CONTENT.contains(&e["type"].as_str().unwrap_or_default()))
        .collect()
}

fn user_actor() -> Actor {
    Actor::User {
        id: PrincipalId::User(UserId::LOCAL),
        device_id: None,
    }
}

fn text_message(role: MessageRole, text: &str) -> EventPayload {
    EventPayload::MessageCompleted(MessageCompleted {
        message_id: format!("m_{}", TurnId::new()),
        role,
        content: vec![json!({"type": "text", "text": text})],
        author: (role == MessageRole::User).then_some(PrincipalId::User(UserId::LOCAL)),
    })
}

/// Legt eine Session ohne Runner direkt im Store an und hängt die Events an.
async fn stored_session(
    env: &Env,
    harness: &str,
    opts: Value,
    events: Vec<EventPayload>,
) -> String {
    let local = env.store.ensure_local().await.unwrap();
    let id = SessionId::new();
    env.store
        .create_session(
            local.org,
            NewSession {
                id,
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: harness.into(),
                cwd: env.work().display().to_string(),
                model: None,
                agent_ref: None,
                project_id: None,
                parent_id: None,
                trigger: SessionTrigger::Api,
                home_node: local.node,
                harness_opts: opts,
            },
        )
        .await
        .unwrap();
    let mut turn = TurnId::new();
    let batch: Vec<Event> = events
        .into_iter()
        .map(|p| {
            if matches!(p, EventPayload::TurnStarted(_)) {
                turn = TurnId::new();
            }
            let actor = match &p {
                EventPayload::MessageCompleted(m) if m.role == MessageRole::User => user_actor(),
                _ => Actor::default(),
            };
            let mut e = Event::new(id, 0, actor, p);
            e.turn_id = Some(turn);
            e
        })
        .collect();
    let record = env.store.session(local.org, id).await.unwrap();
    env.store
        .append(local.org, id, record.head_seq, record.epoch, batch)
        .await
        .unwrap();
    id.to_string()
}

fn started() -> EventPayload {
    EventPayload::TurnStarted(TurnStarted {
        turn_id: TurnId::new(),
        input_id: None,
        author: PrincipalId::User(UserId::LOCAL),
    })
}

fn completed() -> EventPayload {
    EventPayload::TurnCompleted(TurnCompleted {
        turn_id: TurnId::new(),
        stop_reason: "end_turn".into(),
        usage_summary: json!({}),
    })
}

/// Session mit genau 200 Events: `session.created`, drei Verwaltungs-Events, dann 49 Turns zu
/// je vier Events (Nutzer, `turn.started`, Antwort, `turn.completed`). Turn k endet bei
/// `seq` 4 + 4k, Turn 29 also bei 120.
async fn session_with_200_events(env: &Env) -> String {
    let sc = write(
        &env.path("fork.yaml"),
        "turns: [{ emit: [{ echo_input: true }] }]\n",
    );
    let mut events = vec![
        EventPayload::SessionTitleChanged(SessionTitleChanged {
            title: "Quelle".into(),
            source: TitleSource::User,
        }),
        EventPayload::Notice(Notice {
            level: NoticeLevel::Info,
            text: "Hinweis".into(),
        }),
        EventPayload::SessionUnarchived(Empty {}),
    ];
    for k in 1..=49 {
        events.push(text_message(MessageRole::User, &format!("Frage {k}")));
        events.push(started());
        events.push(text_message(
            MessageRole::Assistant,
            &format!("Antwort {k}"),
        ));
        events.push(completed());
    }
    let id = stored_session(env, "fake", json!({"scenario": sc}), events).await;
    assert_eq!(env.events(&id).await.len(), 200);
    id
}

#[tokio::test]
async fn ses_006_ac1_fork_copies_content_events_and_leaves_source_unchanged() {
    let env = Env::start(|_| Vec::new()).await;
    let source = session_with_200_events(&env).await;
    let before = env.events(&source).await;

    let body = env.fork(&source, json!({"at_seq": 120})).await;
    assert_eq!(body["effective_seq"], 120);
    assert_eq!(body["workspace"], "shared", "kein Git-Repository: shared");
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    assert_eq!(body["session"]["title"], "Quelle (Fork)");

    let events = env
        .wait_for(&child, |e| e["type"] == "session.forked")
        .await;
    // Inhalts-Events 1…120 der Quelle, neu nummeriert und in derselben Reihenfolge.
    let expected: Vec<&Value> = content(&before)
        .into_iter()
        .filter(|e| e["seq"].as_u64().unwrap() <= 120)
        .collect();
    let copied = content(&events);
    assert_eq!(copied.len(), expected.len());
    assert_eq!(expected.len(), 116);
    for (c, e) in copied.iter().zip(&expected) {
        assert_eq!(c["type"], e["type"]);
        assert_eq!(c["payload"], e["payload"]);
        assert_eq!(c["session_id"], child.as_str());
        assert_ne!(c["id"], e["id"], "neue Event-IDs");
    }
    for (i, e) in events.iter().enumerate() {
        assert_eq!(e["seq"].as_u64().unwrap(), i as u64 + 1, "lückenlos");
    }
    let forked = one(&events, "session.forked");
    assert_eq!(forked["payload"]["from_session"], source.as_str());
    assert_eq!(forked["payload"]["at_seq"], 120);

    // Die Quelle hat genau ein Event mehr: `session.fork_created`.
    let after = env.events(&source).await;
    assert_eq!(after.len(), 201);
    assert_eq!(&after[..200], &before[..]);
    assert_eq!(after[200]["type"], "session.fork_created");
    assert_eq!(after[200]["payload"]["child"], child.as_str());
    assert_eq!(after[200]["payload"]["at_seq"], 120);
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_006_ac2_fork_mid_turn_starts_at_previous_turn_end() {
    let env = Env::start(|_| Vec::new()).await;
    let source = session_with_200_events(&env).await;
    // seq 121 ist die Frage von Turn 30, 122 sein `turn.started`.
    for at in [121, 122, 123] {
        let body = env.fork(&source, json!({"at_seq": at})).await;
        assert_eq!(body["effective_seq"], 120, "at_seq {at}");
    }
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{source}/fork"),
            Some(json!({"at_seq": 999})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    env.daemon.shutdown().await;
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

#[tokio::test]
async fn ses_006_ac3_new_worktree_isolates_the_fork() {
    let env = Env::start(|_| Vec::new()).await;
    let repo = env.work();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo.join("README.md"), "Projekt\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    // Uncommittete Arbeit der Quelle: kommt als WIP-Snapshot in den Fork.
    write(&repo.join("wip.txt"), "in Arbeit\n");
    let sc = write(
        &env.path("source.yaml"),
        "turns: [{ emit: [{ message: \"erledigt\" }] }]\n",
    );
    let source = env
        .create(json!({"target": "fake", "cwd": repo, "harness_opts": {"scenario": sc}}))
        .await;
    env.turn(&source, "los").await;
    let fork_sc = write(
        &env.path("fork.yaml"),
        "turns: [{ emit: [{ write_file: { path: fork.txt, content: \"nur im Fork\" } }, { message: \"geschrieben\" }] }]\n",
    );
    let body = env
        .fork(&source, json!({"harness_opts": {"scenario": fork_sc}}))
        .await;
    assert_eq!(body["workspace"], "new_worktree", "Default im Repository");
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    let wt = body["session"]["worktree"].clone();
    let branch = wt["branch"].as_str().unwrap();
    let path = PathBuf::from(wt["path"].as_str().unwrap());
    assert!(
        git(&repo, &["branch", "--list", branch]).contains(branch),
        "neuer Branch {branch}"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("wip.txt")).unwrap(),
        "in Arbeit\n",
        "WIP-Snapshot der Quelle"
    );
    assert!(
        git(&repo, &["status", "--porcelain"]).contains("wip.txt"),
        "Quelle unverändert"
    );
    env.turn(&child, "schreib").await;
    assert!(path.join("fork.txt").is_file());
    assert!(
        !repo.join("fork.txt").exists(),
        "im Worktree der Quelle unsichtbar"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_018_ac1_claude_session_forked_to_codex_gets_the_preamble() {
    // Deckt auch SES-007 AC1 (Codex-Runner, `preamble`, Handover im ersten Turn), HAR-018 AC4
    // (`session.forked`) und die Server-Seite von SES-007 AC5 (Fork ab dem letzten `seq`,
    // Quelle bleibt auf ihrem Harness) ab.
    let env = Env::start(|dir| {
        let claude = write(
            &dir.join("claude.yaml"),
            "turns:\n  - emit:\n      - { tool_call: { name: Edit, kind: file_edit, args: { file_path: src/rate_limit.rs } } }\n      - { tool_result: \"ok\" }\n      - { message: \"Rate-Limiter steht, Tests grün.\" }\n",
        );
        let codex = write(
            &dir.join("codex.yaml"),
            "turns: [{ emit: [{ echo_input: true }] }]\n",
        );
        vec![
            ("BETON_CLAUDE_PATH", claude_cmd(&claude, false)),
            ("BETON_CODEX_PATH", codex_cmd(&codex)),
        ]
    })
    .await;
    let source = env
        .create(json!({"target": "claude", "cwd": env.work(), "title": "Rate-Limiter"}))
        .await;
    env.turn(
        &source,
        "Baue einen Rate-Limiter für die Login-Route: 5 Versuche pro Minute.",
    )
    .await;
    let head = env.events(&source).await.len() as u64;

    // „Weiter mit Codex“: Fork ab dem letzten `seq`.
    let body = env.fork(&source, json!({"harness": "codex"})).await;
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    assert_eq!(body["session"]["harness"], "codex");
    assert_eq!(body["session"]["title"], "Rate-Limiter (Codex)");
    assert!(body["effective_seq"].as_u64().unwrap() <= head);
    let events = env
        .wait_for(&child, |e| e["type"] == "session.started")
        .await;
    let forked = one(&events, "session.forked");
    assert_eq!(
        forked["payload"],
        json!({
            "from_session": source, "at_seq": body["effective_seq"], "reason": "user",
            "harness": "codex", "from_harness": "claude", "history_mode": "preamble",
        })
    );
    assert_eq!(
        one(&events, "session.started")["payload"]["harness"],
        "codex"
    );
    let doc = env
        .work()
        .join(".beton/handover")
        .join(format!("{child}.md"));
    let doc = std::fs::read_to_string(doc).unwrap();
    assert!(doc.contains("src/rate_limit.rs"), "{doc}");

    // Der erste Turn erhält die Präambel: Ziel und letzte Änderungen.
    let reply = env
        .turn(&child, "Bitte prüfe den Limiter hinter einem Proxy.")
        .await;
    let reply = reply.join("\n");
    assert!(reply.starts_with("[beton · Übergabe]"), "{reply}");
    assert!(reply.contains("5 Versuche pro Minute"), "Ziel: {reply}");
    assert!(
        reply.contains("src/rate_limit.rs"),
        "letzte Änderungen: {reply}"
    );
    assert!(
        reply.contains(&format!(".beton/handover/{child}.md")),
        "{reply}"
    );
    assert!(
        reply.ends_with("Bitte prüfe den Limiter hinter einem Proxy."),
        "{reply}"
    );

    // Die Quelle bleibt auf Claude Code.
    let (_, s) = env
        .http("GET", &format!("/v1/sessions/{source}"), None)
        .await;
    assert_eq!(s["harness"], "claude");
    env.daemon.shutdown().await;
}

/// Claude-Fake-CLI mit Verlauf: jeder Turn nennt die bekannten Nutzer-Nachrichten und spiegelt
/// dann die Eingabe.
fn persisted_claude(dir: &Path) -> Vec<(&'static str, String)> {
    let sc = write(
        &dir.join("claude.yaml"),
        &format!(
            "turns:\n{}",
            "  - emit: [{ echo_history: true }, { echo_input: true }]\n".repeat(8)
        ),
    );
    let config = dir.join("claude-config");
    std::fs::create_dir_all(&config).unwrap();
    vec![
        ("BETON_CLAUDE_PATH", claude_cmd(&sc, true)),
        ("CLAUDE_CONFIG_DIR", config.display().to_string()),
    ]
}

/// Quelle mit fünf Turns; liefert die `seq` der Turn-Enden.
async fn five_turns(env: &Env) -> (String, Vec<u64>) {
    let source = env
        .create(json!({"target": "claude", "cwd": env.work(), "title": "Fünf Turns"}))
        .await;
    for k in 1..=5 {
        let reply = env.turn(&source, &format!("Turn {k}")).await;
        assert_eq!(reply.last().unwrap(), &format!("Turn {k}"));
    }
    let ends = of_type(&env.events(&source).await, "turn.completed")
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    (source, ends)
}

#[tokio::test]
async fn har_019_ac1_fork_after_turn_3_of_5_knows_only_turns_1_to_3() {
    let env = Env::start(persisted_claude).await;
    let (source, ends) = five_turns(&env).await;
    let body = env.fork(&source, json!({"at_seq": ends[2]})).await;
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    let events = env
        .wait_for(&child, |e| e["type"] == "session.forked")
        .await;
    let forked = one(&events, "session.forked");
    assert_eq!(forked["payload"]["history_mode"], "rebuild");
    assert!(forked["payload"].get("fallback_reason").is_none());
    let reply = env.turn(&child, "Und jetzt?").await;
    assert_eq!(
        reply[0], "Turn 1\nTurn 2\nTurn 3",
        "nativer Verlauf bis Turn 3"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_007_ac2_fork_on_the_same_harness_uses_native_history_without_handover() {
    // Deckt auch HAR-018 AC4 für `native` und `rebuild` ab.
    let env = Env::start(persisted_claude).await;
    let (source, ends) = five_turns(&env).await;

    // Am Ende der Quelle: Vendor-Fork (`--resume --fork-session`).
    let body = env.fork(&source, json!({})).await;
    let native = body["session"]["id"].as_str().unwrap().to_owned();
    let events = env
        .wait_for(&native, |e| e["type"] == "session.forked")
        .await;
    let forked = one(&events, "session.forked");
    assert_eq!(forked["payload"]["history_mode"], "native");
    assert_eq!(forked["payload"]["harness"], "claude");
    assert_eq!(forked["payload"]["from_harness"], "claude");
    let reply = env.turn(&native, "Weiter").await;
    assert_eq!(reply[0], "Turn 1\nTurn 2\nTurn 3\nTurn 4\nTurn 5");
    assert_eq!(reply[1], "Weiter", "kein Handover-Text");

    // Mitten in der Quelle: Rebuild.
    let body = env.fork(&source, json!({"at_seq": ends[1]})).await;
    let rebuilt = body["session"]["id"].as_str().unwrap().to_owned();
    let events = env
        .wait_for(&rebuilt, |e| e["type"] == "session.forked")
        .await;
    assert_eq!(
        one(&events, "session.forked")["payload"]["history_mode"],
        "rebuild"
    );
    let reply = env.turn(&rebuilt, "Weiter").await;
    assert_eq!(reply, ["Turn 1\nTurn 2", "Weiter"]);
    assert!(
        !env.work().join(".beton/handover").exists(),
        "kein Handover-Dokument"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_019_ac3_failed_rebuild_falls_back_to_preamble_with_reason() {
    let env = Env::start(|dir| {
        let mut vars = persisted_claude(dir);
        // Konfigurationsverzeichnis ist eine Datei: Die Session-Datei lässt sich nicht anlegen.
        let blocked = write(&dir.join("blocked"), "keine Verzeichnis");
        vars.retain(|(k, _)| *k != "CLAUDE_CONFIG_DIR");
        vars.push(("CLAUDE_CONFIG_DIR", blocked.display().to_string()));
        vars
    })
    .await;
    let source = env
        .create(json!({"target": "claude", "cwd": env.work(), "title": "Quelle"}))
        .await;
    env.turn(&source, "Turn 1").await;
    env.turn(&source, "Turn 2").await;
    let first_end = of_type(&env.events(&source).await, "turn.completed")[0]["seq"]
        .as_u64()
        .unwrap();
    let body = env.fork(&source, json!({"at_seq": first_end})).await;
    let child = body["session"]["id"].as_str().unwrap().to_owned();
    let events = env
        .wait_for(&child, |e| e["type"] == "session.started")
        .await;
    let forked = one(&events, "session.forked");
    assert_eq!(forked["payload"]["history_mode"], "preamble");
    let reason = forked["payload"]["fallback_reason"].as_str().unwrap();
    assert!(!reason.is_empty());
    assert!(
        of_type(&events, "notice")
            .iter()
            .any(|n| n["payload"]["level"] == "warn"
                && n["payload"]["text"].as_str().unwrap().contains("Präambel")),
        "Hinweis-Event"
    );
    let reply = env.turn(&child, "Weiter").await;
    assert!(
        reply.last().unwrap().starts_with("[beton · Übergabe]"),
        "{reply:?}"
    );
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn har_019_ac2_imported_claude_session_resumes_via_rebuild() {
    let env = Env::start(persisted_claude).await;
    // Log wie nach einem Import (HAR-023): normalisierte Events ohne `raw`, keine native
    // Referenz von beton.
    let id = stored_session(
        &env,
        "claude",
        Value::Null,
        vec![
            EventPayload::SessionImported(SessionImported {
                source: "claude".into(),
                vendor_session_id: "11111111-2222-4333-8444-555555555555".into(),
                imported_at: beton_core::time::Timestamp::now(),
            }),
            text_message(MessageRole::User, "Importierte Frage A"),
            text_message(MessageRole::Assistant, "Antwort A"),
            text_message(MessageRole::User, "Importierte Frage B"),
            text_message(MessageRole::Assistant, "Antwort B"),
        ],
    )
    .await;
    let reply = env.turn(&id, "Weiter").await;
    assert_eq!(reply[0], "Importierte Frage A\nImportierte Frage B");
    assert_eq!(reply[1], "Weiter");
    let events = env.events(&id).await;
    assert_eq!(one(&events, "session.resumed")["payload"]["mode"], "native");
    env.daemon.shutdown().await;
}

#[tokio::test]
async fn ses_007_fork_to_a_harness_without_history_support_is_rejected() {
    let env = Env::start(|_| Vec::new()).await;
    let sc = write(&env.path("s.yaml"), "turns: []\n");
    let source = env
        .create(json!({"target": "fake", "cwd": env.work(), "harness_opts": {"scenario": sc}}))
        .await;
    let (status, body) = env
        .http(
            "POST",
            &format!("/v1/sessions/{source}/fork"),
            Some(json!({"harness": "acp:unbekannt"})),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "harness_incompatible");
    assert_eq!(
        env.events(&source)
            .await
            .iter()
            .filter(|e| e["type"] == "session.fork_created")
            .count(),
        0
    );
    env.daemon.shutdown().await;
}
