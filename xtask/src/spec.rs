//! Liest die Feature-Einträge aus `docs/spec/*.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

/// Alle Feature-Prefixe der Spec (00-overview.md §6).
pub const PREFIXES: [&str; 28] = [
    "HAR", "AGT", "ASY", "POL", "SBX", "PRX", "AUTH", "SEC", "PROTO", "DATA", "SYNC", "SES", "COL",
    "GIT", "DESK", "WEB", "CLI", "TUI", "API", "BRW", "RUN", "PLG", "USE", "VOI", "UX", "OBS",
    "DIST", "QA",
];

/// IDs, die in der Spec absichtlich als Negativbeispiel vorkommen.
const EXAMPLE_IDS: [&str; 1] = ["POL-999"];

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^### ([A-Z]+)-(\d{3}) — (.+)$").expect("gültige Regex"));
static META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*Meilenstein:\*\*\s*(M\d|v2)\s*·\s*\*\*Priorität:\*\*\s*(Must|Should|Could)")
        .expect("gültige Regex")
});
static AC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*- \[[ xX]\] AC(\d+) — (.*)$").expect("gültige Regex"));
static DEFERRED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\(ab (M\d)\)").expect("gültige Regex"));

/// Regex für eine Feature-ID wie `POL-003`.
pub fn id_regex() -> Regex {
    Regex::new(&format!(r"\b(?:{})-\d{{3}}\b", PREFIXES.join("|"))).expect("gültige Regex")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ac {
    pub number: u32,
    /// Meilenstein, ab dem das AC gilt, falls es mit „(ab Mx)“ markiert ist.
    pub deferred_to: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub id: String,
    pub prefix: String,
    pub number: u32,
    pub title: String,
    pub file: String,
    pub milestone: Option<String>,
    pub priority: Option<String>,
    pub acs: Vec<Ac>,
}

#[derive(Debug, Default)]
pub struct Spec {
    pub features: BTreeMap<String, Feature>,
    /// Alle referenzierten IDs mit Datei.
    pub references: Vec<(String, String)>,
    pub duplicates: Vec<String>,
}

impl Spec {
    pub fn load(spec_dir: &Path) -> Result<Self> {
        let mut spec = Spec::default();
        let id_re = id_regex();
        let mut files: Vec<_> = fs::read_dir(spec_dir)
            .with_context(|| format!("{} nicht lesbar", spec_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            if name == "roadmap.md" {
                continue; // generiert, enthält nur Sichten
            }
            let text = fs::read_to_string(&path)?;
            let mut current: Option<Feature> = None;
            for line in text.lines() {
                for m in id_re.find_iter(line) {
                    spec.references.push((m.as_str().to_owned(), name.clone()));
                }
                if let Some(c) = HEADING.captures(line) {
                    if let Some(f) = current.take() {
                        spec.insert(f);
                    }
                    current = Some(Feature {
                        id: format!("{}-{}", &c[1], &c[2]),
                        prefix: c[1].to_owned(),
                        number: c[2].parse()?,
                        title: c[3].trim().to_owned(),
                        file: name.clone(),
                        milestone: None,
                        priority: None,
                        acs: Vec::new(),
                    });
                    continue;
                }
                if line.starts_with("### ") || line.starts_with("## ") {
                    if let Some(f) = current.take() {
                        spec.insert(f);
                    }
                    continue;
                }
                let Some(f) = current.as_mut() else { continue };
                if f.milestone.is_none()
                    && let Some(c) = META.captures(line)
                {
                    f.milestone = Some(c[1].to_owned());
                    f.priority = Some(c[2].to_owned());
                    continue;
                }
                if let Some(c) = AC.captures(line) {
                    let deferred_to = DEFERRED.captures(c[2].trim()).map(|d| d[1].to_owned());
                    f.acs.push(Ac {
                        number: c[1].parse()?,
                        deferred_to,
                    });
                }
            }
            if let Some(f) = current.take() {
                spec.insert(f);
            }
        }
        Ok(spec)
    }

    fn insert(&mut self, feature: Feature) {
        if self.features.contains_key(&feature.id) {
            self.duplicates.push(feature.id.clone());
        }
        self.features.insert(feature.id.clone(), feature);
    }

    pub fn contains(&self, id: &str) -> bool {
        self.features.contains_key(id)
    }

    /// Strukturprüfung der Spec; liefert alle gefundenen Probleme.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for id in &self.duplicates {
            problems.push(format!("{id}: mehrfach definiert"));
        }
        let mut by_prefix: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
        for f in self.features.values() {
            by_prefix.entry(&f.prefix).or_default().insert(f.number);
            if f.milestone.is_none() || f.priority.is_none() {
                problems.push(format!(
                    "{}: Meilenstein/Priorität fehlt ({})",
                    f.id, f.file
                ));
            }
            if f.milestone.as_deref() != Some("v2") && !(2..=6).contains(&f.acs.len()) {
                problems.push(format!(
                    "{}: {} Akzeptanzkriterien, erwartet 2–6 ({})",
                    f.id,
                    f.acs.len(),
                    f.file
                ));
            }
            for (i, ac) in f.acs.iter().enumerate() {
                if ac.number != i as u32 + 1 {
                    problems.push(format!(
                        "{}: AC-Nummerierung lückenhaft bei AC{}",
                        f.id, ac.number
                    ));
                    break;
                }
            }
        }
        for (prefix, numbers) in by_prefix {
            for (i, n) in numbers.iter().enumerate() {
                if *n != i as u32 + 1 {
                    problems.push(format!(
                        "{prefix}: Nummerierung lückenhaft bei {prefix}-{n:03}"
                    ));
                    break;
                }
            }
        }
        let mut dangling = BTreeSet::new();
        for (id, file) in &self.references {
            if !self.contains(id) && !EXAMPLE_IDS.contains(&id.as_str()) {
                dangling.insert(format!(
                    "{id}: referenziert in {file}, aber nicht definiert"
                ));
            }
        }
        problems.extend(dangling);
        problems
    }
}

