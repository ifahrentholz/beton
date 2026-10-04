//! Transcript-Import für Codex (HAR-024) mit synthetischen, anonymisierten Fixtures unter
//! `tests/golden/import/<fall>/` (`rollout.jsonl`, `meta.yaml`). Erwartungen neu schreiben mit
//! `BETON_BLESS=1`. Echte Verläufe unter `~/.codex` werden nie gelesen: Jeder Test arbeitet in
//! einem eigenen temporären `CODEX_HOME` bzw. `HOME`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use beton_core::event::{EventPayload, FsChangeKind};
use beton_harness::HostEnv;
use beton_harness::golden::GOLDEN_WORKDIR;
use beton_harness::import::{
    AccessLog, ImportError, ImportedTranscript, TranscriptImporter, WarningKind, check_golden,
};
use beton_harness_codex::import::CodexImporter;

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
        .filter(|p| p.join("rollout.jsonl").is_file())
        .collect();
    out.sort();
    out
}

/// Legt den Rollout eines Falls wie die CLI unter `<codex_home>/sessions/…` ab.
fn install(codex_home: &Path, case: &Path) -> String {
    let file = codex_home.join(meta(case, "file"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::copy(case.join("rollout.jsonl"), &file).unwrap();
    meta(case, "session_id")
}

fn env_with(vars: &[(&str, &Path)]) -> HostEnv {
    let mut env = HostEnv::default();
    for (k, v) in vars {
        env.vars.insert((*k).into(), v.display().to_string());
    }
    env
}

fn import(case: &Path) -> ImportedTranscript {
    let tmp = tempfile::tempdir().unwrap();
    let id = install(tmp.path(), case);
    let importer = CodexImporter::default();
    let found = importer
        .discover(&env_with(&[("CODEX_HOME", tmp.path())]))
        .unwrap();
    let r = found.iter().find(|r| r.id == id).unwrap();
    assert_eq!(r.cwd.as_deref(), Some(GOLDEN_WORKDIR));
    importer.parse(r).unwrap()
}

#[test]
fn har_024_ac1_fixtures_from_two_codex_versions_match_goldens() {
    let cases = cases();
    let mut versions: Vec<String> = cases.iter().map(|c| meta(c, "cli_version")).collect();
    versions.sort();
    versions.dedup();
    assert!(
        versions.len() >= 2,
        "Fixtures aus mindestens zwei Codex-Versionen: {versions:?}"
    );
    let mut failures = Vec::new();
    for case in &cases {
        let t = import(case);
        assert!(
            !t.warnings
                .iter()
                .any(|w| matches!(w.kind, WarningKind::CorruptLine | WarningKind::LineTooLong)),
            "{}: {:?}",
            case.display(),
            t.warnings
        );
        assert_eq!(t.native_session_ref, meta(case, "session_id"));
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
fn har_024_ac2_apply_patch_yields_tool_call_events_with_diff_and_fs_changed() {
    // Custom-Tool mit relativen Pfaden (0.46.0).
    let t = import(&fixtures().join("codex-0.46.0-patch-abort"));
    let call = t
        .events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolCallRequested(c) if c.tool == "apply_patch" => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let changes = call.args["changes"].as_array().unwrap();
    assert_eq!(changes[0]["path"], "spec/payment_spec.js");
    assert!(
        changes[0]["diff"]
            .as_str()
            .unwrap()
            .contains("+  await pay({ timeout: 10000 });")
    );
    assert!(
        call.args["patch"]
            .as_str()
            .unwrap()
            .starts_with("*** Begin Patch")
    );
    let pos = |pred: &dyn Fn(&EventPayload) -> bool| t.events.iter().position(|e| pred(&e.payload));
    let completed = pos(&|p| matches!(p, EventPayload::ToolCallCompleted(c) if c.call_id == call.call_id && c.status == beton_core::event::ToolStatus::Ok)).unwrap();
    let fs = pos(&|p| matches!(p, EventPayload::FsChanged(_))).unwrap();
    assert!(fs > completed, "fs.changed nach dem Ergebnis");
    let EventPayload::FsChanged(f) = &t.events[fs].payload else {
        unreachable!()
    };
    let got: Vec<(String, FsChangeKind)> = f
        .changes
        .iter()
        .map(|c| (c.path.clone(), c.change))
        .collect();
    assert_eq!(
        got,
        [
            ("spec/payment_spec.js".to_owned(), FsChangeKind::Modified),
            ("spec/helpers/clock.js".to_owned(), FsChangeKind::Added),
        ]
    );
    assert_eq!(f.source, "import");

    // Function mit absoluten Pfaden, Umbenennung und Löschung (0.153.2).
    let t = import(&fixtures().join("codex-0.153.2-exec-rename-mcp"));
    let f = t
        .events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::FsChanged(f) => Some(f.clone()),
            _ => None,
        })
        .unwrap();
    let got: Vec<(String, FsChangeKind, Option<String>)> = f
        .changes
        .iter()
        .map(|c| (c.path.clone(), c.change, c.from.clone()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "src/b.ts".to_owned(),
                FsChangeKind::Renamed,
                Some("src/a.ts".to_owned())
            ),
            ("src/alt.ts".to_owned(), FsChangeKind::Deleted, None),
        ]
    );
}

#[test]
fn har_024_ac3_codex_home_is_respected_and_defaults_to_dot_codex() {
    let case = fixtures().join("codex-0.46.0-patch-abort");
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let id = install(elsewhere.path(), &case);
    let importer = CodexImporter::default();
    // Mit CODEX_HOME: dort, nicht unter ~/.codex.
    let found = importer
        .discover(&env_with(&[
            ("HOME", home.path()),
            ("CODEX_HOME", elsewhere.path()),
        ]))
        .unwrap();
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [id.as_str()]
    );
    // Ohne Variable: ~/.codex (hier leer).
    assert!(
        importer
            .discover(&env_with(&[("HOME", home.path())]))
            .unwrap()
            .is_empty()
    );
    let id2 = install(&home.path().join(".codex"), &case);
    let found = importer
        .discover(&env_with(&[("HOME", home.path())]))
        .unwrap();
    assert_eq!(
        found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [id2.as_str()]
    );
    // Leere Variable zählt als nicht gesetzt.
    let mut env = env_with(&[("HOME", home.path())]);
    env.vars.insert("CODEX_HOME".into(), String::new());
    assert_eq!(importer.discover(&env).unwrap().len(), 1);
    let err = importer.discover(&HostEnv::default()).unwrap_err();
    assert!(matches!(err, ImportError::Unconfigured(_)), "{err}");
}

