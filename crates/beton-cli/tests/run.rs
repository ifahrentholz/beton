//! Integrationstests für `run`, `resume`, `attach` und `session` (CLI-002, CLI-003, API-006,
//! HAR-026 AC1, RUN-002 AC1, SES-002 AC3). Echte Vendor-CLIs kommen nicht vor: `claude`
//! ist die Fake-CLI über `BETON_CLAUDE_PATH` (QA-002), sonst der Fake-Harness.

#![allow(clippy::unwrap_used)]

mod common;

use std::io::{BufRead as _, Write as _};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use beton_sdk::{Client, DaemonInfo};
use common::{
    BackgroundDaemon, Serve, background_home, beton, events, fake_claude, run, status, stderr,
    stdout, wait_until,
};
use serde_json::{Value, json};

const HELLO: &str = "turns:\n  - expect_input: \"sag hallo\"\n    emit:\n      - { message_delta: \"Hallo!\", chunk: 3 }\n";

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

// --------------------------------------------------------------------------- CLI-002 / API-006

#[tokio::test]
async fn cli_002_ac1_run_claude_starts_the_daemon_and_streams_the_answer() {
    let home = background_home();
    let project = tempfile::tempdir().unwrap();
    let _daemon = BackgroundDaemon(home.path().to_path_buf());
    let scenario = write(project.path(), "hallo.yaml", HELLO);
    assert!(DaemonInfo::read(home.path()).is_none(), "noch kein Daemon");

    let mut child = beton(home.path())
        .current_dir(project.path())
        .env("BETON_CLAUDE_PATH", fake_claude(&scenario))
        .args(["run", "claude", "-p", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"sag hallo\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    // API-006 AC1: stdout nur der Antworttext, stderr mit Session-URL.
    assert_eq!(stdout(&out), "Hallo!\n");
    let err = stderr(&out);
    let info = DaemonInfo::read(home.path()).expect("Daemon läuft");
    assert!(
        err.contains(&format!("http://{}/s/ses_", info.http)),
        "{err}"
    );

    // Die Session ist über die API (und damit in der Web-UI) sichtbar.
    let client = Client::local(home.path()).unwrap();
    let sessions = client.all_sessions(false).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["harness"], "claude");
}

#[tokio::test]
async fn api_006_ac1_prompt_from_stdin_writes_only_the_answer() {
    let serve = Serve::start();
    let scenario = serve.scenario(HELLO);
    let mut child = beton(serve.home())
        .current_dir(serve.work.path())
        .args(["run", "fake", "--scenario"])
        .arg(&scenario)
        .args(["-p", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"sag hallo").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Hallo!\n");
    assert!(stderr(&out).contains("/s/ses_"), "{}", stderr(&out));
}

#[tokio::test]
async fn api_006_ac2_json_output_is_exactly_one_object() {
    let serve = Serve::start();
    let scenario = serve.scenario(HELLO);
    let out = run(beton(serve.home())
        .current_dir(serve.work.path())
        .args([
            "run",
            "fake",
            "--output-format",
            "json",
            "-p",
            "sag hallo",
            "--scenario",
        ])
        .arg(&scenario));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let mut stream = serde_json::Deserializer::from_str(&text).into_iter::<Value>();
    let v = stream.next().unwrap().unwrap();
    assert!(stream.next().is_none(), "mehr als ein JSON-Wert: {text}");
    assert_eq!(v["status"], "completed");
    assert_eq!(v["result"], "Hallo!");
    for key in ["session_id", "cost_usd", "usage", "duration_ms"] {
        assert!(v.get(key).is_some(), "{key} fehlt: {v}");
    }
}

#[tokio::test]
async fn api_006_ac3_on_ask_deny_exits_4_without_hanging() {
    let serve = Serve::start();
    let scenario = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake/push-ask.yaml");
    let started = Instant::now();
    let out = run(beton(serve.home())
        .current_dir(serve.work.path())
        .args([
            "run",
            "fake",
            "--on-ask",
            "deny",
            "-p",
            "Bitte pushen",
            "--scenario",
        ])
        .arg(&scenario));
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(stderr(&out).contains("--on-ask deny"), "{}", stderr(&out));
}

#[tokio::test]
async fn cli_002_ac2_continue_picks_the_last_session_in_this_directory() {
    let serve = Serve::start();
    let scenario = serve.scenario(
        "turns:\n  - expect_input: \"eins\"\n    emit: [{ message: \"1\" }]\n  - expect_input: \"zwei\"\n    emit: [{ message: \"2\" }]\n",
    );
    let other = tempfile::tempdir().unwrap();
    let first = run(beton(serve.home())
        .current_dir(serve.work.path())
        .args([
            "run",
            "fake",
            "--output-format",
            "json",
            "-p",
            "eins",
            "--scenario",
        ])
        .arg(&scenario));
    assert!(first.status.success(), "{}", stderr(&first));
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    // Eine jüngere Session in einem anderen Verzeichnis darf nicht gewählt werden.
    let decoy = run(beton(serve.home())
        .current_dir(other.path())
        .args(["run", "fake", "--detach", "--scenario"])
        .arg(&scenario));
    assert!(decoy.status.success(), "{}", stderr(&decoy));

    let second = run(beton(serve.home()).current_dir(serve.work.path()).args([
        "run",
        "-c",
        "--output-format",
        "json",
        "-p",
        "zwei",
    ]));
    assert!(second.status.success(), "{}", stderr(&second));
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second["session_id"], first["session_id"]);
    assert_eq!(second["result"], "2");
}

#[cfg(unix)]
#[tokio::test]
async fn cli_002_ac4_ctrl_c_interrupts_and_twice_detaches() {
    let serve = Serve::start();
    let client = serve.client();
    let scenario = serve.scenario(
        "turns:\n  - expect_input: \"los\"\n    emit: [{ hang: true }]\n  - expect_input: \"nochmal\"\n    emit: [{ hang: true }]\n",
    );
    let mut child = beton(serve.home())
        .current_dir(serve.work.path())
        .args(["run", "fake", "--scenario"])
        .arg(&scenario)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"los\n").unwrap();
    wait_until("Session läuft", || {
        let client = client.clone();
        async move {
            let sessions = client.all_sessions(false).await.unwrap();
            sessions.first().is_some_and(|s| s["status"] == "running")
        }
    })
    .await;
    let id = client.all_sessions(false).await.unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sigint = |pid: u32| {
        std::process::Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .status()
            .unwrap();
    };
    // Einmal Strg+C: Turn wird unterbrochen, der Prozess bleibt.
    sigint(child.id());
    wait_until("Turn unterbrochen", || {
        let client = client.clone();
        let id = id.clone();
        async move {
            events(&client, &id)
                .await
                .iter()
                .any(|e| e["type"] == "turn.interrupted")
        }
    })
    .await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "run endet nach einfachem Strg+C"
    );

    // Neuer Turn, dann zweimal Strg+C binnen 1 s: abkoppeln, Session läuft weiter.
    stdin.write_all(b"nochmal\n").unwrap();
    wait_until("zweiter Turn läuft", || {
        let client = client.clone();
        let id = id.clone();
        async move { status(&client, &id).await == "running" }
    })
    .await;
    sigint(child.id());
    std::thread::sleep(Duration::from_millis(200));
    sigint(child.id());
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "run koppelt nicht ab");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(exit.success(), "{exit}");
    let st = status(&client, &id).await;
    assert!(
        st != "stopped" && st != "failed",
        "Session wurde gestoppt: {st}"
    );
    drop(stdin);
}