/// Ordnungszahl eines Meilensteins (`M0` = 0 … `v2` = 99).
pub fn milestone_rank(ms: &str) -> Result<u32> {
    if ms == "v2" {
        return Ok(99);
    }
    match ms.strip_prefix('M').and_then(|n| n.parse().ok()) {
        Some(n) => Ok(n),
        None => bail!("unbekannter Meilenstein `{ms}`"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = "\
# 99 — Beispiel

### POL-001 — Erste Regel
- **Meilenstein:** M0 · **Priorität:** Must
- **Beschreibung:** siehe POL-002.
- **Akzeptanzkriterien:**
  - [ ] AC1 — erstes
  - [ ] AC2 — (ab M2) später
- **Abhängigkeiten:** —

### POL-002 — Zweite Regel
- **Meilenstein:** M1 · **Priorität:** Should
- **Akzeptanzkriterien:**
  - [ ] AC1 — eins
  - [ ] AC2 — zwei
";

    pub(crate) fn sample_spec() -> (tempfile::TempDir, Spec) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("99-beispiel.md"), SAMPLE).unwrap();
        let spec = Spec::load(dir.path()).unwrap();
        (dir, spec)
    }

    #[test]
    fn parses_features_milestones_and_deferred_acs() {
        let (_dir, spec) = sample_spec();
        let f = &spec.features["POL-001"];
        assert_eq!(f.milestone.as_deref(), Some("M0"));
        assert_eq!(f.priority.as_deref(), Some("Must"));
        assert_eq!(f.acs.len(), 2);
        assert_eq!(f.acs[1].deferred_to.as_deref(), Some("M2"));
        assert!(spec.problems().is_empty(), "{:?}", spec.problems());
    }

    #[test]
    fn reports_dangling_reference() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("99.md"),
            SAMPLE.replace("siehe POL-002", "siehe POL-042"),
        )
        .unwrap();
        let spec = Spec::load(dir.path()).unwrap();
        assert!(spec.problems().iter().any(|p| p.starts_with("POL-042")));
    }

    #[test]
    fn real_spec_is_consistent() {
        let root = crate::repo_root();
        let spec = Spec::load(&root.join("docs/spec")).unwrap();
        assert!(spec.features.len() > 300);
        let problems = spec.problems();
        assert!(
            problems.is_empty(),
            "Spec-Probleme:\n{}",
            problems.join("\n")
        );
    }
}
