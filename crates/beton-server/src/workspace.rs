//! Zugriff auf den Workspace einer Session (SES-017): Pfadbegrenzung, Dateibaum, Lesen,
//! Schreiben mit `If-Match`, Suche.
//!
//! Sicherheitsregel (fail closed): Jeder Pfad ist relativ zum Workspace. Absolute Pfade,
//! `..`-Komponenten, `.git` und Pfade, die nach Auflösung aller Symlinks außerhalb des Workspace
//! liegen, werden mit [`PathError::Forbidden`] abgewiesen – auch wenn das Ziel nicht existiert.

use std::path::{Path, PathBuf};

/// Fehler beim Zugriff auf einen Workspace-Pfad.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// Pfad liegt außerhalb des Workspace oder ist nicht erlaubt (`..`, absolut, `.git`).
    #[error("Pfad `{0}` liegt außerhalb des Workspace oder ist nicht erlaubt")]
    Forbidden(String),
    #[error("`{0}` nicht gefunden")]
    NotFound(String),
    #[error("`{0}`: {1}")]
    Invalid(String, String),
    /// `If-Match` passt nicht zum aktuellen Inhalt.
    #[error("`{0}` wurde inzwischen geändert")]
    Stale(String),
    #[error("E/A-Fehler")]
    Io(String),
}

/// Der Workspace einer Session mit kanonischer Wurzel.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

/// Zerlegt einen relativen Workspace-Pfad in Komponenten. Abgewiesen werden absolute Pfade,
/// `..`, `.git` (auch in anderer Schreibweise), NUL und unter Windows `\\` bzw. Laufwerke.
pub fn components(raw: &str) -> Result<Vec<String>, PathError> {
    let forbidden = || PathError::Forbidden(raw.to_owned());
    if raw.starts_with('/') || raw.contains('\0') {
        return Err(forbidden());
    }
    if cfg!(windows) && (raw.contains('\\') || raw.contains(':')) {
        return Err(forbidden());
    }
    let mut out = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(forbidden()),
            p if p.eq_ignore_ascii_case(".git") => return Err(forbidden()),
            p => out.push(p.to_owned()),
        }
    }
    Ok(out)
}

/// Relativer Pfad mit `/` (für Antworten).
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

impl Workspace {
    /// Öffnet den Workspace; die Wurzel wird kanonisiert.
    pub fn open(root: &Path) -> Result<Self, PathError> {
        let root = root
            .canonicalize()
            .map_err(|_| PathError::NotFound("Workspace".into()))?;
        if !root.is_dir() {
            return Err(PathError::NotFound("Workspace".into()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Liegt ein kanonischer Pfad im Workspace und nicht in einem `.git`?
    fn contained(&self, canonical: &Path) -> bool {
        canonical.starts_with(&self.root)
            && !canonical
                .strip_prefix(&self.root)
                .unwrap_or(canonical)
                .components()
                .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git"))
    }

    /// Kanonischer, nächster existierender Vorfahr von `path` (inklusive `path` selbst) und die
    /// Zahl der Komponenten dahinter. Ein Symlink ohne Ziel zählt als existierend und wird
    /// abgewiesen, weil sein Ziel nicht prüfbar ist.
    fn existing_ancestor(
        &self,
        raw: &str,
        parts: &[String],
    ) -> Result<(PathBuf, usize), PathError> {
        for keep in (0..=parts.len()).rev() {
            let mut probe = self.root.clone();
            probe.extend(&parts[..keep]);
            if std::fs::symlink_metadata(&probe).is_err() {
                continue;
            }
            let canonical = probe
                .canonicalize()
                .map_err(|_| PathError::Forbidden(raw.to_owned()))?;
            if !self.contained(&canonical) {
                return Err(PathError::Forbidden(raw.to_owned()));
            }
            return Ok((canonical, parts.len() - keep));
        }
        Err(PathError::Forbidden(raw.to_owned()))
    }

    /// Bestehender Pfad (Datei oder Verzeichnis) innerhalb des Workspace, kanonisch.
    pub fn resolve_existing(&self, raw: &str) -> Result<PathBuf, PathError> {
        let parts = components(raw)?;
        let (canonical, missing) = self.existing_ancestor(raw, &parts)?;
        if missing > 0 {
            return Err(PathError::NotFound(raw.to_owned()));
        }
        Ok(canonical)
    }

    /// Zielpfad zum Schreiben einer Datei; das Ziel muss nicht existieren. Fehlende
    /// Zwischenverzeichnisse liegen nach der Prüfung garantiert unter einem Verzeichnis im
    /// Workspace, weil ihre Namen weder `..` noch `.git` sein können.
    pub fn resolve_for_write(&self, raw: &str) -> Result<PathBuf, PathError> {
        let parts = components(raw)?;
        if parts.is_empty() {
            return Err(PathError::Invalid(raw.to_owned(), "Dateipfad fehlt".into()));
        }
        let (canonical, missing) = self.existing_ancestor(raw, &parts)?;
        if missing == 0 {
            if canonical.is_dir() {
                return Err(PathError::Invalid(
                    raw.to_owned(),
                    "ist ein Verzeichnis".into(),
                ));
            }
            return Ok(canonical);
        }
        let mut target = canonical;
        target.extend(&parts[parts.len() - missing..]);
        Ok(target)
    }
}

/// SHA-256 als Kleinbuchstaben-Hex (ETag einer Datei).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Inhalt einer Datei mit ETag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContent {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Ergebnis eines Schreibzugriffs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub created: bool,
    pub sha256: String,
    pub size: u64,
}

/// Art eines Eintrags im Dateibaum.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Ein Eintrag im Dateibaum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
}

/// Dateinamen- oder Inhaltssuche.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Name,
    Content,
    /// Dateinamen unscharf aus dem Dateiindex, nach Treffergüte sortiert (`@` im Composer,
    /// WEB-006); ohne Cursor.
    Fuzzy,
}

/// Ein Suchtreffer; bei Inhaltssuche mit Zeile (1-basiert), Spalte (1-basiert, Zeichen) und
/// Zeilentext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub text: Option<String>,
}

