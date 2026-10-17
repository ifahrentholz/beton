//! Zugriff auf den Workspace einer Session (SES-017): Pfadbegrenzung, Dateibaum, Lesen,
//! Schreiben mit `If-Match`, Suche.
//!
//! Sicherheitsregel (fail closed): Jeder Pfad ist relativ zum Workspace. Absolute Pfade,
//! `..`-Komponenten, `.git` und Pfade, die nach Auflösung aller Symlinks außerhalb des Workspace
//! liegen, werden mit [`PathError::Forbidden`] abgewiesen – auch wenn das Ziel nicht existiert.
//!
//! Race-frei (#92, Unix): Lesen, Schreiben und Auflisten lösen den Pfad Komponente für
//! Komponente ab einem Handle auf die Wurzel auf (`openat` mit `O_NOFOLLOW`/`O_DIRECTORY`),
//! Symlinks werden selbst aufgelöst und nur innerhalb des Workspace verfolgt. Ein gleichzeitig
//! laufender Prozess (der Agent) kann ein schon geprüftes Verzeichnis dann nicht mehr gegen
//! einen Symlink nach draußen tauschen: Der Zugriff hält das Verzeichnis selbst, nicht seinen
//! Pfad. Geschrieben wird in eine temporäre Datei im gehaltenen Verzeichnis (`O_EXCL`), dann
//! `renameat` darin. Unter Linux wäre `openat2` mit `RESOLVE_BENEATH` eine Alternative; die
//! schrittweise Auflösung gilt auf beiden Plattformen gleich und verfolgt auch absolute Symlinks
//! in den Workspace. Windows nutzt bis dahin die pfadbasierte Prüfung.

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

    /// Löst `raw` race-frei auf (#92); mit `create` legt es fehlende Verzeichnisse an.
    #[cfg(unix)]
    fn found(&self, raw: &str, create: bool) -> Result<beneath::Found, PathError> {
        let parts = components(raw)?;
        beneath::walk(&self.root, raw, &parts, create)
    }

    /// Bestehender Pfad (Datei oder Verzeichnis) innerhalb des Workspace, kanonisch. Nur zur
    /// Anzeige bzw. Vorprüfung; Zugriffe laufen über [`Workspace::read`],
    /// [`Workspace::open_file`], [`Workspace::write`] und [`Workspace::list`].
    #[cfg(unix)]
    pub fn resolve_existing(&self, raw: &str) -> Result<PathBuf, PathError> {
        let f = self.found(raw, false)?;
        if !f.missing.is_empty() {
            return Err(PathError::NotFound(raw.to_owned()));
        }
        if let Some(name) = &f.name {
            match rustix::fs::statat(
                &f.dir,
                name.as_os_str(),
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(_) => {}
                Err(rustix::io::Errno::NOENT) => return Err(PathError::NotFound(raw.to_owned())),
                Err(e) => return Err(beneath::io(e)),
            }
        }
        Ok(f.path())
    }

    /// Zielpfad zum Schreiben einer Datei; das Ziel muss nicht existieren (nur zur Anzeige bzw.
    /// Vorprüfung, siehe [`Workspace::resolve_existing`]).
    #[cfg(unix)]
    pub fn resolve_for_write(&self, raw: &str) -> Result<PathBuf, PathError> {
        if components(raw)?.is_empty() {
            return Err(PathError::Invalid(raw.to_owned(), "Dateipfad fehlt".into()));
        }
        let f = self.found(raw, false)?;
        let is_dir = f.missing.is_empty()
            && f.name.as_ref().is_some_and(|n| {
                rustix::fs::statat(&f.dir, n.as_os_str(), rustix::fs::AtFlags::empty()).is_ok_and(
                    |st| {
                        rustix::fs::FileType::from_raw_mode(st.st_mode as _)
                            == rustix::fs::FileType::Directory
                    },
                )
            });
        if f.name.is_none() || is_dir {
            return Err(PathError::Invalid(
                raw.to_owned(),
                "ist ein Verzeichnis".into(),
            ));
        }
        Ok(f.path())
    }

    /// Liegt ein kanonischer Pfad im Workspace und nicht in einem `.git`?
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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

/// Race-freie Auflösung unter der Wurzel (#92).
#[cfg(unix)]
mod beneath {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
    use std::path::{Component, Path, PathBuf};

