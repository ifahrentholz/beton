//! „Sandbox erforderlich“ als Default (SBX-006) über die CLI gegen einen echten Daemon:
//! kein stiller Fallback auf unsandboxed, `backend: none` nur mit Bestätigung je Session samt
//! Audit-Eintrag, per User-Policy verbietbar, YOLO nur mit aktiver Sandbox (HAR-027).
//! Echte Vendor-CLIs kommen nicht vor: `claude` und `codex` sind im Daemon die Fake-CLI
//! (QA-002).

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};

use common::{Serve, beton, fake_claude, fake_cli, run, stderr, stdout};
use serde_json::{Value, json};

const HELLO: &str = "turns:\n  - expect_input: \"sag hallo\"\n    emit:\n      - { message_delta: \"Hallo!\", chunk: 3 }\n";

const ECHO_SETTINGS: &str = "turns:\n  - emit: [{ echo_settings: true }]\n";

/// Fake-Vendor-CLIs des Daemons: `claude` hinter einem Skript, das bei jedem Start einer
/// Session (nicht bei `--version`) `marker` anlegt. Fehlt die Datei, wurde kein
/// Harness-Prozess gestartet.
struct Vendors {
    dir: tempfile::TempDir,
    claude: String,
    codex: String,
}

impl Vendors {
    fn new(scenario: &str) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenario.yaml");
        std::fs::write(&path, scenario).unwrap();
        let script = dir.path().join("claude.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --version \"*) ;; *) : > '{}' ;; esac\nexec {} \"$@\"\n",
                dir.path().join("claude.started").display(),
                fake_claude(&path)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let codex = format!(
            "{} --protocol app-server --scenario {}",
            fake_cli().display(),
            path.display()
        );
        Self {
            claude: script.display().to_string(),
            codex,
            dir,
        }
    }

    fn started(&self) -> PathBuf {
        self.dir.path().join("claude.started")
    }

    fn scenario(&self) -> PathBuf {
        self.dir.path().join("scenario.yaml")
    }
}

/// Daemon mit `sandbox:` aus der User-Konfiguration (leer: Default) und den Fake-CLIs.
fn serve(sandbox: &str, vendors: &Vendors, extra: &[(&str, &str)]) -> Serve {
    let mut env = vec![
        ("BETON_CLAUDE_PATH", vendors.claude.as_str()),
        ("BETON_CODEX_PATH", vendors.codex.as_str()),
    ];
    env.extend_from_slice(extra);
    if sandbox.is_empty() {
        // Default-Konfiguration: kein `sandbox:`.
        Serve::start_with_config("# Default-Konfiguration\n", &env)
    } else {
        Serve::start_with_config(&format!("sandbox:\n{sandbox}"), &env)
    }
}

fn run_claude(serve: &Serve, args: &[&str]) -> std::process::Output {
    run(beton(serve.home())
        .current_dir(serve.work.path())
        .args(["run"])
        .args(args))
}

async fn audit(home: &Path, action: &str) -> Vec<Value> {
    let o = run(beton(home).args(["--json", "audit", "list", "--action", action]));
    assert!(o.status.success(), "{}", stderr(&o));
    serde_json::from_str(&stdout(&o)).unwrap()
}

// ---------------------------------------------------------------------------------- SBX-006

