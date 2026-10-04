//! Transcript-Import für Claude Code (HAR-023) mit synthetischen, anonymisierten Fixtures unter
//! `tests/golden/import/<fall>/` (`session.jsonl`, `meta.yaml`). Erwartungen neu schreiben mit
//! `BETON_BLESS=1`. Echte Verläufe unter `~/.claude` werden nie gelesen: Jeder Test arbeitet in
//! einem eigenen temporären `CLAUDE_CONFIG_DIR`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use beton_harness::HostEnv;
use beton_harness::golden::GOLDEN_WORKDIR;
use beton_harness::import::{
    AccessLog, ExternalSessionRef, ImportError, TranscriptImporter, WarningKind, check_golden,
};
use beton_harness_claude::import::ClaudeImporter;
use beton_harness_claude::rebuild::project_dir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/import")
}

fn meta(dir: &Path, key: &str) -> String {
    std::fs::read_to_string(dir.join("meta.yaml"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")).map(str::to_owned))
        .unwrap()
}

fn cases() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("session.jsonl").is_file())
        .collect();
    out.sort();
    out
}

fn env_for(config: &Path) -> HostEnv {
    let mut env = HostEnv::default();
    env.vars
        .insert("CLAUDE_CONFIG_DIR".into(), config.display().to_string());
    env
}

/// Legt die Session-Datei eines Falls wie die CLI ab: `projects/<slug>/<id>.jsonl`.
fn install(config: &Path, case: &Path) -> String {
    let id = meta(case, "session_id");
    let dir = project_dir(config, Path::new(GOLDEN_WORKDIR));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(case.join("session.jsonl"), dir.join(format!("{id}.jsonl"))).unwrap();
    id
}

