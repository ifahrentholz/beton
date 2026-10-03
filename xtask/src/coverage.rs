//! Zuordnung von Akzeptanzkriterien zu Tests (QA-014 AC3).
//!
//! Ein AC gilt als abgedeckt, wenn es irgendwo in Code, Tests oder CI-Workflows
//! referenziert wird – entweder als Testname `<prefix>_<nnn>_ac<k>_…`
//! (z. B. `pol_003_ac2_stricter_rule_wins`) oder als `<PREFIX>-<NNN> AC<k>`
//! (z. B. in TypeScript-Testtiteln oder `# covers: QA-010 AC1` in Workflows).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Result;
use regex::Regex;

use crate::spec::{PREFIXES, Spec, milestone_rank};

/// Verzeichnisse, die nach AC-Referenzen durchsucht werden.
const SCAN_DIRS: [&str; 5] = ["crates", "xtask", "apps", "packages", ".github/workflows"];
const SCAN_EXT: [&str; 5] = ["rs", "ts", "tsx", "yml", "yaml"];

/// Alle referenzierten ACs als `(Feature-ID, AC-Nummer)`.
pub fn covered_acs(root: &Path) -> Result<BTreeSet<(String, u32)>> {
    let lower = PREFIXES.map(str::to_lowercase).join("|");
    let snake = Regex::new(&format!(r"\b({lower})_(\d{{3}})_ac(\d+)"))?;
    let upper = Regex::new(&format!(r"\b({})-(\d{{3}}) AC(\d+)\b", PREFIXES.join("|")))?;
    let mut covered = BTreeSet::new();
    for dir in SCAN_DIRS {
        scan(&root.join(dir), &mut |text| {
            for c in snake.captures_iter(text) {
                covered.insert((format!("{}-{}", c[1].to_uppercase(), &c[2]), parse(&c[3])));
            }
            for c in upper.captures_iter(text) {
                covered.insert((format!("{}-{}", &c[1], &c[2]), parse(&c[3])));
            }
        })?;
    }
    Ok(covered)
}

fn parse(n: &str) -> u32 {
    n.parse().unwrap_or(0)
}

fn scan(dir: &Path, visit: &mut dyn FnMut(&str)) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(()); // Verzeichnis existiert (noch) nicht
    };
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if matches!(name, "target" | "node_modules" | "dist" | ".git") {
                continue;
            }
            scan(&path, visit)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCAN_EXT.contains(&e))
            && let Ok(text) = fs::read_to_string(&path)
        {
            visit(&text);
        }
    }
    Ok(())
}

/// Ein nicht abgedecktes AC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub feature: String,
    pub ac: u32,
    pub priority: String,
    pub title: String,
}

/// ACs der Features eines Meilensteins, die (noch) keinen Test haben. ACs, die per
/// „(ab Mx)“ auf einen späteren Meilenstein verschoben sind, zählen nicht.
pub fn missing_for_milestone(
    spec: &Spec,
    covered: &BTreeSet<(String, u32)>,
    milestone: &str,
) -> Result<Vec<Missing>> {
    let rank = milestone_rank(milestone)?;
    let mut missing = Vec::new();
    for f in spec.features.values() {
        if f.milestone.as_deref() != Some(milestone) {
            continue;
        }
        for ac in &f.acs {
            if let Some(later) = &ac.deferred_to
                && milestone_rank(later)? > rank
            {
                continue;
            }
            if !covered.contains(&(f.id.clone(), ac.number)) {
                missing.push(Missing {
                    feature: f.id.clone(),
                    ac: ac.number,
                    priority: f.priority.clone().unwrap_or_default(),
                    title: f.title.clone(),
                });
            }
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::tests::sample_spec;

    #[test]
    fn qa_014_ac3_lists_acs_without_tests_and_skips_deferred_ones() {
        let (_d, spec) = sample_spec();
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("crates/x/src")).unwrap();
        fs::write(
            root.path().join("crates/x/src/lib.rs"),
            "#[test] fn pol_002_ac1_eins() {}",
        )
        .unwrap();
        let covered = covered_acs(root.path()).unwrap();
        assert!(covered.contains(&("POL-002".into(), 1)));

        // POL-001 AC1 fehlt; AC2 ist auf M2 verschoben und zählt in M0 nicht.
        let m0 = missing_for_milestone(&spec, &covered, "M0").unwrap();
        assert_eq!(m0.len(), 1);
        assert_eq!((m0[0].feature.as_str(), m0[0].ac), ("POL-001", 1));
        assert_eq!(m0[0].priority, "Must");

        let m1 = missing_for_milestone(&spec, &covered, "M1").unwrap();
        assert_eq!(m1.len(), 1);
        assert_eq!((m1[0].feature.as_str(), m1[0].ac), ("POL-002", 2));
    }

    #[test]
    fn qa_014_ac3_recognizes_workflow_annotations() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".github/workflows")).unwrap();
        fs::write(
            root.path().join(".github/workflows/ci.yml"),
            "      # covers: QA-010 AC1\n",
        )
        .unwrap();
        let covered = covered_acs(root.path()).unwrap();
        assert!(covered.contains(&("QA-010".into(), 1)));
    }
}
