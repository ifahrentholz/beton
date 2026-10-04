//! Integrationstests für `beton setup` und `beton doctor` (CLI-005, HAR-016, OBS-005,
//! HAR-003 AC3). `claude` ist die Fake-CLI (QA-002); `PATH` zeigt auf ein leeres
//! Verzeichnis, damit installierte CLIs des Rechners keine Rolle spielen.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::process::Command;

use beton_cli::doctor::{CheckStatus, DoctorReport};
use beton_cli::setup::SetupReport;
use common::{beton, fake_cli, run, stderr, stdout};
use serde_json::Value;

/// `beton` mit leerem `PATH` und optional der Fake-CLI als `claude`.
fn isolated(home: &Path, empty_path: &Path, claude: bool) -> Command {
    let mut c = beton(home);
    c.env("PATH", empty_path)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("BETON_CLAUDE_PATH")
        .env_remove("BETON_CODEX_PATH");
    if claude {
        c.env("BETON_CLAUDE_PATH", fake_cli());
    }
    c
}

#[test]
fn har_016_ac2_setup_check_json_reports_each_harness() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    // Ab M1 ist auch Codex nutzbar; die Fake-CLI spielt `codex login status` (HAR-016).
    let out = run(isolated(home.path(), empty.path(), true)
        .env("BETON_CODEX_PATH", fake_cli())
        .env("ANTHROPIC_API_KEY", "sk-geheim-1234")
        .args(["setup", "--check", "--json"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    // Gegen das veröffentlichte Schema: unbekannte Felder würden hier scheitern.
    let report: SetupReport = serde_json::from_str(&text).unwrap();
    let claude = report.harnesses.iter().find(|h| h.id == "claude").unwrap();
    assert!(claude.installed);
    assert_eq!(claude.version.as_deref(), Some("2.1.0"));
    assert_eq!(
        serde_json::to_value(claude.auth_status).unwrap(),
        "logged_in"
    );
    assert!(claude.api_key_env_found);
    let codex = report.harnesses.iter().find(|h| h.id == "codex").unwrap();
    assert!(codex.installed && codex.supported);
    assert_eq!(
        serde_json::to_value(codex.auth_status).unwrap(),
        "logged_in"
    );
    assert_eq!(codex.install_command, None);
    // Weder Kontodaten der CLI noch der Key-Wert erscheinen.
    assert!(!text.contains("fake-user@example.invalid"), "{text}");
    assert!(!text.contains("sk-geheim"), "{text}");
    // Jeder Eintrag hat die Pflichtfelder.
    let raw: Value = serde_json::from_str(&text).unwrap();
    for h in raw["harnesses"].as_array().unwrap() {
        for key in ["installed", "version", "auth_status", "api_key_env_found"] {
            assert!(h.get(key).is_some(), "{key} fehlt in {h}");
        }
    }
}

#[test]
fn har_016_logged_out_cli_is_reported_with_login_hint() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let out = run(isolated(home.path(), empty.path(), true)
        .env("BETON_CODEX_PATH", fake_cli())
        .env("BETON_FAKE_AUTH", "logged_out")
        .args(["setup", "--non-interactive"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Claude Code: installiert"), "{text}");
    assert!(text.contains("nicht angemeldet"), "{text}");
    assert!(text.contains("auth login"), "{text}");
    assert!(text.contains("codex login"), "{text}");
}

#[test]
fn har_016_missing_codex_is_supported_and_fails_check() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let out = run(isolated(home.path(), empty.path(), true).args(["setup", "--check", "--json"]));
    let report: SetupReport = serde_json::from_str(&stdout(&out)).unwrap();
    let codex = report.harnesses.iter().find(|h| h.id == "codex").unwrap();
    assert!(!codex.installed);
    assert_eq!(
        codex.install_command.as_deref(),
        Some("npm install -g @openai/codex")
    );
    // Ab M1 ist Codex nutzbar: Fehlt die CLI, endet `--check` mit Exit-Code ≠ 0.
    assert_ne!(out.status.code(), Some(0));
}

#[test]
fn cli_005_ac1_non_interactive_setup_reports_missing_cli_with_nonzero_exit() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let out = run(isolated(home.path(), empty.path(), false).args(["setup", "--non-interactive"]));
    assert_ne!(out.status.code(), Some(0), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("Claude Code: nicht installiert"), "{text}");
    assert!(text.contains("Codex: nicht installiert"), "{text}");
    // Nichts installiert: das leere PATH-Verzeichnis bleibt leer.
    assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
}

fn doctor(cmd: &mut Command) -> (Option<i32>, DoctorReport, Value) {
    let out = run(cmd.args(["doctor", "--json"]));
    let text = stdout(&out);
    let report: DoctorReport =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}\n{}", stderr(&out)));
    (
        out.status.code(),
        report,
        serde_json::from_str(&text).unwrap(),
    )
}

#[test]
fn obs_005_ac1_missing_claude_is_a_warning_with_install_hint() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let (code, report, _) = doctor(&mut isolated(home.path(), empty.path(), false));
    assert_eq!(code, Some(1));
    let claude = report
        .checks
        .iter()
        .find(|c| c.id == "harness.claude")
        .unwrap();
    assert_eq!(claude.status, CheckStatus::Warn);
    assert!(
        claude
            .hint
            .as_deref()
            .unwrap()
            .contains("npm install -g @anthropic-ai/claude-code"),
        "{claude:?}"
    );
    assert!(report.checks.iter().all(|c| c.status != CheckStatus::Fail));
}