#[test]
fn har_023_ac1_fixtures_from_two_cli_versions_match_goldens() {
    let cases = cases();
    let mut versions: Vec<String> = cases.iter().map(|c| meta(c, "cli_version")).collect();
    versions.sort();
    versions.dedup();
    assert!(
        versions.len() >= 2,
        "Fixtures aus mindestens zwei CLI-Versionen: {versions:?}"
    );
    let mut failures = Vec::new();
    for case in &cases {
        let tmp = tempfile::tempdir().unwrap();
        let id = install(tmp.path(), case);
        let importer = ClaudeImporter::default();
        let found = importer.discover(&env_for(tmp.path())).unwrap();
        let r = found.iter().find(|r| r.id == id).unwrap();
        assert_eq!(r.cwd.as_deref(), Some(GOLDEN_WORKDIR));
        let t = importer.parse(r).unwrap();
        assert!(
            !t.warnings
                .iter()
                .any(|w| matches!(w.kind, WarningKind::CorruptLine | WarningKind::LineTooLong)),
            "{}: ohne Fehler geparst: {:?}",
            case.display(),
            t.warnings
        );
        assert_eq!(t.native_session_ref, id);
        assert_eq!(
            t.cli_version.as_deref(),
            Some(meta(case, "cli_version").as_str())
        );
        if let Err(e) = check_golden(case, &t, GOLDEN_WORKDIR) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn har_023_subagent_tool_calls_are_nested_under_the_task_call() {
    let case = fixtures().join("claude-2.1.285-rewind-subagent");
    let tmp = tempfile::tempdir().unwrap();
    install(tmp.path(), &case);
    let importer = ClaudeImporter::default();
    let r = importer.discover(&env_for(tmp.path())).unwrap().remove(0);
    let t = importer.parse(&r).unwrap();
    let nested: Vec<_> = t
        .events
        .iter()
        .filter_map(|e| match &e.payload {
            beton_core::event::EventPayload::ToolCallRequested(c) => {
                Some((c.tool.clone(), c.parent_call_id.clone()))
            }
            _ => None,
        })
        .collect();
    assert!(
        nested.contains(&(
            "Grep".to_owned(),
            Some("toolu_01SynthTask000000000000".to_owned())
        )),
        "{nested:?}"
    );
    assert!(nested.contains(&("Task".to_owned(), None)));
}

#[cfg(unix)]
#[test]
fn har_023_ac4_discover_reads_only_session_files_and_never_credentials() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("claude");
    let project = config.join("projects/-w");
    std::fs::create_dir_all(project.join("sub")).unwrap();
    // Dateien der CLI, die beton nie lesen darf.
    let credentials = config.join(".credentials.json");
    std::fs::write(&credentials, r#"{"oauth":"nicht-lesen"}"#).unwrap();
    std::fs::write(config.join("settings.json"), "{}").unwrap();
    std::fs::write(config.join("history.jsonl"), "{}\n").unwrap();
    std::fs::create_dir_all(config.join("todos")).unwrap();
    std::fs::write(
        config.join("todos/11111111-2222-4333-8444-000000000009.jsonl"),
        "{}\n",
    )
    .unwrap();
    // Die echte Session-Datei.
    let session = project.join("11111111-2222-4333-8444-555555555555.jsonl");
    std::fs::write(
        &session,
        r#"{"type":"user","uuid":"a","parentUuid":null,"cwd":"/w","message":{"role":"user","content":"Hallo"}}"#,
    )
    .unwrap();
    // Fallen: falscher Name, Symlink auf die Credentials, Symlink-Verzeichnis, zu tief.
    std::fs::write(project.join("notes.txt"), "x").unwrap();
    symlink(
        &credentials,
        project.join("11111111-2222-4333-8444-666666666666.jsonl"),
    )
    .unwrap();
    symlink(&config, config.join("projects/evil")).unwrap();
    std::fs::write(
        project.join("sub/11111111-2222-4333-8444-777777777777.jsonl"),
        "{}\n",
    )
    .unwrap();

    let log: AccessLog = Arc::default();
    let importer = ClaudeImporter {
        audit: Some(log.clone()),
    };
    let found = importer.discover(&env_for(&config)).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    let real = std::fs::canonicalize(&session).unwrap();
    assert_eq!(found[0].path, real);
    assert_eq!(found[0].title.as_deref(), Some("Hallo"));
    importer.parse(&found[0]).unwrap();
    let opened = log.lock().unwrap().clone();
    assert!(!opened.is_empty());
    assert!(
        opened.iter().all(|p| *p == real),
        "nur die Session-Datei gelesen: {opened:?}"
    );

    // Gefälschte Referenzen werden abgelehnt, bevor etwas geöffnet wird (fail closed).
    let root = std::fs::canonicalize(config.join("projects")).unwrap();
    let canonical_config = std::fs::canonicalize(&config).unwrap();
    for (root, path) in [
        (root.clone(), canonical_config.join(".credentials.json")),
        (
            root.clone(),
            root.join("-w/11111111-2222-4333-8444-666666666666.jsonl"),
        ),
        (root.clone(), root.join("evil/.credentials.json")),
        (
            canonical_config.clone(),
            canonical_config.join(".credentials.json"),
        ),
        (
            canonical_config.clone(),
            canonical_config.join("history.jsonl"),
        ),
    ] {
        let forged = ExternalSessionRef {
            root,
            path: path.clone(),
            ..found[0].clone()
        };
        let err = importer.parse(&forged).unwrap_err();
        assert!(
            matches!(err, ImportError::NotAllowed(_)),
            "{}: {err}",
            path.display()
        );
    }
    let opened = log.lock().unwrap().clone();
    assert!(opened.iter().all(|p| *p == real), "{opened:?}");
}

#[test]
fn har_023_discover_uses_home_without_claude_config_dir_and_ignores_missing_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = HostEnv::default();
    env.vars
        .insert("HOME".into(), tmp.path().display().to_string());
    // Kein `~/.claude`: keine Kandidaten, kein Fehler.
    assert!(ClaudeImporter::default().discover(&env).unwrap().is_empty());
    let case = fixtures().join("claude-1.0.98-interrupt-mcp");
    let id = install(&tmp.path().join(".claude"), &case);
    let found = ClaudeImporter::default().discover(&env).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id);
    assert_eq!(
        found[0].title.as_deref(),
        Some("Flaky Payment-Test stabilisieren")
    );
    // Ohne HOME und Variable: Fehler statt Raten.
    let err = ClaudeImporter::default()
        .discover(&HostEnv::default())
        .unwrap_err();
    assert!(matches!(err, ImportError::Unconfigured(_)), "{err}");
}

#[test]
fn ses_008_native_resumable_checks_the_project_file_without_reading_it() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_for(tmp.path());
    let case = fixtures().join("claude-1.0.98-interrupt-mcp");
    let id = install(tmp.path(), &case);
    let importer = ClaudeImporter::default();
    let cwd = Path::new(GOLDEN_WORKDIR);
    assert!(importer.native_resumable(&env, cwd, &id));
    assert!(!importer.native_resumable(&env, Path::new("/anderswo"), &id));
    assert!(!importer.native_resumable(&env, cwd, "../../.credentials"));
    std::fs::remove_file(project_dir(tmp.path(), cwd).join(format!("{id}.jsonl"))).unwrap();
    assert!(!importer.native_resumable(&env, cwd, &id));
}
