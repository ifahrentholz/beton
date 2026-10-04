//! Dateiindex für die `@`-Suche im Composer (WEB-006 AC2): Pfade eines Workspace im Speicher,
//! unscharfe Suche in wenigen Millisekunden auch bei 20 000 Dateien.
//!
//! Der Index entsteht beim ersten Zugriff (gleiche Regeln wie die Dateisuche: `.gitignore`,
//! keine versteckten Dateien, keine Symlinks) und wird danach im Hintergrund erneuert, sobald er
//! älter als [`FileIndexCache::ttl`] ist; bis dahin antwortet die Suche aus dem alten Stand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Höchstzahl indexierter Dateien je Workspace.
pub const MAX_FILES: usize = 200_000;
/// So viele Workspaces hält der Cache.
const MAX_ROOTS: usize = 16;

/// Ein indexierter Pfad (relativ, `/`-getrennt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPath {
    pub path: String,
    lower: String,
    /// Beginn des Dateinamens in `path` bzw. `lower` (Byte-Index).
    base: usize,
    base_lower: usize,
}

impl IndexedPath {
    pub fn new(path: String) -> Self {
        let lower = path.to_lowercase();
        let base = path.rfind('/').map_or(0, |i| i + 1);
        let base_lower = lower.rfind('/').map_or(0, |i| i + 1);
        Self {
            path,
            lower,
            base,
            base_lower,
        }
    }
}

struct Entry {
    built: Instant,
    files: Arc<Vec<IndexedPath>>,
    refreshing: Arc<AtomicBool>,
}

/// Cache der Dateiindizes je Workspace-Wurzel.
pub struct FileIndexCache {
    entries: Mutex<HashMap<PathBuf, Entry>>,
    /// Ab diesem Alter wird im Hintergrund neu indexiert.
    pub ttl: Duration,
}

impl Default for FileIndexCache {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            ttl: Duration::from_secs(3),
        }
    }
}

impl std::fmt::Debug for FileIndexCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileIndexCache")
    }
}

/// Alle Dateien unter `root` mit den Regeln der Dateisuche (SES-017).
pub fn scan(root: &Path) -> Vec<IndexedPath> {
    let walker = ignore::WalkBuilder::new(root)
        .follow_links(false)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .build();
    walker
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .take(MAX_FILES)
        .map(|e| IndexedPath::new(crate::workspace::relative(root, e.path())))
        .collect()
}

impl FileIndexCache {
    /// Index für `root`; blockiert nur beim ersten Zugriff (aus `spawn_blocking` aufrufen).
    pub fn get(self: &Arc<Self>, root: &Path) -> Arc<Vec<IndexedPath>> {
        let stale = {
            let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries.get(root).map(|e| {
                (
                    e.files.clone(),
                    e.built.elapsed() > self.ttl,
                    e.refreshing.clone(),
                )
            })
        };
        match stale {
            Some((files, false, _)) => files,
            Some((files, true, refreshing)) => {
                if !refreshing.swap(true, Ordering::AcqRel) {
                    let me = self.clone();
                    let root = root.to_path_buf();
                    std::thread::spawn(move || {
                        let fresh = Arc::new(scan(&root));
                        me.put(root, fresh);
                    });
                }
                files
            }
            None => {
                let fresh = Arc::new(scan(root));
                self.put(root.to_path_buf(), fresh.clone());
                fresh
            }
        }
    }

    fn put(&self, root: PathBuf, files: Arc<Vec<IndexedPath>>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= MAX_ROOTS && !entries.contains_key(&root) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, e)| e.built)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                entries.remove(&k);
            }
        }
        entries.insert(
            root,
            Entry {
                built: Instant::now(),
                files,
                refreshing: Arc::default(),
            },
        );
    }
}

/// Bewertet einen Pfad; `None` = kein Treffer. Höher ist besser: Treffer im Dateinamen vor
/// Treffern im Verzeichnis, zusammenhängende vor verstreuten, kurze Pfade vor langen.
fn score(p: &IndexedPath, query: &str, case_sensitive: bool) -> Option<i64> {
    let (hay, base_at) = if case_sensitive {
        (&p.path, p.base)
    } else {
        (&p.lower, p.base_lower)
    };
    let len = i64::try_from(p.path.len()).unwrap_or(i64::MAX);
    let base = &hay[base_at..];
    if let Some(pos) = base.find(query) {
        let start_bonus = if pos == 0 { 400 } else { 0 };
        return Some(3_000 + start_bonus - len);
    }
    if hay.contains(query) {
        return Some(2_000 - len);
    }
    // Unscharf: Zeichen der Anfrage in Reihenfolge; Lücken kosten.
    let mut gaps = 0i64;
    let mut last: Option<usize> = None;
    let mut chars = hay.char_indices();
    for q in query.chars() {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        if let Some(l) = last
            && i > l + 1
        {
            gaps += 1;
        }
        last = Some(i);
    }
    let in_base = last.is_some_and(|l| l >= base_at);
    Some(1_000 - gaps * 40 + if in_base { 100 } else { 0 } - len)
}

/// Unscharfe Suche über die Pfade (Smart-Case wie die Dateisuche).
pub fn fuzzy<'a>(files: &'a [IndexedPath], query: &str, limit: usize) -> Vec<&'a str> {
    let query = query.trim();
    if query.is_empty() {
        return files.iter().take(limit).map(|p| p.path.as_str()).collect();
    }
    let case_sensitive = query.chars().any(char::is_uppercase);
    let q = if case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut hits: Vec<(i64, &str)> = files
        .iter()
        .filter_map(|p| score(p, &q, case_sensitive).map(|s| (s, p.path.as_str())))
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    hits.into_iter().take(limit).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(paths: &[&str]) -> Vec<IndexedPath> {
        paths
            .iter()
            .map(|p| IndexedPath::new((*p).into()))
            .collect()
    }

    #[test]
    fn web_006_ac2_fuzzy_prefers_file_names_and_contiguous_matches() {
        let files = idx(&[
            "docs/adr/0007-middleware-order.md",
            "src/middleware.rs",
            "src/mid/other.rs",
            "src/main.rs",
            "tests/m_i_d.rs",
            "README.md",
        ]);
        let hits = fuzzy(&files, "mid", 10);
        assert_eq!(hits[0], "src/middleware.rs");
        assert!(hits.contains(&"docs/adr/0007-middleware-order.md"));
        assert!(!hits.contains(&"README.md"));
        // Unscharf: Buchstaben in Reihenfolge.
        assert_eq!(fuzzy(&files, "mdlw", 1), vec!["src/middleware.rs"]);
        // Smart-Case.
        assert!(fuzzy(&files, "MID", 10).is_empty());
        assert_eq!(fuzzy(&files, "", 2).len(), 2);
    }

    #[test]
    fn web_006_ac2_index_respects_gitignore_and_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/x.rs"), "").unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        let cache = Arc::new(FileIndexCache {
            ttl: Duration::from_millis(0),
            ..FileIndexCache::default()
        });
        let first = cache.get(root);
        let paths: Vec<&str> = first.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, vec!["src/lib.rs"]);
        std::fs::write(root.join("src/neu.rs"), "").unwrap();
        // Veraltet: sofort der alte Stand, im Hintergrund neu.
        let _ = cache.get(root);
        let start = Instant::now();
        loop {
            if cache.get(root).iter().any(|p| p.path == "src/neu.rs") {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
