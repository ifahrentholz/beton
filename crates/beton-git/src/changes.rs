//! Änderungslisten und zeilengenau adressierbare Diffs (SES-018).
//!
//! Drei Sichten: `uncommitted` (Arbeitsverzeichnis gegenüber `HEAD`, inklusive unversionierter
//! Dateien), `branch` (`git diff <merge-base>...HEAD`) und Turn-Sichten zwischen zwei Bäumen
//! des Schatten-Repositorys ([`crate::snapshot`]). Pfade sind relativ zum Workspace.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::cmd::{Git, GitError, split_nul};

/// Art einer Änderung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Eine geänderte Datei mit Zeilenzahlen; `None` bei Binärdateien.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// Art einer Diff-Zeile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Context,
    Add,
    Delete,
}

/// Eine Zeile eines Hunks mit ihren Zeilennummern auf der alten bzw. neuen Seite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
    /// Die Zeile endet ohne Zeilenumbruch (`\ No newline at end of file`).
    pub no_newline: bool,
}

/// Ein Hunk (`@@ -old_start,old_lines +new_start,new_lines @@`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

/// Diff einer Datei: Unified Diff als Text und zeilengenau zerlegt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub binary: bool,
    /// Unified Diff, wie `git diff` ihn ausgibt.
    pub patch: String,
    pub hunks: Vec<Hunk>,
}

/// Ist `dir` Teil eines Git-Worktrees?
pub fn is_repo(dir: &Path) -> bool {
    Git::new(dir)
        .text(&["rev-parse", "--is-inside-work-tree"])
        .is_ok_and(|s| s == "true")
}

/// ID des leeren Baums im Objektformat des Repositorys (SHA-1 oder SHA-256).
pub fn empty_tree(git: &Git) -> Result<String, GitError> {
    let out = git.run_with_input(&["hash-object", "-t", "tree", "--stdin"], Some(b""))?;
    if out.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(GitError::Failed {
            args: "hash-object -t tree --stdin".into(),
            code: out.code,
            stderr: out.stderr,
        })
    }
}

/// `HEAD` als Commit-ID; ohne Commit der leere Baum.
pub fn head_or_empty(git: &Git) -> Result<String, GitError> {
    match git.text(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) {
        Ok(sha) if !sha.is_empty() => Ok(sha),
        _ => empty_tree(git),
    }
}

fn status_from_letter(letter: char) -> ChangeStatus {
    match letter {
        'A' => ChangeStatus::Added,
        'D' => ChangeStatus::Deleted,
        'R' | 'C' => ChangeStatus::Renamed,
        _ => ChangeStatus::Modified,
    }
}

/// Liest `--name-status -z` und `--numstat -z` desselben Diffs zu einer Liste zusammen.
fn collect(git: &Git, diff_args: &[&str]) -> Result<Vec<ChangedFile>, GitError> {
    let mut status_args = diff_args.to_vec();
    status_args.extend(["--name-status", "-z"]);
    let fields = split_nul(&git.ok(&status_args)?);
    let mut files: BTreeMap<String, ChangedFile> = BTreeMap::new();
    let mut i = 0;
    while i < fields.len() {
        let code = &fields[i];
        let letter = code.chars().next().unwrap_or('M');
        let status = status_from_letter(letter);
        if matches!(letter, 'R' | 'C') {
            let old = fields.get(i + 1).cloned().unwrap_or_default();
            let new = fields.get(i + 2).cloned().unwrap_or_default();
            files.insert(
                new.clone(),
                ChangedFile {
                    path: new,
                    old_path: Some(old),
                    status,
                    additions: None,
                    deletions: None,
                },
            );
            i += 3;
        } else {
            let path = fields.get(i + 1).cloned().unwrap_or_default();
            files.insert(
                path.clone(),
                ChangedFile {
                    path,
                    old_path: None,
                    status,
                    additions: None,
                    deletions: None,
                },
            );
            i += 2;
        }
    }

    let mut numstat_args = diff_args.to_vec();
    numstat_args.extend(["--numstat", "-z"]);
    let fields = split_nul(&git.ok(&numstat_args)?);
    let mut i = 0;
    while i < fields.len() {
        // `<add>\t<del>\t<path>` bzw. bei Umbenennung `<add>\t<del>\t` + alt + neu.
        let mut parts = fields[i].splitn(3, '\t');
        let add = parts.next().and_then(|s| s.parse().ok());
        let del = parts.next().and_then(|s| s.parse().ok());
        let rest = parts.next().unwrap_or_default();
        let path = if rest.is_empty() {
            let new = fields.get(i + 2).cloned().unwrap_or_default();
            i += 3;
            new
        } else {
            i += 1;
            rest.to_owned()
        };
        if let Some(f) = files.get_mut(&path) {
            f.additions = add;
            f.deletions = del;
        }
    }
    Ok(files.into_values().collect())
}

