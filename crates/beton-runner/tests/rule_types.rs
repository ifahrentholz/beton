//! Eingebaute Regeltypen aus WP-42 im Gate des Runners (POL-016 bis POL-018): Der Runner
//! füllt den Kontext, den die entzuckerten Regeln brauchen (`host.home`,
//! `session.project_root`, `tool.write_paths`, Symlink-Auflösung, …). Ohne Daemon; alle
//! Dateien liegen in einem Temp-Verzeichnis, das echte Home wird nie gelesen.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use beton_harness::{Gate, GateDecision, GateOrigin, GateRequest, HarnessId};
use beton_policy::load::Scope;
use beton_runner::policy::{GateEvent, PolicyGate, PolicyOptions};
use tokio::sync::mpsc;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Env {
    /// `<root>/home` (Home), `<root>/wt` (Worktree, Git-Wurzel), `<root>/user` (User-Policies).
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("home/.ssh")).unwrap();
        std::fs::create_dir_all(root.join("wt/.git")).unwrap();
        std::fs::create_dir_all(root.join("user")).unwrap();
        Self { _dir: dir, root }
    }

    fn wt(&self) -> PathBuf {
        self.root.join("wt")
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn policy(&self, rules: &str) {
        std::fs::write(
            self.root.join("user/p.yaml"),
            format!("spec_version: 1\nname: p\nrules:\n{rules}"),
        )
        .unwrap();
    }

    fn gate(&self) -> (PolicyGate, mpsc::UnboundedReceiver<GateEvent>) {
        self.gate_for("claude")
    }

    fn gate_for(&self, harness: &str) -> (PolicyGate, mpsc::UnboundedReceiver<GateEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let opts = PolicyOptions {
            harness: harness.parse::<HarnessId>().unwrap(),
            workdir: self.wt(),
            home: Some(self.home()),
            dirs: vec![(Scope::User, self.root.join("user"))],
            ..PolicyOptions::default()
        };
        (PolicyGate::new(opts, tx), rx)
    }
}

fn req(call: &str, tool: &str, kind: &str, args: serde_json::Value) -> GateRequest {
    GateRequest {
        call_id: call.into(),
        tool: tool.into(),
        kind: kind.into(),
        args,
        origin: GateOrigin::PreTool,
        ..GateRequest::default()
    }
}

