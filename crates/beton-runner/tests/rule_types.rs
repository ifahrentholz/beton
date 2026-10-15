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
        let (tx, rx) = mpsc::unbounded_channel();
        let opts = PolicyOptions {
            harness: "claude".parse::<HarnessId>().unwrap(),
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