/// Zeilen einer Datei im Arbeitsverzeichnis; `None`, wenn sie binär aussieht.
fn count_lines(path: &Path) -> Option<u32> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return None;
    }
    let mut n = bytes.iter().filter(|b| **b == b'\n').count();
    if bytes.last().is_some_and(|b| *b != b'\n') {
        n += 1;
    }
    u32::try_from(n).ok()
}

/// Unversionierte Dateien (ohne ignorierte), relativ zu `dir`.
fn untracked(git: &Git) -> Result<Vec<String>, GitError> {
    Ok(split_nul(&git.ok(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
    ])?))
}

/// Sicht `uncommitted`: Arbeitsverzeichnis gegenüber `HEAD`, inklusive unversionierter Dateien.
/// Liefert die Basis (`HEAD` bzw. leerer Baum) und die Liste.
pub fn uncommitted(dir: &Path) -> Result<(String, Vec<ChangedFile>), GitError> {
    let git = Git::new(dir);
    let base = head_or_empty(&git)?;
    let mut files = collect(&git, &["diff", "--relative", "-M", &base])?;
    for path in untracked(&git)? {
        files.push(ChangedFile {
            additions: count_lines(&dir.join(&path)),
            deletions: count_lines(&dir.join(&path)).map(|_| 0),
            path,
            old_path: None,
            status: ChangeStatus::Added,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((base, files))
}

/// Merge-Base von `base` und `HEAD` sowie `HEAD` selbst.
pub fn merge_base(dir: &Path, base: &str) -> Result<(String, String), GitError> {
    let git = Git::new(dir);
    let head = git.text(&["rev-parse", "--verify", "HEAD^{commit}"])?;
    let mb = git.text(&["merge-base", base, "HEAD"])?;
    Ok((mb, head))
}

/// Sicht `branch`: entspricht `git diff <base>...HEAD`.
pub fn branch(dir: &Path, base: &str) -> Result<(String, String, Vec<ChangedFile>), GitError> {
    let (mb, head) = merge_base(dir, base)?;
    let files = collect(&Git::new(dir), &["diff", "--relative", "-M", &mb, &head])?;
    Ok((mb, head, files))
}

/// Änderungen zwischen zwei Bäumen bzw. Commits (`git diff <a> <b>`).
pub fn between(git: &Git, a: &str, b: &str) -> Result<Vec<ChangedFile>, GitError> {
    collect(git, &["diff", "-M", a, b])
}

fn parse_range(s: &str) -> (u32, u32) {
    let mut it = s.splitn(2, ',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let len = it.next().map_or(1, |v| v.parse().unwrap_or(0));
    (start, len)
}

/// Zerlegt einen Hunk-Kopf `@@ -a,b +c,d @@ …`.
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let mut ranges = rest[..end].split(' ');
    let old = ranges.next()?.strip_prefix('-')?;
    let new = ranges.next()?.strip_prefix('+')?;
    let (os, ol) = parse_range(old);
    let (ns, nl) = parse_range(new);
    Some((os, ol, ns, nl))
}

/// Zerlegt die Ausgabe von `git diff` (eine oder mehrere Dateien) in [`FileDiff`]s.
pub fn parse_patch(text: &str) -> Vec<FileDiff> {
    let mut out: Vec<FileDiff> = Vec::new();
    let mut old_no = 0;
    let mut new_no = 0;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.starts_with("diff --git ") {
            out.push(FileDiff {
                path: String::new(),
                old_path: None,
                status: ChangeStatus::Modified,
                binary: false,
                patch: String::new(),
                hunks: Vec::new(),
            });
        }
        let Some(file) = out.last_mut() else {
            continue;
        };
        file.patch.push_str(line);
        if let Some(hunk) = file.hunks.last_mut() {
            match body.as_bytes().first() {
                Some(b' ') => {
                    old_no += 1;
                    new_no += 1;
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Context,
                        old_line: Some(old_no),
                        new_line: Some(new_no),
                        text: body[1..].to_owned(),
                        no_newline: false,
                    });
                    continue;
                }
                Some(b'+') if !body.starts_with("+++ ") || hunk_open(hunk) => {
                    new_no += 1;
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Add,
                        old_line: None,
                        new_line: Some(new_no),
                        text: body[1..].to_owned(),
                        no_newline: false,
                    });
                    continue;
                }
                Some(b'-') if !body.starts_with("--- ") || hunk_open(hunk) => {
                    old_no += 1;
                    hunk.lines.push(DiffLine {
                        kind: LineKind::Delete,
                        old_line: Some(old_no),
                        new_line: None,
                        text: body[1..].to_owned(),
                        no_newline: false,
                    });
                    continue;
                }
                Some(b'\\') => {
                    if let Some(last) = hunk.lines.last_mut() {
                        last.no_newline = true;
                    }
                    continue;
                }
                _ => {}
            }
        }
        if let Some((os, ol, ns, nl)) = body
            .starts_with("@@ ")
            .then(|| parse_hunk_header(body))
            .flatten()
        {
            old_no = os.saturating_sub(1);
            new_no = ns.saturating_sub(1);
            file.hunks.push(Hunk {
                header: body.to_owned(),
                old_start: os,
                old_lines: ol,
                new_start: ns,
                new_lines: nl,
                lines: Vec::new(),
            });
        } else if let Some(p) = body.strip_prefix("--- ") {
            if let Some(p) = p.strip_prefix("a/") {
                file.old_path = Some(p.to_owned());
            }
        } else if let Some(p) = body.strip_prefix("+++ ") {
            if let Some(p) = p.strip_prefix("b/") {
                file.path = p.to_owned();
            }
        } else if let Some(p) = body.strip_prefix("rename from ") {
            file.old_path = Some(p.to_owned());
            file.status = ChangeStatus::Renamed;
        } else if let Some(p) = body.strip_prefix("rename to ") {
            file.path = p.to_owned();
            file.status = ChangeStatus::Renamed;
        } else if body.starts_with("new file mode") {
            file.status = ChangeStatus::Added;
        } else if body.starts_with("deleted file mode") {
            file.status = ChangeStatus::Deleted;
        } else if body.starts_with("Binary files ") || body == "GIT binary patch" {
            file.binary = true;
        }
    }
    for file in &mut out {
        match file.status {
            ChangeStatus::Added => file.old_path = None,
            ChangeStatus::Deleted => {
                if file.path.is_empty() {
                    file.path = file.old_path.clone().unwrap_or_default();
                }
                file.old_path = None;
            }
            ChangeStatus::Modified => {
                if file.path.is_empty() {
                    file.path = file.old_path.clone().unwrap_or_default();
                }
                file.old_path = None;
            }
            ChangeStatus::Renamed => {}
        }
        if file.path.is_empty() {
            // Binär ohne `+++`-Zeile: Pfad aus `diff --git a/x b/x`.
            if let Some(header) = file.patch.lines().next()
                && let Some((_, b)) = header.rsplit_once(" b/")
            {
                file.path = b.to_owned();
            }
        }
    }
    out
}