/// Größte Datei, die die Inhaltssuche liest.
pub const SEARCH_MAX_FILE: u64 = 5 * 1024 * 1024;
/// Längster Zeilentext in Suchtreffern (Zeichen).
const HIT_TEXT_MAX: usize = 500;

/// Passt `If-Match` (Liste von ETags, `*`, schwache Form) zum aktuellen Inhalt?
fn if_match_ok(header: &str, current: Option<&str>) -> bool {
    header.split(',').map(str::trim).any(|tag| match current {
        None => false,
        Some(_) if tag == "*" => true,
        Some(cur) => tag
            .trim_start_matches("W/")
            .trim_matches('"')
            .eq_ignore_ascii_case(cur),
    })
}

fn io(e: &std::io::Error) -> PathError {
    PathError::Io(e.kind().to_string())
}

impl Workspace {
    /// Liest eine Datei vollständig.
    pub fn read(&self, raw: &str) -> Result<FileContent, PathError> {
        let path = self.resolve_existing(raw)?;
        if !path.is_file() {
            return Err(PathError::Invalid(raw.to_owned(), "ist keine Datei".into()));
        }
        let bytes = std::fs::read(&path).map_err(|e| io(&e))?;
        Ok(FileContent {
            sha256: sha256_hex(&bytes),
            path,
            bytes,
        })
    }

    /// Schreibt eine Datei atomar (temporäre Datei + Umbenennen). Mit `If-Match` nur, wenn der
    /// aktuelle Inhalt dazu passt; sonst [`PathError::Stale`] und keine Änderung.
    pub fn write(
        &self,
        raw: &str,
        bytes: &[u8],
        if_match: Option<&str>,
    ) -> Result<Written, PathError> {
        let target = self.resolve_for_write(raw)?;
        let current = match std::fs::read(&target) {
            Ok(b) => Some(sha256_hex(&b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&e)),
        };
        if let Some(header) = if_match
            && !if_match_ok(header, current.as_deref())
        {
            return Err(PathError::Stale(raw.to_owned()));
        }
        let parent = target
            .parent()
            .ok_or_else(|| PathError::Forbidden(raw.to_owned()))?;
        std::fs::create_dir_all(parent).map_err(|e| io(&e))?;
        // Nach dem Anlegen erneut prüfen: das Verzeichnis muss im Workspace liegen.
        let parent = parent.canonicalize().map_err(|e| io(&e))?;
        if !self.contained(&parent) {
            return Err(PathError::Forbidden(raw.to_owned()));
        }
        let name = target
            .file_name()
            .ok_or_else(|| PathError::Invalid(raw.to_owned(), "Dateiname fehlt".into()))?;
        let final_path = parent.join(name);
        let tmp = parent.join(format!(
            ".{}.beton-{}",
            name.to_string_lossy(),
            &sha256_hex(
                format!("{}{:?}", std::process::id(), std::time::SystemTime::now()).as_bytes()
            )[..8]
        ));
        std::fs::write(&tmp, bytes).map_err(|e| io(&e))?;
        if let Ok(meta) = std::fs::metadata(&final_path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        if let Err(e) = std::fs::rename(&tmp, &final_path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(&e));
        }
        Ok(Written {
            created: current.is_none(),
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
        })
    }

