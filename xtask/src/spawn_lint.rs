//! SBX-001 AC2: Harness- und Tool-Prozesse entstehen nur in `beton-sandbox`.
//!
//! Durchgesetzt von Clippy (`disallowed-methods` in `clippy.toml`, CI-Job `fmt + clippy` mit
//! `-D warnings`). Dieser Check hält die Konfiguration fest und begrenzt die Ausnahmen:
//! - `clippy.toml` verbietet alle Wege, ein `Command` zu bauen;
//! - Ausnahmen (`#[expect(clippy::disallowed_methods, reason = …)]`) gibt es außerhalb von
//!   `beton-sandbox` (und künftig `beton-pty`) nur für die gelisteten beton-eigenen Prozesse
//!   (Stufe 0) und in Tests;
//! - Harness-, Runner-, MCP- und Host-Crates haben keine einzige Ausnahme und bauen kein
//!   `Command` selbst; ein pauschales `allow(clippy::all)`/`allow(clippy::style)` ist
//!   nirgends erlaubt, weil es den Lint abschalten würde.

use std::path::{Path, PathBuf};

/// Pfade, die `clippy.toml` verbieten muss.
pub const DISALLOWED: [&str; 3] = [
    "std::process::Command::new",
    "tokio::process::Command::new",
    "process_wrap::tokio::CommandWrap::with_new",
];

/// Crates (bzw. Pfad-Präfixe), in denen Prozesse ausschließlich über `beton-sandbox` starten.
pub const NO_EXCEPTIONS: [&str; 4] = [
    "crates/beton-harness",
    "crates/beton-runner/",
    "crates/beton-mcp/",
    "crates/beton-host/",
];

/// Crates, in denen Prozesse entstehen dürfen (SBX-001).
pub const SPAWN_CRATES: [&str; 2] = ["crates/beton-sandbox/", "crates/beton-pty/"];

/// Beton-eigene Prozesse (Stufe 0) außerhalb der Sandbox, jeweils mit Grund.
pub const STAGE0_EXCEPTIONS: [(&str, &str); 9] = [
    (
        "crates/beton-cli/src/commands.rs",
        "Browser und Editor des Benutzers",
    ),
    (
        "crates/beton-cli/src/daemon.rs",
        "Start des eigenen Daemons",
    ),
    (
        "crates/beton-cli/src/setup.rs",
        "Installation und Login einer Vendor-CLI im Vordergrund auf Wunsch des Benutzers",
    ),
    (
        "crates/beton-desktop/src/daemon.rs",
        "Start des eigenen Daemons als Sidecar der Desktop-App (DESK-002)",
    ),
    (
        "crates/beton-git/src/cmd.rs",
        "git als Stufe-0-Werkzeug von beton",
    ),
    (
        "crates/beton-server/src/workspace.rs",
        "git der Workspace-API (Stufe 0)",
    ),
    (
        "crates/beton-server/src/credentials.rs",
        "Credential-Quelle `command` des Proxys (PRX-006, Stufe 0)",
    ),
    (
        "crates/beton-fake-cli/src/mcp.rs",
        "Fake-Harness für Tests startet seine MCP-Server wie eine Vendor-CLI",
    ),
    (
        "crates/beton-fake-cli/src/stream_json.rs",
        "Fake-Harness für Tests ruft den PreToolUse-Hook wie Claude Code per Shell auf (HAR-005)",
    ),
];