/// Ist der Hunk noch nicht vollständig (Zeilen `+++ x` / `--- x` sind dann Inhalt)?
fn hunk_open(hunk: &Hunk) -> bool {
    let (mut old, mut new) = (0, 0);
    for l in &hunk.lines {
        match l.kind {
            LineKind::Context => {
                old += 1;
                new += 1;
            }
            LineKind::Add => new += 1,
            LineKind::Delete => old += 1,
        }
    }
    old < hunk.old_lines || new < hunk.new_lines
}

fn single(text: &[u8]) -> Option<FileDiff> {
    parse_patch(&String::from_utf8_lossy(text))
        .into_iter()
        .next()
}

/// Diff einer Datei in der Sicht `uncommitted`; `None`, wenn sie unverändert ist.
pub fn diff_uncommitted(dir: &Path, path: &str) -> Result<Option<FileDiff>, GitError> {
    let git = Git::new(dir).with_config("core.quotePath=false");
    let base = head_or_empty(&git)?;
    if let Some(d) = diff_one(&git, &[&base], path, None)? {
        return Ok(Some(d));
    }
    if untracked(&git)?.iter().any(|p| p == path) {
        // Unversioniert: gegen eine leere Datei.
        let out = git.run(&[
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--",
            "/dev/null",
            path,
        ])?;
        if out.code == Some(0) || out.code == Some(1) {
            return Ok(single(&out.stdout).map(|mut d| {
                d.path = path.to_owned();
                d.old_path = None;
                d.status = ChangeStatus::Added;
                d
            }));
        }
        return Err(GitError::Failed {
            args: format!("diff --no-index /dev/null {path}"),
            code: out.code,
            stderr: out.stderr,
        });
    }
    Ok(None)
}