#[tokio::test]
async fn sbx_006_ac1_run_without_usable_sandbox_fails_and_starts_no_harness() {
    // Kein nutzbares Backend (Linux ohne Landlock und bwrap, nachgestellt über den
    // Test-Schalter der Debug-Builds), Default-Konfiguration: kein Fallback auf unsandboxed.
    let vendors = Vendors::new(HELLO);
    let serve = serve("", &vendors, &[("BETON_TEST_SANDBOX_UNAVAILABLE", "all")]);
    let out = run_claude(&serve, &["claude", "-p", "sag hallo"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("sandbox_unavailable"),
        "{}",
        stderr(&out)
    );
    assert!(
        !vendors.started().exists(),
        "Harness-Prozess wurde gestartet"
    );
    let sessions = serve.client().all_sessions(true).await.unwrap();
    assert!(sessions.is_empty(), "{sessions:?}");
}

#[tokio::test]
async fn sbx_006_backend_none_needs_a_confirmation_per_session_with_audit_entry() {
    let vendors = Vendors::new(HELLO);
    let serve = serve("  backend: none\n", &vendors, &[]);

    // Ohne Bestätigung startet nichts.
    let out = run_claude(&serve, &["claude", "-p", "sag hallo"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("sandbox_confirmation_required"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("--unsandboxed"), "{}", stderr(&out));
    assert!(
        !vendors.started().exists(),
        "Harness-Prozess wurde gestartet"
    );
    assert!(serve.client().all_sessions(true).await.unwrap().is_empty());

    // Mit Bestätigung für genau diese Session; das Audit-Log vermerkt sie.
    let out = run_claude(&serve, &["--unsandboxed", "claude", "-p", "sag hallo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "Hallo!");
    let sessions = serve.client().all_sessions(true).await.unwrap();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let id = sessions[0]["id"].as_str().unwrap();
    let entries = audit(serve.home(), "sandbox.backend_none").await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["target_id"], id);
    assert_eq!(entries[0]["session_id"], id);

    // Eine weitere Session braucht wieder eine eigene Bestätigung.
    let created = serve
        .client()
        .create_session(&json!({
            "target": "claude",
            "cwd": serve.work.path(),
        }))
        .await;
    let err = created.unwrap_err().to_string();
    assert!(err.contains("sandbox_confirmation_required"), "{err}");
}

#[tokio::test]
async fn sbx_006_ac2_yolo_with_backend_none_is_rejected_at_session_start() {
    let vendors = Vendors::new(HELLO);
    let serve = serve("  backend: none\n", &vendors, &[]);
    let out = run_claude(
        &serve,
        &[
            "--unsandboxed",
            "claude",
            "--permission-mode",
            "yolo",
            "-p",
            "sag hallo",
        ],
    );
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("sandbox_required"),
        "{}",
        stderr(&out)
    );
    assert!(
        !vendors.started().exists(),
        "Harness-Prozess wurde gestartet"
    );
}

#[tokio::test]
async fn sbx_006_ac4_user_policy_forbids_backend_none_even_when_confirmed() {
    let vendors = Vendors::new(HELLO);
    let serve = serve("  backend: none\n", &vendors, &[]);
    let policies = serve.home().join("policies");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(
        policies.join("require-sandbox.yaml"),
        "spec_version: 1\nname: require-sandbox\nrules:\n  - id: no-unsandboxed\n    on: session_start\n    when: 'start.sandbox.backend == \"none\"'\n    action: deny\n    reason: Ohne Sandbox ist hier nicht erlaubt\n",
    )
    .unwrap();
    let out = run_claude(&serve, &["--unsandboxed", "claude", "-p", "sag hallo"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(
        !vendors.started().exists(),
        "Harness-Prozess wurde gestartet"
    );
    let sessions = serve.client().all_sessions(true).await.unwrap();
    let id = sessions[0]["id"].as_str().unwrap();
    let events = common::events(&serve.client(), id).await;
    let denied = events
        .iter()
        .find(|e| e["type"] == "session.start_denied")
        .unwrap_or_else(|| panic!("kein session.start_denied: {events:?}"));
    assert!(
        denied
            .to_string()
            .contains("Ohne Sandbox ist hier nicht erlaubt"),
        "{denied}"
    );
}

// ---------------------------------------------------------------------------------- HAR-027

#[tokio::test]
async fn har_027_ac1_yolo_starts_with_an_active_sandbox() {
    // Default `auto` ohne Netz (kein Proxy, keine CA nötig): Tools laufen in Stufe 2.
    let vendors = Vendors::new(ECHO_SETTINGS);
    let serve = serve("  allow_network: false\n", &vendors, &[]);
    let scenario = vendors.scenario();
    let out = run_claude(
        &serve,
        &[
            "fake",
            "--scenario",
            scenario.to_str().unwrap(),
            "--permission-mode",
            "yolo",
            "-p",
            "los",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("mode=yolo"), "{}", stdout(&out));
}

#[tokio::test]
async fn sbx_006_info_reports_the_sandbox_status_for_clients() {
    let vendors = Vendors::new(HELLO);
    let serve_none = serve("  backend: none\n", &vendors, &[]);
    let info: Value = serve_none.client().info().await.unwrap();
    assert_eq!(info["sandbox"]["backend"], "none", "{info}");
    assert_eq!(info["sandbox"]["active"], false, "{info}");
    assert_eq!(info["sandbox"]["yolo"], false, "{info}");

    let off = serve("", &vendors, &[("BETON_TEST_SANDBOX_UNAVAILABLE", "all")]);
    let info: Value = off.client().info().await.unwrap();
    assert_eq!(info["sandbox"]["backend"], "auto", "{info}");
    assert_eq!(info["sandbox"]["active"], false, "{info}");
    assert!(info["sandbox"]["hint"].is_string(), "{info}");
}
