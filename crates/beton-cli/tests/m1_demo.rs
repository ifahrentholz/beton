//! M1-Demo als automatisierter End-to-End-Test (Milestone „M1 — Meta-Harness“, WP-27):
//!
//! 1. Eine Session startet auf Claude Code, wird auf Codex geforkt und dort weitergeführt
//!    (SES-006, SES-007, HAR-018).
//! 2. `beton run maestra` lässt Claude und Codex parallel in eigenen Worktrees implementieren
//!    und vom jeweils anderen Vendor reviewen (AGT-009, AGT-011).
//! 3. Vorhandene Claude- und Codex-Chats lassen sich importieren (SES-008, HAR-023, HAR-024).
//!
//! Gegen `beton serve --dev` als eigenen Prozess, mit der Fake-Vendor-CLI (QA-002) für `claude`
//! und `codex`. Alle Vendor-Verzeichnisse (`HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`) sind
//! temporär und enthalten nur synthetische Fixtures; kein API-Key, kein Netz außer Loopback.

#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Serve, beton, fake_cli, run, stderr, stdout};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const CLAUDE_CHAT: &str = "3f1c2a9e-5b7d-4c8e-9a1f-2d3e4f5a6b7c";
const CODEX_CHAT: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=Demo",
            "-c",
            "user.email=demo@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn call(tool: &str, args: Value) -> Value {
    json!({"mcp_call": {"server": "beton", "tool": tool, "args": args}})
}

fn turn(input: Option<&str>, emit: Vec<Value>) -> Value {
    match input {
        Some(i) => json!({"expect_input": i, "emit": emit}),
        None => json!({"emit": emit}),
    }
}

fn usage() -> Value {
    json!({"usage": {"input_tokens": 400, "output_tokens": 60}})
}

/// Vendor-Verzeichnisse mit je einem synthetischen Chat (Fixtures der Adapter-Crates).
fn vendor_dirs(root: &Path, cwd: &Path) {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let claude = std::fs::read_to_string(crates().join(
        "beton-harness-claude/tests/golden/import/claude-1.0.98-interrupt-mcp/session.jsonl",
    ))
    .unwrap()
    .replace("/beton-golden/workdir", &cwd.display().to_string());
    write(
        &root.join(format!("claude/projects/{slug}/{CLAUDE_CHAT}.jsonl")),
        &claude,
    );
    let codex = std::fs::read_to_string(
        crates()
            .join("beton-harness-codex/tests/golden/import/codex-0.46.0-patch-abort/rollout.jsonl"),
    )
    .unwrap()
    .replace("/beton-golden/workdir", &cwd.display().to_string());
    write(
        &root.join(format!(
            "codex/sessions/2026/09/30/rollout-2026-09-30T08-15-22-{CODEX_CHAT}.jsonl"
        )),
        &codex,
    );
}