    use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags};
    use rustix::io::Errno;

    use super::PathError;

    /// Höchstzahl verfolgter Symlinks (wie `MAXSYMLINKS`) und Schritte insgesamt (gegen einen
    /// Prozess, der Verzeichnisse laufend anlegt und entfernt).
    const MAX_LINKS: usize = 40;
    const MAX_STEPS: usize = 4096;

    /// Verzeichnis öffnen, ohne einem Symlink zu folgen.
    pub(super) const DIR: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    /// Datei lesen, ohne einem Symlink zu folgen.
    pub(super) const FILE: OFlags = OFlags::RDONLY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    /// Aufgelöster Pfad: gehaltenes Verzeichnis, sein Pfad unter der Wurzel und der letzte Name
    /// darin (`None`: das Verzeichnis selbst). `missing`: Komponenten hinter einem fehlenden
    /// Verzeichnis (nur ohne Anlegen).
    pub(super) struct Found {
        pub dir: OwnedFd,
        pub dir_path: PathBuf,
        pub name: Option<OsString>,
        pub missing: Vec<OsString>,
    }

    impl Found {
        /// Pfad unter der Wurzel (für Antworten und Prüfungen).
        pub fn path(&self) -> PathBuf {
            let mut p = self.dir_path.clone();
            p.extend(&self.name);
            p.extend(&self.missing);
            p
        }
    }

    pub(super) fn io(e: Errno) -> PathError {
        PathError::Io(std::io::Error::from(e).kind().to_string())
    }

    fn is_git(name: &std::ffi::OsStr) -> bool {
        name.as_bytes().eq_ignore_ascii_case(b".git")
    }

    /// Komponenten eines Symlink-Ziels; absolute Ziele nur innerhalb der Wurzel (auch über
    /// einen anderen Pfad zu ihr, etwa `/tmp` und `/private/tmp`).
    fn link_parts(root: &Path, target: &Path) -> Option<(bool, Vec<OsString>)> {
        let rel = if target.is_absolute() {
            if let Ok(r) = target.strip_prefix(root) {
                r.to_path_buf()
            } else {
                // Längster vorhandener Vorfahr kanonisch; nur zum Übersetzen, verfolgt wird
                // danach wieder Schritt für Schritt ab der Wurzel.
                let mut probe = target.to_path_buf();
                let mut tail = Vec::new();
                loop {
                    if let Ok(c) = probe.canonicalize() {
                        let mut r = c.strip_prefix(root).ok()?.to_path_buf();
                        r.extend(tail.iter().rev());
                        break r;
                    }
                    tail.push(probe.file_name()?.to_os_string());
                    if !probe.pop() {
                        return None;
                    }
                }
            }
        } else {
            target.to_path_buf()
        };
        let parts = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_os_string()),
                Component::ParentDir => Some(OsString::from("..")),
                _ => None,
            })
            .collect();
        Some((target.is_absolute(), parts))
    }

    /// Löst `parts` unter `root` auf; mit `create` werden fehlende Verzeichnisse angelegt.
    pub(super) fn walk(
        root: &Path,
        raw: &str,
        parts: &[String],
        create: bool,
    ) -> Result<Found, PathError> {
        let forbidden = || PathError::Forbidden(raw.to_owned());
        let root_fd = rustix::fs::openat(CWD, root, DIR, Mode::empty()).map_err(io)?;
        let mut stack: Vec<(OwnedFd, PathBuf)> = vec![(root_fd, root.to_path_buf())];
        let mut pending: VecDeque<OsString> = parts.iter().map(OsString::from).collect();
        let (mut links, mut steps) = (0, 0);
        while let Some(name) = pending.pop_front() {
            steps += 1;
            if steps > MAX_STEPS {
                return Err(forbidden());
            }
            match name.as_bytes() {
                b"" | b"." => continue,
                b".." => {
                    // Nur aus Symlink-Zielen (Eingaben mit `..` lehnt `components` ab).
                    if stack.len() == 1 {
                        return Err(forbidden());
                    }
                    stack.pop();
                    continue;
                }
                _ => {}
            }
            if is_git(&name) {
                return Err(forbidden());
            }
            let Some((dir, _)) = stack.last() else {
                return Err(forbidden());
            };
            let last = pending.is_empty();
            match rustix::fs::statat(dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) if FileType::from_raw_mode(st.st_mode as _) == FileType::Symlink => {
                    links += 1;
                    if links > MAX_LINKS {
                        return Err(forbidden());
                    }
                    let target = rustix::fs::readlinkat(dir, name.as_os_str(), Vec::new())
                        .map_err(|_| forbidden())?;
                    let target = PathBuf::from(OsString::from_vec(target.into_bytes()));
                    let (absolute, rest) = link_parts(root, &target).ok_or_else(forbidden)?;
                    if absolute {
                        stack.truncate(1);
                    }
                    for c in rest.into_iter().rev() {
                        pending.push_front(c);
                    }
                }
                Ok(st)
                    if last || FileType::from_raw_mode(st.st_mode as _) != FileType::Directory =>
                {
                    if !last {
                        return Err(PathError::NotFound(raw.to_owned()));
                    }
                    let (dir, dir_path) = stack.pop().ok_or_else(forbidden)?;
                    return Ok(Found {
                        dir,
                        dir_path,
                        name: Some(name),
                        missing: Vec::new(),
                    });
                }
                Ok(_) => match rustix::fs::openat(dir, name.as_os_str(), DIR, Mode::empty()) {
                    Ok(fd) => {
                        let path = stack
                            .last()
                            .map(|(_, p)| p.join(&name))
                            .ok_or_else(forbidden)?;
                        stack.push((fd, path));
                    }
                    // Inzwischen ausgetauscht (z. B. gegen einen Symlink): neu ansehen.
                    Err(Errno::LOOP | Errno::NOTDIR | Errno::NOENT) => pending.push_front(name),
                    Err(e) => return Err(io(e)),
                },
                Err(Errno::NOENT) if last => {
                    let (dir, dir_path) = stack.pop().ok_or_else(forbidden)?;
                    return Ok(Found {
                        dir,
                        dir_path,
                        name: Some(name),
                        missing: Vec::new(),
                    });
                }
                Err(Errno::NOENT) if create => {
                    match rustix::fs::mkdirat(dir, name.as_os_str(), Mode::from_raw_mode(0o777)) {
                        Ok(()) | Err(Errno::EXIST) => pending.push_front(name),
                        Err(e) => return Err(io(e)),
                    }
                }
                Err(Errno::NOENT) => {
                    let mut missing = vec![name];
                    missing.extend(pending.drain(..));
                    if missing.iter().any(|m| m.as_bytes() == b".." || is_git(m)) {
                        return Err(forbidden());
                    }
                    let (dir, dir_path) = stack.pop().ok_or_else(forbidden)?;
                    let name = Some(missing.remove(0));
                    return Ok(Found {
                        dir,
                        dir_path,
                        name,
                        missing,
                    });
                }
                Err(e) => return Err(io(e)),
            }
        }
        let (dir, dir_path) = stack.pop().ok_or_else(forbidden)?;
        Ok(Found {
            dir,
            dir_path,
            name: None,
            missing: Vec::new(),
        })
    }

    /// Öffnet den letzten Namen als Datei (ohne Symlink); `Ok(None)`, wenn es ihn nicht gibt.
    pub(super) fn open_file(
        found: &Found,
        raw: &str,
    ) -> Result<Option<(std::fs::File, u64)>, PathError> {
        let not_file = || PathError::Invalid(raw.to_owned(), "ist keine Datei".into());
        if !found.missing.is_empty() {
            return Ok(None);
        }
        let Some(name) = &found.name else {
            return Err(not_file());
        };
        let fd = match rustix::fs::openat(&found.dir, name.as_os_str(), FILE, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            // Zwischen Prüfung und Öffnen gegen einen Symlink getauscht: fail closed.
            Err(Errno::LOOP) => return Err(PathError::Forbidden(raw.to_owned())),
            Err(e) => return Err(io(e)),
        };
        let st = rustix::fs::fstat(&fd).map_err(io)?;
        if FileType::from_raw_mode(st.st_mode as _) != FileType::RegularFile {
            return Err(not_file());
        }
        Ok(Some((std::fs::File::from(fd), st.st_size as u64)))
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

#[cfg(unix)]
impl Workspace {
    /// Öffnet eine Datei race-frei (#92); liefert sie samt Größe und Pfad unter der Wurzel.
    pub fn open_file(&self, raw: &str) -> Result<(std::fs::File, u64, PathBuf), PathError> {
        let f = self.found(raw, false)?;
        let (file, size) =
            beneath::open_file(&f, raw)?.ok_or_else(|| PathError::NotFound(raw.to_owned()))?;
        Ok((file, size, f.path()))
    }

    /// Liest eine Datei vollständig.
    pub fn read(&self, raw: &str) -> Result<FileContent, PathError> {
        use std::io::Read as _;
        let (mut file, _, path) = self.open_file(raw)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| io(&e))?;
        Ok(FileContent {
            sha256: sha256_hex(&bytes),
            path,
            bytes,
        })
    }

    /// Schreibt eine Datei atomar (temporäre Datei + `renameat` im gehaltenen Verzeichnis). Mit
    /// `If-Match` nur, wenn der aktuelle Inhalt dazu passt; sonst [`PathError::Stale`] und keine
    /// Änderung.
    pub fn write(
        &self,
        raw: &str,
        bytes: &[u8],
        if_match: Option<&str>,
    ) -> Result<Written, PathError> {
        use rustix::fs::{AtFlags, Mode, OFlags};
        use std::io::{Read as _, Write as _};
        if components(raw)?.is_empty() {
            return Err(PathError::Invalid(raw.to_owned(), "Dateipfad fehlt".into()));
        }
        let found = self.found(raw, false)?;
        let current = match beneath::open_file(&found, raw) {
            Ok(Some((mut file, _))) => {
                let mut b = Vec::new();
                file.read_to_end(&mut b).map_err(|e| io(&e))?;
                Some(sha256_hex(&b))
            }
            Ok(None) => None,
            Err(PathError::Invalid(..)) => {
                return Err(PathError::Invalid(
                    raw.to_owned(),
                    "ist ein Verzeichnis".into(),
                ));
            }
            Err(e) => return Err(e),
        };
        if let Some(header) = if_match
            && !if_match_ok(header, current.as_deref())
        {
            return Err(PathError::Stale(raw.to_owned()));
        }
        let found = if found.missing.is_empty() {
            found
        } else {
            self.found(raw, true)?
        };
        let name = found
            .name
            .clone()
            .ok_or_else(|| PathError::Invalid(raw.to_owned(), "ist ein Verzeichnis".into()))?;
        let tmp = format!(
            ".{}.beton-{}",
            name.to_string_lossy(),
            &sha256_hex(
                format!("{}{:?}", std::process::id(), std::time::SystemTime::now()).as_bytes()
            )[..8]
        );
        let fd = rustix::fs::openat(
            &found.dir,
            tmp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o666),
        )
        .map_err(beneath::io)?;
        let cleanup = |e: PathError| {
            let _ = rustix::fs::unlinkat(&found.dir, tmp.as_str(), AtFlags::empty());
            e
        };
        if let Ok(st) = rustix::fs::statat(&found.dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)
        {
            let _ = rustix::fs::fchmod(&fd, Mode::from_raw_mode((st.st_mode as u32 & 0o7777) as _));
        }
        let mut file = std::fs::File::from(fd);
        file.write_all(bytes).map_err(|e| cleanup(io(&e)))?;
        drop(file);
        rustix::fs::renameat(&found.dir, tmp.as_str(), &found.dir, name.as_os_str())
            .map_err(|e| cleanup(beneath::io(e)))?;
        Ok(Written {
            created: current.is_none(),
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
        })
    }

    /// Einträge eines Verzeichnisses (eine Ebene), nach Namen sortiert, ohne `.git`.
    pub fn list(&self, raw: &str) -> Result<Vec<Entry>, PathError> {
        use rustix::fs::{AtFlags, FileType, Mode};
        let not_dir = || PathError::Invalid(raw.to_owned(), "ist kein Verzeichnis".into());
        let f = self.found(raw, false)?;
        if !f.missing.is_empty() {
            return Err(PathError::NotFound(raw.to_owned()));
        }
        let (dir, dir_path) = match &f.name {
            None => (f.dir, f.dir_path),
            Some(name) => {
                match rustix::fs::openat(&f.dir, name.as_os_str(), beneath::DIR, Mode::empty()) {
                    Ok(fd) => (fd, f.dir_path.join(name)),
                    Err(rustix::io::Errno::NOENT) => {
                        return Err(PathError::NotFound(raw.to_owned()));
                    }
                    Err(rustix::io::Errno::NOTDIR) => return Err(not_dir()),
                    Err(rustix::io::Errno::LOOP) => {
                        return Err(PathError::Forbidden(raw.to_owned()));
                    }
                    Err(e) => return Err(beneath::io(e)),
                }
            }
        };
        let mut out = Vec::new();
        for entry in rustix::fs::Dir::read_from(&dir).map_err(beneath::io)? {
            let entry = entry.map_err(beneath::io)?;
            let name = String::from_utf8_lossy(entry.file_name().to_bytes()).into_owned();
            if name == "." || name == ".." || name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let Ok(st) = rustix::fs::statat(&dir, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
            else {
                continue;
            };
            let kind = match FileType::from_raw_mode(st.st_mode as _) {
                FileType::Symlink => EntryKind::Symlink,
                FileType::Directory => EntryKind::Dir,
                FileType::RegularFile => EntryKind::File,
                _ => EntryKind::Other,
            };
            out.push(Entry {
                path: relative(&self.root, &dir_path.join(&name)),
                name,
                kind,
                size: (kind == EntryKind::File).then_some(st.st_size as u64),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(not(unix))]
impl Workspace {
    /// Öffnet eine Datei (pfadbasiert); liefert sie samt Größe und kanonischem Pfad.
    pub fn open_file(&self, raw: &str) -> Result<(std::fs::File, u64, PathBuf), PathError> {
        let path = self.resolve_existing(raw)?;
        if !path.is_file() {
            return Err(PathError::Invalid(raw.to_owned(), "ist keine Datei".into()));
        }
        let file = std::fs::File::open(&path).map_err(|e| io(&e))?;
        let size = file.metadata().map_err(|e| io(&e))?.len();
        Ok((file, size, path))
    }

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
}

impl Workspace {
    /// Suche mit ripgrep-Semantik: `.gitignore` (in Git-Repositories), versteckte Dateien und
    /// Binärdateien werden übersprungen, Symlinks nicht verfolgt. Smart-Case: case-insensitiv,
    /// solange die Anfrage keine Großbuchstaben enthält. Liefert höchstens `limit` Treffer
    /// nach `after` (Pfad, Zeile) und ob weitere folgen. Der Verzeichnis-Durchlauf ist
    /// pfadbasiert; jeder Treffer wird deshalb erneut race-frei aufgelöst und Inhalte werden nur
    /// darüber gelesen (#92), damit ein gegen einen Symlink getauschtes Verzeichnis nichts von
    /// draußen liefert.
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
                    if pattern.is_match(&rel)
                        && after_key(&rel, 0)
                        && self.resolve_existing(&rel).is_ok()
                    {
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
                    let Ok((file, size, _)) = self.open_file(&rel) else {
                        continue;
                    };
                    if size > SEARCH_MAX_FILE {
                        continue;
                    }
                    let mut bytes = Vec::new();
                    if std::io::Read::read_to_end(
                        &mut std::io::Read::take(file, SEARCH_MAX_FILE),
                        &mut bytes,
                    )
                    .is_err()
                    {
                        continue;
                    }
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
    #[expect(
        clippy::disallowed_methods,
        reason = "Testaufbau: git initialisiert das Repo"
    )]
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

    /// #92: Ersetzt ein gleichzeitig laufender Prozess (der Agent im Workspace) ein
    /// Zwischenverzeichnis immer wieder atomar durch einen Symlink nach draußen, schreibt und
    /// liest die Workspace-API trotzdem nie außerhalb des Workspace (auch keine temporäre Datei)
    /// und listet nichts von draußen. Ohne Austausch gelingt dasselbe im Workspace.
    #[cfg(unix)]
    #[test]
    fn ses_017_swapping_a_directory_for_a_symlink_never_leaks_outside() {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let (_tmp, ws, outside) = ws();
        let root = ws.root().to_path_buf();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/geheim.txt"), "innen").unwrap();
        std::fs::write(outside.join("nur-draussen.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("a.lnk")).unwrap();
        let outside_names = || {
            let mut v: Vec<String> = std::fs::read_dir(&outside)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        let before = outside_names();
        // Kontrollgruppe ohne Austausch: alles im Workspace.
        ws.write("a/neu.txt", b"x", None).unwrap();
        assert_eq!(ws.read("a/geheim.txt").unwrap().bytes, b"innen");

        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let (stop, root) = (stop.clone(), root.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = renameat_with(
                        CWD,
                        root.join("a"),
                        CWD,
                        root.join("a.lnk"),
                        RenameFlags::EXCHANGE,
                    );
                }
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut leaked = None;
        while std::time::Instant::now() < deadline && leaked.is_none() {
            let _ = ws.write("a/neu.txt", b"BOESE", None);
            if let Ok(c) = ws.read("a/geheim.txt")
                && c.bytes != b"innen"
            {
                leaked = Some("gelesen".to_owned());
            }
            if let Ok(entries) = ws.list("a")
                && entries.iter().any(|e| e.name == "nur-draussen.txt")
            {
                leaked = Some("gelistet".to_owned());
            }
            if outside_names() != before {
                leaked = Some(format!("geschrieben: {:?}", outside_names()));
            }
        }
        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();
        assert_eq!(leaked, None);
        assert_eq!(outside_names(), before);
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