#[test]
fn obs_005_ac2_json_has_id_status_message_hint_per_check() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let (_, report, raw) = doctor(&mut isolated(home.path(), empty.path(), true));
    assert!(report.checks.len() >= 6);
    for c in raw["checks"].as_array().unwrap() {
        for key in ["id", "status", "message", "hint"] {
            assert!(c.get(key).is_some(), "{key} fehlt in {c}");
        }
        assert!(["ok", "warn", "fail"].contains(&c["status"].as_str().unwrap()));
    }
    // Das veröffentlichte Schema ist aktuell (Snapshot über `cargo xtask codegen --check`).
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1/doctor.schema.json");
    let published: Value = serde_json::from_str(&std::fs::read_to_string(schema).unwrap()).unwrap();
    let current = serde_json::to_value(schemars::schema_for!(DoctorReport)).unwrap();
    assert_eq!(published, current, "cargo xtask codegen ausführen");
}

#[test]
fn har_003_ac3_doctor_lists_path_version_compatibility_auth_and_source() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let (_, report, _) = doctor(&mut isolated(home.path(), empty.path(), true));
    let claude = report
        .checks
        .iter()
        .find(|c| c.id == "harness.claude")
        .unwrap();
    assert_eq!(claude.status, CheckStatus::Ok, "{claude:?}");
    let d = claude.details.as_ref().unwrap();
    assert_eq!(d["path"], fake_cli().display().to_string());
    assert_eq!(d["version"], "2.1.0");
    assert_eq!(d["compatible"], true);
    assert_eq!(d["source"], "env");
    assert_eq!(d["auth_status"], "logged_in");
    let auth = report
        .checks
        .iter()
        .find(|c| c.id == "auth.claude")
        .unwrap();
    assert_eq!(auth.status, CheckStatus::Ok);
}

/// Alle Dateien mit Größe, Rechten und Änderungszeit.
fn snapshot(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let m = e.metadata().unwrap();
            out.push(format!(
                "{} {} {:?} {:?}",
                e.path().display(),
                m.len(),
                m.permissions(),
                m.modified().ok()
            ));
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    out.sort();
    out
}