#[cfg(unix)]
#[test]
fn har_024_discover_reads_only_rollouts_and_never_auth_json() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join("codex");
    let auth = codex.join("auth.json");
    std::fs::create_dir_all(codex.join("sessions/2026/10/03")).unwrap();
    std::fs::write(&auth, r#"{"tokens":"nicht-lesen"}"#).unwrap();
    std::fs::write(codex.join("config.toml"), "").unwrap();
    let case = fixtures().join("codex-0.153.2-exec-rename-mcp");
    install(&codex, &case);
    std::os::unix::fs::symlink(
        &auth,
        codex.join("sessions/2026/10/03/rollout-2026-10-03T09-00-00-x.jsonl"),
    )
    .unwrap();
    std::fs::write(codex.join("sessions/2026/10/03/notes.txt"), "x").unwrap();
    let log: AccessLog = Arc::default();
    let importer = CodexImporter {
        audit: Some(log.clone()),
    };
    let found = importer
        .discover(&env_with(&[("CODEX_HOME", &codex)]))
        .unwrap();
    assert_eq!(found.len(), 1);
    importer.parse(&found[0]).unwrap();
    let opened = log.lock().unwrap().clone();
    assert!(opened.iter().all(|p| *p == found[0].path), "{opened:?}");
    let forged = beton_harness::import::ExternalSessionRef {
        path: std::fs::canonicalize(&auth).unwrap(),
        ..found[0].clone()
    };
    assert!(matches!(
        importer.parse(&forged).unwrap_err(),
        ImportError::NotAllowed(_)
    ));
}