// --------------------------------------------------------------------------- HAR-026 AC1

/// Event-Log ohne `ts`, Event-IDs und laufzeitabhängige IDs (Session, Runner).
fn normalized(events: &[Value], session: &str) -> String {
    fn walk(v: &mut Value, session: &str) {
        match v {
            Value::Object(map) => {
                map.remove("ts");
                map.remove("id");
                for (_, child) in map.iter_mut() {
                    walk(child, session);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|i| walk(i, session)),
            Value::String(s) => {
                if s == session {
                    *s = "<session>".into();
                } else if s.starts_with("run_")
                    || s.starts_with("evt_")
                    || s.starts_with("msg_user_")
                {
                    *s = "<id>".into();
                } else if s.contains("/T/") || s.starts_with("/tmp") || s.starts_with("/private") {
                    *s = "<pfad>".into();
                }
            }
            _ => {}
        }
    }
    let mut events = events.to_vec();
    for e in &mut events {
        walk(e, session);
    }
    serde_json::to_string_pretty(&events).unwrap()
}

#[tokio::test]
async fn har_026_ac1_identical_input_gives_an_identical_event_log() {
    let serve = Serve::start();
    let client = serve.client();
    let scenario = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake/push-ask.yaml");
    let mut logs = Vec::new();
    for _ in 0..2 {
        let out = run(beton(serve.home())
            .current_dir(serve.work.path())
            .args([
                "run",
                "fake",
                "--on-ask",
                "deny",
                "--output-format",
                "json",
                "-p",
                "Bitte pushen",
                "--scenario",
            ])
            .arg(&scenario));
        assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let id = v["session_id"].as_str().unwrap().to_owned();
        logs.push(normalized(&events(&client, &id).await, &id));
    }
    assert!(logs[0].contains("approval.requested"), "{}", logs[0]);
    assert_eq!(logs[0], logs[1]);
}

// --------------------------------------------------------------------------- RUN-002 AC1

#[cfg(unix)]
fn cwd_of_pid(pid: &str) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "linux") {
        return std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    }
    let out = std::process::Command::new("lsof")
        .args(["-a", "-p", pid, "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(std::path::PathBuf::from))
}

#[cfg(unix)]
#[tokio::test]
async fn run_002_ac1_run_claude_starts_exactly_one_runner_in_the_project() {
    let home = background_home();
    let project = tempfile::tempdir().unwrap();
    let daemon = BackgroundDaemon(home.path().to_path_buf());
    let scenario = write(project.path(), "hallo.yaml", HELLO);
    let out = run(beton(home.path())
        .current_dir(project.path())
        .env("BETON_CLAUDE_PATH", fake_claude(&scenario))
        .args(["run", "claude", "-p", "sag hallo"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let pid = daemon.pid().unwrap().to_string();
    let children = run(std::process::Command::new("pgrep").args(["-P", &pid]));
    let runners: Vec<String> = stdout(&children).lines().map(str::to_owned).collect();
    assert_eq!(runners.len(), 1, "Runner-Prozesse: {runners:?}");
    let cwd = cwd_of_pid(&runners[0]).expect("cwd des Runners");
    assert_eq!(
        cwd.canonicalize().unwrap(),
        project.path().canonicalize().unwrap()
    );
}

// --------------------------------------------------------------------------- SES-002 AC3

#[tokio::test]
async fn ses_002_ac3_attach_from_a_second_terminal_shows_the_running_turn() {
    let serve = Serve::start();
    let client = serve.client();
    let scenario = serve.scenario(
        "turns:\n  - expect_input: \"los\"\n    emit:\n      - { message_delta: \"Live-Text \", chunk: 5 }\n      - { hang: true }\n",
    );
    let created = client
        .create_session(&json!({
            "target": "fake",
            "cwd": serve.work.path(),
            "harness_opts": {"scenario": scenario},
        }))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    wait_until("Session bereit", || {
        let client = client.clone();
        let id = id.clone();
        async move { status(&client, &id).await == "idle" }
    })
    .await;

    let mut attach = beton(serve.home())
        .args(["attach", &id, "--read-only"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = attach.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let mut reader = std::io::BufReader::new(stdout);
        let mut buf = Vec::new();
        while reader.read_until(b' ', &mut buf).unwrap_or(0) > 0 {
            text.push_str(&String::from_utf8_lossy(&buf));
            buf.clear();
            let _ = tx.send(text.clone());
        }
    });
    // Erst angehängt, dann startet der Turn (Web-UI bzw. anderes Terminal).
    std::thread::sleep(Duration::from_millis(500));
    client.input(&id, "los").await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let seen = rx.recv_timeout(Duration::from_secs(20)).unwrap_or_default();
        if seen.contains("Live-Text") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "attach zeigt den Turn nicht: {seen:?}"
        );
    }
    assert_eq!(status(&client, &id).await, "running", "Turn läuft noch");
    let _ = attach.kill();
    let _ = attach.wait();
}

// --------------------------------------------------------------------------- CLI-003

#[tokio::test]
async fn cli_003_ac1_ambiguous_prefix_lists_candidates_and_exits_2() {
    let serve = Serve::start();
    let a = serve.create_idle_session().await;
    let b = serve.create_idle_session().await;
    let out = run(beton(serve.home()).args(["attach", "ses_"]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("mehrdeutig"), "{err}");
    assert!(err.contains(&a) && err.contains(&b), "{err}");
    // Ein eindeutiges Präfix funktioniert.
    let out = run(beton(serve.home()).args(["session", "show", &a[..a.len() - 2], "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["id"], a.as_str());
}

#[tokio::test]
async fn cli_003_session_management_commands() {
    let serve = Serve::start();
    let id = serve.create_idle_session().await;
    let client = serve.client();
    let ok = |args: &[&str]| {
        let out = run(beton(serve.home()).args(args));
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        out
    };
    ok(&["session", "rename", &id, "Neuer Titel"]);
    assert_eq!(client.session(&id).await.unwrap()["title"], "Neuer Titel");
    let show = ok(&["session", "show", "last", "--json"]);
    let v: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(v["id"], id.as_str());
    assert_eq!(
        Path::new(v["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        serve.work.path().canonicalize().unwrap()
    );
    ok(&["session", "interrupt", &id]);
    ok(&["session", "archive", &id]);
    assert!(client.all_sessions(false).await.unwrap().is_empty());
    ok(&["session", "unarchive", &id]);
    assert_eq!(client.all_sessions(false).await.unwrap().len(), 1);
    ok(&["session", "delete", &id]);
    assert!(client.all_sessions(true).await.unwrap().is_empty());
}

#[tokio::test]
async fn cli_003_resume_restarts_a_stopped_session() {
    let serve = Serve::start();
    let client = serve.client();
    let scenario =
        serve.scenario("turns:\n  - expect_input: \"eins\"\n    emit: [{ message: \"1\" }]\n");
    let out = run(beton(serve.home())
        .current_dir(serve.work.path())
        .args([
            "run",
            "fake",
            "--output-format",
            "json",
            "-p",
            "eins",
            "--scenario",
        ])
        .arg(&scenario));
    assert!(out.status.success(), "{}", stderr(&out));
    let id = serde_json::from_slice::<Value>(&out.stdout).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    client.set_archived(&id, true).await.unwrap();
    client.set_archived(&id, false).await.unwrap();
    wait_until("gestoppt", || {
        let client = client.clone();
        let id = id.clone();
        async move { status(&client, &id).await == "stopped" }
    })
    .await;
    // `resume` ohne Eingabe (stdin leer): startet den Runner und koppelt wieder ab.
    let out = run(beton(serve.home()).args(["resume", &id]));
    assert!(out.status.success(), "{}", stderr(&out));
    wait_until("Runner läuft wieder", || {
        let client = client.clone();
        let id = id.clone();
        async move { status(&client, &id).await == "idle" }
    })
    .await;
    assert!(
        stdout(&out).contains('1'),
        "Verlauf fehlt: {}",
        stdout(&out)
    );
}

// --------------------------------------------------------------------------- CLI-004

#[tokio::test]
async fn cli_004_serve_without_foreground_runs_in_the_background() {
    let home = background_home();
    let _daemon = BackgroundDaemon(home.path().to_path_buf());
    let out = run(beton(home.path()).args(["serve", "--port", "0", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
    let client = Client::local(home.path()).unwrap();
    client.info().await.unwrap();
    let again = run(beton(home.path()).args(["serve", "--port", "0"]));
    assert_eq!(again.status.code(), Some(1), "{}", stderr(&again));
}

// --------------------------------------------------------------------------- API-005

#[tokio::test]
async fn api_005_ac2_stream_example_prints_the_deltas() {
    let serve = Serve::start();
    let example = {
        let beton = std::path::PathBuf::from(env!("CARGO_BIN_EXE_beton"));
        let path = beton
            .parent()
            .unwrap()
            .join("examples")
            .join(format!("stream{}", std::env::consts::EXE_SUFFIX));
        if !path.is_file() {
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "-q", "-p", "beton-sdk", "--example", "stream"])
                .status()
                .unwrap();
            assert!(status.success(), "Beispiel bauen");
        }
        path
    };
    let out = run(std::process::Command::new(example)
        .env("BETON_HOME", serve.home())
        .current_dir(serve.work.path()));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Hallo aus dem Fake-Harness\n");
}

// --------------------------------------------------------------------------- WEB-018

#[tokio::test]
async fn web_018_ac3_without_tty_on_ask_wait_leaves_the_decision_to_the_web() {
    let serve = Serve::start();
    let client = serve.client();
    let scenario = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake/push-ask.yaml");
    // Ohne TTY und ohne --on-ask gilt `wait`: nichts wird ohne Zustimmung ausgeführt.
    let child = beton(serve.home())
        .current_dir(serve.work.path())
        .args([
            "run",
            "fake",
            "-p",
            "Bitte pushen",
            "--output-format",
            "json",
            "--scenario",
        ])
        .arg(&scenario)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until("Freigabe offen", || {
        let client = client.clone();
        async move {
            let sessions = client.all_sessions(false).await.unwrap();
            sessions
                .first()
                .is_some_and(|s| s["status"] == "waiting_approval")
        }
    })
    .await;
    let id = client.all_sessions(false).await.unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Noch kein Tool-Ergebnis: der Lauf wartet.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !events(&client, &id)
            .await
            .iter()
            .any(|e| e["type"] == "tool.call.completed"),
        "Tool lief ohne Zustimmung"
    );
    // Entscheidung wie über die Web-Karte.
    let approval_id = events(&client, &id)
        .await
        .iter()
        .find(|e| e["type"] == "approval.requested")
        .and_then(|e| e["payload"]["approval_id"].as_str().map(str::to_owned))
        .unwrap();
    client
        .resolve_approval(&id, &approval_id, true, None)
        .await
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "completed");
    assert!(
        stderr(&out).contains("Wartet auf Freigabe"),
        "{}",
        stderr(&out)
    );
}

// --------------------------------------------------------------------------- SES-015 / SES-016

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
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

/// Repository mit `main` im Arbeitsverzeichnis des Daemons.
fn repo(serve: &Serve) -> std::path::PathBuf {
    let repo = serve.work.path().join("projekt");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hallo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "start"]);
    repo.canonicalize().unwrap()
}

#[tokio::test]
async fn cli_002_ac3_run_with_worktree_starts_the_session_in_a_new_worktree() {
    let serve = Serve::start();
    let repo = repo(&serve);
    let scenario = serve.scenario("turns: []");
    let out = run(beton(serve.home())
        .current_dir(&repo)
        .args(["run", "fake", "--detach", "--json", "--title", "Neue Suche"])
        .args(["--worktree", "--base", "main", "--scenario"])
        .arg(&scenario));
    assert!(out.status.success(), "{}", stderr(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = v["session_id"].as_str().unwrap();
    let session = serve.client().session(id).await.unwrap();
    let wt = &session["worktree"];
    let branch = wt["branch"].as_str().unwrap();
    assert!(branch.starts_with("beton/neue-suche-"), "{branch}");
    assert_eq!(wt["base"], "main");
    let path = Path::new(wt["path"].as_str().unwrap());
    assert!(path.starts_with(serve.home().canonicalize().unwrap().join("worktrees")));
    assert!(path.join("README.md").is_file());
    assert!(git(&repo, &["worktree", "list"]).contains(&path.display().to_string()));
    assert!(stderr(&out).contains("Worktree "), "{}", stderr(&out));

    // Mit Branch-Name; unbekannte Base: Fehler mit verfügbaren Branches, keine Session.
    let out = run(beton(serve.home())
        .current_dir(&repo)
        .args([
            "run",
            "fake",
            "--detach",
            "--worktree=feature/x",
            "--base",
            "gibt-es-nicht",
        ])
        .arg("--scenario")
        .arg(&scenario));
    assert!(!out.status.success());
    assert!(stderr(&out).contains("main"), "{}", stderr(&out));
    assert_eq!(serve.client().all_sessions(true).await.unwrap().len(), 1);
    // `--base` ohne `--worktree` ist ein Usage-Fehler.
    let out = run(beton(serve.home()).args(["run", "fake", "--base", "main"]));
    assert_eq!(out.status.code(), Some(2));
}

#[tokio::test]
async fn ses_016_ac1_session_delete_asks_before_losing_uncommitted_work() {
    let serve = Serve::start();
    let repo = repo(&serve);
    let scenario = serve.scenario("turns: []");
    let created = serve
        .client()
        .create_session(&json!({
            "target": "fake", "cwd": repo, "harness_opts": {"scenario": scenario},
            "worktree": {}
        }))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    let path = std::path::PathBuf::from(created["worktree"]["path"].as_str().unwrap());
    std::fs::write(path.join("wip.txt"), "x\n").unwrap();
    let out = run(beton(serve.home()).args(["session", "delete", &id]));
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("--uncommitted commit|discard"), "{err}");
    assert!(path.join("wip.txt").is_file());
    let out = run(beton(serve.home()).args(["session", "delete", &id, "--uncommitted", "discard"]));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!path.exists());
    assert!(serve.client().all_sessions(true).await.unwrap().is_empty());
}

// --------------------------------------------------------------------------- SES-006 / CLI-002

/// Inhalts-Events (Typ und Nutzlast) einer Session.
fn content_of(events: &[Value]) -> Vec<(Value, Value)> {
    events
        .iter()
        .filter(|e| {
            matches!(
                e["type"].as_str(),
                Some(
                    "message.completed" | "turn.started" | "turn.completed" | "tool.call.requested"
                )
            )
        })
        .map(|e| (e["type"].clone(), e["payload"].clone()))
        .collect()
}

#[tokio::test]
async fn ses_006_ac4_run_fork_creates_the_same_fork_as_the_api() {
    let serve = Serve::start();
    let scenario = serve.scenario(
        "turns:\n  - emit: [{ message: \"eins\" }]\n  - emit: [{ message: \"zwei\" }, { message: \"drei\" }]\n",
    );
    let client = serve.client();
    let created = client
        .create_session(&json!({
            "target": "fake", "cwd": serve.work.path(), "title": "Quelle",
            "harness_opts": {"scenario": scenario},
        }))
        .await
        .unwrap();
    let source = created["id"].as_str().unwrap().to_owned();
    for (n, text) in ["erste", "zweite"].into_iter().enumerate() {
        wait_until("idle", || async {
            status(&client, &source).await == "idle"
        })
        .await;
        client.input(&source, text).await.unwrap();
        wait_until("Turn-Ende", || async {
            events(&client, &source)
                .await
                .iter()
                .filter(|e| e["type"] == "turn.completed")
                .count()
                > n
        })
        .await;
    }
    // Mitten im zweiten Turn: zwischen „zwei“ und „drei“.
    let log = events(&client, &source).await;
    let mid = log
        .iter()
        .find(|e| e["type"] == "message.completed" && e["payload"]["content"][0]["text"] == "zwei")
        .unwrap()["seq"]
        .as_u64()
        .unwrap();

    let api = client
        .fork_session(&source, &json!({"at_seq": mid}))
        .await
        .unwrap();
    let out =
        run(beton(serve.home()).args(["run", "--fork", &format!("{source}@{mid}"), "--detach"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let cli = stdout(&out).trim().to_owned();
    assert!(
        stderr(&out).contains("mitten in einem Turn"),
        "{}",
        stderr(&out)
    );
    let api_id = api["session"]["id"].as_str().unwrap();
    assert_ne!(api_id, cli);
    let a = client.session(api_id).await.unwrap();
    let c = client.session(&cli).await.unwrap();
    for field in ["harness", "title", "kind"] {
        assert_eq!(a[field], c[field], "{field}");
    }
    let forked = |events: &[Value]| {
        events
            .iter()
            .find(|e| e["type"] == "session.forked")
            .map(|e| e["payload"].clone())
    };
    wait_until("session.forked", || async {
        forked(&events(&client, api_id).await).is_some()
            && forked(&events(&client, &cli).await).is_some()
    })
    .await;
    let a_events = events(&client, api_id).await;
    let c_events = events(&client, &cli).await;
    assert_eq!(forked(&a_events), forked(&c_events));
    assert_eq!(forked(&a_events).unwrap()["at_seq"], api["effective_seq"]);
    assert_eq!(content_of(&a_events), content_of(&c_events));
    assert!(!content_of(&c_events).is_empty());
}