/// Szenarien der Fake-CLIs: Claude spielt die Ausgangs-Session, maestra, den Claude-
/// Implementer und den Claude-Reviewer; Codex den Codex-Implementer, den Codex-Reviewer und die
/// geforkte Session. Turns werden nach der Eingabe gewählt.
fn scenarios(dir: &Path) -> (PathBuf, PathBuf) {
    let wait = |a: &str, b: &str| {
        call(
            "session_wait",
            json!({"ids": [a, b], "mode": "all", "timeout": "50s"}),
        )
    };
    let maestra = turn(
        Some("Baue einen Rate-Limiter mit Tests"),
        vec![
            call("session_list", json!({})),
            json!({"message": "Plan: 1. Limiter (impl-claude) 2. Tests (impl-codex)"}),
            call(
                "session_spawn",
                json!({"agent": "impl-claude", "prompt": "Teilaufgabe 1: Limiter", "async": true, "worktree": "new"}),
            ),
            call(
                "session_spawn",
                json!({"agent": "impl-codex", "prompt": "Teilaufgabe 2: Tests", "async": true, "worktree": "new"}),
            ),
            wait("${mcp.1.session_id}", "${mcp.2.session_id}"),
            call(
                "session_spawn",
                json!({"agent": "review-codex", "prompt": "Review Teilaufgabe 1", "async": true}),
            ),
            call(
                "session_spawn",
                json!({"agent": "review-claude", "prompt": "Review Teilaufgabe 2", "async": true}),
            ),
            wait("${mcp.4.session_id}", "${mcp.5.session_id}"),
            json!({"message": "| Teilaufgabe | Implementiert von | Review von | Status |\n| Limiter | impl-claude (Claude Code) | review-codex (Codex) | approved |\n| Tests | impl-codex (Codex) | review-claude (Claude Code) | approved |"}),
        ],
    );
    let claude = json!({
        "select": "by_input",
        "turns": [
            turn(Some("Baue einen Rate-Limiter"), vec![json!({"message": "Rate-Limiter steht (Token-Bucket)."})]),
            maestra,
            turn(
                Some("Teilaufgabe 1: Limiter"),
                vec![
                    json!({"write_file": {"path": "src/limiter.rs", "content": "pub struct Limiter;\n"}}),
                    json!({"message": "BRANCH: limiter", "delay_ms": 6000}),
                ],
            ),
            turn(Some("Review Teilaufgabe 2"), vec![json!({"message": "VERDICT: approve\nBLOCKING:\n- keine"})]),
        ],
    });
    let codex = json!({
        "select": "by_input",
        "turns": [
            turn(
                Some("Teilaufgabe 2: Tests"),
                vec![
                    json!({"write_file": {"path": "tests/limiter.rs", "content": "#[test] fn limits() {}\n"}}),
                    json!({"message": "BRANCH: tests", "delay_ms": 1500}),
                    usage(),
                ],
            ),
            turn(Some("Review Teilaufgabe 1"), vec![json!({"message": "VERDICT: approve\nBLOCKING:\n- keine"}), usage()]),
            // Die geforkte Session: erste Eingabe mit Übergabe-Präambel, danach weiter.
            turn(None, vec![json!({"message": "Übernommen auf Codex: Tests folgen."}), usage()]),
            turn(None, vec![json!({"message": "Tests für den Rate-Limiter ergänzt."}), usage()]),
        ],
    });
    let c = dir.join("claude.yaml");
    std::fs::write(&c, claude.to_string()).unwrap();
    let x = dir.join("codex.yaml");
    std::fs::write(&x, codex.to_string()).unwrap();
    (c, x)
}

/// REST gegen den Daemon mit dem lokalen Token (Testaufbau).
async fn api(serve: &Serve, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let token = std::fs::read_to_string(serve.home().join("auth/local.token")).unwrap();
    let mut stream = tokio::net::TcpStream::connect(&serve.info.http)
        .await
        .unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        host = serve.info.http,
        token = token.trim(),
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
        let mut out = String::new();
        let mut rest = payload;
        while let Some((size, tail)) = rest.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if n == 0 {
                break;
            }
            out.push_str(&tail[..n.min(tail.len())]);
            rest = tail.get(n + 2..).unwrap_or("");
        }
        out
    } else {
        payload.to_owned()
    };
    (
        status,
        serde_json::from_str(&payload).unwrap_or(Value::Null),
    )
}

