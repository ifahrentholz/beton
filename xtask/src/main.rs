//! Entwickler-Werkzeuge für das beton-Repo.
//!
//! ```text
//! cargo xtask codegen [--check]
//! cargo xtask commit-lint [--range <base>..<head>] [--title <text>]
//! cargo xtask dco [--range <base>..<head>]
//! cargo xtask golden-scan
//! cargo xtask spec-check
//! cargo xtask spec-coverage --milestone <M0…M5> [--report]
//! ```

mod codegen;
mod commits;
mod coverage;
mod golden_scan;
mod spec;

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

use crate::commits::Commit;
use crate::spec::Spec;

/// Wurzel des Repos (Elternverzeichnis von `xtask/`).
pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("xtask: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let root = repo_root();
    match command.as_str() {
        "codegen" => run_codegen(&root, &rest),
        "commit-lint" => commit_lint(&root, &rest),
        "dco" => dco(&rest),
        "golden-scan" => golden_scan(&root),
        "spec-check" => spec_check(&root),
        "spec-coverage" => spec_coverage(&root, &rest),
        _ => {
            eprintln!(
                "Verwendung: cargo xtask <codegen|commit-lint|dco|golden-scan|spec-check|spec-coverage> [Optionen]\n\
                 Siehe xtask/src/main.rs."
            );
            Ok(false)
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn load_spec(root: &std::path::Path) -> Result<Spec> {
    Spec::load(&root.join("docs/spec"))
}

/// Commits eines Bereichs (ohne Merge-Commits), standardmäßig nur `HEAD`.
fn commits_in(range: Option<&str>) -> Result<Vec<Commit>> {
    let mut cmd = Command::new("git");
    cmd.args(["log", "--no-merges", "--format=%H%x1f%an%x1f%ae%x1f%B%x1e"]);
    match range {
        Some(r) => cmd.arg(r),
        None => cmd.args(["-1", "HEAD"]),
    };
    let out = cmd.output().context("git log fehlgeschlagen")?;
    if !out.status.success() {
        bail!("git log: {}", String::from_utf8_lossy(&out.stderr));
    }
    let text = String::from_utf8(out.stdout)?;
    Ok(text
        .split('\u{1e}')
        .filter_map(|record| {
            let mut parts = record.trim_start_matches('\n').splitn(4, '\u{1f}');
            Some(Commit {
                hash: parts.next()?.to_owned(),
                author_name: parts.next()?.to_owned(),
                author_email: parts.next()?.to_owned(),
                message: parts.next()?.trim().to_owned(),
            })
        })
        .filter(|c| !c.hash.is_empty())
        .collect())
}

fn golden_scan(root: &std::path::Path) -> Result<bool> {
    let findings = golden_scan::scan(root)?;
    for f in &findings {
        eprintln!("golden-scan: {f}");
    }
    if findings.is_empty() {
        println!("golden-scan: keine Secrets in Golden-Transcripts");
    } else {
        eprintln!("Secrets in Golden-Transcripts gefunden. Aufnahme scrubben und neu committen.");
    }
    Ok(findings.is_empty())
}

fn run_codegen(root: &std::path::Path, args: &[String]) -> Result<bool> {
    let files = codegen::generate()?;
    if args.iter().any(|a| a == "--check") {
        let problems = codegen::drift(root, &files)?;
        for p in &problems {
            eprintln!("codegen: {p}");
        }
        if problems.is_empty() {
            println!("codegen --check: {} Dateien aktuell", files.len());
        } else {
            eprintln!(
                "Generierte Dateien weichen ab. `cargo xtask codegen` ausführen und committen."
            );
        }
        return Ok(problems.is_empty());
    }
    codegen::write(root, &files)?;
    println!("codegen: {} Dateien geschrieben", files.len());
    Ok(true)
}

fn commit_lint(root: &std::path::Path, args: &[String]) -> Result<bool> {
    let spec = load_spec(root)?;
    let mut ok = true;
    if let Some(title) = flag(args, "--title") {
        for err in commits::lint_message(title, &spec) {
            eprintln!("PR-Titel: {err}");
            ok = false;
        }
    }
    let commits = commits_in(flag(args, "--range"))?;
    for c in &commits {
        for err in commits::lint_message(&c.message, &spec) {
            eprintln!("{}: {err}", &c.hash[..c.hash.len().min(10)]);
            ok = false;
        }
    }
    if ok {
        println!("commit-lint: {} Commit(s) ok", commits.len());
    }
    Ok(ok)
}

fn dco(args: &[String]) -> Result<bool> {
    let commits = commits_in(flag(args, "--range"))?;
    let errors: Vec<_> = commits.iter().filter_map(commits::check_dco).collect();
    for e in &errors {
        eprintln!("DCO: {e}");
    }
    if errors.is_empty() {
        println!("dco: {} Commit(s) ok", commits.len());
    }
    Ok(errors.is_empty())
}

fn spec_check(root: &std::path::Path) -> Result<bool> {
    let spec = load_spec(root)?;
    let problems = spec.problems();
    for p in &problems {
        eprintln!("spec: {p}");
    }
    if problems.is_empty() {
        println!("spec-check: {} Features ok", spec.features.len());
    }
    Ok(problems.is_empty())
}

fn spec_coverage(root: &std::path::Path, args: &[String]) -> Result<bool> {
    let milestone = flag(args, "--milestone").context("--milestone <M0…M5> fehlt")?;
    let report_only = args.iter().any(|a| a == "--report");
    let spec = load_spec(root)?;
    let covered = coverage::covered_acs(root)?;
    let missing = coverage::missing_for_milestone(&spec, &covered, milestone)?;
    let total: usize = spec
        .features
        .values()
        .filter(|f| f.milestone.as_deref() == Some(milestone))
        .map(|f| f.acs.len())
        .sum();
    println!(
        "spec-coverage {milestone}: {} von {total} ACs ohne Test",
        missing.len()
    );
    for m in &missing {
        println!("  {} AC{} [{}] {}", m.feature, m.ac, m.priority, m.title);
    }
    let must_missing = missing.iter().any(|m| m.priority == "Must");
    Ok(report_only || !must_missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qa_010_ac2_cargo_deny_blocks_gpl_and_advisories() {
        let text = std::fs::read_to_string(repo_root().join("deny.toml")).unwrap();
        let config: toml::Table = text.parse().unwrap();
        let allow = config["licenses"]["allow"].as_array().unwrap();
        let allowed: Vec<_> = allow.iter().filter_map(|v| v.as_str()).collect();
        assert!(allowed.contains(&"Apache-2.0") && allowed.contains(&"MIT"));
        for license in &allowed {
            assert!(
                !license.contains("GPL"),
                "{license} darf nicht erlaubt sein"
            );
        }
        // Advisories werden nicht pauschal ignoriert.
        let ignored = config["advisories"]
            .get("ignore")
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        assert_eq!(
            ignored, 0,
            "RUSTSEC-Advisories dürfen nicht ignoriert werden"
        );
        assert_eq!(config["sources"]["unknown-registry"].as_str(), Some("deny"));
    }

    #[test]
    fn qa_015_ac3_contributing_explains_dco_signoff_and_rebase() {
        let text = std::fs::read_to_string(repo_root().join("CONTRIBUTING.md")).unwrap();
        for needle in [
            "https://developercertificate.org/",
            "git commit -s",
            "git rebase --signoff",
        ] {
            assert!(
                text.contains(needle),
                "CONTRIBUTING.md erwähnt `{needle}` nicht"
            );
        }
    }

    #[test]
    fn qa_001_ac1_agents_md_documents_test_levels_commands_and_naming() {
        let text = std::fs::read_to_string(repo_root().join("AGENTS.md")).unwrap();
        for needle in [
            "cargo nextest run",
            "pnpm test",
            "pnpm e2e",
            "Unit",
            "Contract",
            "Golden",
            "Integration",
            "E2E",
            "<prefix>_<nnn>_ac<k>_",
        ] {
            assert!(text.contains(needle), "AGENTS.md erwähnt `{needle}` nicht");
        }
    }
}