fn denied(d: &GateDecision) -> bool {
    matches!(d, GateDecision::Deny { .. })
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

#[tokio::test]
async fn pol_016_runner_fills_home_project_root_and_write_paths_and_resolves_symlinks() {
    let env = Env::new();
    std::fs::write(env.home().join(".ssh/id_ed25519"), "x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(env.home().join(".ssh/id_ed25519"), env.wt().join("notes")).unwrap();
    env.policy(
        "  - id: paths\n    type: path_guard\n    params: { write: { allow: [\"${project_root}/**\"] } }\n",
    );
    let (g, _rx) = env.gate();
    // Symlink im Worktree auf ~/.ssh: Lesen abgelehnt (AC2).
    #[cfg(unix)]
    {
        let d = g
            .decide(req(
                "c1",
                "Read",
                "file_read",
                serde_json::json!({ "file_path": path(&env.wt().join("notes")) }),
            ))
            .await;
        assert!(denied(&d), "{d:?}");
    }
    // `~` im Shell-Kommando über das Home der Session.
    let d = g
        .decide(req(
            "c2",
            "Bash",
            "shell",
            serde_json::json!({ "command": "cat ~/.ssh/id_ed25519" }),
        ))
        .await;
    assert!(denied(&d), "{d:?}");
    // Umleitung außerhalb der Projektwurzel (AC3) bzw. Schreiben darin.
    let d = g
        .decide(req(
            "c3",
            "Bash",
            "shell",
            serde_json::json!({ "command": "echo x > /etc/hosts" }),
        ))
        .await;
    assert!(denied(&d), "{d:?}");
    let d = g
        .decide(req(
            "c4",
            "Write",
            "file_write",
            serde_json::json!({ "file_path": path(&env.wt().join("src/a.rs")), "content": "x" }),
        ))
        .await;
    assert_eq!(d, GateDecision::Allow { updated_args: None });
    let d = g
        .decide(req(
            "c5",
            "Read",
            "file_read",
            serde_json::json!({ "file_path": path(&env.wt().join("README.md")) }),
        ))
        .await;
    assert_eq!(d, GateDecision::Allow { updated_args: None });
}

#[tokio::test]
async fn pol_017_runner_reads_the_current_branch_for_pushes_without_refspec() {
    let env = Env::new();
    env.policy("  - id: git\n    type: git_guard\n    params: { push: allow }\n");
    let push = |call: &str| {
        req(
            call,
            "Bash",
            "shell",
            serde_json::json!({ "command": "git push" }),
        )
    };
    std::fs::write(env.wt().join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
    let (g, _rx) = env.gate();
    assert_eq!(
        g.decide(push("p1")).await,
        GateDecision::Allow { updated_args: None }
    );
    // Der Branch wird bei jeder Auswertung gelesen (der Agent kann ihn wechseln).
    std::fs::write(env.wt().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let d = g.decide(push("p2")).await;
    assert!(denied(&d), "{d:?}");
    // Worktree mit `.git`-Datei (git worktree): HEAD im verwiesenen Verzeichnis.
    std::fs::remove_dir_all(env.wt().join(".git")).unwrap();
    let gitdir = env.root.join("repo.git/worktrees/wt");
    std::fs::create_dir_all(&gitdir).unwrap();
    std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/feature/y\n").unwrap();
    std::fs::write(
        env.wt().join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .unwrap();
    assert_eq!(
        g.decide(push("p3")).await,
        GateDecision::Allow { updated_args: None }
    );
    // Losgelöster HEAD: kein Branch, das Ziel ist unbekannt (fail closed).
    std::fs::write(
        gitdir.join("HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .unwrap();
    let d = g.decide(push("p4")).await;
    assert!(denied(&d), "{d:?}");
}

#[tokio::test]
async fn pol_018_runner_counts_identical_calls_and_errors_in_a_row() {
    use beton_core::event::ToolStatus;
    let env = Env::new();
    env.policy("  - id: loop\n    type: loop_detection\n    params: { action: deny }\n");
    let (g, _rx) = env.gate();
    let npm = |call: &str| {
        req(
            call,
            "Bash",
            "shell",
            serde_json::json!({ "command": "npm test" }),
        )
    };
    let other = |call: &str| {
        req(
            call,
            "Bash",
            "shell",
            serde_json::json!({ "command": format!("echo {call}") }),
        )
    };
    let allow = GateDecision::Allow { updated_args: None };
    assert_eq!(g.decide(npm("n1")).await, allow);
    assert_eq!(g.decide(other("o1")).await, allow);
    assert_eq!(g.decide(npm("n2")).await, allow);
    // Dritter identischer Call im Fenster (AC1).
    let d = g.decide(npm("n3")).await;
    assert!(denied(&d), "{d:?}");
    // Fünf Fehler in Folge (AC2); ein erfolgreicher Call setzt zurück.
    for _ in 0..4 {
        g.note_completed(ToolStatus::Error);
    }
    assert_eq!(g.decide(other("o2")).await, allow);
    g.note_completed(ToolStatus::Error);
    let d = g.decide(other("o3")).await;
    assert!(denied(&d), "{d:?}");
    g.note_completed(ToolStatus::Denied);
    assert!(denied(&g.decide(other("o4")).await));
    g.note_completed(ToolStatus::Ok);
    assert_eq!(g.decide(other("o5")).await, allow);
}

/// Review #285 B1: `mcp__builtin__Read` (MCP-Server namens `builtin`) passiert eine Allowlist
/// `allow: [Read]` nicht.
#[tokio::test]
async fn pol_015_mcp_server_named_builtin_is_denied_by_the_gate() {
    let env = Env::new();
    env.policy("  - id: list\n    type: tool_allowlist\n    params: { allow: [Read] }\n");
    let (g, _rx) = env.gate();
    let d = g
        .decide(req(
            "b1",
            "mcp__builtin__Read",
            "mcp",
            serde_json::json!({ "file_path": path(&env.wt().join("a")) }),
        ))
        .await;
    assert!(denied(&d), "{d:?}");
    let d = g
        .decide(req(
            "b2",
            "Read",
            "file_read",
            serde_json::json!({ "file_path": path(&env.wt().join("a")) }),
        ))
        .await;
    assert_eq!(d, GateDecision::Allow { updated_args: None });
}

/// POL-023 AC2, HAR-007 AC2: Ein ACP-`fs/write_text_file` kommt als Datei-Schreiben vor der
/// Ausführung ans Gate (wie es der ACP-Adapter meldet) und wird gegen `path_guard` geprüft:
/// außerhalb des Worktrees abgelehnt, darin erlaubt.
#[tokio::test]
async fn pol_023_ac2_acp_fs_write_is_checked_against_path_guard() {
    let env = Env::new();
    env.policy("  - id: paths\n    type: path_guard\n");
    let (g, _rx) = env.gate_for("acp:fake");
    let write = |call: &str, p: &Path| GateRequest {
        call_id: call.into(),
        tool: "fs/write_text_file".into(),
        kind: "file_write".into(),
        acp_kind: Some("edit".into()),
        args: serde_json::json!({ "path": path(p), "content": "x" }),
        origin: GateOrigin::PreTool,
        ..GateRequest::default()
    };
    let d = g.decide(write("fs-1", &env.home().join(".bashrc"))).await;
    assert!(denied(&d), "{d:?}");
    let d = g.decide(write("fs-2", &env.wt().join("notizen.txt"))).await;
    assert!(!denied(&d), "{d:?}");
}

/// Verbrauchswerte und Session-Daten vom Server (im Runner: `policy.usage`).
struct FixedUsage(beton_runner::policy::Usage);

#[async_trait::async_trait]
impl beton_runner::policy::UsageSource for FixedUsage {
    async fn usage(&self) -> Result<beton_runner::policy::Usage, String> {
        Ok(self.0.clone())
    }
}

/// #196 (POL-004): Der Runner füllt Laufzeit, Run (bis Runs existieren die Session selbst)
/// und die Session-Daten des Servers (Titel, Eltern, Tiefe, Projekt, asynchron).
#[tokio::test]
async fn pol_004_runner_fills_run_duration_and_session_data_of_the_server() {
    let env = Env::new();
    env.policy(
        "  - id: meta\n    on: tool_call\n    when: >-\n      session.title == \"Release\" && session.parent_id == \"ses_parent\" &&\n      session.depth == 1 && session.is_async && session.project == \"prj_1\" &&\n      session.duration_s >= 0.0 && run.id == session.id && run.trigger == \"spawn\" &&\n      run.turns == session.turns && run.duration_s >= 0.0\n    action: deny\n",
    );
    let (tx, _rx) = mpsc::unbounded_channel();
    let opts = PolicyOptions {
        session_id: Some("ses_child".into()),
        harness: "claude".parse::<HarnessId>().unwrap(),
        workdir: env.wt(),
        home: Some(env.home()),
        dirs: vec![(Scope::User, env.root.join("user"))],
        ..PolicyOptions::default()
    };
    let usage = beton_runner::policy::Usage {
        session_title: Some("Release".into()),
        session_parent_id: Some("ses_parent".into()),
        session_depth: Some(1),
        session_is_async: Some(true),
        session_project: Some("prj_1".into()),
        run_trigger: Some("spawn".into()),
        ..beton_runner::policy::Usage::default()
    };
    let gate = PolicyGate::new(opts, tx).with_usage(std::sync::Arc::new(FixedUsage(usage)));
    let d = gate
        .decide(req(
            "c1",
            "Bash",
            "shell",
            serde_json::json!({"command": "ls"}),
        ))
        .await;
    assert!(denied(&d), "alle Werte gefüllt: Regel greift ({d:?})");

    // Ohne Antwort des Servers bleiben die Session-Daten `null`: Die Regel scheitert und
    // lehnt ab (fail closed), statt mit einem Default zu greifen oder nicht.
    env.policy(
        "  - id: meta\n    on: tool_call\n    when: session.depth == 0\n    action: allow\n  - id: rest\n    on: tool_call\n    when: \"!(session.depth == 0)\"\n    action: deny\n",
    );
    let (tx, _rx) = mpsc::unbounded_channel();
    let opts = PolicyOptions {
        harness: "claude".parse::<HarnessId>().unwrap(),
        workdir: env.wt(),
        home: Some(env.home()),
        dirs: vec![(Scope::User, env.root.join("user"))],
        ..PolicyOptions::default()
    };
    let gate = PolicyGate::new(opts, tx);
    let d = gate
        .decide(req(
            "c2",
            "Bash",
            "shell",
            serde_json::json!({"command": "ls"}),
        ))
        .await;
    assert!(denied(&d), "ohne Session-Daten: fail closed ({d:?})");
}
