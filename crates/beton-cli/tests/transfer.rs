//! Integrationstests für `beton import` und `beton export` (CLI-007) gegen einen echten
//! Daemon. Die Vendor-Verzeichnisse sind temporär (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) und
//! enthalten nur die synthetischen Fixtures der Adapter-Crates; echte Verläufe werden nie
//! gelesen.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{Serve, beton, run, stderr, stdout};
use serde_json::Value;

fn fixture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../beton-harness-claude/tests/golden/import/claude-1.0.98-interrupt-mcp/session.jsonl",
    );
    std::fs::read_to_string(path).unwrap()
}

/// Vendor-Verzeichnisse mit `n` Claude-Chats; Chat 0 ist der jüngste.
struct Vendor {
    dir: tempfile::TempDir,
    ids: Vec<String>,
}

impl Vendor {
    fn new(work: &Path, n: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("claude/projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(dir.path().join("codex/sessions")).unwrap();
        let base = fixture().replace("/beton-golden/workdir", &work.display().to_string());
        let mut ids = Vec::new();
        for i in 0..n {
            let id = format!("3f1c2a9e-5b7d-4c8e-9a1f-{i:012}");
            let path = project.join(format!("{id}.jsonl"));
            std::fs::write(
                &path,
                base.replace("Flaky Payment-Test stabilisieren", &format!("Chat {i}")),
            )
            .unwrap();
            let mtime = SystemTime::now() - Duration::from_secs(3600 * (i as u64 + 1));
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
            ids.push(id);
        }
        Self { dir, ids }
    }

    fn env(&self) -> [(String, String); 2] {
        [
            (
                "CLAUDE_CONFIG_DIR".into(),
                self.dir.path().join("claude").display().to_string(),
            ),
            (
                "CODEX_HOME".into(),
                self.dir.path().join("codex").display().to_string(),
            ),
        ]
    }
}

fn serve_with(vendor: &Vendor) -> Serve {
    let env = vendor.env();
    let refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    Serve::start_with(&refs)
}

#[tokio::test]
async fn cli_007_ac1_import_last_5_imports_the_five_newest_claude_sessions_and_prints_ids() {
    let work = tempfile::tempdir().unwrap();
    let vendor = Vendor::new(work.path(), 6);
    let serve = serve_with(&vendor);
    let out = run(beton(serve.home()).args(["import", "--harness", "claude", "--last", "5"]));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "{text}");
    let mut printed = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let rest = line.strip_prefix("✓ ").unwrap();
        let (id, title) = rest.split_once("  ").unwrap();
        assert!(id.starts_with("ses_"), "{line}");
        assert_eq!(title, format!("Chat {i}"), "jüngste zuerst");
        printed.push(id.to_owned());
    }
    assert!(
        stderr(&out).contains("5 Sessions importiert aus Claude Code"),
        "{}",
        stderr(&out)
    );
    // Die fünf jüngsten sind übernommen, der älteste nicht.
    let page = serve
        .client()
        .import_candidates("claude", Some(50), None)
        .await
        .unwrap();
    let imported: HashMap<String, Option<String>> = page
        .items
        .iter()
        .map(|c| {
            (
                c["vendor_session_id"].as_str().unwrap().to_owned(),
                c["imported_session_id"].as_str().map(str::to_owned),
            )
        })
        .collect();
    for (i, id) in vendor.ids.iter().enumerate() {
        if i < 5 {
            assert_eq!(imported[id].as_deref(), Some(printed[i].as_str()));
        } else {
            assert_eq!(imported[id], None);
        }
    }
    // Ein zweiter Lauf überspringt die schon übernommenen.
    let again = run(beton(serve.home()).args(["import", "--harness", "claude", "--last", "5"]));
    assert!(again.status.success());
    assert!(
        stdout(&again)
            .lines()
            .all(|l| l.ends_with("(schon übernommen)"))
    );
}

#[tokio::test]
async fn cli_007_import_without_arguments_and_terminal_lists_candidates_and_exits_2() {
    let work = tempfile::tempdir().unwrap();
    let vendor = Vendor::new(work.path(), 2);
    let serve = serve_with(&vendor);
    let out = run(beton(serve.home()).args(["import"]));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains(&vendor.ids[0]) && text.contains("Chat 1"),
        "{text}"
    );
    assert!(stderr(&out).contains("--last N"));
    // Es wurde nichts importiert.
    assert!(serve.client().all_sessions(true).await.unwrap().is_empty());
}

fn export_file(serve: &Serve, id: &str, path: &Path, extra: &[&str]) {
    let out = run(beton(serve.home())
        .args(["export", id, "-o"])
        .arg(path)
        .args(extra));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).is_empty(), "Daten nur in der Datei");
    assert!(stderr(&out).contains("Events"), "{}", stderr(&out));
}

/// Event-Zeilen ohne Session- und Event-IDs (vergibt der Import neu).
fn events_of(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap();
    let events: Vec<Value> = text
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let seq_of: HashMap<String, Value> = events
        .iter()
        .map(|e| (e["id"].as_str().unwrap().to_owned(), e["seq"].clone()))
        .collect();
    events
        .into_iter()
        .map(|mut e| {
            e["session_id"] = Value::Null;
            e["id"] = Value::Null;
            if let Some(c) = e.get("causation_id").and_then(Value::as_str) {
                e["causation_id"] = seq_of[c].clone();
            }
            e
        })
        .collect()
}

#[tokio::test]
async fn cli_007_ac2_export_then_import_creates_a_session_with_the_same_content() {
    let serve = Serve::start();
    let id = serve.create_idle_session().await;
    let dir = tempfile::tempdir().unwrap();
    let file: PathBuf = dir.path().join("s.jsonl");
    export_file(&serve, &id, &file, &[]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "Export nur für den Eigentümer lesbar");
    }
    let out = run(beton(serve.home()).arg("import").arg(&file));
    assert!(out.status.success(), "{}", stderr(&out));
    let line = stdout(&out);
    assert!(
        line.starts_with("✓ ses_") && line.contains("(inhaltsgleich, neue ID)"),
        "{line}"
    );
    let new_id = line[4..].split_whitespace().next().unwrap().to_owned();
    assert_ne!(new_id, id);
    let again = dir.path().join("again.jsonl");
    export_file(&serve, &new_id, &again, &[]);
    assert_eq!(events_of(&file), events_of(&again));

    // Mit Anhängen als .tar.zst ebenso; ein zweiter Import derselben Datei wird übersprungen.
    let archive = dir.path().join("s.tar.zst");
    export_file(&serve, &id, &archive, &["--with-blobs", "--with-raw"]);
    let out = run(beton(serve.home()).arg("import").arg(&archive));
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(beton(serve.home()).arg("import").arg(&archive));
    assert!(out.status.success());
    assert!(
        stdout(&out).contains("(schon importiert)"),
        "{}",
        stdout(&out)
    );
}

#[tokio::test]
async fn cli_007_a_broken_file_is_rejected_with_line_number() {
    let serve = Serve::start();
    let id = serve.create_idle_session().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("s.jsonl");
    export_file(&serve, &id, &file, &[]);
    let text = std::fs::read_to_string(&file).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.remove(2);
    let broken = dir.path().join("kaputt.jsonl");
    std::fs::write(&broken, lines.join("\n")).unwrap();
    let before = serve.client().all_sessions(true).await.unwrap().len();
    let out = run(beton(serve.home()).arg("import").arg(&broken));
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("Zeile 3"), "{}", stderr(&out));
    assert_eq!(
        serve.client().all_sessions(true).await.unwrap().len(),
        before
    );
}
