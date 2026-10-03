//! Secret-Scan über alle Golden-Transcripts (QA-003 AC2).
//!
//! Durchsucht jede Datei unterhalb eines Verzeichnisses namens `golden` mit denselben
//! Mustern wie die Aufnahme (`beton_harness::golden::secrets`). Läuft in CI und als
//! Pre-Commit-Hook (`.githooks/pre-commit`).

use std::path::{Path, PathBuf};

use anyhow::Result;
use beton_harness::golden::secrets::find_secrets;

const SKIP: [&str; 4] = [".git", "target", "node_modules", "dist"];

/// Alle Funde als lesbare Zeilen `pfad: Zeile n: …`.
pub fn scan(root: &Path) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    for file in golden_files(root, false)? {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for f in find_secrets(&text) {
            findings.push(format!("{rel}: {f}"));
        }
    }
    Ok(findings)
}

fn golden_files(dir: &Path, inside: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            out.extend(golden_files(&path, inside || name == "golden")?);
        } else if inside && is_recording(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Nur Aufnahme-Dateien, nicht der Quellcode eines Moduls namens `golden`.
fn is_recording(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("jsonl" | "json" | "yaml" | "yml" | "txt" | "log")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qa_003_ac2_scan_flags_embedded_test_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let case = tmp.path().join("crates/x/tests/golden/fall");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join("raw.jsonl"),
            "{\"k\":\"sk-ant-api03-abcdefghijklmnop\"}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("README.md"),
            "sk-ant-api03-abcdefghijklmnop",
        )
        .unwrap();
        let findings = scan(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].starts_with("crates/x/tests/golden/fall/raw.jsonl: Zeile 1"));
        assert!(
            !findings[0].contains("abcdefghijklmnop"),
            "Fund zeigt den Wert"
        );
    }

    #[test]
    fn qa_003_ac2_repository_goldens_are_clean() {
        let findings = scan(&crate::repo_root()).unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }
}
