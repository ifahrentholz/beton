//! CI-Check `security-tests-touched` (QA-013 AC2).
//!
//! Ändert ein PR Code in einem sicherheitskritischen Bereich (`beton-policy`, `beton-sandbox`,
//! `beton-proxy`, `beton-secrets`, Auth-Code in `beton-server`), muss er im selben Bereich
//! mindestens eine Testdatei ändern oder neu anlegen. Ausnahme: Label `no-test-needed`, das
//! nur ein Mensch setzen darf (die CI prüft den Setzenden über die GitHub-API).
//!
//! Außerdem QA-013 AC1 in der Übergangsform aus ADR-0036: Ein PR, der irgendeinen dieser
//! Bereiche berührt (Code oder Tests), braucht das Label `security-review`.

/// Ein sicherheitskritischer Bereich: Code-Pfade (Präfixe) und Pfade seiner Tests.
#[derive(Debug, Clone, Copy)]
pub struct Area {
    pub name: &'static str,
    pub code: &'static [&'static str],
    pub tests: &'static [&'static str],
}

/// Die Bereiche aus QA-013; dieselben Pfade stehen in `.github/CODEOWNERS`.
pub const AREAS: [Area; 5] = [
    Area {
        name: "beton-policy",
        code: &["crates/beton-policy/"],
        tests: &["crates/beton-policy/"],
    },
    Area {
        name: "beton-sandbox",
        code: &["crates/beton-sandbox/"],
        tests: &["crates/beton-sandbox/"],
    },
    Area {
        name: "beton-proxy",
        code: &["crates/beton-proxy/"],
        tests: &["crates/beton-proxy/"],
    },
    Area {
        name: "beton-secrets",
        code: &["crates/beton-secrets/"],
        tests: &["crates/beton-secrets/"],
    },
    Area {
        name: "beton-server (Auth)",
        code: &[
            "crates/beton-server/src/local_auth.rs",
            "crates/beton-server/src/security.rs",
        ],
        tests: &["crates/beton-server/tests/"],
    },
];

/// Eine Dateiänderung aus `git diff --name-status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `A`, `M`, `D`, `R…`, `C…`
    pub status: String,
    pub path: String,
}

/// Liest die Ausgabe von `git diff --name-status`; bei Umbenennungen zählt der neue Pfad.
pub fn parse_name_status(text: &str) -> Vec<Change> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.trim().to_owned();
            let path = parts.next_back()?.trim().to_owned();
            (!status.is_empty() && !path.is_empty()).then_some(Change { status, path })
        })
        .collect()
}

/// Ist der Pfad eine Testdatei? Testverzeichnisse (`tests/`, `benches/`, `fuzz/`,
/// `testdata/`), Test-Module (`tests.rs`, `*_test(s).rs`) und Policy-Tests
/// (`*.policytest.yaml`).
pub fn is_test_file(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let (file, dirs) = segments.split_last().unwrap_or((&"", &[]));
    dirs.iter()
        .any(|d| matches!(*d, "tests" | "benches" | "fuzz" | "testdata"))
        || *file == "tests.rs"
        || file.ends_with("_tests.rs")
        || file.ends_with("_test.rs")
        || file.ends_with(".policytest.yaml")
}

/// Zählt die Änderung als Codeänderung? Alles außer Tests und Markdown-Doku.
fn is_code(path: &str) -> bool {
    !is_test_file(path) && !path.ends_with(".md")
}

/// Bereiche mit Codeänderung, aber ohne geänderte oder neue Testdatei.
pub fn violations(changes: &[Change]) -> Vec<&'static str> {
    AREAS
        .iter()
        .filter(|area| {
            let code_changed = changes
                .iter()
                .any(|c| area.code.iter().any(|p| c.path.starts_with(p)) && is_code(&c.path));
            let tests_touched = changes.iter().any(|c| {
                !c.status.starts_with('D')
                    && is_test_file(&c.path)
                    && area.tests.iter().any(|p| c.path.starts_with(p))
            });
            code_changed && !tests_touched
        })
        .map(|area| area.name)
        .collect()
}