fn is_test_path(rel: &str) -> bool {
    rel.contains("/tests/") || rel.ends_with("_tests.rs") || rel.ends_with("/tests.rs")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Liest die verbotenen Methoden aus `clippy.toml`.
fn configured(root: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(root.join("clippy.toml"))
        .map_err(|e| format!("clippy.toml: {e}"))?;
    let table: toml::Table = text.parse().map_err(|e| format!("clippy.toml: {e}"))?;
    let Some(list) = table.get("disallowed-methods").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    Ok(list
        .iter()
        .filter_map(|v| match v {
            toml::Value::String(s) => Some(s.clone()),
            toml::Value::Table(t) => t.get("path").and_then(|p| p.as_str()).map(str::to_owned),
            _ => None,
        })
        .collect())
}

/// Alle Verstöße im Repo unter `root` (leer = in Ordnung).
pub fn check(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    match configured(root) {
        Ok(paths) => {
            for p in DISALLOWED {
                if !paths.iter().any(|c| c == p) {
                    errors.push(format!("clippy.toml: `{p}` fehlt in disallowed-methods"));
                }
            }
        }
        Err(e) => errors.push(e),
    }
    let mut files = Vec::new();
    for dir in ["crates", "xtask"] {
        rust_files(&root.join(dir), &mut files);
    }
    for file in files {
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        // Dieser Check nennt die Muster selbst.
        if rel == "xtask/src/spawn_lint.rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for blanket in [
            "allow(clippy::all",
            "allow(clippy::style",
            "expect(clippy::all",
            "expect(clippy::style",
        ] {
            if text.contains(blanket) {
                errors.push(format!("{rel}: `{blanket})` schaltet den Spawn-Lint ab"));
            }
        }
        if SPAWN_CRATES.iter().any(|c| rel.starts_with(c)) || is_test_path(&rel) {
            continue;
        }
        let has_exception = text.contains("clippy::disallowed_methods");
        let strict = NO_EXCEPTIONS.iter().any(|c| rel.starts_with(c));
        if strict {
            if has_exception {
                errors.push(format!(
                    "{rel}: Ausnahme vom Spawn-Lint in einem Harness-/Runner-Crate; Prozesse starten über beton_sandbox"
                ));
            }
            for needle in ["Command::new", "CommandWrap", "Command::from"] {
                if text.contains(needle) {
                    errors.push(format!(
                        "{rel}: `{needle}` außerhalb von beton-sandbox; Prozesse starten über SandboxProvider::spawn"
                    ));
                }
            }
            continue;
        }
        if has_exception
            && !rel.starts_with("xtask/")
            && !STAGE0_EXCEPTIONS.iter().any(|(f, _)| *f == rel)
        {
            errors.push(format!(
                "{rel}: neue Ausnahme vom Spawn-Lint; nur Stufe-0-Prozesse in xtask/src/spawn_lint.rs::STAGE0_EXCEPTIONS"
            ));
        }
    }
    for (file, _) in STAGE0_EXCEPTIONS {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        if !text.contains("clippy::disallowed_methods") {
            errors.push(format!(
                "{file}: als Ausnahme gelistet, nutzt aber keine (Liste veraltet)"
            ));
        } else if !text.contains("reason =") {
            errors.push(format!("{file}: Ausnahme ohne `reason`"));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    const CLIPPY: &str = "disallowed-methods = [\n  { path = \"std::process::Command::new\", reason = \"x\" },\n  { path = \"tokio::process::Command::new\", reason = \"x\" },\n  { path = \"process_wrap::tokio::CommandWrap::with_new\", reason = \"x\" },\n]\n";

    fn exceptions() -> Vec<(String, String)> {
        STAGE0_EXCEPTIONS
            .iter()
            .map(|(f, _)| {
                (
                    (*f).to_owned(),
                    "#[expect(clippy::disallowed_methods, reason = \"Stufe 0\")]\nfn f() {}\n"
                        .to_owned(),
                )
            })
            .collect()
    }

    fn with(extra: &[(&str, &str)]) -> Vec<String> {
        let mut files: Vec<(String, String)> = exceptions();
        files.push(("clippy.toml".into(), CLIPPY.into()));
        files.extend(
            extra
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned())),
        );
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let dir = tree(&refs);
        check(dir.path())
    }

    #[test]
    fn sbx_001_ac2_repo_has_no_direct_spawns_outside_beton_sandbox() {
        let errors = check(&crate::repo_root());
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn sbx_001_ac2_lint_flags_spawns_and_exceptions_in_harness_crates() {
        assert!(with(&[]).is_empty(), "{:?}", with(&[]));
        let errors = with(&[(
            "crates/beton-harness-codex/src/lib.rs",
            "fn f() { let _ = tokio::process::Command::new(\"codex\"); }\n",
        )]);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("beton-harness-codex") && e.contains("Command::new")),
            "{errors:?}"
        );
        let errors = with(&[(
            "crates/beton-runner/src/lib.rs",
            "#[allow(clippy::disallowed_methods)]\nfn f() {}\n",
        )]);
        assert!(
            errors.iter().any(|e| e.contains("beton-runner")),
            "{errors:?}"
        );
        let errors = with(&[("crates/beton-store/src/lib.rs", "#![allow(clippy::all)]\n")]);
        assert!(
            errors.iter().any(|e| e.contains("clippy::all")),
            "{errors:?}"
        );
        let errors = with(&[(
            "crates/beton-server/src/neu.rs",
            "#[expect(clippy::disallowed_methods, reason = \"x\")]\nfn f() {}\n",
        )]);
        assert!(
            errors.iter().any(|e| e.contains("neue Ausnahme")),
            "{errors:?}"
        );
        // Tests und beton-sandbox dürfen.
        assert!(
            with(&[
                (
                    "crates/beton-runner/tests/e2e.rs",
                    "#![allow(clippy::disallowed_methods)]\n"
                ),
                (
                    "crates/beton-sandbox/src/child.rs",
                    "fn f() { std::process::Command::new(\"x\"); }\n"
                ),
            ])
            .is_empty()
        );
    }

    #[test]
    fn sbx_001_ac2_clippy_config_must_forbid_all_command_constructors() {
        let dir = tree(&[("clippy.toml", "allow-unwrap-in-tests = true\n")]);
        let errors = check(dir.path());
        for p in DISALLOWED {
            assert!(errors.iter().any(|e| e.contains(p)), "{p}: {errors:?}");
        }
    }
}