#[cfg(unix)]
#[tokio::test]
async fn obs_005_ac4_doctor_changes_nothing_in_a_read_only_home() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    // Datenverzeichnis wie nach `beton serve`: Token und Datenbank.
    beton_server::local_auth::LocalToken::load_or_create(home.path()).unwrap();
    let store = beton_store::Store::open(home.path(), beton_store::StoreOptions::default())
        .await
        .unwrap();
    store.ensure_local().await.unwrap();
    store.close().await;
    let set_mode = |mode: u32| {
        for p in [home.path().to_path_buf(), home.path().join("blobs")] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    };
    set_mode(0o500);
    let before = snapshot(home.path());
    let (code, report, _) = doctor(&mut isolated(home.path(), empty.path(), true));
    let after = snapshot(home.path());
    set_mode(0o700);
    assert!(matches!(code, Some(0..=2)), "{code:?}");
    assert_eq!(before, after, "doctor hat etwas verändert");
    assert!(report.checks.iter().any(|c| c.id == "database"));
    // Ohne Daemon keine Verbindung nach außen: der Daemon-Check meldet nur `warn`.
    let daemon = report.checks.iter().find(|c| c.id == "daemon").unwrap();
    assert_eq!(daemon.status, CheckStatus::Warn);
}

#[test]
fn cli_005_ac2_doctor_json_gives_id_status_message_hint() {
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let (_, _, raw) = doctor(&mut isolated(home.path(), empty.path(), false));
    let checks = raw["checks"].as_array().unwrap();
    assert!(!checks.is_empty());
    for c in checks {
        let keys: Vec<&str> = c.as_object().unwrap().keys().map(String::as_str).collect();
        for key in ["id", "status", "message", "hint"] {
            assert!(keys.contains(&key), "{key} fehlt in {c}");
        }
    }
}

#[tokio::test]
async fn ses_016_ac3_doctor_lists_orphaned_worktrees_with_path_and_size() {
    use beton_core::event::{
        Actor, Event, EventPayload, GitWorktreeCreated, SessionKind, SessionTrigger,
    };
    use beton_core::id::{OrgId, SessionId, UserId};
    let home = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let root = home.path().join("worktrees").join("projekt-1a2b3c4d");
    let kept = root.join("beton-mit-session-ab12");
    let orphan = root.join("beton-ohne-session-cd34");
    for (dir, bytes) in [(&kept, 10), (&orphan, 3000)] {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/datei.txt"), vec![b'x'; bytes]).unwrap();
    }
    // Eine Session kennt `kept`.
    let store = beton_store::Store::open(home.path(), beton_store::StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let s = store
        .create_session(
            OrgId::LOCAL,
            beton_store::NewSession {
                id: SessionId::new(),
                owner: UserId::LOCAL,
                kind: SessionKind::Main,
                harness: "fake".into(),
                cwd: "/projekt".into(),
                model: None,
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
    store
        .append(
            OrgId::LOCAL,
            s.id,
            s.head_seq,
            s.epoch,
            vec![Event::new(
                s.id,
                0,
                Actor::default(),
                EventPayload::GitWorktreeCreated(GitWorktreeCreated {
                    path: kept.display().to_string(),
                    branch: "beton/mit-session-ab12".into(),
                    base: "main".into(),
                    base_sha: "0".repeat(40),
                }),
            )],
        )
        .await
        .unwrap();
    store.close().await;

    let (code, report, _) = doctor(&mut isolated(home.path(), empty.path(), true));
    assert_eq!(code, Some(1), "Warnung");
    let c = report.checks.iter().find(|c| c.id == "worktrees").unwrap();
    assert_eq!(c.status, CheckStatus::Warn);
    let orphan_path = orphan.canonicalize().unwrap().display().to_string();
    assert!(c.message.contains(&orphan_path), "{}", c.message);
    assert!(c.message.contains("2,9 KiB"), "Größe: {}", c.message);
    assert!(!c.message.contains("mit-session"), "{}", c.message);
    let details = c.details.as_ref().unwrap();
    assert_eq!(details["orphans"].as_array().unwrap().len(), 1);
    assert_eq!(details["orphans"][0]["path"], orphan_path.as_str());
    assert_eq!(details["orphans"][0]["size_bytes"], 3000);
    assert!(c.hint.as_deref().unwrap().contains("git worktree remove"));
}