async fn events(serve: &Serve, id: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut after = 0;
    loop {
        let (status, page) = api(
            serve,
            "GET",
            &format!("/v1/sessions/{id}/events?after_seq={after}&limit=200"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        let items = page["items"].as_array().unwrap().clone();
        let Some(last) = items.last() else {
            return out;
        };
        after = last["seq"].as_u64().unwrap();
        out.extend(items);
    }
}

async fn wait_for(
    serve: &Serve,
    id: &str,
    what: &str,
    pred: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let start = Instant::now();
    loop {
        let ev = events(serve, id).await;
        if pred(&ev) {
            return ev;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "{what}: {}\n{}",
            serde_json::to_string_pretty(&ev).unwrap(),
            serve.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

fn texts(events: &[Value], role: &str) -> Vec<String> {
    of_type(events, "message.completed")
        .into_iter()
        .filter(|e| e["payload"]["role"] == role)
        .map(|e| {
            e["payload"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["text"].as_str())
                .collect::<String>()
        })
        .collect()
}

/// Ein Kommando im Projekt, ohne `*_API_KEY` in der Umgebung (ADR-0034).
fn cli(serve: &Serve, env: &[(&str, String)]) -> Command {
    let mut c = beton(serve.home());
    c.current_dir(serve.work.path());
    for (k, _) in std::env::vars() {
        if k.ends_with("_API_KEY") {
            c.env_remove(k);
        }
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

#[tokio::test]
async fn m1_demo_fork_to_codex_maestra_across_vendors_and_import() {
    let vendor = tempfile::tempdir().unwrap();
    let (claude_s, codex_s) = scenarios(vendor.path());
    // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
    let link = vendor.path().join("bin/codex");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fake_cli(), &link).unwrap();
    let env: Vec<(&str, String)> = vec![
        (
            "BETON_CLAUDE_PATH",
            format!(
                "{} --protocol stream-json --scenario {}",
                fake_cli().display(),
                claude_s.display()
            ),
        ),
        (
            "BETON_CODEX_PATH",
            format!(
                "{} --protocol app-server --scenario {}",
                link.display(),
                codex_s.display()
            ),
        ),
        ("HOME", vendor.path().join("home").display().to_string()),
        (
            "CLAUDE_CONFIG_DIR",
            vendor.path().join("claude").display().to_string(),
        ),
        (
            "CODEX_HOME",
            vendor.path().join("codex").display().to_string(),
        ),
    ];
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    std::fs::create_dir_all(vendor.path().join("home")).unwrap();
    // Projekt als Git-Repository: maestra und der Fork arbeiten in eigenen Worktrees.
    let serve = Serve::start_without_api_keys(&pairs);
    let work = serve.work.path().to_path_buf();
    git(&work, &["init", "-q", "-b", "main"]);
    write(&work.join("README.md"), "# Demo\n");
    git(&work, &["add", "README.md"]);
    git(&work, &["commit", "-q", "-m", "Start"]);
    vendor_dirs(vendor.path(), &work);

    // ---------------------------------------------------------------- 1. Claude → Fork auf Codex
    let out = run(cli(&serve, &env).args(["run", "claude", "-p", "Baue einen Rate-Limiter"]));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Rate-Limiter steht (Token-Bucket).\n");
    let client = serve.client();
    let sessions = client.all_sessions(false).await.unwrap();
    let source = sessions.iter().find(|s| s["harness"] == "claude").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let forked = client
        .fork_session(&source, &json!({"harness": "codex"}))
        .await
        .unwrap();
    let fork = forked["session"]["id"].as_str().unwrap().to_owned();
    assert_eq!(forked["session"]["harness"], "codex");
    client.input(&fork, "Weiter: Tests ergänzen").await.unwrap();
    let ev = wait_for(&serve, &fork, "Turn auf Codex", |ev| {
        texts(ev, "assistant")
            .iter()
            .any(|t| t.starts_with("Übernommen auf Codex"))
    })
    .await;
    let marker = of_type(&ev, "session.forked");
    assert_eq!(marker[0]["payload"]["from_session"], json!(source));
    assert_eq!(marker[0]["payload"]["from_harness"], "claude");
    assert_eq!(marker[0]["payload"]["harness"], "codex");
    // Der übernommene Verlauf steht im Fork, Codex antwortet weiter.
    assert!(
        texts(&ev, "assistant")
            .iter()
            .any(|t| t == "Rate-Limiter steht (Token-Bucket).")
    );
    assert!(
        texts(&ev, "assistant")
            .iter()
            .any(|t| t.starts_with("Übernommen auf Codex")),
        "{:?}",
        texts(&ev, "assistant")
    );
    client.input(&fork, "Und die Doku?").await.unwrap();
    wait_for(&serve, &fork, "zweiter Turn auf Codex", |ev| {
        texts(ev, "assistant")
            .iter()
            .any(|t| t == "Tests für den Rate-Limiter ergänzt.")
    })
    .await;

    // ---------------------------------------------------------------- 2. beton run maestra
    let out =
        run(cli(&serve, &env).args(["run", "maestra", "-p", "Baue einen Rate-Limiter mit Tests"]));
    assert!(out.status.success(), "{}\n{}", stderr(&out), serve.log());
    let summary = stdout(&out);
    assert!(
        summary
            .contains("| Limiter | impl-claude (Claude Code) | review-codex (Codex) | approved |"),
        "{summary}"
    );
    // Die CLI nennt die Session-URL auf stderr (API-006).
    let err = stderr(&out);
    let maestra = err
        .split("/s/")
        .nth(1)
        .and_then(|rest| {
            rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
        })
        .unwrap_or_else(|| panic!("Session-URL fehlt: {err}"))
        .to_owned();
    let (status, tree) = api(
        &serve,
        "GET",
        &format!("/v1/sessions/{maestra}/subagents"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{tree}");
    let nodes = tree["nodes"].as_array().unwrap();
    let by_agent = |a: &str| {
        nodes
            .iter()
            .find(|n| n["agent"] == a)
            .unwrap_or_else(|| panic!("{a}: {tree}"))
    };
    assert_eq!(nodes[0]["agent"], "maestra");
    // Claude implementiert, Codex reviewt (und umgekehrt), jede Implementierung im eigenen Worktree.
    let (ic, ix) = (by_agent("impl-claude"), by_agent("impl-codex"));
    assert_eq!(
        (ic["harness"].as_str(), ix["harness"].as_str()),
        (Some("claude"), Some("codex"))
    );
    assert_eq!(by_agent("review-codex")["harness"], "codex");
    assert_eq!(by_agent("review-claude")["harness"], "claude");
    let wt_c = PathBuf::from(ic["worktree"]["path"].as_str().unwrap());
    let wt_x = PathBuf::from(ix["worktree"]["path"].as_str().unwrap());
    assert_ne!(wt_c, wt_x);
    assert!(wt_c.join("src/limiter.rs").is_file());
    assert!(wt_x.join("tests/limiter.rs").is_file());
    assert!(
        !work.join("src/limiter.rs").exists(),
        "maestra schreibt nicht selbst"
    );
    let worktrees = git(&work, &["worktree", "list"]);
    for b in [&ic["worktree"]["branch"], &ix["worktree"]["branch"]] {
        assert!(worktrees.contains(b.as_str().unwrap()), "{worktrees}");
    }
    // Parallel: beide Implementierungen gestartet, bevor eine fertig war.
    let ev = events(&serve, &maestra).await;
    let order: Vec<&str> = ev
        .iter()
        .filter_map(|e| match e["type"].as_str() {
            Some(t @ ("agent.spawned" | "agent.completed")) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(&order[..2], ["agent.spawned", "agent.spawned"], "{order:?}");
    // Ohne API-Key: alle Childs laufen über den Login der Vendor-CLI.
    for n in &nodes[1..] {
        assert_eq!(n["auth_source"], "vendor_cli", "{n}");
    }

    // ---------------------------------------------------------------- 3. Import
    for (harness, chat) in [("claude", CLAUDE_CHAT), ("codex", CODEX_CHAT)] {
        let (status, page) = api(
            &serve,
            "GET",
            &format!("/v1/imports/candidates?harness={harness}"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        assert_eq!(page["items"][0]["vendor_session_id"], chat, "{page}");
        let (status, body) = api(
            &serve,
            "POST",
            "/v1/imports",
            Some(json!({"harness": harness, "refs": [chat]})),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["results"][0]["status"], "imported", "{body}");
        let id = body["results"][0]["session_id"].as_str().unwrap();
        let imported = events(&serve, id).await;
        assert_eq!(imported[1]["type"], "session.imported");
        assert_eq!(imported[1]["payload"]["source"], harness);
        assert!(!texts(&imported, "user").is_empty());
        let s = client.session(id).await.unwrap();
        assert_eq!(s["harness"], harness);
    }
}