/// Diff genau einer Datei; `rename_from` nimmt den alten Pfad einer Umbenennung mit in die
/// Pfadangabe, damit `-M` sie erkennt.
fn diff_one(
    git: &Git,
    range: &[&str],
    path: &str,
    rename_from: Option<&str>,
) -> Result<Option<FileDiff>, GitError> {
    let mut args = vec!["diff", "--relative", "-M", "--no-color", "--no-ext-diff"];
    args.extend_from_slice(range);
    args.extend(["--", path]);
    if let Some(old) = rename_from {
        args.push(old);
    }
    let out = git.ok(&args)?;
    let mut files = parse_patch(&String::from_utf8_lossy(&out));
    // Bei genau einer Datei ist der angefragte Pfad maßgeblich (Quoting von Sonderzeichen).
    if files.len() == 1 {
        files[0].path = path.to_owned();
        return Ok(files.pop());
    }
    Ok(files.into_iter().find(|d| d.path == path))
}

/// Diff einer Datei zwischen zwei Bäumen bzw. Commits.
pub fn diff_between(git: &Git, a: &str, b: &str, path: &str) -> Result<Option<FileDiff>, GitError> {
    let git = git.clone().with_config("core.quotePath=false");
    let renamed_from = collect(&git, &["diff", "--relative", "-M", a, b])?
        .into_iter()
        .find(|f| f.path == path)
        .and_then(|f| f.old_path);
    diff_one(&git, &[a, b], path, renamed_from.as_deref())
}

/// Diff einer Datei in der Sicht `branch`: Merge-Base, `HEAD` und Diff.
pub fn diff_branch(
    dir: &Path,
    base: &str,
    path: &str,
) -> Result<(String, String, Option<FileDiff>), GitError> {
    let (mb, head) = merge_base(dir, base)?;
    let diff = diff_between(&Git::new(dir), &mb, &head, path)?;
    Ok((mb, head, diff))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,4 @@
 eins
-zwei
+ZWEI
+zwei-b
 drei
@@ -10,2 +11,2 @@ fn x()
 zehn
-elf
+ELF
\\ No newline at end of file
diff --git a/neu.txt b/neu.txt
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/neu.txt
@@ -0,0 +1 @@
+--- keine Kopfzeile
diff --git a/bild.png b/bild.png
index 1..2 100644
Binary files a/bild.png and b/bild.png differ
";

    #[test]
    fn ses_018_diff_lines_are_addressable_by_old_and_new_line_numbers() {
        let files = parse_patch(PATCH);
        assert_eq!(files.len(), 3);
        let a = &files[0];
        assert_eq!(a.path, "a.txt");
        assert_eq!(a.status, ChangeStatus::Modified);
        assert_eq!(a.hunks.len(), 2);
        let h = &a.hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (1, 3, 1, 4)
        );
        let kinds: Vec<_> = h
            .lines
            .iter()
            .map(|l| (l.kind, l.old_line, l.new_line))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (LineKind::Context, Some(1), Some(1)),
                (LineKind::Delete, Some(2), None),
                (LineKind::Add, None, Some(2)),
                (LineKind::Add, None, Some(3)),
                (LineKind::Context, Some(3), Some(4)),
            ]
        );
        let h2 = &a.hunks[1];
        assert_eq!(h2.lines[0].old_line, Some(10));
        assert_eq!(h2.lines[0].new_line, Some(11));
        assert!(h2.lines.last().unwrap().no_newline);

        let neu = &files[1];
        assert_eq!(neu.path, "neu.txt");
        assert_eq!(neu.status, ChangeStatus::Added);
        // Eine Inhaltszeile, die wie ein Dateikopf aussieht, bleibt Inhalt.
        assert_eq!(neu.hunks[0].lines[0].text, "--- keine Kopfzeile");
        assert_eq!(neu.hunks[0].lines[0].kind, LineKind::Add);

        assert!(files[2].binary);
        assert_eq!(files[2].path, "bild.png");
    }
}