    /// Einträge eines Verzeichnisses (eine Ebene), nach Namen sortiert, ohne `.git`.
    pub fn list(&self, raw: &str) -> Result<Vec<Entry>, PathError> {
        let dir = self.resolve_existing(raw)?;
        if !dir.is_dir() {
            return Err(PathError::Invalid(
                raw.to_owned(),
                "ist kein Verzeichnis".into(),
            ));
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| io(&e))? {
            let entry = entry.map_err(|e| io(&e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| io(&e))?;
            let ft = meta.file_type();
            let kind = if ft.is_symlink() {
                EntryKind::Symlink
            } else if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            out.push(Entry {
                path: relative(&self.root, &dir.join(&name)),
                name,
                kind,
                size: (kind == EntryKind::File).then_some(meta.len()),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Suche mit ripgrep-Semantik: `.gitignore` (in Git-Repositories), versteckte Dateien und
    /// Binärdateien werden übersprungen, Symlinks nicht verfolgt. Smart-Case: case-insensitiv,
    /// solange die Anfrage keine Großbuchstaben enthält. Liefert höchstens `limit` Treffer
    /// nach `after` (Pfad, Zeile) und ob weitere folgen.
    pub fn search(
        &self,
        query: &str,
        mode: SearchMode,
        after: Option<(String, u32)>,
        limit: usize,
    ) -> Result<(Vec<Hit>, bool), PathError> {
        if query.is_empty() {
            return Err(PathError::Invalid("q".into(), "Suchbegriff fehlt".into()));
        }
        let insensitive = !query.chars().any(char::is_uppercase);
        let pattern = regex::RegexBuilder::new(&regex::escape(query))
            .case_insensitive(insensitive)
            .build()
            .map_err(|e| PathError::Invalid("q".into(), e.to_string()))?;
        let after_key = |path: &str, line: u32| match &after {
            None => true,
            Some((p, l)) => (path, line) > (p.as_str(), *l),
        };
        let mut hits = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .follow_links(false)
            .sort_by_file_name(std::cmp::Ord::cmp)
            .build();
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let rel = relative(&self.root, entry.path());
            match mode {
                SearchMode::Name | SearchMode::Fuzzy => {
                    if pattern.is_match(&rel) && after_key(&rel, 0) {
                        hits.push(Hit {
                            path: rel,
                            line: None,
                            column: None,
                            text: None,
                        });
                    }
                }
                SearchMode::Content => {
                    if entry.metadata().is_ok_and(|m| m.len() > SEARCH_MAX_FILE) {
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(entry.path()) else {
                        continue;
                    };
                    if bytes.iter().take(8000).any(|b| *b == 0) {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    for (i, line) in text.lines().enumerate() {
                        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
                        if !after_key(&rel, n) {
                            continue;
                        }
                        if let Some(m) = pattern.find(line) {
                            hits.push(Hit {
                                path: rel.clone(),
                                line: Some(n),
                                column: u32::try_from(line[..m.start()].chars().count() + 1).ok(),
                                text: Some(line.chars().take(HIT_TEXT_MAX).collect()),
                            });
                            if hits.len() > limit {
                                break;
                            }
                        }
                    }
                }
            }
            if hits.len() > limit {
                break;
            }
        }
        let more = hits.len() > limit;
        hits.truncate(limit);
        Ok((hits, more))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::create_dir_all(tmp.path().join("draussen")).unwrap();
        std::fs::write(tmp.path().join("draussen/geheim.txt"), "geheim").unwrap();
        let ws = Workspace::open(&root).unwrap();
        let outside = tmp.path().join("draussen");
        (tmp, ws, outside)
    }

    fn forbidden(r: Result<PathBuf, PathError>) -> bool {
        matches!(r, Err(PathError::Forbidden(_)))
    }

    #[test]
    fn ses_017_ac1_dotdot_and_absolute_paths_are_forbidden() {
        let (_tmp, ws, outside) = ws();
        for raw in [
            "..",
            "../draussen/geheim.txt",
            "src/../../draussen/geheim.txt",
            "src/../lib.rs",
            "./..",
            "/etc/passwd",
            &outside.join("geheim.txt").display().to_string(),
            "src/\0x",
        ] {
            assert!(forbidden(ws.resolve_existing(raw)), "lesen: {raw:?}");
            assert!(forbidden(ws.resolve_for_write(raw)), "schreiben: {raw:?}");
        }
    }

    #[test]
    fn ses_017_ac1_git_directory_is_forbidden() {
        let (_tmp, ws, _) = ws();
        std::fs::create_dir_all(ws.root().join(".git/hooks")).unwrap();
        for raw in [".git", ".git/config", "src/.git/x", ".GIT/hooks/pre-commit"] {
            assert!(forbidden(ws.resolve_existing(raw)), "{raw}");
            assert!(forbidden(ws.resolve_for_write(raw)), "{raw}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn ses_017_ac1_symlinks_leaving_the_workspace_are_forbidden() {
        let (_tmp, ws, outside) = ws();
        let root = ws.root().to_path_buf();
        std::os::unix::fs::symlink(&outside, root.join("raus")).unwrap();
        std::os::unix::fs::symlink(outside.join("geheim.txt"), root.join("geheim")).unwrap();
        std::os::unix::fs::symlink(outside.join("fehlt.txt"), root.join("dangling")).unwrap();
        std::os::unix::fs::symlink(root.join(".git"), root.join("gitlink")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for raw in [
            "raus",
            "raus/geheim.txt",
            "raus/neu.txt",
            "raus/neu/tief.txt",
            "geheim",
            "dangling",
            "gitlink/config",
        ] {
            assert!(forbidden(ws.resolve_existing(raw)), "lesen: {raw}");
            assert!(forbidden(ws.resolve_for_write(raw)), "schreiben: {raw}");
        }
        // Symlinks innerhalb des Workspace sind erlaubt.
        std::os::unix::fs::symlink(root.join("src"), root.join("quellen")).unwrap();
        assert_eq!(
            ws.resolve_existing("quellen/lib.rs").unwrap(),
            root.canonicalize().unwrap().join("src/lib.rs")
        );
        assert!(ws.resolve_for_write("quellen/neu.rs").is_ok());
    }

    #[test]
    fn paths_inside_the_workspace_resolve() {
        let (_tmp, ws, _) = ws();
        let root = ws.root().to_path_buf();
        assert_eq!(ws.resolve_existing("").unwrap(), root);
        assert_eq!(
            ws.resolve_existing("src/lib.rs").unwrap(),
            root.join("src/lib.rs")
        );
        assert_eq!(
            ws.resolve_existing("./src//lib.rs").unwrap(),
            root.join("src/lib.rs")
        );
        assert!(matches!(
            ws.resolve_existing("src/fehlt.rs"),
            Err(PathError::NotFound(_))
        ));
        assert_eq!(
            ws.resolve_for_write("neu/tief/datei.txt").unwrap(),
            root.join("neu/tief/datei.txt")
        );
        assert!(matches!(
            ws.resolve_for_write(""),
            Err(PathError::Invalid(..))
        ));
        assert!(root.starts_with(std::fs::canonicalize(ws.root()).unwrap()));
    }

    #[test]
    fn ses_017_ac2_stale_if_match_is_rejected_and_file_stays_unchanged() {
        let (_tmp, ws, _) = ws();
        let current = ws.read("src/lib.rs").unwrap();
        assert_eq!(current.bytes, b"fn a() {}\n");
        assert_eq!(current.sha256, sha256_hex(b"fn a() {}\n"));
        let written = ws
            .write(
                "src/lib.rs",
                b"fn b() {}\n",
                Some(&format!("\"{}\"", current.sha256)),
            )
            .unwrap();
        assert!(!written.created);
        // Veraltetes ETag: 412-Fall, Inhalt bleibt.
        let err = ws
            .write("src/lib.rs", b"fn c() {}\n", Some(&current.sha256))
            .unwrap_err();
        assert_eq!(err, PathError::Stale("src/lib.rs".into()));
        assert_eq!(ws.read("src/lib.rs").unwrap().bytes, b"fn b() {}\n");
        // `*` verlangt eine vorhandene Datei; ohne If-Match wird angelegt.
        assert!(matches!(
            ws.write("neu.txt", b"x", Some("*")),
            Err(PathError::Stale(_))
        ));
        let created = ws.write("neu/tief.txt", b"x", None).unwrap();
        assert!(created.created);
        assert_eq!(created.sha256, sha256_hex(b"x"));
        assert_eq!(std::fs::read(ws.root().join("neu/tief.txt")).unwrap(), b"x");
        // Mehrere ETags und schwache Form.
        assert!(
            ws.write(
                "neu/tief.txt",
                b"y",
                Some(&format!("\"alt\", W/\"{}\"", sha256_hex(b"x")))
            )
            .is_ok()
        );
    }

    #[test]
    fn ses_017_tree_lists_one_level_without_git() {
        let (_tmp, ws, _) = ws();
        std::fs::create_dir_all(ws.root().join(".git")).unwrap();
        std::fs::write(ws.root().join("b.txt"), "bb").unwrap();
        let entries = ws.list("").unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.path.as_str(), e.kind, e.size))
            .collect();
        assert_eq!(
            names,
            vec![
                ("b.txt", "b.txt", EntryKind::File, Some(2)),
                ("src", "src", EntryKind::Dir, None),
            ]
        );
        assert_eq!(ws.list("src").unwrap()[0].path, "src/lib.rs");
        assert!(matches!(ws.list("src/lib.rs"), Err(PathError::Invalid(..))));
        assert!(matches!(ws.list(".."), Err(PathError::Forbidden(_))));
    }

    #[test]
    fn ses_017_search_by_name_and_content_respects_gitignore() {
        let (_tmp, ws, _) = ws();
        let root = ws.root().to_path_buf();
        // `.gitignore` wirkt (wie bei ripgrep) in Git-Repositories.
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {\n    a();\n}\n").unwrap();
        std::fs::write(root.join("src/bild.bin"), b"fn a\0\0").unwrap();

        let (hits, more) = ws.search("lib", SearchMode::Name, None, 50).unwrap();
        assert!(!more);
        let paths: Vec<_> = hits.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(paths, vec!["src/lib.rs"]);

        let (hits, _) = ws.search("a()", SearchMode::Content, None, 50).unwrap();
        let found: Vec<_> = hits
            .iter()
            .map(|h| (h.path.as_str(), h.line, h.column))
            .collect();
        assert_eq!(
            found,
            vec![
                ("src/lib.rs", Some(1), Some(4)),
                ("src/main.rs", Some(2), Some(5))
            ]
        );
        // Smart-Case: Großbuchstaben machen die Suche case-sensitiv.
        assert!(
            ws.search("FN", SearchMode::Content, None, 50)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            ws.search("fn", SearchMode::Content, None, 50)
                .unwrap()
                .0
                .len(),
            2
        );
        // Pagination über (Pfad, Zeile).
        let (first, more) = ws.search("a()", SearchMode::Content, None, 1).unwrap();
        assert!(more);
        let after = (first[0].path.clone(), first[0].line.unwrap_or(0));
        let (second, more) = ws
            .search("a()", SearchMode::Content, Some(after), 1)
            .unwrap();
        assert!(!more);
        assert_eq!(second[0].path, "src/main.rs");
    }

    mod prop {
        use proptest::prelude::*;

        use super::*;

        fn component() -> impl Strategy<Value = String> {
            prop_oneof![
                Just("..".to_owned()),
                Just(".".to_owned()),
                Just("".to_owned()),
                Just(".git".to_owned()),
                Just("src".to_owned()),
                Just("raus".to_owned()),
                Just("lib.rs".to_owned()),
                "[a-z]{1,4}",
            ]
        }

        proptest! {
            /// Was auch immer aufgelöst wird, liegt nach Auflösung aller Symlinks im Workspace.
            #[test]
            fn ses_017_ac1_resolved_paths_never_leave_the_workspace(
                parts in proptest::collection::vec(component(), 0..6),
                leading_slash in any::<bool>(),
            ) {
                let (_tmp, ws, outside) = ws();
                #[cfg(unix)]
                std::os::unix::fs::symlink(&outside, ws.root().join("raus")).unwrap();
                let _ = &outside;
                let raw = format!("{}{}", if leading_slash { "/" } else { "" }, parts.join("/"));
                let root = ws.root().to_path_buf();
                let inside = |p: &Path| {
                    let mut probe = p.to_path_buf();
                    // Nicht existierende Ziele: nächster existierender Vorfahr.
                    while !probe.exists() {
                        if !probe.pop() { return false; }
                    }
                    probe.canonicalize().is_ok_and(|c| c.starts_with(&root))
                };
                if let Ok(p) = ws.resolve_existing(&raw) {
                    prop_assert!(inside(&p), "{raw} → {}", p.display());
                    prop_assert!(!parts.iter().any(|c| c == ".."), "{raw}");
                }
                if let Ok(p) = ws.resolve_for_write(&raw) {
                    prop_assert!(inside(&p), "{raw} → {}", p.display());
                    prop_assert!(p.starts_with(&root));
                    prop_assert!(!p.components().any(|c| c.as_os_str() == ".git"));
                }
            }
        }
    }
}