/// QA-013 AC1 (ADR-0036): berührte Bereiche, wenn das Label `security-review` fehlt.
pub fn missing_review_label(changes: &[Change], has_label: bool) -> Vec<&'static str> {
    if has_label {
        return Vec::new();
    }
    AREAS
        .iter()
        .filter(|area| {
            changes.iter().any(|c| {
                area.code
                    .iter()
                    .chain(area.tests)
                    .any(|p| c.path.starts_with(p))
            })
        })
        .map(|area| area.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(status: &str, path: &str) -> Change {
        Change {
            status: status.into(),
            path: path.into(),
        }
    }

    #[test]
    fn qa_013_ac1_sandbox_change_requires_security_review_label() {
        let changes = [change("M", "crates/beton-sandbox/src/seatbelt.rs")];
        assert_eq!(missing_review_label(&changes, false), ["beton-sandbox"]);
        assert!(missing_review_label(&changes, true).is_empty());
    }

    #[test]
    fn qa_013_ac1_any_security_path_counts_tests_included() {
        let changes = [
            change("A", "crates/beton-policy/tests/combine.rs"),
            change("M", "crates/beton-server/src/security.rs"),
            change("M", "crates/beton-server/src/queue.rs"),
            change("M", "docs/spec/03-policies.md"),
        ];
        assert_eq!(
            missing_review_label(&changes, false),
            ["beton-policy", "beton-server (Auth)"]
        );
        let unrelated = [change("M", "crates/beton-server/src/queue.rs")];
        assert!(missing_review_label(&unrelated, false).is_empty());
    }

    #[test]
    fn qa_013_ac2_sandbox_code_without_test_change_fails() {
        let changes = [change("M", "crates/beton-sandbox/src/seatbelt.rs")];
        assert_eq!(violations(&changes), ["beton-sandbox"]);
    }

    #[test]
    fn qa_013_ac2_sandbox_code_with_new_test_passes() {
        let changes = [
            change("M", "crates/beton-sandbox/src/seatbelt.rs"),
            change("A", "crates/beton-sandbox/tests/escape.rs"),
        ];
        assert!(violations(&changes).is_empty());
    }

    #[test]
    fn qa_013_ac2_test_in_another_area_does_not_count() {
        let changes = [
            change("M", "crates/beton-policy/src/combine.rs"),
            change("M", "crates/beton-sandbox/tests/escape.rs"),
        ];
        assert_eq!(violations(&changes), ["beton-policy"]);
    }

    #[test]
    fn qa_013_ac2_deleting_a_test_is_no_test_change() {
        let changes = [
            change("M", "crates/beton-proxy/src/lib.rs"),
            change("D", "crates/beton-proxy/tests/rules.rs"),
        ];
        assert_eq!(violations(&changes), ["beton-proxy"]);
    }

    #[test]
    fn qa_013_ac2_server_auth_code_needs_server_tests() {
        let changes = [change("M", "crates/beton-server/src/local_auth.rs")];
        assert_eq!(violations(&changes), ["beton-server (Auth)"]);
        let other = [change("M", "crates/beton-server/src/queue.rs")];
        assert!(violations(&other).is_empty());
        let with_test = [
            change("M", "crates/beton-server/src/security.rs"),
            change("M", "crates/beton-server/tests/security.rs"),
        ];
        assert!(violations(&with_test).is_empty());
    }

    #[test]
    fn qa_013_ac2_docs_only_and_test_modules_are_recognized() {
        assert!(violations(&[change("M", "crates/beton-secrets/README.md")]).is_empty());
        assert!(is_test_file("crates/beton-policy/src/cel/tests.rs"));
        assert!(is_test_file(
            "crates/beton-policy/policies/git.policytest.yaml"
        ));
        assert!(is_test_file("crates/beton-sandbox/fuzz/fuzz_targets/a.rs"));
        assert!(!is_test_file("crates/beton-policy/src/attestation.rs"));
    }

    #[test]
    fn qa_013_ac2_parses_git_name_status() {
        let text = "M\tcrates/a/src/x.rs\nR100\told/tests/a.rs\tcrates/beton-policy/tests/a.rs\n";
        let changes = parse_name_status(text);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[1].path, "crates/beton-policy/tests/a.rs");
    }

    #[test]
    fn qa_013_ac3_pr_template_contains_the_security_checklist() {
        let text =
            std::fs::read_to_string(crate::repo_root().join(".github/pull_request_template.md"))
                .unwrap();
        assert!(
            text.contains("Test zuerst (rot) — Implementierung (grün) — Escape-/Policy-Suite grün"),
            "Checkliste aus QA-013 AC3 fehlt in der PR-Vorlage"
        );
    }

    /// Die CODEOWNERS-Datei weist alle Bereiche dem Maintainer zu (Voraussetzung für
    /// QA-013 AC1; die Branch-Protection selbst stellt der Maintainer ein).
    #[test]
    fn codeowners_cover_all_security_critical_areas() {
        let text = std::fs::read_to_string(crate::repo_root().join(".github/CODEOWNERS")).unwrap();
        for area in AREAS {
            for path in area.code {
                assert!(
                    text.lines()
                        .any(|l| l.starts_with(&format!("/{path}")) && l.contains("@ifahrentholz")),
                    "CODEOWNERS deckt {path} nicht ab"
                );
            }
        }
    }
}
